//! JSON data model for ratl dialogs.
//!
//! The model is deliberately declarative: a dialog is a tree of elements plus a
//! small amount of presentation configuration.  Interactive elements carry an
//! `id`; that id is used as the key in the result object printed by the binary.

use std::collections::HashSet;

use ratatui::style::{Color, Modifier, Style};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// A colour accepted by the JSON format.
///
/// Accepted forms:
/// * a name such as `"cyan"`, `"dark_gray"`, or `"reset"`
/// * a hex string such as `"#89b4fa"`
/// * an ANSI index such as `12`
/// * an RGB array such as `[137, 180, 250]`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorValue(pub Color);

impl Default for ColorValue {
    fn default() -> Self {
        Self(Color::Reset)
    }
}

impl<'de> Deserialize<'de> for ColorValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let color = match value {
            Value::String(text) => parse_color_name(&text).map_err(D::Error::custom)?,
            Value::Number(number) => {
                let index = number
                    .as_u64()
                    .filter(|index| *index <= u8::MAX as u64)
                    .ok_or_else(|| D::Error::custom("colour index must be between 0 and 255"))?;
                Color::Indexed(index as u8)
            }
            Value::Array(values) => {
                if values.len() != 3 {
                    return Err(D::Error::custom("RGB colours need exactly three values"));
                }
                let mut rgb = [0_u8; 3];
                for (slot, value) in rgb.iter_mut().zip(values) {
                    *slot = value
                        .as_u64()
                        .filter(|component| *component <= u8::MAX as u64)
                        .ok_or_else(|| {
                            D::Error::custom("RGB components must be integers from 0 to 255")
                        })? as u8;
                }
                Color::Rgb(rgb[0], rgb[1], rgb[2])
            }
            _ => {
                return Err(D::Error::custom(
                    "colour must be a name, hex string, ANSI index, or [r, g, b]",
                ));
            }
        };
        Ok(Self(color))
    }
}

fn parse_color_name(raw: &str) -> Result<Color, String> {
    let name = raw.trim();
    if let Some(hex) = name.strip_prefix('#') {
        return parse_hex_color(hex);
    }

    if let Some(inner) = name
        .strip_prefix("rgb(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let components: Vec<&str> = inner.split(',').map(str::trim).collect();
        if components.len() != 3 {
            return Err(format!("invalid RGB colour {raw:?}"));
        }
        let mut rgb = [0_u8; 3];
        for (slot, component) in rgb.iter_mut().zip(components) {
            *slot = component
                .parse::<u8>()
                .map_err(|_| format!("invalid RGB component {component:?}"))?;
        }
        return Ok(Color::Rgb(rgb[0], rgb[1], rgb[2]));
    }

    let normalized = name.to_ascii_lowercase().replace([' ', '-'], "_");
    let color = match normalized.as_str() {
        "reset" | "default" => Color::Reset,
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "gray" | "grey" => Color::Gray,
        "dark_gray" | "darkgray" | "dark_grey" | "darkgrey" => Color::DarkGray,
        "light_red" | "lightred" | "bright_red" => Color::LightRed,
        "light_green" | "lightgreen" | "bright_green" => Color::LightGreen,
        "light_yellow" | "lightyellow" | "bright_yellow" => Color::LightYellow,
        "light_blue" | "lightblue" | "bright_blue" => Color::LightBlue,
        "light_magenta" | "lightmagenta" | "bright_magenta" => Color::LightMagenta,
        "light_cyan" | "lightcyan" | "bright_cyan" => Color::LightCyan,
        "white" => Color::White,
        _ => return Err(format!("unknown colour {raw:?}")),
    };
    Ok(color)
}

fn parse_hex_color(hex: &str) -> Result<Color, String> {
    let expanded;
    let hex = match hex.len() {
        3 => {
            expanded = hex
                .chars()
                .flat_map(|character| [character, character])
                .collect::<String>();
            expanded.as_str()
        }
        6 => hex,
        _ => return Err(format!("hex colour #{hex} must have 3 or 6 digits")),
    };

    let red =
        u8::from_str_radix(&hex[0..2], 16).map_err(|_| format!("invalid hex colour #{hex}"))?;
    let green =
        u8::from_str_radix(&hex[2..4], 16).map_err(|_| format!("invalid hex colour #{hex}"))?;
    let blue =
        u8::from_str_radix(&hex[4..6], 16).map_err(|_| format!("invalid hex colour #{hex}"))?;
    Ok(Color::Rgb(red, green, blue))
}

/// Per-element overrides on top of the dialog theme.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StyleSpec {
    pub fg: Option<ColorValue>,
    pub bg: Option<ColorValue>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underlined: Option<bool>,
    pub dim: Option<bool>,
}

impl StyleSpec {
    pub fn apply(&self, mut style: Style) -> Style {
        if let Some(color) = self.fg {
            style = style.fg(color.0);
        }
        if let Some(color) = self.bg {
            style = style.bg(color.0);
        }
        if self.bold == Some(true) {
            style = style.add_modifier(Modifier::BOLD);
        }
        if self.italic == Some(true) {
            style = style.add_modifier(Modifier::ITALIC);
        }
        if self.underlined == Some(true) {
            style = style.add_modifier(Modifier::UNDERLINED);
        }
        if self.dim == Some(true) {
            style = style.add_modifier(Modifier::DIM);
        }
        style
    }
}

/// A width expressed either in terminal cells or as a percentage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    Cells(u16),
    Percent(u16),
}

impl Dimension {
    pub fn resolve(self, total: u16) -> u16 {
        match self {
            Self::Cells(cells) => cells,
            Self::Percent(percent) => {
                let percent = u32::from(percent.min(100));
                ((u32::from(total) * percent) / 100) as u16
            }
        }
    }
}

impl Default for Dimension {
    fn default() -> Self {
        Self::Percent(80)
    }
}

impl<'de> Deserialize<'de> for Dimension {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Cells(u16),
            Text(String),
        }

        match Raw::deserialize(deserializer)? {
            Raw::Cells(cells) => Ok(Self::Cells(cells)),
            Raw::Text(text) => {
                let text = text.trim();
                if let Some(percent) = text.strip_suffix('%') {
                    let percent = percent.trim().parse::<u16>().map_err(D::Error::custom)?;
                    if !(1..=100).contains(&percent) {
                        return Err(D::Error::custom("percentage width must be from 1 to 100"));
                    }
                    Ok(Self::Percent(percent))
                } else {
                    Ok(Self::Cells(text.parse::<u16>().map_err(D::Error::custom)?))
                }
            }
        }
    }
}

/// Colours and spacing used by a dialog.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Theme {
    pub foreground: ColorValue,
    pub background: ColorValue,
    pub label: ColorValue,
    pub muted: ColorValue,
    pub border: ColorValue,
    pub title: ColorValue,
    pub accent: ColorValue,
    pub focus_foreground: ColorValue,
    pub focus_background: ColorValue,
    pub input_foreground: ColorValue,
    pub input_background: ColorValue,
    pub danger: ColorValue,
    pub error: ColorValue,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            foreground: ColorValue(Color::Reset),
            background: ColorValue(Color::Reset),
            label: ColorValue(Color::Reset),
            muted: ColorValue(Color::DarkGray),
            border: ColorValue(Color::Cyan),
            title: ColorValue(Color::Cyan),
            accent: ColorValue(Color::LightCyan),
            focus_foreground: ColorValue(Color::Black),
            focus_background: ColorValue(Color::Cyan),
            input_foreground: ColorValue(Color::Reset),
            input_background: ColorValue(Color::DarkGray),
            danger: ColorValue(Color::LightRed),
            error: ColorValue(Color::Red),
        }
    }
}

impl Theme {
    pub fn base(&self) -> Style {
        Style::default().fg(self.foreground.0).bg(self.background.0)
    }

    pub fn label(&self) -> Style {
        Style::default().fg(self.label.0).bg(self.background.0)
    }

    pub fn muted(&self) -> Style {
        Style::default().fg(self.muted.0).bg(self.background.0)
    }

    pub fn focus(&self) -> Style {
        Style::default()
            .fg(self.focus_foreground.0)
            .bg(self.focus_background.0)
            .add_modifier(Modifier::BOLD)
    }

    pub fn input(&self) -> Style {
        Style::default()
            .fg(self.input_foreground.0)
            .bg(self.input_background.0)
    }

    pub fn accent(&self) -> Style {
        Style::default().fg(self.accent.0).bg(self.background.0)
    }

    pub fn border(&self) -> Style {
        Style::default().fg(self.border.0).bg(self.background.0)
    }

    pub fn title(&self) -> Style {
        Style::default()
            .fg(self.title.0)
            .bg(self.background.0)
            .add_modifier(Modifier::BOLD)
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    #[default]
    Column,
    Row,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

impl Align {
    pub fn as_ratatui(self) -> ratatui::layout::Alignment {
        match self {
            Self::Left => ratatui::layout::Alignment::Left,
            Self::Center => ratatui::layout::Alignment::Center,
            Self::Right => ratatui::layout::Alignment::Right,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BorderStyle {
    #[default]
    Rounded,
    Plain,
    Double,
    Thick,
    None,
}

impl BorderStyle {
    pub fn as_ratatui(self) -> ratatui::widgets::BorderType {
        match self {
            Self::Plain | Self::None => ratatui::widgets::BorderType::Plain,
            Self::Rounded => ratatui::widgets::BorderType::Rounded,
            Self::Double => ratatui::widgets::BorderType::Double,
            Self::Thick => ratatui::widgets::BorderType::Thick,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ButtonAction {
    #[default]
    Submit,
    Cancel,
    Reset,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ButtonKind {
    #[default]
    Normal,
    Primary,
    Danger,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ButtonSpec {
    #[serde(default)]
    pub id: Option<String>,
    pub label: String,
    #[serde(default)]
    pub action: ButtonAction,
    #[serde(default)]
    pub kind: ButtonKind,
}

fn default_buttons() -> Vec<ButtonSpec> {
    vec![
        ButtonSpec {
            id: Some("ok".to_owned()),
            label: "OK".to_owned(),
            action: ButtonAction::Submit,
            kind: ButtonKind::Primary,
        },
        ButtonSpec {
            id: Some("cancel".to_owned()),
            label: "Cancel".to_owned(),
            action: ButtonAction::Cancel,
            kind: ButtonKind::Normal,
        },
    ]
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ChoiceOptionDef {
    Plain(String),
    Detailed {
        label: String,
        #[serde(default)]
        value: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceOption {
    pub label: String,
    pub value: String,
}

impl ChoiceOptionDef {
    pub fn into_option(self) -> ChoiceOption {
        match self {
            Self::Plain(label) => ChoiceOption {
                value: label.clone(),
                label,
            },
            Self::Detailed { label, value } => ChoiceOption {
                value: value.unwrap_or_else(|| label.clone()),
                label,
            },
        }
    }
}

/// One element in a dialog tree.
///
/// The JSON discriminator is `type`; for example:
///
/// ```json
/// {"type": "checkbox", "id": "remember", "label": "Remember me"}
/// ```
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Element {
    Label {
        text: String,
        #[serde(default)]
        style: StyleSpec,
        #[serde(default)]
        align: Align,
    },
    Separator {
        #[serde(default)]
        style: StyleSpec,
    },
    Spacer {
        #[serde(default = "default_spacer")]
        height: u16,
    },
    Box {
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        direction: Direction,
        #[serde(default)]
        border: BorderStyle,
        #[serde(default = "default_padding")]
        padding: u16,
        #[serde(default)]
        gap: u16,
        #[serde(default)]
        style: StyleSpec,
        #[serde(default)]
        children: Vec<Element>,
    },
    Row {
        #[serde(default)]
        gap: u16,
        #[serde(default)]
        children: Vec<Element>,
    },
    Column {
        #[serde(default)]
        gap: u16,
        #[serde(default)]
        children: Vec<Element>,
    },
    Grid {
        #[serde(default = "default_columns")]
        columns: u16,
        #[serde(default)]
        column_gap: u16,
        #[serde(default)]
        row_gap: u16,
        #[serde(default)]
        children: Vec<Element>,
    },
    Checkbox {
        id: String,
        label: String,
        #[serde(default)]
        checked: bool,
        #[serde(default)]
        style: StyleSpec,
    },
    Radio {
        id: String,
        #[serde(default)]
        label: Option<String>,
        options: Vec<ChoiceOptionDef>,
        #[serde(default)]
        selected: usize,
        #[serde(default)]
        style: StyleSpec,
    },
    Text {
        id: String,
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        value: String,
        #[serde(default)]
        placeholder: Option<String>,
        #[serde(default)]
        password: bool,
        #[serde(default)]
        max_length: Option<usize>,
        #[serde(default)]
        style: StyleSpec,
    },
    Select {
        id: String,
        #[serde(default)]
        label: Option<String>,
        options: Vec<ChoiceOptionDef>,
        #[serde(default)]
        selected: usize,
        #[serde(default)]
        style: StyleSpec,
    },
    Slider {
        id: String,
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        min: i64,
        #[serde(default = "default_slider_max")]
        max: i64,
        #[serde(default = "default_slider_step")]
        step: u32,
        #[serde(default)]
        value: Option<i64>,
        #[serde(default)]
        style: StyleSpec,
    },
}

fn default_spacer() -> u16 {
    1
}
fn default_padding() -> u16 {
    1
}
fn default_columns() -> u16 {
    2
}
fn default_slider_max() -> i64 {
    100
}
fn default_slider_step() -> u32 {
    1
}

impl Element {
    pub fn children(&self) -> Option<&[Element]> {
        match self {
            Self::Box { children, .. }
            | Self::Row { children, .. }
            | Self::Column { children, .. }
            | Self::Grid { children, .. } => Some(children),
            _ => None,
        }
    }

    pub fn control_id(&self) -> Option<&str> {
        match self {
            Self::Checkbox { id, .. }
            | Self::Radio { id, .. }
            | Self::Text { id, .. }
            | Self::Select { id, .. }
            | Self::Slider { id, .. } => Some(id),
            _ => None,
        }
    }
}

/// Top-level dialog document.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dialog {
    #[serde(rename = "$schema", default)]
    pub schema: Option<String>,
    #[serde(default = "default_title")]
    pub title: String,
    #[serde(default)]
    pub theme: Theme,
    #[serde(default)]
    pub width: Dimension,
    #[serde(default = "default_min_width")]
    pub min_width: u16,
    #[serde(default = "default_max_width")]
    pub max_width: u16,
    #[serde(default = "default_padding")]
    pub padding: u16,
    #[serde(default)]
    pub children: Vec<Element>,
    #[serde(default = "default_buttons")]
    pub buttons: Vec<ButtonSpec>,
}

fn default_title() -> String {
    "ratl".to_owned()
}
fn default_min_width() -> u16 {
    40
}
fn default_max_width() -> u16 {
    100
}

impl Dialog {
    pub fn validate(&self) -> Result<(), String> {
        if self.min_width == 0 {
            return Err("min_width must be greater than zero".to_owned());
        }
        if self.max_width < self.min_width {
            return Err("max_width must be greater than or equal to min_width".to_owned());
        }
        if self.padding > 20 {
            return Err("padding must not be greater than 20".to_owned());
        }
        if self.children.is_empty() {
            return Err("dialog children must not be empty".to_owned());
        }

        let mut ids = HashSet::new();
        validate_elements(&self.children, &mut ids)?;

        let mut button_labels = HashSet::new();
        for button in &self.buttons {
            if button.label.trim().is_empty() {
                return Err("button labels must not be empty".to_owned());
            }
            if let Some(id) = &button.id {
                if id.trim().is_empty() {
                    return Err("button ids must not be empty".to_owned());
                }
                if !button_labels.insert(id.clone()) {
                    return Err(format!("duplicate button id {id:?}"));
                }
            }
        }
        Ok(())
    }
}

fn validate_elements(elements: &[Element], ids: &mut HashSet<String>) -> Result<(), String> {
    for element in elements {
        if let Some(id) = element.control_id() {
            if id.trim().is_empty() {
                return Err("control ids must not be empty".to_owned());
            }
            if !ids.insert(id.to_owned()) {
                return Err(format!("duplicate control id {id:?}"));
            }
        }

        match element {
            Element::Radio {
                id,
                options,
                selected,
                ..
            }
            | Element::Select {
                id,
                options,
                selected,
                ..
            } => {
                if options.is_empty() {
                    return Err(format!("control {id:?} must have at least one option"));
                }
                if *selected >= options.len() {
                    return Err(format!(
                        "control {id:?} has selected index {selected}, but only {} option(s) exist",
                        options.len()
                    ));
                }
            }
            Element::Text {
                id,
                value,
                max_length,
                ..
            } => {
                if *max_length == Some(0) {
                    return Err(format!("text control {id:?} has max_length 0"));
                }
                if let Some(max_length) = max_length
                    && value.chars().count() > *max_length
                {
                    return Err(format!(
                        "text control {id:?} has a value longer than max_length {max_length}"
                    ));
                }
            }
            Element::Slider {
                id,
                min,
                max,
                step,
                value,
                ..
            } => {
                if min > max {
                    return Err(format!("slider {id:?} has min greater than max"));
                }
                if *step == 0 {
                    return Err(format!("slider {id:?} has step 0"));
                }
                if let Some(value) = value
                    && (*value < *min || *value > *max)
                {
                    return Err(format!(
                        "slider {id:?} value {value} is outside {min}..={max}"
                    ));
                }
            }
            Element::Grid { columns: 0, .. } => {
                return Err("grid columns must be greater than zero".to_owned());
            }
            _ => {}
        }

        if let Some(children) = element.children() {
            validate_elements(children, ids)?;
        }
    }
    Ok(())
}

/// Human-readable count used by `--validate`.
pub fn control_count(elements: &[Element]) -> usize {
    elements
        .iter()
        .map(|element| {
            usize::from(element.control_id().is_some())
                + element.children().map_or(0, control_count)
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo() -> Dialog {
        serde_json::from_str(include_str!("../ratl.json")).expect("demo parses")
    }

    #[test]
    fn demo_is_valid_and_has_controls() {
        let dialog = demo();
        dialog.validate().expect("demo validates");
        assert_eq!(control_count(&dialog.children), 9);
        assert_eq!(dialog.buttons.len(), 3);
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let dialog: Dialog = serde_json::from_str(
            r#"{
                "children": [
                    {"type":"text","id":"same"},
                    {"type":"checkbox","id":"same","label":"Duplicate"}
                ]
            }"#,
        )
        .unwrap();
        let error = dialog.validate().unwrap_err();
        assert!(error.contains("duplicate control id"));
    }

    #[test]
    fn color_forms_parse() {
        let name: ColorValue = serde_json::from_str("\"dark_gray\"").unwrap();
        assert_eq!(name.0, Color::DarkGray);

        let hex: ColorValue = serde_json::from_str("\"#89b4fa\"").unwrap();
        assert_eq!(hex.0, Color::Rgb(0x89, 0xb4, 0xfa));

        let indexed: ColorValue = serde_json::from_str("12").unwrap();
        assert_eq!(indexed.0, Color::Indexed(12));

        let rgb: ColorValue = serde_json::from_str("[137, 180, 250]").unwrap();
        assert_eq!(rgb.0, Color::Rgb(137, 180, 250));
    }

    #[test]
    fn percentage_dimensions_resolve() {
        let dimension: Dimension = serde_json::from_str("\"75%\"").unwrap();
        assert_eq!(dimension.resolve(80), 60);
        let cells: Dimension = serde_json::from_str("42").unwrap();
        assert_eq!(cells.resolve(80), 42);
    }

    #[test]
    fn invalid_slider_range_is_rejected() {
        let dialog: Dialog =
            serde_json::from_str(r#"{"children":[{"type":"slider","id":"s","min":10,"max":0}]}"#)
                .unwrap();
        assert!(
            dialog
                .validate()
                .unwrap_err()
                .contains("min greater than max")
        );
    }
}
