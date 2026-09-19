//! Centered popup geometry and borderless rendering helpers.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::theme::Theme;

/// Compute a rectangle centered within `area`, sized as a percentage.
pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

/// Clear the area and draw a borderless raised panel with a green `▎` accent
/// bar, a title line and `lines` as the body.
pub fn render_popup(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    lines: Vec<Line<'_>>,
    theme: &Theme,
) {
    if area.width < 4 || area.height < 3 {
        return;
    }
    frame.render_widget(Clear, area);
    let surface = Style::default().bg(theme.panel_alt);
    frame.render_widget(ratatui::widgets::Block::default().style(surface), area);
    crate::views::accent_bar(frame, area, theme);

    let content = Rect {
        x: area.x + 2,
        y: area.y + 1,
        width: area.width.saturating_sub(3),
        height: area.height.saturating_sub(2),
    };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(content);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(title.to_string(), theme.header()))).style(surface),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().fg(theme.fg).bg(theme.panel_alt))
            .wrap(Wrap { trim: false }),
        rows[2],
    );
}
