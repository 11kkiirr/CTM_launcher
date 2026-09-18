//! A minimal button widget and line helper.
#![allow(dead_code)]

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::theme::Theme;

/// A focusable button rendered as `[ label ]`.
#[derive(Debug, Clone, Copy)]
pub struct Button<'a> {
    pub label: &'a str,
    pub focused: bool,
    pub enabled: bool,
    pub theme: &'a Theme,
}

impl<'a> Button<'a> {
    pub fn new(label: &'a str, theme: &'a Theme) -> Self {
        Self {
            label,
            focused: false,
            enabled: true,
            theme,
        }
    }

    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// The rendered width including brackets and padding.
    pub fn width(&self) -> u16 {
        self.label.chars().count() as u16 + 4
    }
}

impl Widget for Button<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let style = if !self.enabled {
            Style::default().fg(self.theme.muted)
        } else if self.focused {
            Style::default()
                .fg(self.theme.bg)
                .bg(self.theme.green)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(self.theme.green)
        };
        let text = format!("[ {} ]", self.label);
        buf.set_stringn(area.x, area.y, text, area.width as usize, style);
    }
}

/// Build a `Line` for a button, suitable for embedding in a paragraph.
pub fn button_line<'a>(label: &'a str, focused: bool, enabled: bool, theme: &Theme) -> Line<'a> {
    let style = if !enabled {
        Style::default().fg(theme.muted)
    } else if focused {
        Style::default()
            .fg(theme.bg)
            .bg(theme.green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.green)
    };
    Line::from(Span::styled(format!("[ {label} ]"), style))
}
