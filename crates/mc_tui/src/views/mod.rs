//! Isolated UI screens.

pub mod accounts;
pub mod instances;
pub mod logs;
pub mod modpacks;
pub mod mods;
pub mod settings;
pub mod tiles;

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::{ListState, Paragraph};
use ratatui::Frame;

use crate::app::{App, ButtonId, HitAction};

/// Whether `rect` contains the mouse position.
pub(crate) fn rect_contains(rect: Rect, (x, y): (u16, u16)) -> bool {
    x >= rect.x
        && x < rect.x.saturating_add(rect.width)
        && y >= rect.y
        && y < rect.y.saturating_add(rect.height)
}

/// Index of the list row currently under the mouse, if any.
pub(crate) fn hovered_index(app: &App, inner: Rect, offset: usize, len: usize) -> Option<usize> {
    let pos = app.mouse_pos?;
    if !rect_contains(inner, pos) {
        return None;
    }
    let idx = offset + (pos.1 - inner.y) as usize;
    (idx < len).then_some(idx)
}

/// Style for a list row, taking selection and hover into account.
pub(crate) fn row_style(
    app: &App,
    idx: usize,
    selected: Option<usize>,
    hovered: Option<usize>,
) -> Style {
    if selected == Some(idx) {
        app.theme.selection()
    } else if hovered == Some(idx) {
        app.theme.hover()
    } else {
        app.theme.base()
    }
}

/// Move a list selection by `delta`, clamping to `len`.
pub(crate) fn move_sel(state: &mut ListState, len: usize, delta: i32) {
    if len == 0 {
        state.select(None);
        return;
    }
    let current = state.selected().unwrap_or(0) as i32;
    let next = (current + delta).clamp(0, len as i32 - 1) as usize;
    state.select(Some(next));
}

/// Select the first or last item.
pub(crate) fn jump(state: &mut ListState, len: usize, to_end: bool) {
    if len == 0 {
        state.select(None);
    } else {
        state.select(Some(if to_end { len - 1 } else { 0 }));
    }
}

/// Render a horizontal row of clickable buttons and register their hitboxes.
pub(crate) fn buttons_row(
    app: &mut App,
    frame: &mut Frame,
    mut x: u16,
    y: u16,
    max_width: u16,
    buttons: &[(&str, ButtonId)],
) {
    for (label, id) in buttons {
        let text = format!("[ {label} ]");
        let width = text.chars().count() as u16;
        if x + width > max_width {
            break;
        }
        let rect = Rect {
            x,
            y,
            width,
            height: 1,
        };
        let style = if app.is_hovered(rect) {
            app.theme.hover()
        } else {
            app.theme.accent()
        };
        frame.render_widget(Paragraph::new(Span::styled(text, style)), rect);
        app.hitboxes.push(crate::app::Hitbox {
            rect,
            action: HitAction::Button(*id),
        });
        x += width + 1;
    }
}

/// Register hitboxes for the visible rows of a list block.
pub(crate) fn register_rows(
    hitboxes: &mut Vec<crate::app::Hitbox>,
    state: &ListState,
    inner: Rect,
    total: usize,
    make: impl Fn(usize) -> HitAction,
) {
    if inner.height == 0 {
        return;
    }
    let offset = state.offset();
    let visible = inner.height as usize;
    for row in 0..visible {
        let idx = offset + row;
        if idx >= total {
            break;
        }
        let rect = Rect {
            x: inner.x,
            y: inner.y + row as u16,
            width: inner.width,
            height: 1,
        };
        hitboxes.push(crate::app::Hitbox {
            rect,
            action: make(idx),
        });
    }
}
