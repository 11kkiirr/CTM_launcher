//! Modern, minimal, borderless palette.
//!
//! The UI is built from flat dark cards separated by background contrast rather
//! than box-drawing borders. A single vibrant green (`#50FA7B`) is used as the
//! accent for selections, the left `▎` focus bar and keybindings.

use ratatui::style::{Color, Modifier, Style};

/// The launcher's color theme.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct Theme {
    /// Matte near-black terminal background.
    pub bg: Color,
    /// Slightly lifted background used for the header/footer bars.
    pub bg_alt: Color,
    /// Card / sidebar background.
    pub panel: Color,
    /// Raised surface for hover states and overlays.
    pub panel_alt: Color,
    /// Default foreground text.
    pub fg: Color,
    /// Muted grey for labels, paths and versions.
    pub muted: Color,
    /// Cool secondary (comment) tone.
    pub comment: Color,
    /// Primary vibrant green accent.
    pub green: Color,
    /// Brighter green used for the strongest emphasis.
    pub green_bright: Color,
    /// Deep green used for subtle dividers.
    pub green_dim: Color,
    /// Soft dark-green background of a selected row.
    pub selection_bg: Color,
    /// Background of a hovered row.
    pub hover_bg: Color,
    pub warning: Color,
    pub error: Color,
    pub info: Color,
    pub debug: Color,
    pub trace: Color,
    /// Rarely used hairline colour (overlays/dividers).
    pub border: Color,
    /// Focused accent colour.
    pub border_focused: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            bg: Color::Rgb(0x0D, 0x0D, 0x0D),
            bg_alt: Color::Rgb(0x12, 0x12, 0x12),
            panel: Color::Rgb(0x18, 0x18, 0x18),
            panel_alt: Color::Rgb(0x1E, 0x1E, 0x1E),
            fg: Color::Rgb(0xF8, 0xF8, 0xF2),
            muted: Color::Rgb(0x75, 0x75, 0x75),
            comment: Color::Rgb(0x62, 0x72, 0xA4),
            green: Color::Rgb(0x50, 0xFA, 0x7B),
            green_bright: Color::Rgb(0x00, 0xFF, 0x87),
            green_dim: Color::Rgb(0x2A, 0x5A, 0x3A),
            selection_bg: Color::Rgb(0x14, 0x2B, 0x1E),
            hover_bg: Color::Rgb(0x1E, 0x1E, 0x1E),
            warning: Color::Rgb(0xF1, 0xFA, 0x8C),
            error: Color::Rgb(0xFF, 0x55, 0x55),
            info: Color::Rgb(0x8B, 0xE9, 0xFD),
            debug: Color::Rgb(0x62, 0x72, 0xA4),
            trace: Color::Rgb(0x44, 0x47, 0x5A),
            border: Color::Rgb(0x2A, 0x2A, 0x2A),
            border_focused: Color::Rgb(0x50, 0xFA, 0x7B),
        }
    }
}

impl Theme {
    /// Base text on the root background.
    pub fn base(&self) -> Style {
        Style::default().fg(self.fg).bg(self.bg)
    }

    /// Text on the header/footer bars.
    pub fn bar(&self) -> Style {
        Style::default().fg(self.fg).bg(self.bg_alt)
    }

    /// Text on a card / sidebar surface.
    pub fn card(&self) -> Style {
        Style::default().fg(self.fg).bg(self.panel)
    }

    /// Dimmed text on a card.
    pub fn card_dim(&self) -> Style {
        Style::default().fg(self.muted).bg(self.panel)
    }

    /// Cool secondary text on a card.
    pub fn card_comment(&self) -> Style {
        Style::default().fg(self.comment).bg(self.panel)
    }

    /// Emphasized green text (background is inherited).
    pub fn accent(&self) -> Style {
        Style::default().fg(self.green)
    }

    /// Brightest green emphasis.
    pub fn accent_bright(&self) -> Style {
        Style::default()
            .fg(self.green_bright)
            .add_modifier(Modifier::BOLD)
    }

    /// Secondary/dimmed text (background inherited).
    pub fn dim(&self) -> Style {
        Style::default().fg(self.muted)
    }

    /// Cool secondary text (background inherited).
    pub fn comment_style(&self) -> Style {
        Style::default().fg(self.comment)
    }

    /// Selected list row: green on a soft dark-green background.
    pub fn selection(&self) -> Style {
        Style::default()
            .bg(self.selection_bg)
            .fg(self.green)
            .add_modifier(Modifier::BOLD)
    }

    /// Hovered element.
    pub fn hover(&self) -> Style {
        Style::default().bg(self.hover_bg).fg(self.green)
    }

    /// A non-focused card row.
    pub fn row(&self) -> Style {
        Style::default().fg(self.fg).bg(self.panel)
    }

    /// A selected card row.
    pub fn row_selected(&self) -> Style {
        Style::default()
            .bg(self.selection_bg)
            .fg(self.green)
            .add_modifier(Modifier::BOLD)
    }

    /// A hovered card row.
    pub fn row_hover(&self) -> Style {
        Style::default().bg(self.hover_bg).fg(self.green)
    }

    /// Style for a header line / section title.
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

    /// Colour associated with a log level label.
    pub fn log_level_color(&self, level: mc_core::logs::LogLevel) -> Color {
        use mc_core::logs::LogLevel::*;
        match level {
            Error | Fatal => self.error,
            Warn => self.warning,
            Info => self.green,
            Debug => self.info,
            Trace => self.trace,
            Unknown => self.muted,
        }
    }
}
