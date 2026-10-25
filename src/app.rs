//! Runtime state, keyboard handling, and result serialization.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use image::RgbaImage;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Map, Value};

use crate::model::{ButtonAction, ChoiceOption, Dialog, Element};

/// A decoded image, kept in RGBA so the renderer can blend transparency over
/// the dialog background.
#[derive(Debug, Clone)]
pub struct LoadedImage {
    pub image: RgbaImage,
}

#[derive(Debug, Clone)]
pub enum ControlState {
    Bool(bool),
    Choice {
        options: Vec<ChoiceOption>,
        selected: usize,
    },
    Text {
        value: String,
        cursor: usize,
        max_length: Option<usize>,
    },
    Slider {
        value: i64,
        min: i64,
        max: i64,
        step: u32,
    },
}

impl ControlState {
    pub fn to_json(&self) -> Value {
        match self {
            Self::Bool(value) => Value::Bool(*value),
            Self::Choice { options, selected } => options
                .get(*selected)
                .map_or(Value::Null, |option| Value::String(option.value.clone())),
            Self::Text { value, .. } => Value::String(value.clone()),
            Self::Slider { value, .. } => Value::from(*value),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FocusTarget {
    Control(String),
    Button(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeAction {
    Submit,
    Cancel,
}

impl OutcomeAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Submit => "submit",
            Self::Cancel => "cancel",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Outcome {
    pub action: OutcomeAction,
    pub button: Option<String>,
    pub values: Map<String, Value>,
}

impl Outcome {
    pub fn to_json(&self) -> Value {
        let mut object = Map::new();
        object.insert(
            "action".to_owned(),
            Value::String(self.action.as_str().to_owned()),
        );
        if let Some(button) = &self.button {
            object.insert("button".to_owned(), Value::String(button.clone()));
        }
        object.insert("values".to_owned(), Value::Object(self.values.clone()));
        Value::Object(object)
    }
}

#[derive(Debug, Clone)]
pub enum KeyAction {
    Continue,
    Finish(Outcome),
}

pub struct App {
    pub dialog: Dialog,
    pub states: HashMap<String, ControlState>,
    pub focusables: Vec<FocusTarget>,
    pub focus: usize,
    pub scroll: u16,
    /// Decoded images keyed by the path exactly as it appears in the dialog.
    pub images: HashMap<String, Result<LoadedImage, String>>,
}

impl App {
    pub fn new(dialog: Dialog) -> Result<Self, String> {
        dialog.validate()?;

        let mut states = HashMap::new();
        let mut focusables = Vec::new();
        collect_state(&dialog.children, &mut states, &mut focusables);
        for (index, _) in dialog.buttons.iter().enumerate() {
            focusables.push(FocusTarget::Button(index));
        }
        let images = load_images(&dialog.children, dialog.base_dir.as_deref());

        Ok(Self {
            dialog,
            states,
            focusables,
            focus: 0,
            scroll: 0,
            images,
        })
    }

    pub fn focused_control(&self) -> Option<&str> {
        match self.focusables.get(self.focus) {
            Some(FocusTarget::Control(id)) => Some(id),
            _ => None,
        }
    }

    pub fn focused_button(&self) -> Option<usize> {
        match self.focusables.get(self.focus) {
            Some(FocusTarget::Button(index)) => Some(*index),
            _ => None,
        }
    }

    pub fn state(&self, id: &str) -> Option<&ControlState> {
        self.states.get(id)
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> KeyAction {
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('q'))
        {
            return KeyAction::Finish(self.finish(OutcomeAction::Cancel, None));
        }

        match key.code {
            KeyCode::Esc => {
                return KeyAction::Finish(self.finish(OutcomeAction::Cancel, None));
            }
            KeyCode::Tab => {
                self.move_focus(1);
                return KeyAction::Continue;
            }
            KeyCode::BackTab => {
                self.move_focus(-1);
                return KeyAction::Continue;
            }
            KeyCode::Enter => {
                return self.activate_focused();
            }
            KeyCode::Up => {
                self.vertical_key(-1);
                return KeyAction::Continue;
            }
            KeyCode::Down => {
                self.vertical_key(1);
                return KeyAction::Continue;
            }
            KeyCode::Left => {
                self.horizontal_key(-1);
                return KeyAction::Continue;
            }
            KeyCode::Right => {
                self.horizontal_key(1);
                return KeyAction::Continue;
            }
            KeyCode::Home => {
                self.home_end_key(true);
                return KeyAction::Continue;
            }
            KeyCode::End => {
                self.home_end_key(false);
                return KeyAction::Continue;
            }
            KeyCode::Char(' ') => {
                if let Some(action) = self.space_key() {
                    return action;
                }
                return KeyAction::Continue;
            }
            KeyCode::Backspace => {
                self.edit_text(|text| text.backspace());
                return KeyAction::Continue;
            }
            KeyCode::Delete => {
                self.edit_text(|text| text.delete());
                return KeyAction::Continue;
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.edit_text(|text| text.insert(character));
                return KeyAction::Continue;
            }
            _ => {}
        }

        KeyAction::Continue
    }

    fn activate_focused(&mut self) -> KeyAction {
        match self.focusables.get(self.focus).cloned() {
            Some(FocusTarget::Button(index)) => self.activate_button(index),
            Some(FocusTarget::Control(_)) | None => {
                KeyAction::Finish(self.finish(OutcomeAction::Submit, None))
            }
        }
    }

    fn activate_button(&mut self, index: usize) -> KeyAction {
        let Some(button) = self.dialog.buttons.get(index) else {
            return KeyAction::Continue;
        };
        let label = button.label.clone();
        let id = button.id.clone();
        match button.action {
            ButtonAction::Submit => {
                let name = id.unwrap_or(label);
                KeyAction::Finish(self.finish(OutcomeAction::Submit, Some(name)))
            }
            ButtonAction::Cancel => {
                let name = id.unwrap_or(label);
                KeyAction::Finish(self.finish(OutcomeAction::Cancel, Some(name)))
            }
            ButtonAction::Reset => {
                self.reset();
                KeyAction::Continue
            }
        }
    }

    fn finish(&mut self, action: OutcomeAction, button: Option<String>) -> Outcome {
        let mut values = Map::new();
        for target in &self.focusables {
            if let FocusTarget::Control(id) = target
                && let Some(state) = self.states.get(id)
            {
                values.insert(id.clone(), state.to_json());
            }
        }
        Outcome {
            action,
            button,
            values,
        }
    }

    fn reset(&mut self) {
        let mut states = HashMap::new();
        let mut controls = Vec::new();
        collect_state(&self.dialog.children, &mut states, &mut controls);
        self.states = states;
        self.focus = 0;
        self.scroll = 0;
    }

    fn move_focus(&mut self, delta: i32) {
        if self.focusables.is_empty() {
            return;
        }
        let length = self.focusables.len() as i32;
        let next = (self.focus as i32 + delta).rem_euclid(length);
        self.focus = next as usize;
    }

    fn vertical_key(&mut self, delta: i32) {
        let Some(target) = self.focusables.get(self.focus).cloned() else {
            return;
        };
        match target {
            FocusTarget::Control(id) => match self.states.get_mut(&id) {
                Some(ControlState::Choice { options, selected }) => {
                    *selected = cycle(*selected, delta, options.len());
                }
                Some(ControlState::Slider {
                    value,
                    min,
                    max,
                    step,
                    ..
                }) => {
                    let next = if delta < 0 {
                        value.saturating_sub(i64::from(*step))
                    } else {
                        value.saturating_add(i64::from(*step))
                    };
                    *value = next.clamp(*min, *max);
                }
                _ => self.move_focus(delta),
            },
            FocusTarget::Button(_) => self.move_focus(delta),
        }
    }

    fn horizontal_key(&mut self, delta: i32) {
        let Some(target) = self.focusables.get(self.focus).cloned() else {
            return;
        };
        match target {
            FocusTarget::Control(id) => match self.states.get_mut(&id) {
                Some(ControlState::Choice { options, selected }) => {
                    *selected = cycle(*selected, delta, options.len());
                }
                Some(ControlState::Slider {
                    value,
                    min,
                    max,
                    step,
                    ..
                }) => {
                    let next = if delta < 0 {
                        value.saturating_sub(i64::from(*step))
                    } else {
                        value.saturating_add(i64::from(*step))
                    };
                    *value = next.clamp(*min, *max);
                }
                Some(ControlState::Text { cursor, value, .. }) => {
                    if delta < 0 {
                        *cursor = cursor.saturating_sub(1);
                    } else {
                        *cursor = (*cursor + 1).min(value.chars().count());
                    }
                }
                _ => {}
            },
            FocusTarget::Button(_) => self.move_focus(delta),
        }
    }

    fn home_end_key(&mut self, home: bool) {
        let Some(id) = self.focused_control().map(str::to_owned) else {
            return;
        };
        match self.states.get_mut(&id) {
            Some(ControlState::Text { value, cursor, .. }) => {
                *cursor = if home { 0 } else { value.chars().count() };
            }
            Some(ControlState::Choice { options, selected }) => {
                *selected = if home {
                    0
                } else {
                    options.len().saturating_sub(1)
                };
            }
            Some(ControlState::Slider {
                value, min, max, ..
            }) => {
                *value = if home { *min } else { *max };
            }
            _ => {}
        }
    }

    fn space_key(&mut self) -> Option<KeyAction> {
        match self.focusables.get(self.focus).cloned() {
            Some(FocusTarget::Button(index)) => Some(self.activate_button(index)),
            Some(FocusTarget::Control(id)) => {
                if matches!(self.states.get(&id), Some(ControlState::Text { .. })) {
                    self.edit_text(|text| text.insert(' '));
                    return None;
                }
                if let Some(ControlState::Bool(value)) = self.states.get_mut(&id) {
                    *value = !*value;
                } else if let Some(ControlState::Choice { options, selected }) =
                    self.states.get_mut(&id)
                {
                    *selected = cycle(*selected, 1, options.len());
                }
                None
            }
            None => None,
        }
    }

    fn edit_text(&mut self, edit: impl FnOnce(&mut TextEdit<'_>)) {
        let Some(id) = self.focused_control().map(str::to_owned) else {
            return;
        };
        let Some(ControlState::Text {
            value,
            cursor,
            max_length,
        }) = self.states.get_mut(&id)
        else {
            return;
        };
        let mut text = TextEdit {
            value,
            cursor,
            max_length: *max_length,
        };
        edit(&mut text);
    }
}

fn cycle(current: usize, delta: i32, length: usize) -> usize {
    if length == 0 {
        return 0;
    }
    (current as i32 + delta).rem_euclid(length as i32) as usize
}

struct TextEdit<'a> {
    value: &'a mut String,
    cursor: &'a mut usize,
    max_length: Option<usize>,
}

impl TextEdit<'_> {
    fn insert(&mut self, character: char) {
        if self
            .max_length
            .is_some_and(|max| self.value.chars().count() >= max)
        {
            return;
        }
        let index = char_to_byte(self.value, *self.cursor);
        self.value.insert(index, character);
        *self.cursor += 1;
    }

    fn backspace(&mut self) {
        if *self.cursor == 0 {
            return;
        }
        let end = char_to_byte(self.value, *self.cursor);
        let start = char_to_byte(self.value, *self.cursor - 1);
        self.value.replace_range(start..end, "");
        *self.cursor -= 1;
    }

    fn delete(&mut self) {
        if *self.cursor >= self.value.chars().count() {
            return;
        }
        let start = char_to_byte(self.value, *self.cursor);
        let end = char_to_byte(self.value, *self.cursor + 1);
        self.value.replace_range(start..end, "");
    }
}

fn char_to_byte(value: &str, char_index: usize) -> usize {
    value
        .char_indices()
        .nth(char_index)
        .map_or(value.len(), |(index, _)| index)
}

fn load_images(
    elements: &[Element],
    base_dir: Option<&Path>,
) -> HashMap<String, Result<LoadedImage, String>> {
    let mut images = HashMap::new();
    collect_images(elements, base_dir, &mut images);
    images
}

fn collect_images(
    elements: &[Element],
    base_dir: Option<&Path>,
    images: &mut HashMap<String, Result<LoadedImage, String>>,
) {
    for element in elements {
        if let Element::Image { path, .. } = element {
            images
                .entry(path.clone())
                .or_insert_with(|| load_image(path, base_dir));
        }
        if let Some(children) = element.children() {
            collect_images(children, base_dir, images);
        }
    }
}

fn load_image(raw: &str, base_dir: Option<&Path>) -> Result<LoadedImage, String> {
    let path = resolve_image_path(raw, base_dir);
    let image =
        image::open(&path).map_err(|error| format!("cannot load {}: {error}", path.display()))?;
    Ok(LoadedImage {
        image: image.to_rgba8(),
    })
}

fn resolve_image_path(raw: &str, base_dir: Option<&Path>) -> PathBuf {
    let path = expand_home(raw);
    if path.is_absolute() {
        path
    } else if let Some(base_dir) = base_dir {
        base_dir.join(path)
    } else {
        path
    }
}

fn expand_home(raw: &str) -> PathBuf {
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    if raw == "~" {
        return home().unwrap_or_else(|| PathBuf::from(raw));
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return home()
            .map(|home| home.join(rest))
            .unwrap_or_else(|| PathBuf::from(raw));
    }
    if let Some(rest) = raw.strip_prefix("${HOME}/") {
        return home()
            .map(|home| home.join(rest))
            .unwrap_or_else(|| PathBuf::from(raw));
    }
    if let Some(rest) = raw.strip_prefix("$HOME/") {
        return home()
            .map(|home| home.join(rest))
            .unwrap_or_else(|| PathBuf::from(raw));
    }
    PathBuf::from(raw)
}

fn collect_state(
    elements: &[Element],
    states: &mut HashMap<String, ControlState>,
    focusables: &mut Vec<FocusTarget>,
) {
    for element in elements {
        match element {
            Element::Checkbox { id, checked, .. } => {
                states.insert(id.clone(), ControlState::Bool(*checked));
                focusables.push(FocusTarget::Control(id.clone()));
            }
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
                let options = options
                    .clone()
                    .into_iter()
                    .map(crate::model::ChoiceOptionDef::into_option)
                    .collect();
                states.insert(
                    id.clone(),
                    ControlState::Choice {
                        options,
                        selected: *selected,
                    },
                );
                focusables.push(FocusTarget::Control(id.clone()));
            }
            Element::Text {
                id,
                value,
                max_length,
                ..
            } => {
                states.insert(
                    id.clone(),
                    ControlState::Text {
                        value: value.clone(),
                        cursor: value.chars().count(),
                        max_length: *max_length,
                    },
                );
                focusables.push(FocusTarget::Control(id.clone()));
            }
            Element::Slider {
                id,
                min,
                max,
                step,
                value,
                ..
            } => {
                states.insert(
                    id.clone(),
                    ControlState::Slider {
                        value: value.unwrap_or(*min),
                        min: *min,
                        max: *max,
                        step: *step,
                    },
                );
                focusables.push(FocusTarget::Control(id.clone()));
            }
            _ => {}
        }

        if let Some(children) = element.children() {
            collect_state(children, states, focusables);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app() -> App {
        let dialog: Dialog = serde_json::from_str(
            r#"{
                "children": [
                    {"type":"text","id":"name","value":"Ada","max_length":5},
                    {"type":"checkbox","id":"loud","label":"Loud"}
                ],
                "buttons": [
                    {"label":"OK","action":"submit"},
                    {"label":"Cancel","action":"cancel"}
                ]
            }"#,
        )
        .unwrap();
        App::new(dialog).unwrap()
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn edits_text_and_enforces_max_length() {
        let mut app = test_app();
        app.handle_key(key(KeyCode::Char('!')));
        app.handle_key(key(KeyCode::Char(' ')));
        app.handle_key(key(KeyCode::Char('x')));
        assert_eq!(
            app.state("name").map(|state| state.to_json()),
            Some(Value::String("Ada! ".to_owned()))
        );
    }

    #[test]
    fn tab_reaches_and_toggles_checkbox() {
        let mut app = test_app();
        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.focused_control(), Some("loud"));
        app.handle_key(key(KeyCode::Char(' ')));
        assert_eq!(
            app.state("loud").map(ControlState::to_json),
            Some(Value::Bool(true))
        );
    }

    #[test]
    fn enter_submits_focused_control() {
        let mut app = test_app();
        let outcome = match app.handle_key(key(KeyCode::Enter)) {
            KeyAction::Finish(outcome) => outcome,
            KeyAction::Continue => panic!("expected submit"),
        };
        assert_eq!(outcome.action, OutcomeAction::Submit);
        assert_eq!(
            outcome.values.get("name"),
            Some(&Value::String("Ada".to_owned()))
        );
        assert_eq!(outcome.values.get("loud"), Some(&Value::Bool(false)));
    }

    #[test]
    fn escape_cancels() {
        let mut app = test_app();
        let outcome = match app.handle_key(key(KeyCode::Esc)) {
            KeyAction::Finish(outcome) => outcome,
            KeyAction::Continue => panic!("expected cancel"),
        };
        assert_eq!(outcome.action, OutcomeAction::Cancel);
    }

    #[test]
    fn choices_cycle_with_arrows() {
        let dialog: Dialog = serde_json::from_str(
            r#"{
                "children": [
                    {"type":"radio","id":"mode","options":["a","b","c"],"selected":1}
                ]
            }"#,
        )
        .unwrap();
        let mut app = App::new(dialog).unwrap();
        app.handle_key(key(KeyCode::Right));
        assert_eq!(
            app.state("mode").map(ControlState::to_json),
            Some(Value::String("c".to_owned()))
        );
        app.handle_key(key(KeyCode::Right));
        assert_eq!(
            app.state("mode").map(ControlState::to_json),
            Some(Value::String("a".to_owned()))
        );
    }
}
