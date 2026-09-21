//! Persisted launcher-wide settings (`settings.json`).

use std::path::PathBuf;

use mc_core::instance::GcPreset;
use mc_core::util::{read_json_or_default, write_json, Paths};
use serde::{Deserialize, Serialize};

/// Corner to which the ASCII-art background is anchored.
#[derive(Debug, Clone, Default, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsciiBgAnchor {
    TopLeft,
    TopRight,
    BottomLeft,
    #[default]
    BottomRight,
}

impl AsciiBgAnchor {
    pub const ALL: &'static [AsciiBgAnchor] = &[
        AsciiBgAnchor::TopLeft,
        AsciiBgAnchor::TopRight,
        AsciiBgAnchor::BottomLeft,
        AsciiBgAnchor::BottomRight,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AsciiBgAnchor::TopLeft => "Top-Left",
            AsciiBgAnchor::TopRight => "Top-Right",
            AsciiBgAnchor::BottomLeft => "Bottom-Left",
            AsciiBgAnchor::BottomRight => "Bottom-Right",
        }
    }
}

/// Global defaults applied to newly created instances and the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LauncherSettings {
    /// Explicit Java executable used when an instance does not override one.
    #[serde(default)]
    pub java_path: Option<PathBuf>,
    #[serde(default = "default_min_memory")]
    pub default_min_memory_mb: u32,
    #[serde(default = "default_max_memory")]
    pub default_max_memory_mb: u32,
    #[serde(default)]
    pub default_gc: GcPreset,
    /// Whether the global progress bar is shown.
    #[serde(default = "default_true")]
    pub show_progress: bool,
    /// Whether quitting asks for confirmation.
    #[serde(default)]
    pub confirm_quit: bool,
    /// Whether the console auto-scrolls to the newest line.
    #[serde(default = "default_true")]
    pub log_auto_scroll: bool,
    /// Which corner the ASCII-art background is anchored to.
    #[serde(default)]
    pub ascii_bg_anchor: AsciiBgAnchor,
}

fn default_min_memory() -> u32 {
    512
}
fn default_max_memory() -> u32 {
    4096
}
fn default_true() -> bool {
    true
}

impl Default for LauncherSettings {
    fn default() -> Self {
        Self {
            java_path: None,
            default_min_memory_mb: default_min_memory(),
            default_max_memory_mb: default_max_memory(),
            default_gc: GcPreset::G1,
            show_progress: true,
            confirm_quit: false,
            log_auto_scroll: true,
            ascii_bg_anchor: AsciiBgAnchor::BottomRight,
        }
    }
}

impl LauncherSettings {
    /// Load settings from the config directory, falling back to defaults.
    pub async fn load(paths: &Paths) -> Self {
        read_json_or_default(paths.settings_file())
            .await
            .unwrap_or_default()
    }

    /// Persist settings to the config directory.
    pub async fn save(&self, paths: &Paths) -> mc_core::Result<()> {
        write_json(paths.settings_file(), self).await
    }
}
