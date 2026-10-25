//! Rendering and layout for declarative dialogs.

use std::collections::HashMap;

use image::RgbaImage;
use image::imageops::{self, FilterType};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, ControlState, LoadedImage};
use crate::model::{
    Align, BorderStyle, ButtonKind, Direction, Element, ImageFit, StyleSpec, VerticalAlign,
};

/// A select shows a scrolling window instead of the whole list. Eight options
/// fits the settings dialogs comfortably and still makes it obvious that there
/// is more than the current value.
const SELECT_VISIBLE_OPTIONS: usize = 8;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let theme = &app.dialog.theme;
    frame.buffer_mut().set_style(area, theme.base());

    let width = dialog_width(app, area.width);
    let padding = app.dialog.padding;
    let content_width = width
        .saturating_sub(2)
        .saturating_sub(padding.saturating_mul(2))
        .max(1);
    let natural_height = children_height(&app.dialog.children, content_width, Direction::Column, 0);
    let footer_height = if app.dialog.buttons.is_empty() { 0 } else { 2 };
    let desired_height = natural_height
        .saturating_add(2)
        .saturating_add(padding.saturating_mul(2))
        .saturating_add(footer_height);
    let max_height = area.height.saturating_sub(2).max(1);
    let height = desired_height
        .min(max_height)
        .max(5.min(area.height))
        .min(area.height);
    let dialog_area = center_rect(area, width, height);

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderStyle::Rounded.as_ratatui())
        .border_style(theme.border())
        .style(theme.base());
    if !app.dialog.title.is_empty() {
        block = block.title(Line::from(Span::styled(
            format!(" {} ", app.dialog.title),
            theme.title(),
        )));
    }
    let inner = block.inner(dialog_area);
    Clear.render(dialog_area, frame.buffer_mut());
    block.render(dialog_area, frame.buffer_mut());
    let inner = inner.inner(Margin {
        horizontal: padding,
        vertical: padding,
    });
    let footer_height = if app.dialog.buttons.is_empty() { 0 } else { 2 };
    let content_height = inner.height.saturating_sub(footer_height);
    let content_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: content_height,
    };
    let footer_area = Rect {
        x: inner.x,
        y: inner.y.saturating_add(content_height).saturating_add(1),
        width: inner.width,
        height: 1,
    };

    let buffer_height = natural_height.max(content_height).max(1);
    let mut content_buffer = Buffer::empty(Rect::new(0, 0, content_width, buffer_height));
    content_buffer.set_style(content_buffer.area, theme.base());

    let focused_control = app.focused_control().map(str::to_owned);
    let mut context = RenderContext {
        app,
        focused_control: focused_control.as_deref(),
        focus_rects: HashMap::new(),
    };
    render_children(
        &mut context,
        &app.dialog.children,
        Rect::new(0, 0, content_width, buffer_height),
        Direction::Column,
        0,
        &mut content_buffer,
    );

    let focus_rect = focused_control
        .as_deref()
        .and_then(|id| context.focus_rects.get(id).copied());
    let scroll = focus_rect.map_or(app.scroll, |rect| {
        let mut scroll = app.scroll;
        if rect.y < scroll {
            scroll = rect.y;
        }
        if rect.bottom() > scroll.saturating_add(content_height) {
            scroll = rect.bottom().saturating_sub(content_height);
        }
        scroll
    });
    let max_scroll = natural_height.saturating_sub(content_height);
    drop(context);
    app.scroll = scroll.min(max_scroll);

    if content_height > 0 {
        let view_height = content_height.min(content_buffer.area.height);
        let view_width = content_area.width.min(content_buffer.area.width);
        for row in 0..view_height {
            let source_y = app.scroll.saturating_add(row);
            if source_y >= content_buffer.area.height {
                break;
            }
            for column in 0..view_width {
                let cell = content_buffer[(column, source_y)].clone();
                frame.buffer_mut()[(content_area.x + column, content_area.y + row)] = cell;
            }
        }
    }

    if !app.dialog.buttons.is_empty() {
        draw_buttons(
            frame.buffer_mut(),
            footer_area,
            app,
            focused_control.is_none(),
        );
    }
}

fn dialog_width(app: &App, terminal_width: u16) -> u16 {
    let available = terminal_width.saturating_sub(2).max(1);
    let requested = app.dialog.width.resolve(available);
    requested
        .max(app.dialog.min_width)
        .min(app.dialog.max_width)
        .min(available)
        .max(10.min(available))
}

fn center_rect(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

struct RenderContext<'a> {
    app: &'a App,
    focused_control: Option<&'a str>,
    focus_rects: HashMap<String, Rect>,
}

fn render_children(
    context: &mut RenderContext<'_>,
    children: &[Element],
    area: Rect,
    direction: Direction,
    gap: u16,
    buffer: &mut Buffer,
) {
    if children.is_empty() || area.width == 0 || area.height == 0 {
        return;
    }
    match direction {
        Direction::Column => {
            let mut y = area.y;
            for child in children {
                let height = element_height(child, area.width);
                if height == 0 {
                    continue;
                }
                let remaining = area.bottom().saturating_sub(y);
                if remaining == 0 {
                    break;
                }
                let height = height.min(remaining);
                render_element(
                    context,
                    child,
                    Rect::new(area.x, y, area.width, height),
                    buffer,
                );
                y = y.saturating_add(height).saturating_add(gap);
            }
        }
        Direction::Row => {
            let widths = split_widths(area.width, children.len(), gap);
            let mut x = area.x;
            for (child, width) in children.iter().zip(widths) {
                if width == 0 {
                    continue;
                }
                render_element(
                    context,
                    child,
                    Rect::new(x, area.y, width, area.height),
                    buffer,
                );
                x = x.saturating_add(width).saturating_add(gap);
            }
        }
    }
}

fn render_element(
    context: &mut RenderContext<'_>,
    element: &Element,
    area: Rect,
    buffer: &mut Buffer,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    match element {
        Element::Label { text, style, align } => {
            let style = style.apply(context.app.dialog.theme.label());
            let paragraph = Paragraph::new(text.as_str())
                .style(style)
                .alignment(align.as_ratatui())
                .wrap(Wrap { trim: false });
            paragraph.render(area, buffer);
        }
        Element::Separator { style } => {
            let style = style.apply(context.app.dialog.theme.muted());
            buffer.set_style(area, style);
            let line = "─".repeat(area.width as usize);
            buffer.set_stringn(area.x, area.y, line, area.width as usize, style);
        }
        Element::Spacer { .. } => {}
        Element::Image {
            path,
            width,
            fit,
            align,
            valign,
            alt,
            style,
            ..
        } => {
            render_image(
                context,
                path,
                *width,
                *fit,
                *align,
                *valign,
                alt.as_deref(),
                style,
                area,
                buffer,
            );
        }
        Element::Box {
            title,
            direction,
            border,
            padding,
            gap,
            style,
            children,
        } => {
            let base = style.apply(context.app.dialog.theme.base());
            let borders = if *border == BorderStyle::None {
                Borders::NONE
            } else {
                Borders::ALL
            };
            let mut block = Block::default()
                .borders(borders)
                .border_type(border.as_ratatui())
                .border_style(style.apply(context.app.dialog.theme.border()))
                .style(base);
            if let Some(title) = title
                && !title.is_empty()
            {
                block = block.title(Line::from(Span::styled(
                    format!(" {title} "),
                    style.apply(context.app.dialog.theme.title()),
                )));
            }
            let inner = block.inner(area);
            block.render(area, buffer);
            let inner = if *border == BorderStyle::None {
                area
            } else {
                inner
            };
            let inner = inner.inner(Margin {
                horizontal: *padding,
                vertical: *padding,
            });
            render_children(context, children, inner, *direction, *gap, buffer);
        }
        Element::Row { gap, children } => {
            render_children(context, children, area, Direction::Row, *gap, buffer);
        }
        Element::Column { gap, children } => {
            render_children(context, children, area, Direction::Column, *gap, buffer);
        }
        Element::Grid {
            columns,
            column_gap,
            row_gap,
            children,
        } => {
            render_grid(
                context,
                children,
                area,
                *columns,
                *column_gap,
                *row_gap,
                buffer,
            );
        }
        Element::Checkbox {
            id, label, style, ..
        } => {
            context.focus_rects.insert(id.clone(), area);
            let checked = matches!(context.app.state(id), Some(ControlState::Bool(true)));
            let marker = if checked { "[x]" } else { "[ ]" };
            let line = format!("{marker} {label}");
            let mut style = style.apply(context.app.dialog.theme.label());
            if context.focused_control == Some(id.as_str()) {
                style = context.app.dialog.theme.focus();
            }
            buffer.set_style(area, style);
            buffer.set_stringn(area.x, area.y, line, area.width as usize, style);
        }
        Element::Radio {
            id, label, style, ..
        } => {
            context.focus_rects.insert(id.clone(), area);
            let state = context.app.state(id);
            let (current, option_labels) = match state {
                Some(ControlState::Choice { options, selected }) => (
                    *selected,
                    options.iter().map(|option| option.label.clone()).collect(),
                ),
                _ => (0, Vec::new()),
            };
            let focused = context.focused_control == Some(id.as_str());
            let base = style.apply(context.app.dialog.theme.label());
            let mut y = area.y;
            if let Some(label) = label
                && !label.is_empty()
                && y < area.bottom()
            {
                buffer.set_stringn(area.x, y, label, area.width as usize, base);
                y = y.saturating_add(1);
            }
            for (index, option) in option_labels.iter().enumerate() {
                if y >= area.bottom() {
                    break;
                }
                let marker = if index == current { "(*)" } else { "( )" };
                let line = format!("{marker} {option}");
                let line_style = if focused && index == current {
                    context.app.dialog.theme.focus()
                } else {
                    base
                };
                let line_area = Rect::new(area.x, y, area.width, 1);
                buffer.set_style(line_area, line_style);
                buffer.set_stringn(
                    line_area.x,
                    line_area.y,
                    line,
                    line_area.width as usize,
                    line_style,
                );
                y = y.saturating_add(1);
            }
        }
        Element::Text {
            id,
            label,
            placeholder,
            password,
            style,
            ..
        } => {
            context.focus_rects.insert(id.clone(), area);
            let focused = context.focused_control == Some(id.as_str());
            let base = style.apply(context.app.dialog.theme.label());
            let mut input_y = area.y;
            if let Some(label) = label
                && !label.is_empty()
                && area.height > 1
            {
                buffer.set_stringn(area.x, input_y, label, area.width as usize, base);
                input_y = input_y.saturating_add(1);
            }
            if input_y >= area.bottom() {
                return;
            }
            let input_area = Rect::new(area.x, input_y, area.width, 1);
            let mut input_style = style.apply(context.app.dialog.theme.input());
            if focused {
                input_style = context.app.dialog.theme.focus();
            }
            buffer.set_style(input_area, input_style);

            let (value, cursor) = match context.app.state(id) {
                Some(ControlState::Text { value, cursor, .. }) => (value.as_str(), *cursor),
                _ => ("", 0),
            };
            if value.is_empty() && !focused {
                if let Some(placeholder) = placeholder {
                    let placeholder_style = context.app.dialog.theme.muted();
                    buffer.set_stringn(
                        input_area.x,
                        input_area.y,
                        placeholder,
                        input_area.width as usize,
                        placeholder_style,
                    );
                }
                return;
            }

            let chars: Vec<char> = if *password {
                std::iter::repeat_n('•', value.chars().count()).collect()
            } else {
                value.chars().collect()
            };
            let cursor = cursor.min(chars.len());
            let width = input_area.width as usize;
            let start = if focused && cursor >= width {
                cursor.saturating_add(1).saturating_sub(width)
            } else {
                0
            };
            let mut display = chars;
            if focused {
                display.insert(cursor, '▏');
            }
            let display: String = display.into_iter().skip(start).take(width).collect();
            buffer.set_stringn(
                input_area.x,
                input_area.y,
                display,
                input_area.width as usize,
                input_style,
            );

            if focused {
                let before_cursor = cursor.saturating_sub(start);
                let column = input_area
                    .x
                    .saturating_add(before_cursor as u16)
                    .min(input_area.right().saturating_sub(1));
                buffer[(column, input_area.y)].set_style(
                    Style::default()
                        .fg(context.app.dialog.theme.accent.0)
                        .bg(context.app.dialog.theme.focus_background.0)
                        .add_modifier(Modifier::BOLD),
                );
            }
        }
        Element::Select {
            id,
            label,
            options,
            selected,
            style,
        } => {
            context.focus_rects.insert(id.clone(), area);
            let state = context.app.state(id);
            let (current, option_labels) = match state {
                Some(ControlState::Choice { options, selected }) => (
                    *selected,
                    options
                        .iter()
                        .map(|option| option.label.clone())
                        .collect::<Vec<_>>(),
                ),
                _ => (
                    *selected,
                    options
                        .iter()
                        .map(|option| match option {
                            crate::model::ChoiceOptionDef::Plain(value) => value.clone(),
                            crate::model::ChoiceOptionDef::Detailed { label, .. } => label.clone(),
                        })
                        .collect::<Vec<_>>(),
                ),
            };
            let focused = context.focused_control == Some(id.as_str());
            let base = style.apply(context.app.dialog.theme.label());
            let total = option_labels.len();
            let mut line_y = area.y;
            if let Some(label) = label
                && !label.is_empty()
                && area.height > 1
            {
                let heading = if total > SELECT_VISIBLE_OPTIONS {
                    format!(
                        "{label}  [{}/{}]",
                        current.saturating_add(1).min(total),
                        total
                    )
                } else {
                    label.clone()
                };
                buffer.set_stringn(area.x, line_y, heading, area.width as usize, base);
                line_y = line_y.saturating_add(1);
            }
            if total == 0 {
                if line_y < area.bottom() {
                    buffer.set_stringn(
                        area.x,
                        line_y,
                        "(no options)",
                        area.width as usize,
                        context.app.dialog.theme.muted(),
                    );
                }
                return;
            }
            let current = current.min(total - 1);
            let visible = usize::from(area.height.saturating_sub(line_y.saturating_sub(area.y)))
                .min(SELECT_VISIBLE_OPTIONS)
                .min(total);
            if visible == 0 {
                return;
            }
            let start = if total <= visible {
                0
            } else {
                current.saturating_sub(visible / 2).min(total - visible)
            };
            for index in start..start + visible {
                if line_y >= area.bottom() {
                    break;
                }
                let marker = if index == current { "(*)" } else { "( )" };
                let line = format!("{marker} {}", option_labels[index]);
                let line_style = if focused && index == current {
                    context.app.dialog.theme.focus()
                } else if index == current {
                    context.app.dialog.theme.accent()
                } else {
                    base
                };
                let line_area = Rect::new(area.x, line_y, area.width, 1);
                buffer.set_style(line_area, line_style);
                buffer.set_stringn(
                    line_area.x,
                    line_area.y,
                    line,
                    line_area.width as usize,
                    line_style,
                );
                if area.width > 0 {
                    if index == start && start > 0 {
                        buffer.set_stringn(
                            area.right().saturating_sub(1),
                            line_y,
                            "↑",
                            1,
                            context.app.dialog.theme.muted(),
                        );
                    }
                    if index == start + visible - 1 && start + visible < total {
                        buffer.set_stringn(
                            area.right().saturating_sub(1),
                            line_y,
                            "↓",
                            1,
                            context.app.dialog.theme.muted(),
                        );
                    }
                }
                line_y = line_y.saturating_add(1);
            }
        }
        Element::Slider {
            id,
            label,
            min,
            max,
            style,
            ..
        } => {
            context.focus_rects.insert(id.clone(), area);
            let value = match context.app.state(id) {
                Some(ControlState::Slider { value, .. }) => *value,
                _ => *min,
            };
            let focused = context.focused_control == Some(id.as_str());
            let base = style.apply(context.app.dialog.theme.label());
            let mut line_y = area.y;
            if let Some(label) = label
                && !label.is_empty()
                && area.height > 1
            {
                let line = format!("{label}: {value}");
                buffer.set_stringn(area.x, line_y, line, area.width as usize, base);
                line_y = line_y.saturating_add(1);
            }
            if line_y >= area.bottom() {
                return;
            }
            let span = max.saturating_sub(*min);
            let progress = if span <= 0 {
                1.0
            } else {
                (value - *min) as f64 / span as f64
            }
            .clamp(0.0, 1.0);
            let value_text = value.to_string();
            let bar_width = (area.width as usize)
                .saturating_sub(value_text.len())
                .saturating_sub(4)
                .max(1);
            let filled = ((bar_width as f64) * progress).round() as usize;
            let bar = format!(
                "[{}{}] {value_text}",
                "█".repeat(filled.min(bar_width)),
                "░".repeat(bar_width.saturating_sub(filled))
            );
            let line_area = Rect::new(area.x, line_y, area.width, 1);
            let line_style = if focused {
                context.app.dialog.theme.focus()
            } else {
                style.apply(context.app.dialog.theme.input())
            };
            buffer.set_style(line_area, line_style);
            buffer.set_stringn(
                line_area.x,
                line_area.y,
                bar,
                line_area.width as usize,
                line_style,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_image(
    context: &RenderContext<'_>,
    path: &str,
    width: Option<crate::model::Dimension>,
    fit: ImageFit,
    align: Align,
    valign: VerticalAlign,
    alt: Option<&str>,
    style: &StyleSpec,
    area: Rect,
    buffer: &mut Buffer,
) {
    let base = style.apply(context.app.dialog.theme.base());
    buffer.set_style(area, base);
    if area.width == 0 || area.height == 0 {
        return;
    }

    let requested_width = width
        .map_or(area.width, |width| width.resolve(area.width))
        .min(area.width)
        .max(1);
    let offset = match align {
        Align::Left => 0,
        Align::Center => area.width.saturating_sub(requested_width) / 2,
        Align::Right => area.width.saturating_sub(requested_width),
    };
    let image_area = Rect {
        x: area.x.saturating_add(offset),
        y: area.y,
        width: requested_width,
        height: area.height,
    };

    match context.app.images.get(path) {
        Some(Ok(loaded)) => {
            let background = color_to_rgb(base.bg.unwrap_or(Color::Reset));
            draw_loaded_image(loaded, image_area, fit, valign, background, &base, buffer);
        }
        Some(Err(error)) => {
            draw_image_error(context, image_area, alt, error, style, buffer);
        }
        None => {
            draw_image_error(
                context,
                image_area,
                alt,
                "image was not loaded",
                style,
                buffer,
            );
        }
    }
}

fn draw_loaded_image(
    loaded: &LoadedImage,
    area: Rect,
    fit: ImageFit,
    valign: VerticalAlign,
    background: [u8; 3],
    base_style: &Style,
    buffer: &mut Buffer,
) {
    let canvas_width = u32::from(area.width);
    let canvas_height = u32::from(area.height).saturating_mul(2);
    let image = &loaded.image;
    if canvas_width == 0 || canvas_height == 0 || image.width() == 0 || image.height() == 0 {
        return;
    }

    let (rendered, x_offset, y_offset) = match fit {
        ImageFit::Contain => {
            let (draw_width, draw_height) =
                contain_size(image.width(), image.height(), canvas_width, canvas_height);
            let rendered = imageops::resize(image, draw_width, draw_height, FilterType::Triangle);
            let x_offset = (canvas_width - draw_width) / 2;
            let y_offset = match valign {
                VerticalAlign::Top => 0,
                VerticalAlign::Center => (canvas_height - draw_height) / 2,
                VerticalAlign::Bottom => canvas_height - draw_height,
            };
            (rendered, x_offset, y_offset)
        }
        ImageFit::Cover => (cover_image(image, canvas_width, canvas_height), 0, 0),
        ImageFit::Stretch => (
            imageops::resize(image, canvas_width, canvas_height, FilterType::Triangle),
            0,
            0,
        ),
    };

    for row in 0..area.height {
        let top_y = u32::from(row).saturating_mul(2);
        let bottom_y = top_y.saturating_add(1);
        for column in 0..area.width {
            let x = u32::from(column);
            let top = pixel_at(&rendered, x, top_y, x_offset, y_offset);
            let bottom = pixel_at(&rendered, x, bottom_y, x_offset, y_offset);
            let cell = &mut buffer[(area.x + column, area.y + row)];
            match (top, bottom) {
                (None, None) => {
                    cell.set_symbol(" ");
                    cell.set_style(*base_style);
                }
                (Some(top), Some(bottom)) => {
                    cell.set_symbol("▀");
                    let top = blend(top, background);
                    let bottom = blend(bottom, background);
                    cell.set_fg(Color::Rgb(top[0], top[1], top[2]));
                    cell.set_bg(Color::Rgb(bottom[0], bottom[1], bottom[2]));
                }
                (Some(top), None) => {
                    cell.set_symbol("▀");
                    let top = blend(top, background);
                    cell.set_fg(Color::Rgb(top[0], top[1], top[2]));
                    cell.set_bg(Color::Rgb(background[0], background[1], background[2]));
                }
                (None, Some(bottom)) => {
                    cell.set_symbol("▄");
                    let bottom = blend(bottom, background);
                    cell.set_fg(Color::Rgb(bottom[0], bottom[1], bottom[2]));
                    cell.set_bg(Color::Rgb(background[0], background[1], background[2]));
                }
            }
        }
    }
}

fn contain_size(
    image_width: u32,
    image_height: u32,
    canvas_width: u32,
    canvas_height: u32,
) -> (u32, u32) {
    let scale =
        (canvas_width as f64 / image_width as f64).min(canvas_height as f64 / image_height as f64);
    let width = ((image_width as f64 * scale).round() as u32).clamp(1, canvas_width);
    let height = ((image_height as f64 * scale).round() as u32).clamp(1, canvas_height);
    (width, height)
}

fn cover_image(image: &RgbaImage, canvas_width: u32, canvas_height: u32) -> RgbaImage {
    let image_width = image.width();
    let image_height = image.height();
    let canvas_aspect = canvas_width as f64 / canvas_height as f64;
    let image_aspect = image_width as f64 / image_height as f64;
    let cropped = if image_aspect > canvas_aspect {
        let width = ((image_height as f64 * canvas_aspect).round() as u32).clamp(1, image_width);
        let x = (image_width - width) / 2;
        imageops::crop_imm(image, x, 0, width, image_height).to_image()
    } else {
        let height = ((image_width as f64 / canvas_aspect).round() as u32).clamp(1, image_height);
        let y = (image_height - height) / 2;
        imageops::crop_imm(image, 0, y, image_width, height).to_image()
    };
    imageops::resize(&cropped, canvas_width, canvas_height, FilterType::Triangle)
}

fn pixel_at(
    image: &RgbaImage,
    canvas_x: u32,
    canvas_y: u32,
    x_offset: u32,
    y_offset: u32,
) -> Option<[u8; 4]> {
    let x = canvas_x.checked_sub(x_offset)?;
    let y = canvas_y.checked_sub(y_offset)?;
    if x >= image.width() || y >= image.height() {
        return None;
    }
    let pixel = image.get_pixel(x, y);
    Some([pixel[0], pixel[1], pixel[2], pixel[3]])
}

fn blend(pixel: [u8; 4], background: [u8; 3]) -> [u8; 3] {
    let alpha = u32::from(pixel[3]);
    let inverse = 255_u32.saturating_sub(alpha);
    [
        ((u32::from(pixel[0]) * alpha + u32::from(background[0]) * inverse) / 255) as u8,
        ((u32::from(pixel[1]) * alpha + u32::from(background[1]) * inverse) / 255) as u8,
        ((u32::from(pixel[2]) * alpha + u32::from(background[2]) * inverse) / 255) as u8,
    ]
}

fn color_to_rgb(color: Color) -> [u8; 3] {
    match color {
        Color::Rgb(red, green, blue) => [red, green, blue],
        Color::Black => [0, 0, 0],
        Color::Red => [128, 0, 0],
        Color::Green => [0, 128, 0],
        Color::Yellow => [128, 128, 0],
        Color::Blue => [0, 0, 128],
        Color::Magenta => [128, 0, 128],
        Color::Cyan => [0, 128, 128],
        Color::Gray => [192, 192, 192],
        Color::DarkGray => [128, 128, 128],
        Color::LightRed => [255, 0, 0],
        Color::LightGreen => [0, 255, 0],
        Color::LightYellow => [255, 255, 0],
        Color::LightBlue => [0, 0, 255],
        Color::LightMagenta => [255, 0, 255],
        Color::LightCyan => [0, 255, 255],
        Color::White => [255, 255, 255],
        _ => [0, 0, 0],
    }
}

fn draw_image_error(
    context: &RenderContext<'_>,
    area: Rect,
    alt: Option<&str>,
    error: &str,
    style: &StyleSpec,
    buffer: &mut Buffer,
) {
    let message = match alt {
        Some(alt) if !alt.is_empty() => format!("{alt}\n{error}"),
        _ => format!("image: {error}"),
    };
    let paragraph = Paragraph::new(message)
        .style(style.apply(context.app.dialog.theme.muted()))
        .wrap(Wrap { trim: false });
    paragraph.render(area, buffer);
}

fn render_grid(
    context: &mut RenderContext<'_>,
    children: &[Element],
    area: Rect,
    columns: u16,
    column_gap: u16,
    row_gap: u16,
    buffer: &mut Buffer,
) {
    let columns = columns.max(1);
    let widths = split_widths(area.width, usize::from(columns), column_gap);
    let mut y = area.y;
    for row in children.chunks(usize::from(columns)) {
        let row_height = row
            .iter()
            .zip(&widths)
            .map(|(child, width)| element_height(child, *width))
            .max()
            .unwrap_or(0);
        if row_height == 0 {
            continue;
        }
        let remaining = area.bottom().saturating_sub(y);
        if remaining == 0 {
            break;
        }
        let row_height = row_height.min(remaining);
        let mut x = area.x;
        for (child, width) in row.iter().zip(&widths) {
            if *width == 0 {
                continue;
            }
            render_element(context, child, Rect::new(x, y, *width, row_height), buffer);
            x = x.saturating_add(*width).saturating_add(column_gap);
        }
        y = y.saturating_add(row_height).saturating_add(row_gap);
    }
}

fn draw_buttons(buffer: &mut Buffer, area: Rect, app: &App, has_focus: bool) {
    let labels: Vec<String> = app
        .dialog
        .buttons
        .iter()
        .map(|button| format!("[ {} ]", button.label))
        .collect();
    let widths: Vec<u16> = labels
        .iter()
        .map(|label| label.width().try_into().unwrap_or(u16::MAX))
        .collect();
    let gap = 2_u16;
    let total = widths
        .iter()
        .copied()
        .sum::<u16>()
        .saturating_add(gap.saturating_mul(widths.len().saturating_sub(1) as u16));
    let mut x = area.right().saturating_sub(total).max(area.x);
    for (index, (label, width)) in labels.into_iter().zip(widths).enumerate() {
        if width == 0 || x >= area.right() {
            break;
        }
        let width = width.min(area.right().saturating_sub(x));
        let button = &app.dialog.buttons[index];
        let focused = has_focus && app.focused_button() == Some(index);
        let mut style = match button.kind {
            ButtonKind::Normal => app.dialog.theme.base(),
            ButtonKind::Primary => app.dialog.theme.accent().add_modifier(Modifier::BOLD),
            ButtonKind::Danger => Style::default()
                .fg(app.dialog.theme.danger.0)
                .bg(app.dialog.theme.background.0)
                .add_modifier(Modifier::BOLD),
        };
        if focused {
            style = app.dialog.theme.focus();
        }
        let button_area = Rect::new(x, area.y, width, 1);
        buffer.set_style(button_area, style);
        buffer.set_stringn(x, area.y, label, width as usize, style);
        x = x.saturating_add(width).saturating_add(gap);
    }
}

fn children_height(children: &[Element], width: u16, direction: Direction, gap: u16) -> u16 {
    match direction {
        Direction::Column => children
            .iter()
            .map(|child| element_height(child, width))
            .fold(0_u16, |total, height| {
                if total == 0 {
                    height
                } else {
                    total.saturating_add(gap).saturating_add(height)
                }
            }),
        Direction::Row => {
            let widths = split_widths(width, children.len(), gap);
            children
                .iter()
                .zip(widths)
                .map(|(child, width)| element_height(child, width))
                .max()
                .unwrap_or(0)
        }
    }
}

fn element_height(element: &Element, width: u16) -> u16 {
    match element {
        Element::Label { text, .. } => wrapped_height(text, width),
        Element::Separator { .. } => 1,
        Element::Spacer { height } => *height,
        Element::Image { height, .. } => *height,
        Element::Box {
            direction,
            border,
            padding,
            gap,
            children,
            ..
        } => {
            let border_height = if *border == BorderStyle::None { 0 } else { 2 };
            let horizontal = if *border == BorderStyle::None { 0 } else { 2 };
            let inner_width = width
                .saturating_sub(horizontal)
                .saturating_sub(padding.saturating_mul(2))
                .max(1);
            let content = children_height(children, inner_width, *direction, *gap);
            content
                .saturating_add(border_height)
                .saturating_add(padding.saturating_mul(2))
        }
        Element::Row { gap, children } => children_height(children, width, Direction::Row, *gap),
        Element::Column { gap, children } => {
            children_height(children, width, Direction::Column, *gap)
        }
        Element::Grid {
            columns,
            column_gap,
            row_gap,
            children,
        } => {
            let columns = (*columns).max(1) as usize;
            let widths = split_widths(width, columns, *column_gap);
            let mut total = 0_u16;
            let mut rows = 0_u16;
            for row in children.chunks(columns) {
                let row_height = row
                    .iter()
                    .zip(&widths)
                    .map(|(child, child_width)| element_height(child, *child_width))
                    .max()
                    .unwrap_or(0);
                if row_height == 0 {
                    continue;
                }
                if rows > 0 {
                    total = total.saturating_add(*row_gap);
                }
                total = total.saturating_add(row_height);
                rows = rows.saturating_add(1);
            }
            total
        }
        Element::Checkbox { .. } => 1,
        Element::Radio { label, options, .. } => {
            u16::from(label.as_ref().is_some_and(|label| !label.is_empty()))
                .saturating_add(options.len() as u16)
        }
        Element::Text { label, .. } => 1_u16.saturating_add(u16::from(
            label.as_ref().is_some_and(|label| !label.is_empty()),
        )),
        Element::Select { label, options, .. } => {
            u16::from(label.as_ref().is_some_and(|label| !label.is_empty()))
                .saturating_add(options.len().clamp(1, SELECT_VISIBLE_OPTIONS) as u16)
        }
        Element::Slider { label, .. } => 1_u16.saturating_add(u16::from(
            label.as_ref().is_some_and(|label| !label.is_empty()),
        )),
    }
}

fn split_widths(total: u16, count: usize, gap: u16) -> Vec<u16> {
    if count == 0 {
        return Vec::new();
    }
    let gaps = gap.saturating_mul(count.saturating_sub(1) as u16);
    let available = total.saturating_sub(gaps);
    let base = available / count as u16;
    let remainder = available % count as u16;
    (0..count)
        .map(|index| base.saturating_add(u16::from(index < remainder as usize)))
        .collect()
}

fn wrapped_height(text: &str, width: u16) -> u16 {
    let width = usize::from(width.max(1));
    let mut total = 0_usize;
    for raw_line in text.split('\n') {
        if raw_line.is_empty() {
            total = total.saturating_add(1);
            continue;
        }
        let mut lines = 1_usize;
        let mut used = 0_usize;
        for word in raw_line.split_whitespace() {
            let word_width = word.width();
            if used == 0 {
                used = word_width;
            } else if used.saturating_add(1).saturating_add(word_width) <= width {
                used = used.saturating_add(1).saturating_add(word_width);
            } else {
                lines = lines.saturating_add(1);
                used = word_width;
            }
            while used > width {
                lines = lines.saturating_add(used / width);
                used %= width;
            }
        }
        total = total.saturating_add(lines);
    }
    total.max(1).min(u16::MAX as usize) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Dialog;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn demo_renders_without_panicking() {
        let dialog: Dialog = serde_json::from_str(include_str!("../ratl.json")).unwrap();
        let mut app = App::new(dialog).unwrap();
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();

        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains("ratl demo"));
        assert!(rendered.contains("Ada Lovelace"));
        assert!(rendered.contains("Save"));
        assert!(rendered.contains("Recovery"));
    }

    #[test]
    fn tiny_terminal_does_not_panic() {
        let dialog: Dialog = serde_json::from_str(include_str!("../ratl.json")).unwrap();
        let mut app = App::new(dialog).unwrap();
        let backend = TestBackend::new(20, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    }

    #[test]
    fn select_renders_multiple_options() {
        let dialog: Dialog = serde_json::from_value(serde_json::json!({
            "children": [
                {
                    "type": "select",
                    "id": "choice",
                    "label": "Pick one",
                    "options": ["one", "two", "three", "four"],
                    "selected": 2
                }
            ],
            "buttons": []
        }))
        .unwrap();
        let mut app = App::new(dialog).unwrap();
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();

        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains("Pick one"));
        assert!(rendered.contains("( ) one"));
        assert!(rendered.contains("( ) two"));
        assert!(rendered.contains("(*) three"));
        assert!(rendered.contains("( ) four"));
    }

    #[test]
    fn select_scrolls_to_the_selected_option() {
        let options: Vec<String> = (0..20).map(|index| format!("option {index}")).collect();
        let dialog: Dialog = serde_json::from_value(serde_json::json!({
            "children": [
                {
                    "type": "select",
                    "id": "choice",
                    "label": "Pick one",
                    "options": options,
                    "selected": 15
                }
            ],
            "buttons": []
        }))
        .unwrap();
        let mut app = App::new(dialog).unwrap();
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();

        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains("(*) option 15"));
        assert!(!rendered.contains("option 0"));
    }

    #[test]
    fn select_height_caps_long_lists() {
        let options: Vec<String> = (0..20).map(|index| format!("option {index}")).collect();
        let dialog: Dialog = serde_json::from_value(serde_json::json!({
            "children": [
                {
                    "type": "select",
                    "id": "choice",
                    "label": "Pick one",
                    "options": options
                }
            ],
            "buttons": []
        }))
        .unwrap();
        assert_eq!(
            element_height(&dialog.children[0], 40),
            1 + SELECT_VISIBLE_OPTIONS as u16
        );
    }

    #[test]
    fn image_element_draws_half_blocks() {
        let path = std::env::temp_dir().join(format!(
            "ratl-image-test-{}-{}.png",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut image = image::RgbaImage::new(4, 4);
        for pixel in image.pixels_mut() {
            *pixel = image::Rgba([255, 0, 0, 255]);
        }
        image
            .save_with_format(&path, image::ImageFormat::Png)
            .unwrap();

        let dialog: Dialog = serde_json::from_value(serde_json::json!({
            "children": [
                {
                    "type": "image",
                    "path": path.to_string_lossy().to_string(),
                    "height": 4
                }
            ],
            "buttons": []
        }))
        .unwrap();
        let mut app = App::new(dialog).unwrap();
        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();

        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains("▀"));
        let _ = std::fs::remove_file(path);
    }
}
