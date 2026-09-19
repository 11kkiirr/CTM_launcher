//! Minimalist green color palette and shared styles.
//!
//! The palette is deliberately small: a dark slate background, soft gray
//! borders and emerald/forest green highlights.

use ratatui::style::{Color, Modifier, Style};

/// The launcher's color theme.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct Theme {
    /// Primary background.
    pub bg: Color,
    /// Slightly darker background used for the sidebar and headers.
    pub bg_alt: Color,
    /// Panel background.
    pub panel: Color,
    /// Default foreground text.
    pub fg: Color,
    /// Dimmed/secondary text.
    pub muted: Color,
    /// Soft gray borders.
    pub border: Color,
    /// Brighter border for focused panels.
    pub border_focused: Color,
    /// Emerald green accent.
    pub green: Color,
    /// Forest green for selections.
    pub forest: Color,
    /// Background of a selected list row.
    pub selection_bg: Color,
    pub warning: Color,
    pub error: Color,
    pub info: Color,
    pub debug: Color,
    pub trace: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            bg: Color::Rgb(0x1B, 0x1E, 0x24),
            bg_alt: Color::Rgb(0x16, 0x18, 0x1D),
            panel: Color::Rgb(0x20, 0x24, 0x2B),
            fg: Color::Rgb(0xD8, 0xDE, 0xE9),
            muted: Color::Rgb(0x7A, 0x81, 0x94),
            border: Color::Rgb(0x3B, 0x42, 0x52),
            border_focused: Color::Rgb(0x5F, 0xA9, 0x7A),
            green: Color::Rgb(0x6F, 0xE3, 0x9A),
            forest: Color::Rgb(0x3E, 0x9B, 0x5F),
            selection_bg: Color::Rgb(0x24, 0x35, 0x2B),
            warning: Color::Rgb(0xE9, 0xC4, 0x6A),
            error: Color::Rgb(0xE8, 0x8B, 0x9A),
            info: Color::Rgb(0x8A, 0xB4, 0xF8),
            debug: Color::Rgb(0x8F, 0xD9, 0xC8),
            trace: Color::Rgb(0x6C, 0x70, 0x86),
        }
    }
}

impl Theme {
    /// Base text style on the primary background.
    pub fn base(&self) -> Style {
        Style::default().fg(self.fg).bg(self.bg)
    }

    /// Style for a normal (unfocused) block border.
    pub fn block_border(&self) -> Style {
        Style::default().fg(self.border)
    }

    /// Style for a focused block border.
    pub fn block_border_focused(&self) -> Style {
        Style::default().fg(self.border_focused)
    }

    /// Style for emphasized green text.
    pub fn accent(&self) -> Style {
        Style::default().fg(self.green)
    }

    /// Style for secondary/dimmed text.
    pub fn dim(&self) -> Style {
        Style::default().fg(self.muted)
    }

    /// Style applied to a selected list row.
    pub fn selection(&self) -> Style {
        Style::default()
            .bg(self.selection_bg)
            .fg(self.green)
            .add_modifier(Modifier::BOLD)
    }

    /// Style applied to a hovered element.
    pub fn hover(&self) -> Style {
        Style::default().bg(self.selection_bg).fg(self.green)
    }

    /// Style for a highlighted button.
    #[allow(dead_code)]
    pub fn button(&self) -> Style {
        Style::default()
            .fg(self.bg)
            .bg(self.forest)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for a header line.
    pub fn header(&self) -> Style {
        Style::default().fg(self.green).add_modifier(Modifier::BOLD)
    }

    /// Style for warning text.
    pub fn warning_style(&self) -> Style {
        Style::default().fg(self.warning)
    }

    /// Style for error text.
    pub fn error_style(&self) -> Style {
        Style::default().fg(self.error)
    }

    /// Style for informational text.
    pub fn info_style(&self) -> Style {
        Style::default().fg(self.info)
    }

    /// Color associated with a log level label.
    pub fn log_level_color(&self, level: mc_core::logs::LogLevel) -> Color {
        use mc_core::logs::LogLevel::*;
        match level {
            Error | Fatal => self.error,
            Warn => self.warning,
            Info => self.info,
            Debug => self.debug,
            Trace => self.trace,
            Unknown => self.muted,
        }
    }
}
