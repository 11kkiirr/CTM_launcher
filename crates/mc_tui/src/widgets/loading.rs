//! Animated placeholders shown while an image is being downloaded/decoded.
//!
//! Two looks share the (light grey) loading palette: a wide sweeping
//! "skeleton" shimmer for medium surfaces (screenshot tiles, gallery
//! thumbnails) and a compact braille spinner everywhere else — tiny icons and
//! large previews alike. Both are driven by [`crate::app::App::tick`].

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::theme::Theme;

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Centered braille spinner over `area`.
pub fn spinner(frame: &mut Frame, area: Rect, bg: Color, theme: &Theme, tick: u64) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let glyph = SPINNER[(tick as usize / 2) % SPINNER.len()];
    let row = Rect {
        y: area.y + area.height / 2,
        height: 1,
        ..area
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            glyph,
            Style::default().fg(theme.comment).bg(bg),
        )))
        .alignment(Alignment::Center),
        row,
    );
}

/// A light grey gradient band sweeping diagonally across a subtly lifted
/// surface — the classic "skeleton" loader.
pub fn skeleton(frame: &mut Frame, area: Rect, bg: Color, theme: &Theme, tick: u64) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let base = mix(bg, theme.muted, 0.30);
    let w = area.width as i32;
    let band = (w as f32 / 3.0).clamp(3.0, 12.0);
    let span = w as f32 + band * 2.0;
    let head = ((tick as f32 * 1.1) % span) - band;

    for row in 0..area.height {
        let row_head = head + row as f32 * 0.8;
        let mut spans = Vec::with_capacity(area.width as usize);
        for col in 0..w {
            let d = (col as f32 - row_head).abs();
            let t = (1.0 - d / band).clamp(0.0, 1.0);
            let color = mix(base, theme.comment, t * t);
            spans.push(Span::styled(" ", Style::default().bg(color)));
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect {
                y: area.y + row,
                height: 1,
                ..area
            },
        );
    }
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    let (ar, ag, ab) = rgb(a);
    let (br, bg, bb) = rgb(b);
    let t = t.clamp(0.0, 1.0);
    Color::Rgb(
        (ar as f32 + (br as f32 - ar as f32) * t).round() as u8,
        (ag as f32 + (bg as f32 - ag as f32) * t).round() as u8,
        (ab as f32 + (bb as f32 - ab as f32) * t).round() as u8,
    )
}

fn rgb(c: Color) -> (u8, u8, u8) {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Black => (0, 0, 0),
        _ => (0x18, 0x18, 0x18),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn placeholders_render_without_panicking() {
        let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
        let theme = Theme::default();
        terminal
            .draw(|frame| {
                let tile = Rect {
                    x: 0,
                    y: 0,
                    width: 30,
                    height: 7,
                };
                skeleton(frame, tile, theme.panel, &theme, 7);
                let icon = Rect {
                    x: 0,
                    y: 8,
                    width: 8,
                    height: 3,
                };
                spinner(frame, icon, theme.panel, &theme, 3);
                let big = Rect {
                    x: 0,
                    y: 11,
                    width: 40,
                    height: 1,
                };
                spinner(frame, big, theme.panel, &theme, 5);
                spinner(frame, Rect::default(), theme.panel, &theme, 1);
            })
            .unwrap();
    }
}
