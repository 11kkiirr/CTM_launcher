//! Isolated UI screens.

pub mod accounts;
pub mod browse;
pub mod instance_settings;
pub mod logs;
pub mod modpacks;
pub mod mods;
pub mod resourcepacks;
pub mod screenshots;
pub mod settings;
pub mod shaders;
pub mod tiles;
pub mod versions;
pub mod worlds;

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, ListState, Paragraph};
use ratatui::Frame;

use crate::app::{rect_contains, App, ButtonId, HitAction};
use crate::theme::Theme;

/// The padded content rectangle of a card (1 column and 1 row of breathing room).
pub(crate) fn inner(area: Rect) -> Rect {
    Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

/// Draw a flat card: a filled dark surface with no borders.
///
/// Focused cards additionally get a green `▎` accent bar down the left edge.
/// Returns the padded inner rectangle for content.
pub(crate) fn card(app: &App, frame: &mut Frame, area: Rect, _focused: bool) -> Rect {
    frame.render_widget(Block::default().style(app.theme.card()), area);
    inner(area)
}

/// Draw the vertical `▎` accent bar along the left edge of `area`.
pub(crate) fn accent_bar(frame: &mut Frame, area: Rect, theme: &Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let bar = Rect {
        x: area.x,
        y: area.y,
        width: 1,
        height: area.height,
    };
    let lines: Vec<Line> = (0..area.height)
        .map(|_| Line::from(Span::styled("▎", theme.accent())))
        .collect();
    frame.render_widget(Paragraph::new(lines), bar);
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
        app.theme.row_selected()
    } else if hovered == Some(idx) {
        app.theme.row_hover()
    } else {
        app.theme.row()
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

/// Render a horizontal row of clickable action buttons and register their
/// hitboxes. Flat text chips on a dark background, matching pill_row style.
pub(crate) fn buttons_row(
    app: &mut App,
    frame: &mut Frame,
    mut x: u16,
    y: u16,
    max_width: u16,
    buttons: &[(&str, &str, ButtonId)],
) {
    for (label, key, id) in buttons {
        let width = label.chars().count() as u16 + key.chars().count() as u16 + 4;
        if x + width > max_width {
            break;
        }
        let rect = Rect {
            x,
            y,
            width,
            height: 1,
        };
        let hovered = app.is_hovered(rect);
        let bg = if hovered {
            app.theme.selection_bg
        } else {
            app.theme.panel_alt
        };
        let label_style = if hovered {
            app.theme.accent_bright()
        } else {
            Style::default().fg(app.theme.fg)
        };
        let line = Line::from(vec![
            Span::styled(format!(" {label}  "), label_style),
            Span::styled(key.to_string(), app.theme.accent()),
            Span::styled(" ", app.theme.comment_style()),
        ]);
        frame.render_widget(Paragraph::new(line).style(Style::default().bg(bg)), rect);
        app.hitboxes.push(crate::app::Hitbox {
            rect,
            action: HitAction::Button(*id),
        });
        x += width + 1;
    }
}

/// Render a row of tabs. The active tab is bright green, the rest muted.
pub(crate) fn tab_row(
    app: &mut App,
    frame: &mut Frame,
    mut x: u16,
    y: u16,
    max_width: u16,
    tabs: &[(&str, &str, ButtonId, bool)],
) {
    for (label, key, id, active) in tabs {
        let width = label.chars().count() as u16 + key.chars().count() as u16 + 4;
        if x + width > max_width {
            break;
        }
        let rect = Rect {
            x,
            y,
            width,
            height: 1,
        };
        let hovered = app.is_hovered(rect);
        let bg = if hovered {
            app.theme.selection_bg
        } else {
            app.theme.panel_alt
        };
        let label_style = if *active {
            app.theme.accent_bright()
        } else if hovered {
            app.theme.accent()
        } else {
            Style::default().fg(app.theme.muted)
        };
        let line = Line::from(vec![
            Span::styled(format!(" {label}  "), label_style),
            Span::styled(key.to_string(), app.theme.accent()),
            Span::styled(" ", app.theme.comment_style()),
        ]);
        frame.render_widget(Paragraph::new(line).style(Style::default().bg(bg)), rect);
        app.hitboxes.push(crate::app::Hitbox {
            rect,
            action: HitAction::Button(*id),
        });
        x += width + 1;
    }
}

/// Render a horizontal row of keybinding "pills", e.g. `Launch  Enter`.
/// Bracket-free flat blocks on a dark background.
///
/// Used for the modernized top action toolbar.
pub(crate) fn pill_row(
    app: &mut App,
    frame: &mut Frame,
    mut x: u16,
    y: u16,
    max_width: u16,
    pills: &[(&str, &str, ButtonId)],
) {
    for (label, key, id) in pills {
        let width = label.chars().count() as u16 + key.chars().count() as u16 + 4;
        if x + width > max_width {
            break;
        }
        let rect = Rect {
            x,
            y,
            width,
            height: 1,
        };
        let hovered = app.is_hovered(rect);
        let bg = if hovered {
            app.theme.selection_bg
        } else {
            app.theme.panel_alt
        };
        let label_style = if hovered {
            app.theme.accent_bright()
        } else {
            Style::default().fg(app.theme.fg)
        };
        let line = Line::from(vec![
            Span::styled(format!(" {label}  "), label_style),
            Span::styled(key.to_string(), app.theme.accent()),
            Span::styled(" ", app.theme.comment_style()),
        ]);
        frame.render_widget(Paragraph::new(line).style(Style::default().bg(bg)), rect);
        app.hitboxes.push(crate::app::Hitbox {
            rect,
            action: HitAction::Button(*id),
        });
        x += width + 1;
    }
}

/// A card section title: a green label with an optional muted suffix.
pub(crate) fn section_title(label: &str, suffix: &str, theme: &Theme) -> Line<'static> {
    let mut spans = vec![Span::styled(label.to_string(), theme.header())];
    if !suffix.is_empty() {
        spans.push(Span::styled(format!("  {suffix}"), theme.card_dim()));
    }
    Line::from(spans)
}

/// Truncate a string to `max` display columns, appending `…` when clipped.
pub(crate) fn truncate(input: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = input.chars().collect();
    if chars.len() <= max {
        return input.to_string();
    }
    let mut out: String = chars.into_iter().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
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
