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
pub mod settings_ui;
pub mod shaders;
pub mod tiles;
pub mod versions;
pub mod worlds;

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
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
    let current = match state.selected() {
        Some(i) => i as i32,
        None => {
            let start = if delta < 0 { len as i32 - 1 } else { 0 };
            state.select(Some(start as usize));
            return;
        }
    };
    let next = (current + delta).clamp(0, len as i32 - 1) as usize;
    state.select(Some(next));
}

/// Mouse wheel over a list: scroll the *view* by a few rows, leaving the
/// selection alone unless it would leave the viewport (vim-style edge
/// follow). The `List` widget keeps a visible selection on screen by
/// itself, so keyboard navigation needs no extra work.
pub(crate) fn list_wheel(state: &mut ListState, len: usize, visible: usize, delta: i32) {
    if len == 0 {
        return;
    }
    let vis = visible.max(1);
    let max_off = len.saturating_sub(vis);
    let off = (state.offset() as i32 + delta * 3).clamp(0, max_off as i32) as usize;
    *state.offset_mut() = off;
    if let Some(sel) = state.selected() {
        let edge = (off + vis).saturating_sub(1).min(len.saturating_sub(1));
        state.select(Some(sel.clamp(off, edge)));
    }
}

/// Select the first or last item.
pub(crate) fn jump(state: &mut ListState, len: usize, to_end: bool) {
    if len == 0 {
        state.select(None);
    } else {
        state.select(Some(if to_end { len - 1 } else { 0 }));
    }
}

pub(crate) fn begin_action_row(app: &mut App, x: u16, y: u16, right: u16, widths: &[u16]) -> (u16, Rect) {
    let mut content_w = 0u16;
    for w in widths {
        content_w = content_w.saturating_add(w.saturating_add(1));
    }
    let viewport = Rect {
        x,
        y,
        width: right.saturating_sub(x),
        height: 1,
    };
    let max_scroll = content_w.saturating_sub(viewport.width);
    app.toolbar_max = max_scroll;
    app.toolbar_area = viewport;
    app.toolbar_scroll = app.toolbar_scroll.min(max_scroll);
    if app.toolbar_target > max_scroll {
        app.toolbar_target = max_scroll;
    }
    (app.toolbar_scroll, viewport)
}

pub(crate) fn action_cell(viewport: Rect, virtual_x: u16, width: u16, scroll: u16) -> Option<(Rect, u16)> {
    let start = viewport.x as i32 + virtual_x as i32 - scroll as i32;
    let vis_start = start.max(viewport.x as i32);
    let vis_end = (start + width as i32).min(viewport.right() as i32);
    if vis_start >= vis_end {
        return None;
    }
    let skip = if start < viewport.x as i32 {
        (viewport.x as i32 - start) as u16
    } else {
        0
    };
    let rect = Rect {
        x: vis_start as u16,
        y: viewport.y,
        width: (vis_end - vis_start) as u16,
        height: 1,
    };
    Some((rect, skip))
}

pub(crate) fn row_widths<'a>(items: impl Iterator<Item = (&'a str, &'a str)>) -> Vec<u16> {
    items
        .map(|(label, key)| label.chars().count() as u16 + key.chars().count() as u16 + 4)
        .collect()
}

/// Height of a toolbar pill: label row capped with half-blocks top and
/// bottom so it reads as 2 rows.
const PILL_H: u16 = 3;

/// Draw one 3-row toolbar pill (cap / label / toe) and register its hitbox.
pub(crate) fn pill_cell(
    app: &mut App,
    frame: &mut Frame,
    rect: Rect,
    skip: u16,
    line: Line<'_>,
    bg: Color,
    action: HitAction,
) {
    let edge = Style::default().fg(bg).bg(app.theme.bg);
    frame.render_widget(
        Paragraph::new(Span::styled("▄".repeat(rect.width as usize), edge))
            .style(Style::default().bg(app.theme.bg)),
        Rect { height: 1, ..rect },
    );
    frame.render_widget(
        Paragraph::new(line).style(Style::default().bg(bg)).scroll((0, skip)),
        Rect {
            y: rect.y + 1,
            height: 1,
            ..rect
        },
    );
    frame.render_widget(
        Paragraph::new(Span::styled("▀".repeat(rect.width as usize), edge))
            .style(Style::default().bg(app.theme.bg)),
        Rect {
            y: rect.y + 2,
            height: 1,
            ..rect
        },
    );
    app.hitboxes.push(crate::app::Hitbox { rect, action });
}

/// Render a row of tabs. The active tab is bright green, the rest muted.
/// Scrolls horizontally while the mouse hovers over the row.
///
/// Same 3-row pill look as the toolbar buttons.
pub(crate) fn tab_row(
    app: &mut App,
    frame: &mut Frame,
    x: u16,
    y: u16,
    max_width: u16,
    tabs: &[(&str, &str, ButtonId, bool)],
) {
    let widths = row_widths(tabs.iter().map(|(l, k, _, _)| (*l, *k)));
    let (scroll, viewport) = begin_action_row(app, x, y, max_width, &widths);
    app.toolbar_area.height = PILL_H;
    let mut vx = 0u16;
    for (i, (label, key, id, active)) in tabs.iter().enumerate() {
        if let Some((cell, skip)) = action_cell(viewport, vx, widths[i], scroll) {
            let rect = Rect {
                y: viewport.y,
                height: PILL_H,
                ..cell
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
            pill_cell(app, frame, rect, skip, line, bg, HitAction::Button(*id));
        }
        vx = vx.saturating_add(widths[i] + 1);
    }
}

/// Render a horizontal row of keybinding "pills", e.g. `Launch  Enter`.
/// Bracket-free flat blocks on a dark background.
///
/// Used for the modernized top action toolbar. Scrolls horizontally while the
/// mouse hovers over the row.
///
/// Each pill is 3 rows tall but capped with half-blocks top and bottom so it
/// reads as 2 rows.
pub(crate) fn pill_row(
    app: &mut App,
    frame: &mut Frame,
    x: u16,
    y: u16,
    max_width: u16,
    pills: &[(&str, &str, ButtonId)],
) {
    let widths = row_widths(pills.iter().map(|(l, k, _)| (*l, *k)));
    let (scroll, viewport) = begin_action_row(app, x, y, max_width, &widths);
    app.toolbar_area.height = PILL_H;
    let mut vx = 0u16;
    for (i, (label, key, id)) in pills.iter().enumerate() {
        if let Some((cell, skip)) = action_cell(viewport, vx, widths[i], scroll) {
            let rect = Rect {
                y: viewport.y,
                height: PILL_H,
                ..cell
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
            pill_cell(app, frame, rect, skip, line, bg, HitAction::Button(*id));
        }
        vx = vx.saturating_add(widths[i] + 1);
    }
}

/// Render a horizontal row of clickable action buttons.
/// Same 3-row pill look as [`pill_row`].
pub(crate) fn buttons_row(
    app: &mut App,
    frame: &mut Frame,
    x: u16,
    y: u16,
    max_width: u16,
    buttons: &[(&str, &str, ButtonId)],
) {
    pill_row(app, frame, x, y, max_width, buttons);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_scrolls_view_and_keeps_selection_visible() {
        let mut state = ListState::default();
        state.select(Some(0));
        // 10 items, 6 visible: one notch moves the view; the selection
        // is pulled along the edge only because it left the viewport.
        list_wheel(&mut state, 10, 6, 1);
        assert_eq!(state.offset(), 3);
        assert_eq!(state.selected(), Some(3));
        // A selection inside the new view never moves.
        state.select(Some(5));
        list_wheel(&mut state, 10, 6, 1);
        assert_eq!(state.offset(), 4);
        assert_eq!(state.selected(), Some(5));
        // Clamp at the end, selection stays.
        list_wheel(&mut state, 10, 6, 10);
        assert_eq!(state.offset(), 4);
        assert_eq!(state.selected(), Some(5));
        // Back to the top, selection stays.
        list_wheel(&mut state, 10, 6, -10);
        assert_eq!(state.offset(), 0);
        assert_eq!(state.selected(), Some(5));
        // Short list: no scrolling possible.
        let mut short = ListState::default();
        short.select(Some(1));
        list_wheel(&mut short, 2, 4, 1);
        assert_eq!(short.offset(), 0);
        assert_eq!(short.selected(), Some(1));
    }
}
