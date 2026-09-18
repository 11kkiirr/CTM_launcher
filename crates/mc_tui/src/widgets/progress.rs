//! A labelled progress bar.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Gauge, Widget};

use crate::theme::Theme;

/// A green progress gauge with an optional label.
#[derive(Debug, Clone)]
pub struct ProgressBar<'a> {
    pub ratio: f64,
    pub label: &'a str,
    pub theme: &'a Theme,
}

impl<'a> ProgressBar<'a> {
    pub fn new(ratio: f64, label: &'a str, theme: &'a Theme) -> Self {
        Self {
            ratio,
            label,
            theme,
        }
    }
}

impl Widget for ProgressBar<'_> {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(self.theme.green).bg(self.theme.bg_alt))
            .ratio(self.ratio.clamp(0.0, 1.0))
            .label(self.label);
        gauge.render(area, buf);
    }
}
