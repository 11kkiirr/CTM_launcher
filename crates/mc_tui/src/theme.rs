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
            bg: Color::Rgb(0x1E, 0x1E, 0x2E),
            bg_alt: Color::Rgb(0x18, 0x18, 0x25),
            panel: Color::Rgb(0x1E, 0x1E, 0x2E),
            fg: Color::Rgb(0xCD, 0xD6, 0xF4),
            muted: Color::Rgb(0x6C, 0x70, 0x86),
            border: Color::Rgb(0x45, 0x47, 0x5A),
            border_focused: Color::Rgb(0x2E, 0xA0, 0x43),
            green: Color::Rgb(0x50, 0xFA, 0x7B),
            forest: Color::Rgb(0x2E, 0xA0, 0x43),
            selection_bg: Color::Rgb(0x26, 0x3A, 0x2E),
            warning: Color::Rgb(0xF9, 0xE2, 0xAF),
            error: Color::Rgb(0xF3, 0x8B, 0xA8),
            info: Color::Rgb(0x89, 0xB4, 0xFA),
            debug: Color::Rgb(0x94, 0xE2, 0xD5),
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
