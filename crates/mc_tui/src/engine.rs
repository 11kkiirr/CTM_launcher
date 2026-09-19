//! Messages sent from background engine tasks to the UI thread.

use mc_core::auth::{Account, DeviceCodePrompt};
use mc_core::launch::{JavaInstallation, LogReceiver, ProcessHandle};
use mc_core::logs::CrashAnalysis;
use mc_core::modrinth::{InstalledMod, Project, SearchResults, Version};
use mc_core::util::Progress;

/// An event produced by an asynchronous engine task.
#[derive(Debug)]
pub enum EngineEvent {
    /// A transient status-bar message.
    Status(String),
    /// Progress update for the global progress bar.
    Progress { ratio: Option<f64>, label: String },
    /// Clear the progress bar.
    ProgressDone,
    /// A fresh batch of log lines (used when loading a saved log file).
    LogLines(Vec<String>),
    /// A native file dialog returned a path (or None when cancelled/unavailable).
    ImportPathPicked(Option<String>),
    /// A list of human-readable lines to show in a message overlay.
    Message(Vec<String>),
    /// A game process started successfully.
    Started {
        command: String,
        version: String,
        handle: Box<ProcessHandle>,
        logs: Box<LogReceiver>,
    },
    /// Instances changed on disk; the UI should reload.
    InstancesChanged,
    /// Accounts changed on disk; the UI should reload.
    AccountsChanged,
    /// A freshly loaded account store.
    AccountsReloaded(mc_core::auth::AccountStore),
    /// Modrinth search results.
    SearchResults(SearchResults),
    /// Modrinth mod-search results (Mod Manager pane).
    ModSearchResults(SearchResults),
    /// Version list for the version picker overlay.
    VersionList {
        target: crate::forms::PickerTarget,
        versions: Vec<String>,
    },
    /// Create-wizard Modrinth search results.
    WizardSearch(SearchResults),
    /// Create-wizard project with its versions.
    WizardProject {
        project: Box<Project>,
        versions: Vec<Version>,
    },
    /// A project with its available versions.
    Project {
        project: Box<Project>,
        versions: Vec<Version>,
    },
    /// Installed mods changed on disk.
    ModsChanged,
    /// Freshly scanned installed mods.
    InstalledMods(Vec<InstalledMod>),
    /// A crash report was analyzed.
    Crash(Option<CrashAnalysis>),
    /// A device-code prompt is ready for display.
    DeviceCode(Box<DeviceCodePrompt>),
    /// A Microsoft account completed authentication.
    Authenticated(Box<Account>),
    /// Java runtimes were discovered.
    Java(Vec<JavaInstallation>),
    /// A success message for a toast.
    Toast(String),
    /// An error message for a toast.
    Error(String),
}

/// Convenience alias for the sender side.
pub type EngineSender = tokio::sync::mpsc::UnboundedSender<EngineEvent>;
/// Convenience alias for the receiver side.
pub type EngineReceiver = tokio::sync::mpsc::UnboundedReceiver<EngineEvent>;

/// Translate a core progress tick into a UI event.
pub fn progress_event(progress: Progress) -> EngineEvent {
    EngineEvent::Progress {
        ratio: progress.fraction(),
        label: progress.message,
    }
}
