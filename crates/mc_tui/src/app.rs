//! Application state, event routing and background-task orchestration.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use mc_core::auth::microsoft::MicrosoftAuth;
use mc_core::auth::{Account, AccountKind, AccountStore};
use mc_core::install::Installer;
use mc_core::instance::{Instance, InstanceManager, JvmConfig, LoaderType};
use mc_core::launch::{
    java, resolve_instance_version, select_java, JavaInstallation, Launcher, LogReceiver,
    ProcessHandle,
};
use mc_core::logs::{self, CrashAnalysis, LogBuffer};
use mc_core::modpack::ModpackInstaller;
use mc_core::modrinth::{self, InstalledMod, ModrinthClient, Project, SearchHit, SearchResults, Version};
use mc_core::skins::{SkinClient, SkinVariant};
use mc_core::util::{Paths, Progress, ProgressCallback};
use mc_core::CoreError;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, ListState, Paragraph};
use ratatui::Frame;
use tokio::sync::mpsc;

use crate::engine::{progress_event, EngineEvent, EngineReceiver, EngineSender};
use crate::forms::{
    gc_options, parse_gc, ConfirmAction, Form, FormAction, Overlay, PickerTarget, TextAction,
};
use crate::settings::LauncherSettings;
use crate::theme::Theme;
use crate::views::browse::{Browse, BrowseFocus, BrowseKind, SideFilter};
use crate::wizard::BuildKind;

/// The Azure application (client) id used for Microsoft device-code auth.
pub const CLIENT_ID: &str = mc_core::auth::microsoft::DEFAULT_CLIENT_ID;

/// Height in rows of a chunky sidebar navigation block button.
const NAV_BUTTON_HEIGHT: u16 = 3;

/// Navigation entries shown in the right-hand panel.
///
/// The first group is global, the second is scoped to the selected build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    /// Instance picker (home) with the build action toolbar.
    Instances,
    /// Modrinth browser (mods / modpacks / resourcepacks / shaders).
    Browse,
    /// Mod manager.
    Mods,
    /// Modpack browser / `.mrpack` import.
    Modpacks,
    /// Build version, loader and reinstall tools.
    Versions,
    /// JVM, memory and Java configuration.
    Jvm,
    /// Console / logs.
    Logs,
    /// Account switcher and skins.
    Accounts,
    /// Global launcher settings.
    Launcher,
    /// Resource pack manager.
    ResourcePacks,
    /// Shader pack manager.
    Shaders,
    /// World manager.
    Worlds,
    /// Screenshot viewer.
    Screenshots,
}

impl Nav {
    /// Every page, including ones no longer shown in the sidebar menu.
    #[allow(dead_code)]
    pub fn all() -> [Nav; 12] {
        [
            Nav::Instances,
            Nav::Mods,
            Nav::ResourcePacks,
            Nav::Shaders,
            Nav::Worlds,
            Nav::Screenshots,
            Nav::Modpacks,
            Nav::Versions,
            Nav::Jvm,
            Nav::Logs,
            Nav::Accounts,
            Nav::Launcher,
        ]
    }

    /// The pages shown in the right-hand navigation menu, in order.
    ///
    /// `Browse`, `Modpacks` and `Accounts` are intentionally omitted; they are
    /// reached from the instance toolbar, the Mods page and the header account
    /// badge respectively.
    pub fn menu() -> [Nav; 9] {
        [
            Nav::Instances,
            Nav::Mods,
            Nav::ResourcePacks,
            Nav::Shaders,
            Nav::Worlds,
            Nav::Screenshots,
            Nav::Versions,
            Nav::Jvm,
            Nav::Logs,
        ]
    }

    /// 1-based index shown next to a menu entry, if it is in the menu.
    #[allow(dead_code)]
    pub fn number(&self) -> Option<usize> {
        Self::menu().iter().position(|n| n == self).map(|i| i + 1)
    }

    pub fn label(&self) -> &'static str {
        match self {
            Nav::Instances => "Instances",
            Nav::Browse => "Browse",
            Nav::Mods => "Mods",
            Nav::Modpacks => "Modpacks",
            Nav::Versions => "Versions",
            Nav::Jvm => "JVM Settings",
            Nav::Logs => "Logs",
            Nav::Accounts => "Accounts",
            Nav::Launcher => "Launcher Settings",
            Nav::ResourcePacks => "Resource Packs",
            Nav::Shaders => "Shaders",
            Nav::Worlds => "Worlds",
            Nav::Screenshots => "Screenshots",
        }
    }

    /// Short label used inside the sidebar menu buttons.
    pub fn menu_label(&self) -> &'static str {
        match self {
            Nav::Jvm => "JVM",
            Nav::ResourcePacks => "Res Packs",
            other => other.label(),
        }
    }

    #[allow(dead_code)]
    pub fn icon(&self) -> &'static str {
        match self {
            Nav::Instances => "⌂",
            Nav::Browse => "▦",
            Nav::Mods => "✦",
            Nav::Modpacks => "⛁",
            Nav::Versions => "❖",
            Nav::Jvm => "⚙",
            Nav::Logs => "≣",
            Nav::Accounts => "☺",
            Nav::Launcher => "⚒",
            Nav::ResourcePacks => "◧",
            Nav::Shaders => "☀",
            Nav::Worlds => "🌐",
            Nav::Screenshots => "📷",
        }
    }

    /// Whether this page requires a selected build.
    #[allow(dead_code)]
    pub fn is_build_scoped(&self) -> bool {
        matches!(
            self,
            Nav::Mods
                | Nav::Modpacks
                | Nav::Versions
                | Nav::Jvm
                | Nav::Logs
                | Nav::ResourcePacks
                | Nav::Shaders
                | Nav::Worlds
                | Nav::Screenshots
        )
    }
}

/// Which region currently owns keyboard navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    Content,
}

/// A clickable region registered during rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitAction {
    NavItem(Nav),
    InstanceTile(usize),
    AddInstance,
    SearchRow(usize),
    ProjectVersionRow(usize),
    ModRow(usize),
    ModSearchRow(usize),
    AccountRow(usize),
    SettingsRow(usize),
    Button(ButtonId),
    /// Modrinth browser: switch the active content type.
    BrowseKindTab(BrowseKind),
    /// Modrinth browser: open a search result.
    BrowseResult(usize),
    /// Modrinth browser: select a version in the detail view.
    BrowseVersion(usize),
    /// Modrinth browser: install the selected version.
    BrowseInstall,
    /// Modrinth browser: quick-install the latest compatible release from the list.
    BrowseQuickInstall(usize),
    /// Modrinth browser: sidebar filter action.
    BrowseFilter(crate::views::browse::FilterItem),
    /// Modrinth browser: go to previous page.
    BrowsePagePrev,
    /// Modrinth browser: go to next page.
    BrowsePageNext,
    /// A click inside a modal overlay.
    Overlay(OverlayAction),
}

/// A clickable region inside a modal overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayAction {
    /// Create-build wizard: switch to a kind tab.
    WizardTab(BuildKind),
    /// Create-build wizard: focus a field row.
    WizardField(usize),
    /// Create-build wizard: open a version picker for a field.
    WizardPick(usize),
    /// Create-build wizard: submit the active form (Create/Import).
    WizardSubmit,
    /// Create-build wizard: open the native file dialog for `.mrpack` import.
    WizardBrowseImport,
    /// Create-build wizard: select a Modrinth search result.
    WizardResult(usize),
    /// Create-build wizard: open the selected Modrinth result.
    WizardResultOpen,
    /// Create-build wizard: select a project version.
    WizardVersion(usize),
    /// Create-build wizard: install the selected project version.
    WizardVersionInstall,
    /// Version picker: select and apply an item.
    PickerItem(usize),
    /// Settings/edit form: focus a field.
    FormField(usize),
    /// Form: submit.
    FormSubmit,
    /// Text prompt: no-op click (keeps input focused).
    TextDone,
    /// Confirmation dialog: accept.
    ConfirmYes,
    /// Confirmation dialog: decline.
    ConfirmNo,
    /// Message dialog: close.
    MessageClose,
}

/// Identifiers for on-screen action buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonId {
    Launch,
    NewInstance,
    EditInstance,
    DeleteInstance,
    InstallInstance,
    RenameInstance,
    ChangeVersion,
    Search,
    ImportModpack,
    InstallProject,
    ToggleMod,
    DeleteMod,
    ModSearch,
    UpdateMods,
    BrowseMods,
    OfflineLogin,
    MicrosoftLogin,
    SetActiveAccount,
    DeleteAccount,
    ChangeSkin,
    ClearLogs,
    PauseLogs,
    FollowLogs,
    AnalyzeCrash,
    SaveSettings,
    EditSettings,
    DetectJava,
}

/// A registered mouse hit region.
#[derive(Debug, Clone, Copy)]
pub struct Hitbox {
    pub rect: Rect,
    pub action: HitAction,
}

/// A short-lived status message.
#[derive(Debug, Clone)]
pub struct Toast {
    pub message: String,
    pub error: bool,
    pub at: Instant,
}

/// Deferred action decoded from a version-picker key press.
enum PickerKey {
    None,
    Cancel,
    Select(PickerTarget, String),
}

/// A running game process and its log pipe.
pub struct RunningProcess {
    pub handle: ProcessHandle,
    pub logs: LogReceiver,
    pub version: String,
    pub started: Instant,
}

/// The complete UI state.
pub struct App {
    pub paths: Paths,
    pub client: reqwest::Client,
    pub theme: Theme,
    pub settings: LauncherSettings,

    pub nav: Nav,
    pub focus: Focus,
    pub should_quit: bool,

    /// Current mouse position, used for hover highlighting.
    pub mouse_pos: Option<(u16, u16)>,
    /// First visible tile row in the instance sidebar.
    pub tile_scroll: usize,
    /// Number of tile columns computed during the last render.
    pub tile_columns: usize,
    /// Sidebar rectangle from the last render (for wheel routing).
    pub sidebar_area: Rect,

    pub tick: u64,
    pub status: String,
    pub toast: Option<Toast>,
    pub progress: Option<(Option<f64>, String)>,
    pub overlay: Option<Overlay>,
    /// A create wizard stashed while a nested version picker is open.
    pub pending_wizard: Option<crate::wizard::CreateWizard>,
    pub hitboxes: Vec<Hitbox>,

    pub instance_manager: InstanceManager,
    pub instances: Vec<Instance>,
    pub instance_state: ListState,

    pub accounts: AccountStore,
    pub account_state: ListState,

    pub modrinth: ModrinthClient,
    pub search_query: String,
    pub search_results: Vec<SearchHit>,
    pub search_state: ListState,
    pub selected_project: Option<Box<Project>>,
    pub project_versions: Vec<Version>,
    pub project_state: ListState,

    pub mod_search_query: String,
    pub mod_search_results: Vec<SearchHit>,
    pub mod_search_state: ListState,
    pub mods_focus_search: bool,

    pub installed_mods: Vec<InstalledMod>,
    pub mods_state: ListState,
    /// A background mod scan is in flight (show a placeholder while empty).
    pub mods_scanning: bool,

    pub resource_packs: Vec<String>,
    pub resource_packs_state: ListState,
    pub shaders: Vec<String>,
    pub shaders_state: ListState,
    pub worlds: Vec<String>,
    pub worlds_state: ListState,
    pub screenshots: Vec<String>,
    pub screenshots_state: ListState,

    pub log_buffer: LogBuffer,
    /// Index of the first visible log line.
    pub log_scroll: usize,
    /// When true the view sticks to the newest line.
    pub log_follow: bool,
    /// Number of visible log rows from the last render.
    pub log_visible: usize,
    pub log_search: String,

    /// Modrinth browser state.
    pub browse: Browse,
    /// Decoded images keyed by URL (icons, gallery previews).
    pub browse_images: HashMap<String, mc_core::img::RgbaImage>,
    /// The page to return to when leaving the browser (not in the sidebar).
    pub browse_return: Nav,
    /// Terminal image protocol + font-size picker, negotiated at startup.
    pub picker: ratatui_image::picker::Picker,
    /// Per-URL render state for the negotiated protocol (kitty/sixel/iterm2/
    /// halfblocks). Each entry caches the encoded image data.
    pub browse_protocols: HashMap<String, ratatui_image::protocol::StatefulProtocol>,

    /// Local file images keyed by absolute path (screenshots, world icons).
    pub local_images: HashMap<String, mc_core::img::RgbaImage>,
    /// Per-path protocol cache for local images.
    pub local_protocols: HashMap<String, ratatui_image::protocol::StatefulProtocol>,

    pub crash_analysis: Option<CrashAnalysis>,

    pub running: Option<RunningProcess>,
    pub last_command: Option<String>,

    pub java_installations: Vec<JavaInstallation>,
    pub settings_field: usize,

    pub engine_tx: EngineSender,
    pub engine_rx: EngineReceiver,
}

impl App {
    /// Create the application, loading persisted state and discovering Java.
    pub async fn new(paths: Paths, client: reqwest::Client) -> anyhow::Result<Self> {
        paths.ensure_layout()?;
        let settings = LauncherSettings::load(&paths).await;
        let instance_manager = InstanceManager::new(paths.clone());
        let instances = instance_manager.list().unwrap_or_default();
        let accounts = AccountStore::load(paths.accounts_file()).await?;
        let (engine_tx, engine_rx) = mpsc::unbounded_channel();

        let mut app = Self {
            client: client.clone(),
            modrinth: ModrinthClient::new(client),
            paths,
            theme: Theme::default(),
            settings,
            nav: Nav::Instances,
            focus: Focus::Content,
            should_quit: false,
            mouse_pos: None,
            tile_scroll: 0,
            tile_columns: 2,
            sidebar_area: Rect::default(),
            tick: 0,
            status: "Ready".to_string(),
            toast: None,
            progress: None,
            overlay: None,
            pending_wizard: None,
            hitboxes: Vec::new(),
            instance_manager,
            instances,
            instance_state: ListState::default(),
            accounts,
            account_state: ListState::default(),
            search_query: String::new(),
            search_results: Vec::new(),
            search_state: ListState::default(),
            selected_project: None,
            project_versions: Vec::new(),
            project_state: ListState::default(),
            mod_search_query: String::new(),
            mod_search_results: Vec::new(),
            mod_search_state: ListState::default(),
            mods_focus_search: false,
            installed_mods: Vec::new(),
            mods_state: ListState::default(),
            mods_scanning: false,

            resource_packs: Vec::new(),
            resource_packs_state: ListState::default(),
            shaders: Vec::new(),
            shaders_state: ListState::default(),
            worlds: Vec::new(),
            worlds_state: ListState::default(),
            screenshots: Vec::new(),
            screenshots_state: ListState::default(),
            log_buffer: LogBuffer::new(5000),
            log_scroll: 0,
            log_follow: true,
            log_visible: 0,
            log_search: String::new(),
            browse: Browse::default(),
            browse_images: HashMap::new(),
            browse_return: Nav::Mods,
            picker: ratatui_image::picker::Picker::halfblocks(),
            browse_protocols: HashMap::new(),
            local_images: HashMap::new(),
            local_protocols: HashMap::new(),
            crash_analysis: None,
            running: None,
            last_command: None,
            java_installations: Vec::new(),
            settings_field: 0,
            engine_tx,
            engine_rx,
        };

        if !app.instances.is_empty() {
            app.instance_state.select(Some(0));
        }
        if !app.accounts.accounts().is_empty() {
            app.account_state.select(Some(0));
        }
        app.log_buffer.filter.min_level = logs::LogLevel::Info;
        app.spawn_java_discovery();
        app.reload_mods();
        Ok(app)
    }

    // ---------------------------------------------------------------------
    // Event loop
    // ---------------------------------------------------------------------

    /// Run the main loop against a terminal until the user quits.
    pub async fn run(
        &mut self,
        terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    ) -> anyhow::Result<()> {
        use futures::StreamExt;
        let mut reader = crossterm::event::EventStream::new();
        let mut ticker = tokio::time::interval(Duration::from_millis(33));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // Coalesce input: only redraw when something changed, and at most once
        // per frame budget. This keeps rapid mouse movement from flooding the
        // terminal and building up a laggy backlog of frames.
        let frame_budget = Duration::from_millis(16);
        let mut needs_draw = true;
        let mut last_draw = Instant::now() - frame_budget;

        while !self.should_quit {
            if needs_draw && last_draw.elapsed() >= frame_budget {
                terminal.draw(|frame| self.render(frame))?;
                last_draw = Instant::now();
                needs_draw = false;
            }

            tokio::select! {
                maybe_event = reader.next() => {
                    if let Some(Ok(event)) = maybe_event {
                        if self.handle_terminal_event(event) {
                            needs_draw = true;
                        }
                    }
                }
                maybe_msg = self.engine_rx.recv() => {
                    if let Some(msg) = maybe_msg {
                        self.handle_engine_event(msg);
                        needs_draw = true;
                    }
                }
                _ = ticker.tick() => {
                    self.on_tick();
                    needs_draw = true;
                }
            }
        }
        Ok(())
    }

    /// Handle a terminal event, returning whether the UI needs a redraw.
    pub(crate) fn handle_terminal_event(&mut self, event: crossterm::event::Event) -> bool {
        match event {
            crossterm::event::Event::Key(key) => {
                if key.kind == crossterm::event::KeyEventKind::Press {
                    self.handle_key(key);
                    true
                } else {
                    false
                }
            }
            crossterm::event::Event::Mouse(mouse) => self.handle_mouse(mouse),
            crossterm::event::Event::Resize(_, _) => true,
            crossterm::event::Event::FocusGained => true,
            _ => false,
        }
    }

    /// Periodic work; always triggers a redraw for spinner animations.
    pub(crate) fn on_tick(&mut self) -> bool {
        self.tick = self.tick.wrapping_add(1);
        self.drain_process();
        if let Some(toast) = &self.toast {
            if toast.at.elapsed() > Duration::from_secs(6) {
                self.toast = None;
            }
        }
        true
    }

    /// Drain process output; returns whether anything changed.
    pub(crate) fn drain_process(&mut self) -> bool {
        let mut lines = Vec::new();
        let mut exited: Option<Option<i32>> = None;

        if let Some(running) = &mut self.running {
            while let Some(line) = running.logs.try_recv() {
                lines.push(line.text);
            }
            match running.handle.try_wait() {
                Ok(Some(status)) => exited = Some(status.code()),
                Ok(None) => {}
                Err(_) => exited = Some(None),
            }
        }

        let had_lines = !lines.is_empty();
        for line in lines {
            self.log_buffer.push_line(&line);
        }

        let mut changed = had_lines;
        if let Some(code) = exited {
            let version = self
                .running
                .as_ref()
                .map(|r| r.version.clone())
                .unwrap_or_default();
            self.running = None;
            self.progress = None;
            self.on_process_exit(&version, code);
            changed = true;
        }

        changed
    }

    pub(crate) fn on_process_exit(&mut self, version: &str, code: Option<i32>) {
        if code == Some(0) {
            self.set_toast(format!("Game exited normally ({version})"), false);
            return;
        }
        // Non-zero exit: look for a crash report and analyze it.
        if let Some(instance) = self.selected_instance() {
            match logs::analyze_latest(instance) {
                Ok(Some(analysis)) => {
                    let headline = analysis.headline.clone();
                    self.crash_analysis = Some(analysis);
                    self.set_toast(
                        format!("Game crashed ({version}): {headline} — press 'a' in Logs"),
                        true,
                    );
                }
                _ => {
                    self.set_toast(
                        format!(
                            "Game exited with code {} ({version})",
                            code.map_or("?".into(), |c| c.to_string())
                        ),
                        true,
                    );
                }
            }
        }
    }

    pub(crate) fn handle_engine_event(&mut self, event: EngineEvent) {
        match event {
            EngineEvent::Status(message) => self.status = message,
            EngineEvent::Progress { ratio, label } => {
                self.progress = Some((ratio, label));
            }
            EngineEvent::ProgressDone => self.progress = None,
            EngineEvent::LogLines(lines) => {
                for line in lines {
                    self.log_buffer.push_line(&line);
                }
            }
            EngineEvent::Started {
                command,
                version,
                handle,
                logs,
            } => {
                self.last_command = Some(command);
                self.log_buffer.clear();
                self.progress = None;
                self.running = Some(RunningProcess {
                    handle: *handle,
                    logs: *logs,
                    version: version.clone(),
                    started: Instant::now(),
                });
                self.set_toast(format!("Launched {version}"), false);
                self.nav = Nav::Logs;
            }
            EngineEvent::InstancesChanged => {
                self.reload_instances();
                self.reload_mods();
            }
            EngineEvent::ImportPathPicked(Some(path)) => {
                self.import_modpack(std::path::PathBuf::from(path.trim()));
            }
            EngineEvent::ImportPathPicked(None) => {
                // Dialog unavailable or cancelled: fall back to manual entry.
                self.open_import_prompt();
            }
            EngineEvent::AccountsChanged => {
                self.reload_accounts();
            }
            EngineEvent::AccountsReloaded(store) => {
                self.accounts = store;
                if self.accounts.accounts().is_empty() {
                    self.account_state.select(None);
                } else if self.account_state.selected().unwrap_or(0)
                    >= self.accounts.accounts().len()
                {
                    self.account_state
                        .select(Some(self.accounts.accounts().len() - 1));
                } else if self.account_state.selected().is_none() {
                    self.account_state.select(Some(0));
                }
            }
            EngineEvent::Message(lines) => {
                self.overlay = Some(Overlay::message("Updates", lines));
            }
            EngineEvent::SearchResults(results) => {
                self.search_results = results.hits;
                self.search_state.select(if self.search_results.is_empty() {
                    None
                } else {
                    Some(0)
                });
            }
            EngineEvent::ModSearchResults(results) => {
                self.mod_search_results = results.hits;
                self.mods_focus_search = true;
                self.mod_search_state
                    .select(if self.mod_search_results.is_empty() {
                        None
                    } else {
                        Some(0)
                    });
            }
            EngineEvent::BrowseResults(results) => {
                self.browse.total = results.total_hits;
                self.browse.results = results.hits;
                self.browse.loading = false;
                if self.browse.selected >= self.browse.results.len() {
                    self.browse.selected = self.browse.results.len().saturating_sub(1);
                }
            }
            EngineEvent::BrowseProject { project, versions } => {
                self.browse.detail = Some(*project);
                self.browse.versions = versions;
                self.browse.version_selected = 0;
                self.browse.focus = BrowseFocus::Body;
                self.browse.body = Vec::new();
                self.browse.body_for = String::new();
                self.browse.body_scroll = 0;
                // Fresh render state per project: the terminal drops the image
                // data once its unicode placeholders are gone, so a cached
                // kitty protocol would not re-transmit on a later open.
                self.browse_protocols.clear();
                self.browse_fetch_images();
            }
            EngineEvent::BrowseImage { url, data } => {
                if let Some(data) = data {
                    if let Ok(img) = mc_core::img::decode_image(&data) {
                        self.browse_images.insert(url, img);
                    } else if let Ok(dynamic) = image::load_from_memory(&data) {
                        let rgba = dynamic.to_rgba8();
                        let w = rgba.width();
                        let h = rgba.height();
                        self.browse_images.insert(
                            url,
                            mc_core::img::RgbaImage {
                                width: w,
                                height: h,
                                pixels: rgba.into_raw(),
                            },
                        );
                    }
                }
            }
            EngineEvent::VersionList { target, versions } => {
                self.show_version_picker(target, versions);
            }
            EngineEvent::WizardSearch(results) => {
                if let Some(mut wizard) = self.pending_wizard.take() {
                    wizard.results = results.hits;
                    wizard.selected = 0;
                    wizard.step = crate::wizard::WizardStep::ModrinthSearch;
                    self.overlay = Some(Overlay::Wizard(wizard));
                }
            }
            EngineEvent::WizardProject { project, versions } => {
                if let Some(mut wizard) = self.pending_wizard.take() {
                    wizard.project = Some(*project);
                    wizard.project_versions = versions;
                    wizard.selected = 0;
                    wizard.step = crate::wizard::WizardStep::ModrinthProject;
                    self.overlay = Some(Overlay::Wizard(wizard));
                }
            }
            EngineEvent::Project { project, versions } => {
                self.selected_project = Some(project);
                self.project_versions = versions;
                self.project_state
                    .select(if self.project_versions.is_empty() {
                        None
                    } else {
                        Some(0)
                    });
            }
            EngineEvent::ModsChanged => {
                self.reload_mods();
            }
            EngineEvent::InstalledMods(mods) => {
                self.installed_mods = mods;
                self.mods_scanning = false;
                if self.installed_mods.is_empty() {
                    self.mods_state.select(None);
                } else if self.mods_state.selected().is_none() {
                    self.mods_state.select(Some(0));
                }
            }
            EngineEvent::Crash(analysis) => {
                self.crash_analysis = analysis;
            }
            EngineEvent::DeviceCode(prompt) => {
                self.overlay = Some(Overlay::DeviceCode(prompt));
            }
            EngineEvent::Authenticated(account) => {
                self.overlay = None;
                let account = *account;
                let name = account.username.clone();
                tokio::spawn({
                    let paths = self.paths.clone();
                    let tx = self.engine_tx.clone();
                    async move {
                        if let Ok(mut store) = AccountStore::load(paths.accounts_file()).await {
                            let _ = store.upsert(account).await;
                        }
                        let _ = tx.send(EngineEvent::AccountsChanged);
                        let _ = tx.send(EngineEvent::Toast(format!("Signed in as {name}")));
                    }
                });
            }
            EngineEvent::Java(list) => {
                self.java_installations = list;
            }
            EngineEvent::Toast(message) => self.set_toast(message, false),
            EngineEvent::Error(message) => self.set_toast(message, true),
            EngineEvent::ResourcePacks(items) => {
                self.resource_packs = items;
                if self.resource_packs_state.selected().is_none() && !self.resource_packs.is_empty() {
                    self.resource_packs_state.select(Some(0));
                }
            }
            EngineEvent::ShaderPacks(items) => {
                self.shaders = items;
                if self.shaders_state.selected().is_none() && !self.shaders.is_empty() {
                    self.shaders_state.select(Some(0));
                }
            }
            EngineEvent::Worlds(items) => {
                self.worlds = items;
                if self.worlds_state.selected().is_none() && !self.worlds.is_empty() {
                    self.worlds_state.select(Some(0));
                }
            }
            EngineEvent::Screenshots(items) => {
                self.screenshots = items;
                if self.screenshots_state.selected().is_none() && !self.screenshots.is_empty() {
                    self.screenshots_state.select(Some(0));
                }
            }
            EngineEvent::LocalImage { path, img } => {
                if let Some(img) = img {
                    self.local_images.insert(path, img);
                }
            }
        }
    }

    // ---------------------------------------------------------------------
    // Input
    // ---------------------------------------------------------------------

    pub(crate) fn handle_key(&mut self, key: KeyEvent) {
        if self.overlay.is_some() {
            self.handle_overlay_key(key);
            return;
        }

        match key.code {
            KeyCode::Char('q') if key.modifiers.is_empty() => self.request_quit(),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            KeyCode::Char('?') => self.show_help(),
            KeyCode::Tab => self.toggle_focus(),
            KeyCode::BackTab => self.toggle_focus(),
            KeyCode::F(2) => self.open_nav(Nav::Accounts),
            KeyCode::F(3) => self.open_nav(Nav::Launcher),
            KeyCode::Char(c @ '1'..='5') => {
                let idx = (c as u8 - b'1') as usize;
                if let Some(nav) = Nav::menu().get(idx) {
                    self.open_nav(*nav);
                }
            }
            KeyCode::Char('n') if self.nav == Nav::Instances => self.open_create_instance_form(),
            _ => self.handle_view_key(key),
        }
    }

    pub(crate) fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Sidebar => Focus::Content,
            Focus::Content => Focus::Sidebar,
        };
    }

    /// Switch the active navigation page.
    pub(crate) fn open_nav(&mut self, nav: Nav) {
        // Remember where the browser was entered from; it has no sidebar entry.
        if nav == Nav::Browse {
            self.browse_return = self.nav;
        }
        self.nav = nav;
        self.focus = Focus::Content;
        match nav {
            Nav::Instances => self.reload_instances(),
            Nav::Browse => self.open_browse(),
            Nav::Mods => self.reload_mods(),
            Nav::Accounts => self.reload_accounts(),
            Nav::Logs if self.running.is_none() && self.log_buffer.is_empty() => {
                self.load_latest_log();
            }
            Nav::ResourcePacks => self.reload_resource_packs(),
            Nav::Shaders => self.reload_shaders(),
            Nav::Worlds => self.reload_worlds(),
            Nav::Screenshots => self.reload_screenshots(),
            _ => {}
        }
    }

    pub(crate) fn handle_view_key(&mut self, key: KeyEvent) {
        if self.focus == Focus::Sidebar {
            self.handle_sidebar_key(key);
            return;
        }
        // Esc returns to the instance picker (Modpacks handles its own Esc).
        if key.code == KeyCode::Esc && self.nav != Nav::Modpacks {
            // In the browser, Esc first closes the project detail page, then
            // returns to the page it was entered from.
            if self.nav == Nav::Browse {
                if self.browse.in_detail() {
                    self.browse_close_detail();
                } else {
                    self.open_nav(self.browse_return);
                }
                return;
            }
            self.open_nav(Nav::Instances);
            return;
        }
        match self.nav {
            Nav::Instances => self.key_instance_grid(key),
            Nav::Browse => self.key_browse(key),
            Nav::Mods => self.key_mods(key),
            Nav::Modpacks => self.key_modpacks(key),
            Nav::Versions => self.key_versions(key),
            Nav::Jvm => self.key_instance_settings(key),
            Nav::Logs => self.key_logs(key),
            Nav::Accounts => self.key_accounts(key),
            Nav::Launcher => self.key_settings(key),
            Nav::ResourcePacks => self.key_resource_packs(key),
            Nav::Shaders => self.key_shaders(key),
            Nav::Worlds => self.key_worlds(key),
            Nav::Screenshots => self.key_screenshots(key),
        }
    }

    pub(crate) fn handle_sidebar_key(&mut self, key: KeyEvent) {
        let all = Nav::menu();
        let idx = all.iter().position(|n| *n == self.nav).unwrap_or(0);
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                let next = (idx + 1).min(all.len() - 1);
                self.open_nav(all[next]);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let prev = idx.saturating_sub(1);
                self.open_nav(all[prev]);
            }
            KeyCode::Enter => self.focus = Focus::Content,
            _ => {}
        }
    }

    /// Handle a mouse event; returns whether the UI needs a redraw.
    ///
    /// Motion events only trigger a redraw when the hovered element actually
    /// changes, which prevents a flood of redraws while the pointer moves.
    pub(crate) fn handle_mouse(&mut self, mouse: MouseEvent) -> bool {
        let new_pos = (mouse.column, mouse.row);
        let old_action = self.mouse_pos.and_then(|pos| self.hit_action_at(pos));
        self.mouse_pos = Some(new_pos);
        let new_action = self.hit_action_at(new_pos);
        let hover_changed = old_action != new_action;

        if self.overlay.is_some() {
            let result = match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    if let Some(HitAction::Overlay(action)) = self.hit_action_at(new_pos) {
                        self.dispatch_overlay_action(action);
                    } else {
                        self.handle_overlay_click();
                    }
                    true
                }
                MouseEventKind::ScrollDown => {
                    self.overlay_scroll(1);
                    true
                }
                MouseEventKind::ScrollUp => {
                    self.overlay_scroll(-1);
                    true
                }
                _ => hover_changed,
            };
            return result;
        }

        match mouse.kind {
            MouseEventKind::ScrollDown => {
                self.scroll_active(1);
                true
            }
            MouseEventKind::ScrollUp => {
                self.scroll_active(-1);
                true
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(hit) = self.hit_action_at(new_pos) {
                    self.dispatch_hit(hit);
                    true
                } else {
                    hover_changed
                }
            }
            _ => hover_changed,
        }
    }

    /// The hitbox action under a screen position, if any.
    pub(crate) fn hit_action_at(&self, pos: (u16, u16)) -> Option<HitAction> {
        self.hitboxes
            .iter()
            .find(|h| rect_contains(h.rect, pos))
            .map(|h| h.action)
    }

    pub(crate) fn dispatch_hit(&mut self, action: HitAction) {
        match action {
            HitAction::NavItem(nav) => self.open_nav(nav),
            HitAction::InstanceTile(idx) => self.select_instance(idx),
            HitAction::AddInstance => self.open_create_instance_form(),
            HitAction::SearchRow(idx) => {
                self.search_state.select(Some(idx));
                self.focus = Focus::Content;
            }
            HitAction::ProjectVersionRow(idx) => {
                self.project_state.select(Some(idx));
                self.focus = Focus::Content;
            }
            HitAction::ModRow(idx) => {
                self.mods_state.select(Some(idx));
                self.focus = Focus::Content;
            }
            HitAction::ModSearchRow(idx) => {
                self.mod_search_state.select(Some(idx));
                self.focus = Focus::Content;
            }
            HitAction::AccountRow(idx) => {
                self.account_state.select(Some(idx));
                self.focus = Focus::Content;
            }
            HitAction::SettingsRow(idx) => {
                self.settings_field = idx;
                self.focus = Focus::Content;
            }
            HitAction::Button(button) => self.dispatch_button(button),
            HitAction::BrowseKindTab(kind) => self.browse_switch_kind(kind),
            HitAction::BrowseResult(idx) => self.browse_select_result(idx),
            HitAction::BrowseVersion(idx) => self.browse_select_version(idx),
            HitAction::BrowseInstall => self.browse_install(),
            HitAction::BrowseQuickInstall(idx) => self.browse_quick_install(idx),
            HitAction::BrowseFilter(item) => self.browse_filter_click(item),
            HitAction::BrowsePagePrev => self.browse_prev_page(),
            HitAction::BrowsePageNext => self.browse_next_page(),
            HitAction::Overlay(action) => self.dispatch_overlay_action(action),
        }
    }

    /// Select an instance from the tile grid.
    pub(crate) fn select_instance(&mut self, idx: usize) {
        if idx >= self.instances.len() {
            return;
        }
        self.instance_state.select(Some(idx));
        self.focus = Focus::Content;
        self.reload_mods();
    }

    pub(crate) fn dispatch_button(&mut self, button: ButtonId) {
        match button {
            ButtonId::Launch => self.launch_selected(),
            ButtonId::NewInstance => self.open_create_instance_form(),
            ButtonId::EditInstance => self.open_edit_instance_form(),
            ButtonId::DeleteInstance => self.confirm_delete_instance(),
            ButtonId::InstallInstance => self.install_selected_instance(),
            ButtonId::ChangeVersion => self.open_change_version_picker(),
            ButtonId::RenameInstance => self.open_rename_instance_form(),
            ButtonId::Search => self.open_search_prompt(),
            ButtonId::ImportModpack => self.browse_for_mrpack(),
            ButtonId::InstallProject => self.install_selected_project(),
            ButtonId::ToggleMod => self.toggle_selected_mod(),
            ButtonId::DeleteMod => self.confirm_delete_mod(),
            ButtonId::ModSearch => self.open_mod_search_prompt(),
            ButtonId::UpdateMods => self.check_mod_updates(),
            ButtonId::BrowseMods => self.open_nav(Nav::Browse),
            ButtonId::OfflineLogin => self.open_offline_login(),
            ButtonId::MicrosoftLogin => self.start_microsoft_login(),
            ButtonId::SetActiveAccount => self.set_active_account(),
            ButtonId::DeleteAccount => self.confirm_delete_account(),
            ButtonId::ChangeSkin => self.open_skin_prompt(),
            ButtonId::ClearLogs => self.log_buffer.clear(),
            ButtonId::PauseLogs => self.log_buffer.paused = !self.log_buffer.paused,
            ButtonId::FollowLogs => self.log_follow = !self.log_follow,
            ButtonId::AnalyzeCrash => self.analyze_crash(),
            ButtonId::SaveSettings => self.save_settings(),
            ButtonId::EditSettings => self.open_settings_form(),
            ButtonId::DetectJava => self.spawn_java_discovery(),
        }
    }

    pub(crate) fn scroll_active(&mut self, delta: i32) {
        if self.overlay.is_some() {
            return;
        }
        match self.nav {
            Nav::Instances => self.scroll_tiles(delta),
            Nav::Browse => self.browse_scroll(delta),
            Nav::Versions => {}
            Nav::Modpacks => {
                let step = delta * 4;
                if self.selected_project.is_some() {
                    move_selection(&mut self.project_state, self.project_versions.len(), step);
                } else {
                    move_selection(&mut self.search_state, self.search_results.len(), step);
                }
            }
            Nav::Mods => {
                let step = delta * 4;
                if self.mods_focus_search {
                    move_selection(
                        &mut self.mod_search_state,
                        self.mod_search_results.len(),
                        step,
                    );
                } else {
                    move_selection(&mut self.mods_state, self.installed_mods.len(), step);
                }
            }
            Nav::Logs => self.scroll_logs(delta * 3),
            Nav::Jvm => {
                let len = settings_field_count();
                let next = (self.settings_field as i32 + delta).clamp(0, len as i32 - 1) as usize;
                self.settings_field = next;
            }
            Nav::Accounts => move_selection(
                &mut self.account_state,
                self.accounts.accounts().len(),
                delta * 4,
            ),
            Nav::Launcher => {
                let len = settings_field_count();
                let next = (self.settings_field as i32 + delta).clamp(0, len as i32 - 1) as usize;
                self.settings_field = next;
            }
            Nav::ResourcePacks | Nav::Shaders | Nav::Worlds | Nav::Screenshots => {}
        }
    }

    fn scroll_tiles(&mut self, delta: i32) {
        let rows = self.tile_rows();
        let next = (self.tile_scroll as i32 + delta).clamp(0, rows.saturating_sub(1) as i32);
        self.tile_scroll = next as usize;
    }

    /// Scroll the log viewport by `delta` lines (positive = towards newest).
    pub(crate) fn scroll_logs(&mut self, delta: i32) {
        let total = self.log_buffer.visible().count();
        let visible = self.log_visible.max(1);
        let max_scroll = total.saturating_sub(visible);
        let base = if self.log_follow {
            max_scroll
        } else {
            self.log_scroll
        };
        let next = (base as i32 + delta).clamp(0, max_scroll as i32) as usize;
        self.log_scroll = next;
        self.log_follow = next >= max_scroll;
    }

    /// Jump the log viewport to the very top or bottom.
    pub(crate) fn jump_logs(&mut self, to_end: bool) {
        if to_end {
            self.log_follow = true;
        } else {
            self.log_scroll = 0;
            self.log_follow = false;
        }
    }

    /// Total number of tile rows for the current instance count.
    pub(crate) fn tile_rows(&self) -> usize {
        let cols = self.tile_columns.max(1);
        self.instances.len().div_ceil(cols)
    }

    /// Ensure the selected tile is within the visible window.
    pub(crate) fn ensure_tile_visible(&mut self, visible_rows: usize) {
        if visible_rows == 0 {
            return;
        }
        let cols = self.tile_columns.max(1);
        let selected_row = self.instance_state.selected().unwrap_or(0) / cols;
        if selected_row < self.tile_scroll {
            self.tile_scroll = selected_row;
        } else if selected_row >= self.tile_scroll + visible_rows {
            self.tile_scroll = selected_row + 1 - visible_rows;
        }
        let max_scroll = self.tile_rows().saturating_sub(visible_rows);
        self.tile_scroll = self.tile_scroll.min(max_scroll);
    }

    // ---------------------------------------------------------------------
    // Overlay input
    // ---------------------------------------------------------------------

    pub(crate) fn handle_overlay_key(&mut self, key: KeyEvent) {
        let Some(overlay) = self.overlay.as_mut() else {
            return;
        };
        match overlay {
            Overlay::Text { value, action, .. } => match key.code {
                KeyCode::Esc => self.overlay = None,
                KeyCode::Enter => {
                    let text = value.clone();
                    let action = action.clone();
                    self.overlay = None;
                    self.submit_text(action, text);
                }
                KeyCode::Backspace => {
                    value.pop();
                }
                KeyCode::Char(c) => value.push(c),
                _ => {}
            },
            Overlay::Form(form) => match key.code {
                KeyCode::Esc => self.overlay = None,
                KeyCode::Enter => {
                    let form = form.clone();
                    self.overlay = None;
                    self.submit_form(form);
                }
                KeyCode::Tab | KeyCode::Down => form.next_field(),
                KeyCode::BackTab | KeyCode::Up => form.prev_field(),
                KeyCode::Left => form.cycle(false),
                KeyCode::Right => form.cycle(true),
                KeyCode::Char(' ') => form.cycle(true),
                KeyCode::Backspace => form.backspace(),
                KeyCode::Char(c) => form.input_char(c),
                _ => {}
            },
            Overlay::Confirm { action, .. } => {
                let action = action.clone();
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                        self.overlay = None;
                        self.confirm(action);
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => self.overlay = None,
                    _ => {}
                }
            }
            Overlay::Message { .. } => {
                if matches!(key.code, KeyCode::Enter | KeyCode::Esc) {
                    self.overlay = None;
                }
            }
            Overlay::DeviceCode(_) => {
                if key.code == KeyCode::Esc {
                    self.overlay = None;
                }
            }
            Overlay::Picker(_) => self.handle_picker_key(key),
            Overlay::Wizard(_) => self.handle_wizard_key(key),
        }
    }

    pub(crate) fn handle_picker_key(&mut self, key: KeyEvent) {
        let action = {
            let Some(Overlay::Picker(picker)) = self.overlay.as_mut() else {
                return;
            };
            match key.code {
                KeyCode::Esc => PickerKey::Cancel,
                KeyCode::Down | KeyCode::Char('j') => {
                    picker.move_selection(1);
                    PickerKey::None
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    picker.move_selection(-1);
                    PickerKey::None
                }
                KeyCode::Backspace => {
                    picker.query.pop();
                    picker.refilter();
                    PickerKey::None
                }
                KeyCode::Char(c) => {
                    picker.query.push(c);
                    picker.refilter();
                    PickerKey::None
                }
                KeyCode::Enter => match picker.selected_value().map(str::to_string) {
                    Some(value) => PickerKey::Select(picker.target.clone(), value),
                    None => PickerKey::None,
                },
                _ => PickerKey::None,
            }
        };
        match action {
            PickerKey::None => {}
            PickerKey::Cancel => self.cancel_picker(),
            PickerKey::Select(target, value) => {
                self.overlay = None;
                self.apply_picker_value(target, value);
            }
        }
    }

    pub(crate) fn handle_overlay_click(&mut self) {
        // Clicking anywhere on a confirm/message overlay acts as "accept".
        match self.overlay.as_ref() {
            Some(Overlay::Confirm { action, .. }) => {
                let action = action.clone();
                self.overlay = None;
                self.confirm(action);
            }
            Some(Overlay::Message { .. }) => self.overlay = None,
            _ => {}
        }
    }

    /// Route a click on an overlay hitbox.
    pub(crate) fn dispatch_overlay_action(&mut self, action: OverlayAction) {
        match action {
            OverlayAction::WizardTab(kind) => self.wizard_set_kind(kind),
            OverlayAction::WizardField(idx) => self.wizard_focus_field(idx),
            OverlayAction::WizardPick(idx) => self.wizard_pick_field(idx),
            OverlayAction::WizardSubmit => self.wizard_submit_clicked(),
            OverlayAction::WizardBrowseImport => self.browse_for_mrpack(),
            OverlayAction::WizardResult(idx) => self.wizard_click_result(idx),
            OverlayAction::WizardResultOpen => self.wizard_open_selected_project(),
            OverlayAction::WizardVersion(idx) => self.wizard_select_version(idx),
            OverlayAction::WizardVersionInstall => self.wizard_install_selected_version(),
            OverlayAction::PickerItem(idx) => self.picker_select(idx),
            OverlayAction::FormField(idx) => {
                let Some(Overlay::Form(form)) = self.overlay.as_mut() else {
                    return;
                };
                form.active = idx;
            }
            OverlayAction::FormSubmit => {
                let Some(Overlay::Form(form)) = self.overlay.clone() else {
                    return;
                };
                self.overlay = None;
                self.submit_form(form);
            }
            OverlayAction::TextDone => {}
            OverlayAction::ConfirmYes => {
                let Some(Overlay::Confirm { action, .. }) = self.overlay.clone() else {
                    return;
                };
                self.overlay = None;
                self.confirm(action);
            }
            OverlayAction::ConfirmNo => self.overlay = None,
            OverlayAction::MessageClose => self.overlay = None,
        }
    }

    /// Scroll inside the active overlay (picker / wizard lists).
    pub(crate) fn overlay_scroll(&mut self, delta: i32) {
        match self.overlay.as_mut() {
            Some(Overlay::Picker(picker)) => picker.move_selection(delta * 4),
            Some(Overlay::Wizard(wizard)) => wizard.move_selection(delta * 4),
            _ => {}
        }
    }

    /// Select and apply a version-picker item.
    pub(crate) fn picker_select(&mut self, idx: usize) {
        let Some(Overlay::Picker(picker)) = self.overlay.as_mut() else {
            return;
        };
        picker.selected = idx;
        let Some(value) = picker.selected_value().map(str::to_string) else {
            return;
        };
        let target = picker.target.clone();
        self.overlay = None;
        self.apply_picker_value(target, value);
    }

    pub(crate) fn submit_text(&mut self, action: TextAction, text: String) {
        match action {
            TextAction::SearchModrinth => self.run_search(text),
            TextAction::SearchMods => self.run_mod_search(text),
            TextAction::BrowseSearch => self.run_browse_search(text),
            TextAction::SearchLogs => {
                self.log_search = text.clone();
                self.log_buffer.filter.search = (!text.is_empty()).then_some(text);
            }
            TextAction::ImportPath => self.import_modpack(PathBuf::from(text)),
            TextAction::JavaPath => {
                let path = PathBuf::from(text.trim());
                if !path.as_os_str().is_empty() {
                    self.settings.java_path = Some(path);
                    self.save_settings_async();
                }
            }
            TextAction::CustomJvmArgs => {
                if let Some(instance) = self.selected_instance().cloned() {
                    self.update_instance_jvm(instance.id(), |jvm| {
                        jvm.custom_jvm_args = split_args(&text);
                    });
                }
            }
            TextAction::CustomGameArgs => {
                if let Some(instance) = self.selected_instance().cloned() {
                    self.update_instance_jvm(instance.id(), |jvm| {
                        jvm.extra_game_args = split_args(&text);
                    });
                }
            }
            TextAction::SkinUrl => self.change_skin(text),
            TextAction::RenameInstance => self.rename_instance(text),
            TextAction::None => {}
        }
    }

    pub(crate) fn submit_form(&mut self, form: Form) {
        match form.action {
            FormAction::CreateInstance => self.open_create_wizard(),
            FormAction::EditInstanceSettings => self.save_instance_settings_from_form(&form),
            FormAction::ImportModpack => {
                if let Some(path) = form.text_value("Archive") {
                    self.import_modpack(PathBuf::from(path.trim()));
                }
            }
            FormAction::OfflineLogin => {
                if let Some(name) = form.text_value("Username") {
                    self.offline_login(name.trim());
                }
            }
            FormAction::SetJavaPath => {
                if let Some(path) = form.text_value("Java Path") {
                    self.settings.java_path = Some(PathBuf::from(path.trim()));
                    self.save_settings_async();
                }
            }
            FormAction::EditLauncherSettings => self.apply_launcher_settings_form(&form),
        }
    }

    // ---------------------------------------------------------------------
    // Actions: instances
    // ---------------------------------------------------------------------

    pub fn selected_instance(&self) -> Option<&Instance> {
        self.instance_state
            .selected()
            .and_then(|idx| self.instances.get(idx))
    }

    pub(crate) fn reload_instances(&mut self) {
        self.instances = self.instance_manager.list().unwrap_or_default();
        if self.instances.is_empty() {
            self.instance_state.select(None);
        } else if self.instance_state.selected().unwrap_or(0) >= self.instances.len() {
            self.instance_state.select(Some(self.instances.len() - 1));
        } else if self.instance_state.selected().is_none() {
            self.instance_state.select(Some(0));
        }
    }

    pub(crate) fn open_create_instance_form(&mut self) {
        self.open_create_wizard();
    }

    /// Create an instance and install its loader in the background.
    pub(crate) fn create_instance_async(
        &mut self,
        name: String,
        game_version: String,
        loader: LoaderType,
        loader_version: Option<String>,
    ) {
        let manager = self.instance_manager.clone();
        let installer = Installer::new(
            self.client.clone(),
            self.paths.clone(),
            self.progress_callback(),
        );
        let tx = self.engine_tx.clone();
        let min = self.settings.default_min_memory_mb;
        let max = self.settings.default_max_memory_mb;
        let gc = self.settings.default_gc;
        self.progress = Some((None, format!("Creating {name}")));

        tokio::spawn(async move {
            let result: Result<Instance, CoreError> = async {
                let mut instance = manager
                    .create(&name, &game_version, loader, loader_version.clone())
                    .await?;
                instance.metadata.jvm.min_memory_mb = min;
                instance.metadata.jvm.max_memory_mb = max;
                instance.metadata.jvm.gc = gc;
                instance.save().await?;

                let _ = tx.send(EngineEvent::Status(format!(
                    "Installing {} ...",
                    instance.metadata.descriptor()
                )));
                installer
                    .install_loader(&game_version, loader, loader_version.as_deref())
                    .await?;
                Ok(instance)
            }
            .await;

            match result {
                Ok(instance) => {
                    let _ = tx.send(EngineEvent::ProgressDone);
                    let _ = tx.send(EngineEvent::InstancesChanged);
                    let _ = tx.send(EngineEvent::Toast(format!("Created {}", instance.name())));
                }
                Err(err) => {
                    let _ = tx.send(EngineEvent::ProgressDone);
                    let _ = tx.send(EngineEvent::Error(format!("Create failed: {err}")));
                }
            }
        });
    }

    /// Fetch the Mojang version list for a picker.
    pub(crate) fn request_game_versions(&mut self) {
        self.fetch_game_versions(PickerTarget::WizardGame);
    }

    /// Open the version picker to change the selected build's game version.
    pub(crate) fn open_change_version_picker(&mut self) {
        self.fetch_game_versions(PickerTarget::ChangeGameVersion);
    }

    fn fetch_game_versions(&mut self, target: PickerTarget) {
        let client = self.client.clone();
        let tx = self.engine_tx.clone();
        self.progress = Some((None, "Loading Minecraft versions...".into()));
        tokio::spawn(async move {
            let result = mc_core::install::common::fetch_manifest(&client).await;
            let _ = tx.send(EngineEvent::ProgressDone);
            match result {
                Ok(manifest) => {
                    let versions = manifest.versions.into_iter().map(|v| v.id).collect();
                    let _ = tx.send(EngineEvent::VersionList { target, versions });
                }
                Err(err) => {
                    let _ = tx.send(EngineEvent::Error(format!("Version list failed: {err}")));
                }
            }
        });
    }

    /// Fetch loader versions for the wizard's current loader/game version.
    pub(crate) fn request_loader_versions(&mut self) {
        let Some(wizard) = self.pending_wizard.as_ref() else {
            return;
        };
        let loader = wizard.loader();
        let game = wizard.game_version.clone();
        let installer = Installer::new(
            self.client.clone(),
            self.paths.clone(),
            self.progress_callback(),
        );
        let tx = self.engine_tx.clone();
        self.progress = Some((None, "Loading loader versions...".into()));
        tokio::spawn(async move {
            let result: Result<Vec<String>, CoreError> = match loader {
                LoaderType::Fabric => {
                    mc_core::install::fabric::available_loaders(&installer, &game).await
                }
                LoaderType::Quilt => {
                    mc_core::install::quilt::available_loaders(&installer, &game).await
                }
                LoaderType::Forge => {
                    mc_core::install::forge::available_loaders(&installer, &game).await
                }
                LoaderType::NeoForge => {
                    mc_core::install::neoforge::available_loaders(&installer, &game).await
                }
                _ => Ok(Vec::new()),
            };
            let _ = tx.send(EngineEvent::ProgressDone);
            match result {
                Ok(versions) => {
                    let _ = tx.send(EngineEvent::VersionList {
                        target: PickerTarget::WizardLoader,
                        versions,
                    });
                }
                Err(err) => {
                    let _ = tx.send(EngineEvent::Error(format!("Loader list failed: {err}")));
                }
            }
        });
    }

    /// Change the selected build's game version and reinstall it.
    pub(crate) fn change_instance_version(&mut self, game_version: String) {
        let Some(instance) = self.selected_instance().cloned() else {
            return;
        };
        let Ok(mut inst) = self.instance_manager.get(instance.id()) else {
            return;
        };
        inst.metadata.game_version = game_version.clone();
        // The previous loader build may not exist for the new version.
        inst.metadata.loader_version = None;
        let loader = inst.metadata.loader;
        let installer = Installer::new(
            self.client.clone(),
            self.paths.clone(),
            self.progress_callback(),
        );
        let tx = self.engine_tx.clone();
        self.progress = Some((None, format!("Reinstalling {} ...", inst.name())));
        tokio::spawn(async move {
            let result: Result<(), CoreError> = async {
                inst.save().await?;
                installer
                    .install_loader(&game_version, loader, None)
                    .await?;
                Ok(())
            }
            .await;
            let _ = tx.send(EngineEvent::ProgressDone);
            match result {
                Ok(()) => {
                    let _ = tx.send(EngineEvent::InstancesChanged);
                    let _ = tx.send(EngineEvent::Toast("Version changed".into()));
                }
                Err(err) => {
                    let _ = tx.send(EngineEvent::Error(format!("Change failed: {err}")));
                }
            }
        });
    }

    /// Install a Modrinth modpack project version by downloading its `.mrpack`.
    pub(crate) fn install_modrinth_modpack(
        &mut self,
        name: String,
        version: mc_core::modrinth::Version,
    ) {
        let Some(file) = version.primary_file().cloned() else {
            self.set_toast("This modpack version has no downloadable file", true);
            return;
        };
        let installer = Installer::new(
            self.client.clone(),
            self.paths.clone(),
            self.progress_callback(),
        );
        let instances = self.instance_manager.clone();
        let paths = self.paths.clone();
        let progress = self.progress_callback();
        let client = self.client.clone();
        let tx = self.engine_tx.clone();
        self.progress = Some((None, format!("Downloading {name}...")));

        tokio::spawn(async move {
            let result: Result<Instance, CoreError> = async {
                let archive = paths.downloads_dir().join(&file.filename);
                mc_core::util::download_file(&client, &file.url, &archive, file.sha1(), None)
                    .await?;
                let importer = ModpackInstaller::new(installer, instances, paths, progress);
                importer.import(&archive, Some(&name)).await
            }
            .await;
            let _ = tx.send(EngineEvent::ProgressDone);
            match result {
                Ok(instance) => {
                    let _ = tx.send(EngineEvent::InstancesChanged);
                    let _ = tx.send(EngineEvent::Toast(format!("Imported {}", instance.name())));
                }
                Err(err) => {
                    let _ = tx.send(EngineEvent::Error(format!("Import failed: {err}")));
                }
            }
        });
    }

    pub(crate) fn open_edit_instance_form(&mut self) {
        let Some(instance) = self.selected_instance().cloned() else {
            self.set_toast("No instance selected", true);
            return;
        };
        let jvm = &instance.metadata.jvm;
        let form = Form::new(
            format!("Edit {}", instance.name()),
            FormAction::EditInstanceSettings,
        )
        .push_number("Min RAM (MB)", jvm.min_memory_mb)
        .push_number("Max RAM (MB)", jvm.max_memory_mb)
        .push_choice("Garbage Collector", gc_options(), gc_index(jvm.gc))
        .push_text(
            "Java Path",
            jvm.java_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
        )
        .with_hint("blank = auto-detect")
        .push_bool("Fullscreen", jvm.fullscreen)
        .push_text("Custom JVM Args", jvm.custom_jvm_args.join(" "))
        .push_text("Extra Game Args", jvm.extra_game_args.join(" "));
        self.overlay = Some(Overlay::Form(form));
    }

    /// Prompt for a new display name for the selected build.
    pub(crate) fn open_rename_instance_form(&mut self) {
        let Some(instance) = self.selected_instance().cloned() else {
            self.set_toast("No instance selected", true);
            return;
        };
        self.overlay = Some(Overlay::text_with(
            "Rename Build",
            "New name: ",
            instance.name().to_string(),
            TextAction::RenameInstance,
        ));
    }

    pub(crate) fn rename_instance(&mut self, new_name: String) {
        let Some(instance) = self.selected_instance().cloned() else {
            return;
        };
        let trimmed = new_name.trim().to_string();
        if trimmed.is_empty() {
            self.set_toast("Name cannot be empty", true);
            return;
        }
        let manager = self.instance_manager.clone();
        let tx = self.engine_tx.clone();
        let id = instance.id().to_string();
        tokio::spawn(async move {
            match manager.rename(&id, &trimmed).await {
                Ok(_) => {
                    let _ = tx.send(EngineEvent::InstancesChanged);
                }
                Err(err) => {
                    let _ = tx.send(EngineEvent::Error(format!("Rename failed: {err}")));
                }
            }
        });
    }

    pub(crate) fn save_instance_settings_from_form(&mut self, form: &Form) {
        let Some(instance) = self.selected_instance().cloned() else {
            return;
        };
        let min = form.number_value("Min RAM (MB)").unwrap_or(512);
        let max = form.number_value("Max RAM (MB)").unwrap_or(4096);
        let gc = parse_gc(form.choice_value("Garbage Collector").unwrap_or("G1GC"));
        let java_path = form
            .text_value("Java Path")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        let fullscreen = form.bool_value("Fullscreen").unwrap_or(false);
        let jvm_args = split_args(form.text_value("Custom JVM Args").unwrap_or(""));
        let game_args = split_args(form.text_value("Extra Game Args").unwrap_or(""));

        self.update_instance_jvm(instance.id(), |jvm| {
            jvm.min_memory_mb = min;
            jvm.max_memory_mb = max;
            jvm.gc = gc;
            jvm.java_path = java_path;
            jvm.fullscreen = fullscreen;
            jvm.custom_jvm_args = jvm_args;
            jvm.extra_game_args = game_args;
        });
    }

    pub(crate) fn update_instance_jvm<F>(&mut self, id: &str, mutate: F)
    where
        F: FnOnce(&mut JvmConfig),
    {
        let Ok(mut instance) = self.instance_manager.get(id) else {
            return;
        };
        mutate(&mut instance.metadata.jvm);
        let tx = self.engine_tx.clone();
        tokio::spawn(async move {
            if instance.save().await.is_ok() {
                let _ = tx.send(EngineEvent::InstancesChanged);
                let _ = tx.send(EngineEvent::Toast("Instance updated".into()));
            } else {
                let _ = tx.send(EngineEvent::Error("Failed to save instance".into()));
            }
        });
    }

    pub(crate) fn confirm_delete_instance(&mut self) {
        let Some(instance) = self.selected_instance().cloned() else {
            return;
        };
        self.overlay = Some(Overlay::confirm(
            "Delete Instance",
            format!(
                "Delete '{}' and all of its files? This cannot be undone.",
                instance.name()
            ),
            ConfirmAction::DeleteInstance(instance.id().to_string()),
        ));
    }

    pub(crate) fn install_selected_instance(&mut self) {
        let Some(instance) = self.selected_instance().cloned() else {
            self.set_toast("No instance selected", true);
            return;
        };
        let installer = Installer::new(
            self.client.clone(),
            self.paths.clone(),
            self.progress_callback(),
        );
        let tx = self.engine_tx.clone();
        self.progress = Some((None, format!("Installing {}", instance.name())));
        tokio::spawn(async move {
            let result = installer
                .install_loader(
                    &instance.metadata.game_version,
                    instance.metadata.loader,
                    instance.metadata.loader_version.as_deref(),
                )
                .await;
            let _ = tx.send(EngineEvent::ProgressDone);
            match result {
                Ok(version) => {
                    let _ = tx.send(EngineEvent::Toast(format!("Installed {}", version.id)));
                }
                Err(err) => {
                    let _ = tx.send(EngineEvent::Error(format!("Install failed: {err}")));
                }
            }
        });
    }

    pub(crate) fn launch_selected(&mut self) {
        let Some(instance) = self.selected_instance().cloned() else {
            self.set_toast("No instance selected", true);
            return;
        };
        let Some(account) = self.accounts.active().cloned() else {
            self.set_toast("No account selected — add one in Accounts", true);
            return;
        };
        if self.running.is_some() {
            self.set_toast("A game is already running", true);
            return;
        }

        let client = self.client.clone();
        let paths = self.paths.clone();
        let tx = self.engine_tx.clone();
        let progress = self.progress_callback();
        self.progress = Some((None, format!("Preparing {}", instance.name())));

        tokio::spawn(async move {
            let result: Result<_, CoreError> = async {
                // Refresh an expired Microsoft token before launching.
                let account = if account.kind == AccountKind::Microsoft
                    && !account.token_valid(120)
                    && account.refresh_token.is_some()
                {
                    let auth = MicrosoftAuth::new(client.clone());
                    match auth.refresh_account(&account).await {
                        Ok(updated) => {
                            let _ = tx.send(EngineEvent::Authenticated(Box::new(updated.clone())));
                            updated
                        }
                        Err(_) => account,
                    }
                } else {
                    account
                };

                let installer = Installer::new(client.clone(), paths.clone(), progress.clone());
                let resolved = match resolve_instance_version(&paths, &instance).await {
                    Ok(resolved) => resolved,
                    Err(_) => {
                        let _ = tx.send(EngineEvent::Status(format!(
                            "Installing {} ...",
                            instance.metadata.descriptor()
                        )));
                        installer
                            .install_loader(
                                &instance.metadata.game_version,
                                instance.metadata.loader,
                                instance.metadata.loader_version.as_deref(),
                            )
                            .await?;
                        resolve_instance_version(&paths, &instance).await?
                    }
                };
                let version_id = resolved.id.clone();
                let required = resolved.details.required_java_major();
                let component = resolved.details.java_component().map(str::to_string);
                let java = select_java(
                    &client,
                    &paths,
                    &instance,
                    component.as_deref(),
                    required,
                    Some(progress.clone()),
                )
                .await?;

                let launcher = Launcher::new(client.clone(), paths.clone(), progress);
                let plan = launcher
                    .prepare(&instance, &account, &java, &version_id, CLIENT_ID)
                    .await?;
                let (handle, logs) = launcher.start(&plan).await?;
                Ok((plan, version_id, handle, logs))
            }
            .await;

            match result {
                Ok((plan, version_id, handle, logs)) => {
                    let _ = tx.send(EngineEvent::Started {
                        command: plan.display(),
                        version: version_id,
                        handle: Box::new(handle),
                        logs: Box::new(logs),
                    });
                }
                Err(err) => {
                    let _ = tx.send(EngineEvent::ProgressDone);
                    let _ = tx.send(EngineEvent::Error(format!("Launch failed: {err}")));
                }
            }
        });
    }

    // ---------------------------------------------------------------------
    // Actions: modpacks
    // ---------------------------------------------------------------------

    pub(crate) fn open_search_prompt(&mut self) {
        self.overlay = Some(Overlay::text(
            "Search Modpacks",
            "Query: ",
            TextAction::SearchModrinth,
        ));
    }

    pub(crate) fn open_import_prompt(&mut self) {
        self.overlay = Some(Overlay::text(
            "Import .mrpack",
            "Archive path: ",
            TextAction::ImportPath,
        ));
    }

    /// Open a native file dialog to pick a `.mrpack` file.
    ///
    /// Uses `zenity`/`kdialog` when available; falls back to the manual path
    /// prompt otherwise.
    pub(crate) fn browse_for_mrpack(&mut self) {
        let tx = self.engine_tx.clone();
        tokio::spawn(async move {
            let result = tokio::process::Command::new("zenity")
                .arg("--file-selection")
                .arg("--title=Select .mrpack file")
                .arg("--file-filter=*.mrpack")
                .output()
                .await;
            match result {
                Ok(output) if output.status.success() => {
                    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    let _ = tx.send(EngineEvent::ImportPathPicked(
                        (!path.is_empty()).then_some(path),
                    ));
                }
                _ => {
                    let _ = tx.send(EngineEvent::ImportPathPicked(None));
                }
            }
        });
    }

    pub(crate) fn run_search(&mut self, query: String) {
        self.search_query = query.clone();
        self.selected_project = None;
        let modrinth = self.modrinth.clone();
        let tx = self.engine_tx.clone();
        self.progress = Some((None, format!("Searching '{query}'...")));
        tokio::spawn(async move {
            let result = modrinth
                .search(&query, Some("modpack"), None, None, None, 30, 0, &[])
                .await;
            let _ = tx.send(EngineEvent::ProgressDone);
            match result {
                Ok(results) => {
                    let _ = tx.send(EngineEvent::SearchResults(results));
                }
                Err(err) => {
                    let _ = tx.send(EngineEvent::Error(format!("Search failed: {err}")));
                }
            }
        });
    }

    pub(crate) fn open_project(&mut self, hit: &SearchHit) {
        let modrinth = self.modrinth.clone();
        let tx = self.engine_tx.clone();
        let id = hit.project_id.clone();
        self.progress = Some((None, "Loading project...".into()));
        tokio::spawn(async move {
            let result = async {
                let project = modrinth.project(&id).await?;
                let versions = modrinth.versions_filtered(&project.id, None, None).await?;
                Ok::<_, CoreError>((project, versions))
            }
            .await;
            let _ = tx.send(EngineEvent::ProgressDone);
            match result {
                Ok((project, versions)) => {
                    let _ = tx.send(EngineEvent::Project {
                        project: Box::new(project),
                        versions,
                    });
                }
                Err(err) => {
                    let _ = tx.send(EngineEvent::Error(format!("Load failed: {err}")));
                }
            }
        });
    }

    pub(crate) fn install_selected_project(&mut self) {
        let Some(project) = self.selected_project.clone() else {
            self.set_toast("Open a project first", true);
            return;
        };
        let Some(instance) = self.selected_instance().cloned() else {
            self.set_toast("Select an instance in the Instances tab first", true);
            return;
        };
        let version = self
            .project_state
            .selected()
            .and_then(|idx| self.project_versions.get(idx))
            .cloned();
        let Some(version) = version else {
            self.set_toast("No version selected", true);
            return;
        };

        let client = self.client.clone();
        let tx = self.engine_tx.clone();
        let mods_dir = instance.mods_dir();
        let loader = instance.metadata.loader.as_str().to_string();
        let game_version = instance.metadata.game_version.clone();
        let modrinth = self.modrinth.clone();
        self.progress = Some((None, format!("Installing {}", version.name)));

        tokio::spawn(async move {
            let result: Result<Vec<String>, CoreError> = async {
                let Some(file) = version.primary_file() else {
                    return Err(CoreError::Modrinth(
                        "version has no downloadable file".into(),
                    ));
                };
                modrinth::install_version_file(&client, file, &mods_dir, None).await?;
                let mut installed = vec![file.filename.clone()];

                // Resolve and install required dependencies.
                let deps = modrinth::resolve_dependencies(
                    &modrinth,
                    &version,
                    Some(&game_version),
                    Some(&loader),
                    3,
                )
                .await?;
                for dep in deps {
                    if let Some(dep_file) = dep.primary_file() {
                        if modrinth::install_version_file(&client, dep_file, &mods_dir, None)
                            .await
                            .is_ok()
                        {
                            installed.push(dep_file.filename.clone());
                        }
                    }
                }
                Ok(installed)
            }
            .await;

            let _ = tx.send(EngineEvent::ProgressDone);
            match result {
                Ok(files) => {
                    let _ = tx.send(EngineEvent::ModsChanged);
                    let _ = tx.send(EngineEvent::Toast(format!(
                        "Installed {} file(s) from {}",
                        files.len(),
                        project.title
                    )));
                }
                Err(err) => {
                    let _ = tx.send(EngineEvent::Error(format!("Install failed: {err}")));
                }
            }
        });
    }

    pub(crate) fn install_selected_mod_search(&mut self) {
        let Some(hit) = self
            .mod_search_state
            .selected()
            .and_then(|idx| self.mod_search_results.get(idx))
            .cloned()
        else {
            self.set_toast("No search result selected", true);
            return;
        };
        let Some(instance) = self.selected_instance().cloned() else {
            self.set_toast("Select an instance first", true);
            return;
        };
        let modrinth = self.modrinth.clone();
        let client = self.client.clone();
        let tx = self.engine_tx.clone();
        let mods_dir = instance.mods_dir();
        let game_version = instance.metadata.game_version.clone();
        let loader = instance.metadata.loader.as_str().to_string();
        let title = hit.title.clone();
        self.progress = Some((None, format!("Installing {title}...")));

        tokio::spawn(async move {
            let result: Result<Vec<String>, CoreError> = async {
                let Some(version) = modrinth
                    .latest_version(&hit.project_id, Some(&game_version), Some(&loader))
                    .await?
                else {
                    return Err(CoreError::Modrinth(format!(
                        "no compatible version of {title} for {game_version}/{loader}"
                    )));
                };
                let mut installed = Vec::new();
                if let Some(file) = version.primary_file() {
                    modrinth::install_version_file(&client, file, &mods_dir, None).await?;
                    installed.push(file.filename.clone());
                }
                for dep in modrinth::resolve_dependencies(
                    &modrinth,
                    &version,
                    Some(&game_version),
                    Some(&loader),
                    3,
                )
                .await?
                {
                    if let Some(file) = dep.primary_file() {
                        if modrinth::install_version_file(&client, file, &mods_dir, None)
                            .await
                            .is_ok()
                        {
                            installed.push(file.filename.clone());
                        }
                    }
                }
                Ok(installed)
            }
            .await;

            let _ = tx.send(EngineEvent::ProgressDone);
            match result {
                Ok(files) => {
                    let _ = tx.send(EngineEvent::ModsChanged);
                    let _ = tx.send(EngineEvent::Toast(format!(
                        "Installed {} file(s) from {title}",
                        files.len()
                    )));
                }
                Err(err) => {
                    let _ = tx.send(EngineEvent::Error(format!("Install failed: {err}")));
                }
            }
        });
    }

    pub(crate) fn import_modpack(&mut self, path: PathBuf) {
        if path.as_os_str().is_empty() {
            return;
        }
        let installer = Installer::new(
            self.client.clone(),
            self.paths.clone(),
            self.progress_callback(),
        );
        let instances = self.instance_manager.clone();
        let paths = self.paths.clone();
        let progress = self.progress_callback();
        let tx = self.engine_tx.clone();
        self.progress = Some((None, "Importing modpack...".into()));

        tokio::spawn(async move {
            let importer = ModpackInstaller::new(installer, instances, paths, progress);
            let result = importer.import(&path, None).await;
            let _ = tx.send(EngineEvent::ProgressDone);
            match result {
                Ok(instance) => {
                    let _ = tx.send(EngineEvent::InstancesChanged);
                    let _ = tx.send(EngineEvent::Toast(format!("Imported {}", instance.name())));
                }
                Err(err) => {
                    let _ = tx.send(EngineEvent::Error(format!("Import failed: {err}")));
                }
            }
        });
    }

    // ---------------------------------------------------------------------
    // Actions: mods
    // ---------------------------------------------------------------------

    pub(crate) fn reload_mods(&mut self) {
        let Some(instance) = self.selected_instance().cloned() else {
            self.installed_mods.clear();
            self.mods_state.select(None);
            self.mods_scanning = false;
            return;
        };
        self.mods_scanning = true;
        let tx = self.engine_tx.clone();
        let mods_dir = instance.mods_dir();
        tokio::spawn(async move {
            match modrinth::scan_installed_mods(&mods_dir).await {
                Ok(mods) => {
                    let _ = tx.send(EngineEvent::InstalledMods(mods));
                }
                Err(err) => {
                    let _ = tx.send(EngineEvent::Error(format!("Scan failed: {err}")));
                }
            }
        });
    }

    pub(crate) fn reload_resource_packs(&self) {
        let Some(instance) = self.selected_instance().cloned() else { return };
        let tx = self.engine_tx.clone();
        let dir = instance.resourcepacks_dir();
        tokio::spawn(async move {
            let items = Self::scan_dir_names(&dir).await;
            let _ = tx.send(EngineEvent::ResourcePacks(items));
        });
    }

    pub(crate) fn reload_shaders(&self) {
        let Some(instance) = self.selected_instance().cloned() else { return };
        let tx = self.engine_tx.clone();
        let dir = instance.shaders_dir();
        tokio::spawn(async move {
            let items = Self::scan_dir_names(&dir).await;
            let _ = tx.send(EngineEvent::ShaderPacks(items));
        });
    }

    pub(crate) fn reload_worlds(&self) {
        let Some(instance) = self.selected_instance().cloned() else { return };
        let tx = self.engine_tx.clone();
        let dir = instance.saves_dir();
        tokio::spawn(async move {
            let items = Self::scan_dir_names(&dir).await;
            let _ = tx.send(EngineEvent::Worlds(items));
        });
    }

    pub(crate) fn reload_screenshots(&self) {
        let Some(instance) = self.selected_instance().cloned() else { return };
        let tx = self.engine_tx.clone();
        let dir = instance.game_dir().join("screenshots");
        tokio::spawn(async move {
            let items = Self::scan_dir_names(&dir).await;
            let _ = tx.send(EngineEvent::Screenshots(items));
        });
    }

    async fn scan_dir_names(dir: &std::path::Path) -> Vec<String> {
        let mut items = Vec::new();
        if let Ok(mut entries) = tokio::fs::read_dir(dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if let Ok(name) = entry.file_name().into_string() {
                    items.push(name);
                }
            }
        }
        items.sort();
        items
    }

    pub(crate) fn open_mod_search_prompt(&mut self) {
        self.overlay = Some(Overlay::text(
            "Search Mods",
            "Query: ",
            TextAction::SearchMods,
        ));
    }

    pub(crate) fn run_mod_search(&mut self, query: String) {
        self.mod_search_query = query.clone();
        let Some(instance) = self.selected_instance().cloned() else {
            self.set_toast("Select an instance first", true);
            return;
        };
        let modrinth = self.modrinth.clone();
        let tx = self.engine_tx.clone();
        let loader = instance.metadata.loader.as_str().to_string();
        let game_version = instance.metadata.game_version.clone();
        self.progress = Some((None, format!("Searching mods '{query}'...")));
        tokio::spawn(async move {
            let result = modrinth
                .search(
                    &query,
                    Some("mod"),
                    Some(&game_version),
                    Some(&loader),
                    None,
                    30,
                    0,
                    &[],
                )
                .await;
            let _ = tx.send(EngineEvent::ProgressDone);
            match result {
                Ok(results) => {
                    let _ = tx.send(EngineEvent::ModSearchResults(results));
                }
                Err(err) => {
                    let _ = tx.send(EngineEvent::Error(format!("Search failed: {err}")));
                }
            }
        });
    }

    // ---------------------------------------------------------------------
    // Actions: Modrinth browser
    // ---------------------------------------------------------------------

    /// Navigate to the browser, loading the first page of popular projects for
    /// the active content type when nothing is loaded yet.
    pub(crate) fn open_browse(&mut self) {
        if self.browse.results.is_empty() && !self.browse.loading {
            self.browse_load_first_page();
        }
    }

    pub(crate) fn open_browse_search(&mut self) {
        self.overlay = Some(Overlay::text(
            format!("Search {}", self.browse.kind.label()),
            "Query (empty = popular): ",
            TextAction::BrowseSearch,
        ));
    }

    pub(crate) fn run_browse_search(&mut self, query: String) {
        self.browse.query = query.trim().to_string();
        self.browse_close_detail();
        self.browse_load_first_page();
    }

    pub(crate) fn browse_query_reset(&mut self) {
        self.browse.query = String::new();
    }

    pub(crate) fn browse_load_first_page(&mut self) {
        self.browse.results.clear();
        self.browse.selected = 0;
        self.browse.offset = 0;
        self.browse.total = 0;
        self.browse_do_search(0);
    }

    pub(crate) fn browse_next_page(&mut self) {
        if self.browse.loading {
            return;
        }
        let next = self.browse.offset + 30;
        if next >= self.browse.total && self.browse.total > 0 {
            return;
        }
        self.browse.offset = next;
        self.browse.selected = 0;
        self.browse.results.clear();
        self.browse_do_search(next);
    }

    pub(crate) fn browse_prev_page(&mut self) {
        if self.browse.loading {
            return;
        }
        if self.browse.offset == 0 {
            return;
        }
        let prev = self.browse.offset.saturating_sub(30);
        self.browse.offset = prev;
        self.browse.selected = 0;
        self.browse.results.clear();
        self.browse_do_search(prev);
    }

    fn browse_do_search(&mut self, offset: u32) {
        let kind = self.browse.kind;
        let query = self.browse.query.clone();
        let sort = self.browse.sort;
        let filter_compat = self.browse.filter_compat;
        let filter_side = self.browse.filter_side;
        let filter_categories = self.browse.filter_categories.clone();
        let filter_loaders = self.browse.filter_loaders.clone();
        let game_version = if filter_compat {
            self.selected_instance()
                .map(|i| i.metadata.game_version.clone())
        } else {
            None
        };
        let loader = if filter_compat {
            self.selected_instance()
                .map(|i| i.metadata.loader.as_str().to_string())
        } else {
            None
        };
        let modrinth = self.modrinth.clone();
        let tx = self.engine_tx.clone();
        self.browse.loading = true;
        let label = if query.is_empty() {
            format!("Loading popular {}...", kind.label())
        } else {
            format!("Searching '{}'...", query)
        };
        self.progress = Some((None, label));
        tokio::spawn(async move {
            let mut categories = filter_categories;
            // Add selected loaders as category facets (Modrinth uses categories for loader filtering).
            for loader in &filter_loaders {
                categories.push(loader.clone());
            }
            match filter_side {
                SideFilter::Client => categories.push("client-side".to_string()),
                SideFilter::Server => categories.push("server-side".to_string()),
                SideFilter::All => {}
            }
            let gv = game_version.as_deref();
            let ld = loader.as_deref();
            let result = modrinth
                .search(
                    &query,
                    Some(kind.project_type()),
                    gv,
                    ld,
                    Some(sort.as_str()),
                    30,
                    offset,
                    &categories,
                )
                .await;
            let _ = tx.send(EngineEvent::ProgressDone);
            match result {
                Ok(results) => {
                    let _ = tx.send(EngineEvent::BrowseResults(results));
                }
                Err(err) => {
                    let _ = tx.send(EngineEvent::BrowseResults(SearchResults {
                        hits: Vec::new(),
                        offset,
                        limit: 30,
                        total_hits: 0,
                    }));
                    let _ = tx.send(EngineEvent::Error(format!("Search failed: {err}")));
                }
            }
        });
    }

    pub(crate) fn browse_open_selected(&mut self) {
        let Some(hit) = self.browse.results.get(self.browse.selected).cloned() else {
            return;
        };
        self.browse_open_project(&hit.project_id);
    }

    fn browse_open_project(&mut self, id: &str) {
        let id = id.to_string();
        let modrinth = self.modrinth.clone();
        let tx = self.engine_tx.clone();
        self.progress = Some((None, "Loading project...".into()));
        tokio::spawn(async move {
            let result = async {
                let project = modrinth.project(&id).await?;
                let versions = modrinth.versions_filtered(&project.id, None, None).await?;
                Ok::<_, CoreError>((project, versions))
            }
            .await;
            let _ = tx.send(EngineEvent::ProgressDone);
            match result {
                Ok((project, versions)) => {
                    let _ = tx.send(EngineEvent::BrowseProject {
                        project: Box::new(project),
                        versions,
                    });
                }
                Err(err) => {
                    let _ = tx.send(EngineEvent::Error(format!("Load failed: {err}")));
                }
            }
        });
    }

    pub(crate) fn browse_close_detail(&mut self) {
        self.browse.detail = None;
        self.browse.versions.clear();
        self.browse.version_selected = 0;
        self.browse.focus = BrowseFocus::List;
        self.browse.body.clear();
        self.browse.body_for = String::new();
        self.browse.body_scroll = 0;
    }

    /// Kick off fetches for the project icon and a couple of gallery images.
    fn browse_fetch_images(&mut self) {
        let Some(project) = self.browse.detail.clone() else {
            return;
        };
        let mut urls: Vec<String> = Vec::new();
        if let Some(url) = project.icon_url {
            urls.push(url);
        }
        for image in project.gallery.iter().take(2) {
            urls.push(image.url.clone());
        }
        for url in urls {
            self.browse_fetch_image(url.as_str());
        }
    }

    pub(crate) fn browse_fetch_image(&mut self, url: &str) {
        let key = url.to_string();
        if self.browse_images.contains_key(&key) {
            return;
        }
        if !self.browse.image_requested.insert(key.clone()) {
            return;
        }
        let modrinth = self.modrinth.clone();
        let tx = self.engine_tx.clone();
        tokio::spawn(async move {
            let data = match modrinth.get_bytes(&key, 6 * 1024 * 1024).await {
                Ok(bytes) => Some(bytes),
                Err(_) => None,
            };
            let _ = tx.send(EngineEvent::BrowseImage {
                url: key,
                data,
            });
        });
    }

    /// Load a local image file into `local_images` if not already cached.
    pub(crate) fn load_local_image(&mut self, path: &str) {
        if self.local_images.contains_key(path) {
            return;
        }
        let key = path.to_string();
        let path_buf = std::path::PathBuf::from(path);
        let tx = self.engine_tx.clone();
        let key_clone = key.clone();
        tokio::spawn(async move {
            let result = tokio::fs::read(&path_buf).await;
            let img = result.ok().and_then(|data| {
                mc_core::img::decode_image(&data).ok().or_else(|| {
                    image::load_from_memory(&data).ok().map(|dynamic| {
                        let rgba = dynamic.to_rgba8();
                        mc_core::img::RgbaImage {
                            width: rgba.width(),
                            height: rgba.height(),
                            pixels: rgba.into_raw(),
                        }
                    })
                })
            });
            let _ = tx.send(EngineEvent::LocalImage {
                path: key_clone,
                img,
            });
        });
    }

    /// Install the selected version: modpacks import a new build, everything
    /// else installs a file into the selected instance's folder.
    pub(crate) fn browse_install(&mut self) {
        let Some(project) = self.browse.detail.clone() else {
            self.set_toast("Open a project first", true);
            return;
        };
        let Some(version) = self
            .browse
            .versions
            .get(self.browse.version_selected)
            .cloned()
        else {
            self.set_toast("No version selected", true);
            return;
        };
        let title = project.title.clone();
        let is_modpack = self.browse.kind == BrowseKind::Modpacks;

        if is_modpack {
            self.browse_close_detail();
            self.install_modrinth_modpack(title, version);
            return;
        }

        let Some(instance) = self.selected_instance().cloned() else {
            self.set_toast("Select an instance in the Instances tab first", true);
            return;
        };
        let dest_dir = match self.browse.kind {
            BrowseKind::Mods => instance.mods_dir(),
            BrowseKind::Resourcepacks => instance.resourcepacks_dir(),
            _ => instance.shaders_dir(),
        };
        let game_version = instance.metadata.game_version.clone();
        let loader = instance.metadata.loader.as_str().to_string();
        let resolve_deps = self.browse.kind == BrowseKind::Mods;

        let client = self.client.clone();
        let modrinth = self.modrinth.clone();
        let tx = self.engine_tx.clone();
        let title = project.title.clone();
        self.progress = Some((None, format!("Installing {}", version.name)));
        tokio::spawn(async move {
            let result: Result<Vec<String>, CoreError> = async {
                let Some(file) = version.primary_file() else {
                    return Err(CoreError::Modrinth(
                        "version has no downloadable file".into(),
                    ));
                };
                modrinth::install_version_file(&client, file, &dest_dir, None).await?;
                let mut installed = vec![file.filename.clone()];
                if resolve_deps {
                    let deps = modrinth::resolve_dependencies(
                        &modrinth,
                        &version,
                        Some(&game_version),
                        Some(&loader),
                        3,
                    )
                    .await?;
                    for dep in deps {
                        if let Some(dep_file) = dep.primary_file() {
                            if modrinth::install_version_file(&client, dep_file, &dest_dir, None)
                                .await
                                .is_ok()
                            {
                                installed.push(dep_file.filename.clone());
                            }
                        }
                    }
                }
                Ok(installed)
            }
            .await;
            let _ = tx.send(EngineEvent::ProgressDone);
            match result {
                Ok(files) => {
                    let _ = tx.send(EngineEvent::ModsChanged);
                    let _ = tx.send(EngineEvent::Toast(format!(
                        "Installed {} file(s) from {}",
                        files.len(),
                        title
                    )));
                }
                Err(err) => {
                    let _ = tx.send(EngineEvent::Error(format!("Install failed: {err}")));
                }
            }
        });
    }

    /// Quick-install the latest compatible release for a mod directly from the
    /// browse list (without opening the detail view).
    pub(crate) fn browse_quick_install(&mut self, idx: usize) {
        let Some(hit) = self.browse.results.get(idx).cloned() else {
            return;
        };
        let Some(instance) = self.selected_instance().cloned() else {
            self.set_toast("Select an instance first", true);
            return;
        };
        let is_modpack = self.browse.kind == BrowseKind::Modpacks;
        if is_modpack {
            self.set_toast("Open a modpack to install it", true);
            return;
        }
        let dest_dir = match self.browse.kind {
            BrowseKind::Mods => instance.mods_dir(),
            BrowseKind::Resourcepacks => instance.resourcepacks_dir(),
            _ => instance.shaders_dir(),
        };
        let game_version = instance.metadata.game_version.clone();
        let loader = instance.metadata.loader.as_str().to_string();
        let client = self.client.clone();
        let modrinth = self.modrinth.clone();
        let tx = self.engine_tx.clone();
        let title = hit.title.clone();
        let project_id = hit.project_id.clone();
        self.progress = Some((None, format!("Installing {}...", title)));
        tokio::spawn(async move {
            let result: Result<Vec<String>, CoreError> = async {
                let Some(version) = modrinth
                    .latest_version(&project_id, Some(&game_version), Some(&loader))
                    .await?
                else {
                    return Err(CoreError::Modrinth(
                        "no compatible version found".into(),
                    ));
                };
                let Some(file) = version.primary_file() else {
                    return Err(CoreError::Modrinth(
                        "version has no downloadable file".into(),
                    ));
                };
                modrinth::install_version_file(&client, file, &dest_dir, None).await?;
                let mut installed = vec![file.filename.clone()];
                let deps = modrinth::resolve_dependencies(
                    &modrinth,
                    &version,
                    Some(&game_version),
                    Some(&loader),
                    3,
                )
                .await?;
                for dep in deps {
                    if let Some(dep_file) = dep.primary_file() {
                        if modrinth::install_version_file(&client, dep_file, &dest_dir, None)
                            .await
                            .is_ok()
                        {
                            installed.push(dep_file.filename.clone());
                        }
                    }
                }
                Ok(installed)
            }
            .await;
            let _ = tx.send(EngineEvent::ProgressDone);
            match result {
                Ok(files) => {
                    let _ = tx.send(EngineEvent::ModsChanged);
                    let _ = tx.send(EngineEvent::Toast(format!(
                        "Installed {} file(s) from {}",
                        files.len(),
                        title
                    )));
                }
                Err(err) => {
                    let _ = tx.send(EngineEvent::Error(format!("Install failed: {err}")));
                }
            }
        });
    }

    /// Handle a click on a sidebar filter item.
    pub(crate) fn browse_filter_click(&mut self, item: crate::views::browse::FilterItem) {
        use crate::views::browse::FilterItem;
        match item {
            FilterItem::Sort => {
                self.browse.sort = self.browse.sort.cycle();
                self.browse_load_first_page();
            }
            FilterItem::Side => {
                self.browse.filter_side = self.browse.filter_side.cycle();
                self.browse_load_first_page();
            }
            FilterItem::Compat => {
                self.browse.filter_compat = !self.browse.filter_compat;
                self.browse_load_first_page();
            }
            FilterItem::Loader(idx) => {
                let loaders = self.browse.kind.loaders();
                if let Some(loader) = loaders.get(idx) {
                    let loader = loader.to_string();
                    if let Some(pos) = self.browse.filter_loaders.iter().position(|l| *l == loader) {
                        self.browse.filter_loaders.remove(pos);
                    } else {
                        self.browse.filter_loaders.push(loader);
                    }
                    self.browse_load_first_page();
                }
            }
            FilterItem::Category(idx) => {
                let cats = self.browse.kind.categories();
                if let Some(cat) = cats.get(idx) {
                    let cat = cat.to_string();
                    if let Some(pos) = self.browse.filter_categories.iter().position(|c| *c == cat) {
                        self.browse.filter_categories.remove(pos);
                    } else {
                        self.browse.filter_categories.push(cat);
                    }
                    self.browse_load_first_page();
                }
            }
        }
    }

    pub(crate) fn toggle_selected_mod(&mut self) {
        let Some(idx) = self.mods_state.selected() else {
            return;
        };
        let Some(module) = self.installed_mods.get(idx).cloned() else {
            return;
        };
        // Optimistic local flip so the toggle is instant; the async reload
        // confirms it once the rename completes.
        if let Some(current) = self.installed_mods.get_mut(idx) {
            current.enabled = !current.enabled;
        }
        let tx = self.engine_tx.clone();
        tokio::spawn(async move {
            match modrinth::toggle_mod(&module.path).await {
                Ok(_) => {
                    let _ = tx.send(EngineEvent::ModsChanged);
                }
                Err(err) => {
                    let _ = tx.send(EngineEvent::Error(format!("Toggle failed: {err}")));
                }
            }
        });
    }

    pub(crate) fn confirm_delete_mod(&mut self) {
        let Some(idx) = self.mods_state.selected() else {
            return;
        };
        let Some(module) = self.installed_mods.get(idx).cloned() else {
            return;
        };
        self.overlay = Some(Overlay::confirm(
            "Delete Mod",
            format!("Delete '{}'?", module.file_name),
            ConfirmAction::DeleteMod(module.path),
        ));
    }

    pub(crate) fn check_mod_updates(&mut self) {
        let Some(instance) = self.selected_instance().cloned() else {
            self.set_toast("Select an instance first", true);
            return;
        };
        if self.installed_mods.is_empty() {
            self.set_toast("No installed mods to check", true);
            return;
        }
        let modrinth = self.modrinth.clone();
        let tx = self.engine_tx.clone();
        let mods: Vec<InstalledMod> = self.installed_mods.clone();
        let game_version = instance.metadata.game_version.clone();
        let loader = instance.metadata.loader.as_str().to_string();
        self.progress = Some((None, "Checking for updates...".into()));

        tokio::spawn(async move {
            let mut updates = Vec::new();
            for module in mods {
                // SHA-1 is computed lazily here so the initial mod scan stays fast.
                let sha1 = match modrinth::update_check_sha1(&module).await {
                    Ok(hash) => hash,
                    Err(_) => continue,
                };
                if sha1.is_empty() {
                    continue;
                }
                if let Ok(Some((installed, latest))) = modrinth
                    .check_update(&sha1, Some(&game_version), Some(&loader))
                    .await
                {
                    updates.push(format!(
                        "{}: {} -> {}",
                        module.file_name, installed.version_number, latest.version_number
                    ));
                }
            }
            let _ = tx.send(EngineEvent::ProgressDone);
            if updates.is_empty() {
                let _ = tx.send(EngineEvent::Toast("All mods are up to date".into()));
            } else {
                let _ = tx.send(EngineEvent::Message(updates));
            }
        });
    }

    // ---------------------------------------------------------------------
    // Actions: accounts
    // ---------------------------------------------------------------------

    pub(crate) fn reload_accounts(&mut self) {
        let path = self.paths.accounts_file();
        let tx = self.engine_tx.clone();
        tokio::spawn(async move {
            if let Ok(store) = AccountStore::load(path).await {
                let _ = tx.send(EngineEvent::AccountsReloaded(store));
            }
        });
    }

    pub(crate) fn open_offline_login(&mut self) {
        let form = Form::new("Offline Account", FormAction::OfflineLogin)
            .push_text("Username", "Player")
            .with_hint("3-16 chars, A-Z 0-9 _");
        self.overlay = Some(Overlay::Form(form));
    }

    pub(crate) fn offline_login(&mut self, name: &str) {
        if !mc_core::auth::offline::valid_username(name) {
            self.set_toast("Invalid username (3-16 chars, A-Z 0-9 _)", true);
            return;
        }
        let account = Account::offline(name);
        let path = self.paths.accounts_file();
        let tx = self.engine_tx.clone();
        let name = name.to_string();
        tokio::spawn(async move {
            match AccountStore::load(&path).await {
                Ok(mut store) => {
                    if store.upsert(account).await.is_ok() {
                        let _ = tx.send(EngineEvent::AccountsChanged);
                        let _ =
                            tx.send(EngineEvent::Toast(format!("Added offline account {name}")));
                    } else {
                        let _ = tx.send(EngineEvent::Error("Failed to save account".into()));
                    }
                }
                Err(err) => {
                    let _ = tx.send(EngineEvent::Error(format!("Account store error: {err}")));
                }
            }
        });
    }

    pub(crate) fn start_microsoft_login(&mut self) {
        let auth = MicrosoftAuth::new(self.client.clone());
        let tx = self.engine_tx.clone();
        self.progress = Some((None, "Requesting device code...".into()));
        tokio::spawn(async move {
            let prompt = match auth.request_device_code().await {
                Ok(prompt) => prompt,
                Err(err) => {
                    let _ = tx.send(EngineEvent::ProgressDone);
                    let _ = tx.send(EngineEvent::Error(format!("Device code failed: {err}")));
                    return;
                }
            };
            let _ = tx.send(EngineEvent::ProgressDone);
            let _ = tx.send(EngineEvent::DeviceCode(Box::new(prompt.clone())));

            let _ = tx.send(EngineEvent::Status(
                "Completing Xbox/Minecraft login...".into(),
            ));
            match auth.login_device_code(&prompt).await {
                Ok(account) => {
                    let _ = tx.send(EngineEvent::Authenticated(Box::new(account)));
                }
                Err(err) => {
                    let _ = tx.send(EngineEvent::Error(format!("Login failed: {err}")));
                }
            }
        });
    }

    pub(crate) fn set_active_account(&mut self) {
        let Some(idx) = self.account_state.selected() else {
            return;
        };
        let Some(account) = self.accounts.accounts().get(idx).cloned() else {
            return;
        };
        let path = self.paths.accounts_file();
        let tx = self.engine_tx.clone();
        let name = account.username.clone();
        tokio::spawn(async move {
            if let Ok(mut store) = AccountStore::load(&path).await {
                let _ = store.set_active(&account.id).await;
                let _ = tx.send(EngineEvent::AccountsChanged);
                let _ = tx.send(EngineEvent::Toast(format!("Active account: {name}")));
            }
        });
    }

    pub(crate) fn confirm_delete_account(&mut self) {
        let Some(idx) = self.account_state.selected() else {
            return;
        };
        let Some(account) = self.accounts.accounts().get(idx).cloned() else {
            return;
        };
        self.overlay = Some(Overlay::confirm(
            "Remove Account",
            format!("Remove account '{}'?", account.username),
            ConfirmAction::DeleteAccount(account.id),
        ));
    }

    pub(crate) fn open_skin_prompt(&mut self) {
        self.overlay = Some(Overlay::text(
            "Change Skin",
            "Image URL or local .png path: ",
            TextAction::SkinUrl,
        ));
    }

    pub(crate) fn change_skin(&mut self, input: String) {
        let input = input.trim().to_string();
        if input.is_empty() {
            return;
        }
        let Some(account) = self.accounts.active().cloned() else {
            self.set_toast("No active account", true);
            return;
        };
        let Some(token) = account.access_token.clone() else {
            self.set_toast("Skin changes require a Microsoft account", true);
            return;
        };
        let client = self.client.clone();
        let tx = self.engine_tx.clone();
        self.progress = Some((None, "Updating skin...".into()));

        tokio::spawn(async move {
            let skin = SkinClient::new(client);
            let result = if input.starts_with("http://") || input.starts_with("https://") {
                skin.set_skin_url(&token, &input, SkinVariant::Classic)
                    .await
            } else {
                let filename = std::path::Path::new(&input)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("skin.png")
                    .to_string();
                match tokio::fs::read(&input).await {
                    Ok(bytes) => {
                        skin.upload_skin(&token, bytes, &filename, SkinVariant::Classic)
                            .await
                    }
                    Err(err) => Err(CoreError::Skin(format!("cannot read file: {err}"))),
                }
            };
            let _ = tx.send(EngineEvent::ProgressDone);
            match result {
                Ok(()) => {
                    let _ = tx.send(EngineEvent::AccountsChanged);
                    let _ = tx.send(EngineEvent::Toast("Skin updated".into()));
                }
                Err(err) => {
                    let _ = tx.send(EngineEvent::Error(format!("Skin update failed: {err}")));
                }
            }
        });
    }

    // ---------------------------------------------------------------------
    // Actions: logs & settings
    // ---------------------------------------------------------------------

    pub(crate) fn load_latest_log(&mut self) {
        let Some(instance) = self.selected_instance().cloned() else {
            return;
        };
        let path = instance.logs_dir().join("latest.log");
        let tx = self.engine_tx.clone();
        tokio::spawn(async move {
            if let Ok(contents) = tokio::fs::read_to_string(&path).await {
                let lines: Vec<String> = contents
                    .lines()
                    .rev()
                    .take(2000)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                let _ = tx.send(EngineEvent::LogLines(lines));
            }
        });
    }

    pub(crate) fn analyze_crash(&mut self) {
        if self.crash_analysis.is_some() {
            if let Some(analysis) = self.crash_analysis.clone() {
                let mut lines = vec![format!("Headline: {}", analysis.headline), String::new()];
                for cause in &analysis.causes {
                    lines.push(format!("• {cause}"));
                }
                for rec in &analysis.recommendations {
                    lines.push(format!("→ {rec}"));
                }
                if let Some(mismatch) = &analysis.java_mismatch {
                    lines.push(format!("Java: {}", mismatch.detail));
                }
                if !analysis.missing_dependencies.is_empty() {
                    lines.push(format!(
                        "Missing: {}",
                        analysis.missing_dependencies.join(", ")
                    ));
                }
                self.overlay = Some(Overlay::message("Crash Analysis", lines));
            }
            return;
        }
        let Some(instance) = self.selected_instance().cloned() else {
            return;
        };
        let tx = self.engine_tx.clone();
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || logs::analyze_latest(&instance))
                .await
                .ok()
                .and_then(|r| r.ok())
                .flatten();
            let _ = tx.send(EngineEvent::Crash(result));
        });
    }

    pub(crate) fn save_settings(&mut self) {
        self.save_settings_async();
        self.set_toast("Settings saved", false);
    }

    pub(crate) fn open_settings_form(&mut self) {
        let s = &self.settings;
        let form = Form::new("Launcher Settings", FormAction::EditLauncherSettings)
            .push_text(
                "Java Path",
                s.java_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default(),
            )
            .with_hint("blank = auto-detect")
            .push_number("Default Min RAM (MB)", s.default_min_memory_mb)
            .push_number("Default Max RAM (MB)", s.default_max_memory_mb)
            .push_choice("Default GC", gc_options(), gc_index(s.default_gc))
            .push_bool("Show Progress", s.show_progress)
            .push_bool("Confirm Quit", s.confirm_quit)
            .push_bool("Auto-scroll Logs", s.log_auto_scroll);
        self.overlay = Some(Overlay::Form(form));
    }

    fn apply_launcher_settings_form(&mut self, form: &Form) {
        self.settings.java_path = form
            .text_value("Java Path")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        self.settings.default_min_memory_mb =
            form.number_value("Default Min RAM (MB)").unwrap_or(512);
        self.settings.default_max_memory_mb =
            form.number_value("Default Max RAM (MB)").unwrap_or(4096);
        self.settings.default_gc = parse_gc(form.choice_value("Default GC").unwrap_or("G1GC"));
        self.settings.show_progress = form.bool_value("Show Progress").unwrap_or(true);
        self.settings.confirm_quit = form.bool_value("Confirm Quit").unwrap_or(false);
        self.settings.log_auto_scroll = form.bool_value("Auto-scroll Logs").unwrap_or(true);
        self.save_settings_async();
        self.set_toast("Settings saved", false);
    }

    /// Toggle a boolean launcher setting by field index.
    pub(crate) fn toggle_setting(&mut self, field: usize) {
        match field {
            4 => self.settings.show_progress = !self.settings.show_progress,
            5 => self.settings.confirm_quit = !self.settings.confirm_quit,
            6 => self.settings.log_auto_scroll = !self.settings.log_auto_scroll,
            _ => return,
        }
        self.save_settings_async();
    }

    /// Cycle the default GC preset.
    pub(crate) fn cycle_setting_gc(&mut self, forward: bool) {
        let options = mc_core::instance::GcPreset::all();
        let current = options
            .iter()
            .position(|g| *g == self.settings.default_gc)
            .unwrap_or(0);
        let next = if forward {
            (current + 1) % options.len()
        } else {
            (current + options.len() - 1) % options.len()
        };
        self.settings.default_gc = options[next];
        self.save_settings_async();
    }

    pub(crate) fn save_settings_async(&self) {
        let settings = self.settings.clone();
        let paths = self.paths.clone();
        tokio::spawn(async move {
            let _ = settings.save(&paths).await;
        });
    }

    pub(crate) fn spawn_java_discovery(&self) {
        let tx = self.engine_tx.clone();
        tokio::spawn(async move {
            let list = java::discover().await;
            let _ = tx.send(EngineEvent::Java(list));
        });
    }

    pub(crate) fn confirm(&mut self, action: ConfirmAction) {
        match action {
            ConfirmAction::DeleteInstance(id) => {
                let manager = self.instance_manager.clone();
                let tx = self.engine_tx.clone();
                tokio::spawn(async move {
                    match manager.delete(&id).await {
                        Ok(()) => {
                            let _ = tx.send(EngineEvent::InstancesChanged);
                            let _ = tx.send(EngineEvent::Toast("Instance deleted".into()));
                        }
                        Err(err) => {
                            let _ = tx.send(EngineEvent::Error(format!("Delete failed: {err}")));
                        }
                    }
                });
            }
            ConfirmAction::DeleteMod(path) => {
                let tx = self.engine_tx.clone();
                tokio::spawn(async move {
                    match modrinth::remove_mod(&path).await {
                        Ok(()) => {
                            let _ = tx.send(EngineEvent::ModsChanged);
                            let _ = tx.send(EngineEvent::Toast("Mod deleted".into()));
                        }
                        Err(err) => {
                            let _ = tx.send(EngineEvent::Error(format!("Delete failed: {err}")));
                        }
                    }
                });
            }
            ConfirmAction::DeleteAccount(id) => {
                let path = self.paths.accounts_file();
                let tx = self.engine_tx.clone();
                tokio::spawn(async move {
                    if let Ok(mut store) = AccountStore::load(&path).await {
                        let _ = store.remove(&id).await;
                        let _ = tx.send(EngineEvent::AccountsChanged);
                    }
                });
            }
            ConfirmAction::Quit => self.should_quit = true,
            ConfirmAction::None => {}
        }
    }

    pub(crate) fn request_quit(&mut self) {
        if self.settings.confirm_quit {
            self.overlay = Some(Overlay::confirm(
                "Quit",
                "Quit CTMLauncher?",
                ConfirmAction::Quit,
            ));
        } else {
            self.should_quit = true;
        }
    }

    pub(crate) fn show_help(&mut self) {
        let lines = vec![
            "Navigation (right panel)".to_string(),
            "  ↑↓ / jk      move through the navigation menu".to_string(),
            "  1-5          Instances/Mods/Versions/JVM/Logs".to_string(),
            "  F2 / F3      Accounts / Launcher Settings".to_string(),
            "  Esc          back to the Instances page".to_string(),
            "  Tab          toggle panel/content focus".to_string(),
            "  Enter        primary action".to_string(),
            "  q / Ctrl-C   quit".to_string(),
            String::new(),
            "Instances (main area + toolbar)".to_string(),
            "  arrows/hjkl  move between build cards".to_string(),
            "  Enter        launch the selected build".to_string(),
            "  n new · e edit · i install · p import · v versions · d delete".to_string(),
            String::new(),
            "Mods / Modpacks / Versions / JVM".to_string(),
            "  Mods: t pane · Space toggle · s search · u updates · d delete".to_string(),
            "  Modpacks: / search · Enter open · i install · m import .mrpack".to_string(),
            "  Versions: c change version · r reinstall".to_string(),
            String::new(),
            "Logs".to_string(),
            "  j/k or wheel scroll · PgUp/PgDn page · g/G top/bottom".to_string(),
            "  f follow · p pause · c clear · / filter · a analyze crash · l level".to_string(),
            String::new(),
            "Mouse: hover highlights; click cards, nav blocks, lists and buttons.".to_string(),
        ];
        self.overlay = Some(Overlay::message("Help", lines));
    }

    // ---------------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------------

    pub(crate) fn progress_callback(&self) -> ProgressCallback {
        let tx = self.engine_tx.clone();
        Arc::new(move |progress: Progress| {
            let _ = tx.send(progress_event(progress));
        }) as ProgressCallback
    }

    pub(crate) fn set_toast(&mut self, message: impl Into<String>, error: bool) {
        self.toast = Some(Toast {
            message: message.into(),
            error,
            at: Instant::now(),
        });
    }

    pub(crate) fn push_hitbox(&mut self, rect: Rect, action: HitAction) {
        if rect.width > 0 && rect.height > 0 {
            self.hitboxes.push(Hitbox { rect, action });
        }
    }

    /// Whether the mouse currently hovers `rect`.
    pub(crate) fn is_hovered(&self, rect: Rect) -> bool {
        self.mouse_pos
            .map(|pos| rect_contains(rect, pos))
            .unwrap_or(false)
    }

    // ---------------------------------------------------------------------
    // Rendering
    // ---------------------------------------------------------------------

    pub(crate) fn render(&mut self, frame: &mut Frame) {
        self.hitboxes.clear();
        let area = frame.area();
        frame.render_widget(Block::default().style(self.theme.base()), area);

        let progress_active = self.settings.show_progress && self.progress.is_some();
        let mut rows = vec![
            Constraint::Length(1), // header bar
            Constraint::Min(3),    // body
            Constraint::Length(1), // status bar
        ];
        if progress_active {
            rows.push(Constraint::Length(1));
        }
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(rows)
            .split(area);

        self.render_header(frame, chunks[0]);

        // The body is inset by one row on top so the cards float below the
        // header. Content sits on the left, the sidebar on the right, with a
        // single column of background between them.
        let body_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(3)])
            .split(chunks[1]);
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(20),
                Constraint::Length(1),
                Constraint::Length(30),
            ])
            .split(body_rows[1]);

        let content = columns[0];
        self.sidebar_area = columns[2];
        // Align the sidebar card with the content card (which sits below its
        // toolbar + gap).
        let sidebar = Rect {
            y: columns[2].y + 2,
            height: columns[2].height.saturating_sub(2),
            ..columns[2]
        };
        self.render_nav_panel(frame, sidebar);

        match self.nav {
            Nav::Instances => self.render_instance_grid(frame, content),
            Nav::Accounts => self.render_accounts(frame, content),
            Nav::Launcher => self.render_settings(frame, content),
            Nav::Browse => self.render_browse(frame, content),
            _ if self.selected_instance().is_none() => self.render_empty_state(frame, content),
            Nav::Mods => self.render_mods(frame, content),
            Nav::Modpacks => self.render_modpacks(frame, content),
            Nav::Versions => self.render_versions(frame, content),
            Nav::Jvm => self.render_instance_settings(frame, content),
            Nav::Logs => self.render_logs(frame, content),
            Nav::ResourcePacks => self.render_resource_packs(frame, content),
            Nav::Shaders => self.render_shaders(frame, content),
            Nav::Worlds => self.render_worlds(frame, content),
            Nav::Screenshots => self.render_screenshots(frame, content),
        }

        self.render_footer(frame, chunks[2]);
        if progress_active {
            self.render_progress(frame, chunks[3]);
        }
        self.render_overlay(frame, area);
    }

    /// The right-hand navigation + build info sidebar, a single flat card.
    pub(crate) fn render_nav_panel(&mut self, frame: &mut Frame, area: Rect) {
        let focused = self.focus == Focus::Sidebar;
        let inner = crate::views::card(self, frame, area, focused);

        // 1-char left indent for all sidebar content.
        let content = Rect {
            x: inner.x + 1,
            y: inner.y,
            width: inner.width.saturating_sub(1),
            height: inner.height,
        };

        let menu = Nav::menu();
        let nav_height = menu.len() as u16 * NAV_BUTTON_HEIGHT;
        let has_build = self.selected_instance().is_some();
        let build_detail_height = if has_build { 5 } else { 0 };

        // Layout: build name (1) → "Navigation" (1) → nav buttons → spacer → build details
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(if has_build { 1 } else { 0 }), // build name
                Constraint::Length(1),                             // "Navigation" header
                Constraint::Length(nav_height),                   // nav buttons
                Constraint::Min(1),                               // spacer
                Constraint::Length(1),                            // "Build Info" header
                Constraint::Length(build_detail_height),          // build details
            ])
            .split(content);

        // Build name pinned above navigation.
        if has_build {
            if let Some(instance) = self.selected_instance() {
                let name = truncate_str(instance.name(), chunks[0].width as usize);
                frame.render_widget(
                    Paragraph::new(Span::styled(name, self.theme.header()))
                        .alignment(ratatui::layout::Alignment::Center)
                        .style(self.theme.card()),
                    chunks[0],
                );
            }
        }

        // Empty space where "Navigation" was.

        for (idx, nav) in menu.iter().enumerate() {
            let y = chunks[2].y + idx as u16 * NAV_BUTTON_HEIGHT;
            if y + NAV_BUTTON_HEIGHT > chunks[2].y + chunks[2].height {
                break;
            }
            let rect = Rect {
                x: chunks[2].x,
                y,
                width: chunks[2].width,
                height: NAV_BUTTON_HEIGHT,
            };
            self.render_nav_button(frame, rect, *nav, idx + 1);
        }

        frame.render_widget(
            Paragraph::new(Span::styled("Build Info", self.theme.card_comment()))
                .style(self.theme.card()),
            chunks[4],
        );
        self.render_build_info(frame, chunks[5]);
    }

    /// A chunky, padded navigation block button with an index number. The
    /// active entry gets a solid dark-green fill. No borders.
    fn render_nav_button(&mut self, frame: &mut Frame, rect: Rect, nav: Nav, number: usize) {
        let selected = nav == self.nav;
        let hovered = self.is_hovered(rect);
        let bg = if selected || hovered {
            self.theme.selection_bg
        } else {
            self.theme.panel_alt
        };
        frame.render_widget(Block::default().style(Style::default().bg(bg)), rect);

        // Left accent bar: green for active, muted gray for inactive.
        let bar_style = if selected {
            Style::default().fg(self.theme.green).bg(bg)
        } else {
            Style::default().fg(self.theme.muted).bg(bg)
        };
        if rect.height > 0 {
            let bar_rect = Rect {
                x: rect.x,
                y: rect.y,
                width: 1,
                height: rect.height,
            };
            let bar_lines: Vec<Line> = (0..rect.height)
                .map(|_| Line::from(Span::styled("▎", bar_style)))
                .collect();
            frame.render_widget(Paragraph::new(bar_lines), bar_rect);
        }

        let surface = Style::default().bg(bg);
        let num_style = if selected {
            self.theme.accent_bright()
        } else if hovered {
            self.theme.accent()
        } else {
            self.theme.comment_style()
        };
        let label_style = if selected {
            self.theme.accent_bright()
        } else if hovered {
            self.theme.accent()
        } else {
            Style::default().fg(self.theme.fg).bg(bg)
        };
        let row = Rect {
            x: rect.x + 1,
            y: rect.y + rect.height / 2,
            width: rect.width.saturating_sub(1),
            height: 1,
        };
        if rect.height > 0 {
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(format!(" {number}   "), num_style),
                    Span::styled(nav.menu_label().to_string(), label_style),
                ]))
                .style(surface),
                row,
            );
        }
        self.push_hitbox(rect, HitAction::NavItem(nav));
    }

    fn render_build_info(&mut self, frame: &mut Frame, area: Rect) {
        let Some(instance) = self.selected_instance().cloned() else {
            frame.render_widget(
                Paragraph::new(Span::styled("No build selected.", self.theme.card_dim()))
                    .style(self.theme.card()),
                area,
            );
            return;
        };

        let jvm = &instance.metadata.jvm;
        let lines = vec![
            info_line("Version", &instance.metadata.game_version, &self.theme),
            info_line("Loader", instance.metadata.loader.label(), &self.theme),
            info_line("Memory", &format!("{} MB", jvm.max_memory_mb), &self.theme),
            info_line("GC", jvm.gc.label(), &self.theme),
            info_line("Mods", &self.installed_mods.len().to_string(), &self.theme),
        ];
        frame.render_widget(Paragraph::new(lines).style(self.theme.card()), area);
    }

    pub(crate) fn render_empty_state(&mut self, frame: &mut Frame, area: Rect) {
        let inner = crate::views::card(self, frame, area, true);

        let lines = vec![
            Line::from(""),
            Line::from(Span::styled("No build selected", self.theme.header())),
            Line::from(Span::styled(
                "Pick a build from the Instances page, or create a new one.",
                self.theme.card_dim(),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Press 'n' or click [+ New Build].",
                self.theme.accent(),
            )),
        ];
        frame.render_widget(Paragraph::new(lines).style(self.theme.card()), inner);
    }

    pub(crate) fn render_header(&mut self, frame: &mut Frame, area: Rect) {
        frame.render_widget(Block::default().style(self.theme.bar()), area);

        let active = self
            .accounts
            .active()
            .map(|a| a.username.clone())
            .unwrap_or_else(|| "no account".to_string());

        let subtitle = match self.nav {
            Nav::Instances => "Instances".to_string(),
            Nav::Browse => "Modrinth Browser".to_string(),
            Nav::Accounts => "Accounts".to_string(),
            Nav::Launcher => "Launcher Settings".to_string(),
            _ => self
                .selected_instance()
                .map(|i| i.name().to_string())
                .unwrap_or_else(|| "no build".to_string()),
        };
        let left = Line::from(vec![
            Span::styled(" CTMLauncher", self.theme.accent_bright()),
            Span::styled("  ›  ", self.theme.comment_style()),
            Span::styled(subtitle, self.theme.dim()),
        ]);
        frame.render_widget(Paragraph::new(left).style(self.theme.bar()), area);

        // Top-right: the account badge and the launcher settings button side by
        // side. Both remain reachable now that they are out of the nav menu.
        let account = format!("@ {active}");
        let account_w = account.chars().count() as u16;
        let settings = "⚙ Settings";
        let settings_w = settings.chars().count() as u16;
        let total = account_w + 2 + settings_w;
        if total + 2 < area.width {
            let start = area.x + area.width - total - 1;

            let acc_rect = Rect {
                x: start,
                y: area.y,
                width: account_w,
                height: 1,
            };
            let acc_style = if self.nav == Nav::Accounts {
                self.theme.row_selected()
            } else if self.is_hovered(acc_rect) {
                self.theme.hover()
            } else {
                self.theme.accent()
            };
            frame.render_widget(
                Paragraph::new(Span::styled(account, acc_style)).style(self.theme.bar()),
                acc_rect,
            );
            self.push_hitbox(acc_rect, HitAction::NavItem(Nav::Accounts));

            let set_rect = Rect {
                x: start + account_w + 2,
                y: area.y,
                width: settings_w,
                height: 1,
            };
            let set_style = if self.nav == Nav::Launcher {
                self.theme.row_selected()
            } else if self.is_hovered(set_rect) {
                self.theme.hover()
            } else {
                self.theme.accent()
            };
            frame.render_widget(
                Paragraph::new(Span::styled(settings, set_style)).style(self.theme.bar()),
                set_rect,
            );
            self.push_hitbox(set_rect, HitAction::NavItem(Nav::Launcher));
        }
    }

    /// A one-line status bar: status message on the left, green-hotkey hints on
    /// the right.
    pub(crate) fn render_footer(&mut self, frame: &mut Frame, area: Rect) {
        frame.render_widget(Block::default().style(self.theme.bar()), area);

        let hints = footer_hints(self.nav);
        let mut spans: Vec<Span> = Vec::new();
        for (idx, (key, label)) in hints.iter().enumerate() {
            if idx > 0 {
                spans.push(Span::styled("  ", self.theme.comment_style()));
            }
            spans.push(Span::styled(key.to_string(), self.theme.accent()));
            spans.push(Span::styled(format!(" {label}"), self.theme.dim()));
        }
        let hint_width: u16 = spans.iter().map(|s| s.width() as u16).sum();
        let hint_rect = Rect {
            x: area.x + area.width.saturating_sub(hint_width),
            y: area.y,
            width: hint_width.min(area.width),
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(spans)).style(self.theme.bar()),
            hint_rect,
        );

        let status_width = area.width.saturating_sub(hint_width).saturating_sub(2);
        if status_width > 4 {
            let (status_style, status_text) = if let Some(toast) = &self.toast {
                let style = if toast.error {
                    self.theme.error_style()
                } else {
                    self.theme.accent()
                };
                (style, toast.message.clone())
            } else {
                (self.theme.dim(), self.status.clone())
            };
            let busy = self.progress.is_some();
            let marker = if busy {
                let frames = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
                format!("{} ", frames[(self.tick as usize) % frames.len()])
            } else {
                "● ".to_string()
            };
            let rect = Rect {
                x: area.x,
                y: area.y,
                width: status_width,
                height: 1,
            };
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(format!(" {marker}"), status_style),
                    Span::styled(
                        truncate_str(&status_text, status_width.saturating_sub(3) as usize),
                        status_style,
                    ),
                ]))
                .style(self.theme.bar()),
                rect,
            );
        }
    }

    fn render_progress(&mut self, frame: &mut Frame, area: Rect) {
        if let Some((ratio, label)) = &self.progress {
            let gauge = crate::widgets::ProgressBar::new(ratio.unwrap_or(0.0), label, &self.theme);
            frame.render_widget(gauge, area);
        }
    }

    pub(crate) fn render_overlay(&mut self, frame: &mut Frame, area: Rect) {
        let Some(overlay) = self.overlay.clone() else {
            return;
        };
        // A modal scrim: blank the screen behind the dialog so no fragments of
        // the underlying view show through around the popup. Overlay hitboxes
        // replace the content hitboxes while a dialog is open.
        self.hitboxes.clear();
        frame.render_widget(ratatui::widgets::Clear, area);
        frame.render_widget(Block::default().style(self.theme.base()), area);
        let surface = Style::default().bg(self.theme.panel_alt);
        match overlay {
            Overlay::Text {
                title,
                prompt,
                value,
                ..
            } => {
                let popup = crate::widgets::centered_rect(60, 20, area);
                let lines = vec![
                    Line::from(Span::styled(prompt, self.theme.dim())),
                    Line::from(Span::styled(format!("{value}█"), self.theme.accent())),
                    Line::from(""),
                    Line::from(Span::styled("Enter confirm · Esc cancel", self.theme.dim())),
                ];
                crate::widgets::render_popup(frame, popup, &title, lines, &self.theme);
                self.push_hitbox(popup, HitAction::Overlay(OverlayAction::TextDone));
            }
            Overlay::Form(form) => {
                let height = (form.fields.len() as u16 * 2 + 4).min(area.height.saturating_sub(2));
                let popup = crate::widgets::centered_rect(
                    64,
                    (height * 100 / area.height.max(1)).max(20),
                    area,
                );
                frame.render_widget(ratatui::widgets::Clear, popup);
                frame.render_widget(Block::default().style(surface), popup);
                crate::views::accent_bar(frame, popup, &self.theme);

                let content = Rect {
                    x: popup.x + 2,
                    y: popup.y + 1,
                    width: popup.width.saturating_sub(3),
                    height: popup.height.saturating_sub(2),
                };
                let rows = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Min(1),
                    ])
                    .split(content);
                frame.render_widget(
                    Paragraph::new(Span::styled(form.title.clone(), self.theme.header()))
                        .style(surface),
                    rows[0],
                );

                let field_area = rows[2];
                let mut y = field_area.y;
                for (idx, field) in form.fields.iter().enumerate() {
                    if y + 2 > field_area.y + field_area.height {
                        break;
                    }
                    let active = idx == form.active;
                    let label_style = if active {
                        self.theme.accent()
                    } else {
                        self.theme.dim()
                    };
                    frame.render_widget(
                        Paragraph::new(Line::from(Span::styled(
                            format!("{} {}", if active { "▸" } else { " " }, field.label),
                            label_style,
                        )))
                        .style(surface),
                        Rect {
                            x: field_area.x,
                            y,
                            width: field_area.width,
                            height: 1,
                        },
                    );
                    let value_style = if active {
                        self.theme.row_selected()
                    } else {
                        self.theme.row()
                    };
                    let display = match &field.kind {
                        crate::forms::FieldKind::Bool(_) => {
                            format!("< {} >", field.value)
                        }
                        crate::forms::FieldKind::Choice { .. } => format!("< {} >", field.value),
                        _ if active => format!("{}█", field.value),
                        _ => field.value.clone(),
                    };
                    let hint = if field.hint.is_empty() {
                        String::new()
                    } else {
                        format!("   ({})", field.hint)
                    };
                    frame.render_widget(
                        Paragraph::new(Line::from(vec![
                            Span::styled(format!("  {display}"), value_style),
                            Span::styled(hint, self.theme.dim()),
                        ]))
                        .style(surface),
                        Rect {
                            x: field_area.x,
                            y: y + 1,
                            width: field_area.width,
                            height: 1,
                        },
                    );
                    self.push_hitbox(
                        Rect {
                            x: field_area.x,
                            y,
                            width: field_area.width,
                            height: 2,
                        },
                        HitAction::Overlay(OverlayAction::FormField(idx)),
                    );
                    y += 2;
                }
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        "Tab/↑↓ field · ←→ change · Space toggle · Enter submit · Esc cancel",
                        self.theme.dim(),
                    ))
                    .style(surface),
                    Rect {
                        x: field_area.x,
                        y: y + 1,
                        width: field_area.width,
                        height: 1,
                    },
                );
                self.push_hitbox(
                    Rect {
                        x: field_area.x,
                        y: y + 1,
                        width: field_area.width,
                        height: 1,
                    },
                    HitAction::Overlay(OverlayAction::FormSubmit),
                );
            }
            Overlay::Confirm { title, message, .. } => {
                let popup = crate::widgets::centered_rect(52, 22, area);
                let lines = vec![
                    Line::from(Span::styled(message, self.theme.warning_style())),
                    Line::from(""),
                    Line::from(Span::styled("[Y]es   [N]o", self.theme.accent())),
                ];
                crate::widgets::render_popup(frame, popup, &title, lines, &self.theme);
                let mid = popup.width / 2;
                self.push_hitbox(
                    Rect {
                        x: popup.x,
                        y: popup.y,
                        width: mid,
                        height: popup.height,
                    },
                    HitAction::Overlay(OverlayAction::ConfirmYes),
                );
                self.push_hitbox(
                    Rect {
                        x: popup.x + mid,
                        y: popup.y,
                        width: popup.width - mid,
                        height: popup.height,
                    },
                    HitAction::Overlay(OverlayAction::ConfirmNo),
                );
            }
            Overlay::Message { title, lines } => {
                let popup = crate::widgets::centered_rect(74, 84, area);
                let mut rendered: Vec<Line> = lines.into_iter().map(Line::from).collect();
                rendered.push(Line::from(""));
                rendered.push(Line::from(Span::styled(
                    "Enter/Esc to close",
                    self.theme.dim(),
                )));
                crate::widgets::render_popup(frame, popup, &title, rendered, &self.theme);
                self.push_hitbox(popup, HitAction::Overlay(OverlayAction::MessageClose));
            }
            Overlay::DeviceCode(prompt) => {
                let popup = crate::widgets::centered_rect(64, 40, area);
                let lines = vec![
                    Line::from(Span::styled(
                        "1. Open the URL below in a browser:",
                        self.theme.dim(),
                    )),
                    Line::from(Span::styled(
                        prompt.verification_uri.clone(),
                        self.theme.info_style(),
                    )),
                    Line::from(""),
                    Line::from(Span::styled("2. Enter this code:", self.theme.dim())),
                    Line::from(Span::styled(prompt.user_code.clone(), self.theme.header())),
                    Line::from(""),
                    Line::from(Span::styled(prompt.message.clone(), self.theme.dim())),
                    Line::from(""),
                    Line::from(Span::styled(
                        "Waiting for sign-in... Esc to dismiss",
                        self.theme.accent(),
                    )),
                ];
                crate::widgets::render_popup(frame, popup, "Microsoft Sign-in", lines, &self.theme);
            }
            Overlay::Wizard(_) => self.render_wizard(frame, area),
            Overlay::Picker(picker) => {
                let popup = crate::widgets::centered_rect(60, 72, area);
                frame.render_widget(ratatui::widgets::Clear, popup);
                frame.render_widget(Block::default().style(surface), popup);
                crate::views::accent_bar(frame, popup, &self.theme);

                let content = Rect {
                    x: popup.x + 2,
                    y: popup.y + 1,
                    width: popup.width.saturating_sub(3),
                    height: popup.height.saturating_sub(2),
                };
                let rows = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Min(1),
                    ])
                    .split(content);
                frame.render_widget(
                    Paragraph::new(Span::styled(picker.title.clone(), self.theme.header()))
                        .style(surface),
                    rows[0],
                );
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled("Filter  ", self.theme.comment_style()),
                        Span::styled(format!("{}█", picker.query), self.theme.accent()),
                    ]))
                    .style(surface),
                    rows[2],
                );

                let list_area = rows[3];
                let visible = list_area.height as usize;
                let start = picker.selected.saturating_sub(visible.saturating_sub(1));
                for row in 0..visible {
                    let idx = start + row;
                    if idx >= picker.filtered.len() {
                        break;
                    }
                    let real = picker.filtered[idx];
                    let Some(label) = picker.items.get(real) else {
                        continue;
                    };
                    let rect = Rect {
                        x: list_area.x,
                        y: list_area.y + row as u16,
                        width: list_area.width,
                        height: 1,
                    };
                    let style = if idx == picker.selected {
                        self.theme.row_selected()
                    } else {
                        self.theme.row()
                    };
                    frame.render_widget(
                        Paragraph::new(Span::styled(label.clone(), style)).style(surface),
                        rect,
                    );
                    self.push_hitbox(rect, HitAction::Overlay(OverlayAction::PickerItem(idx)));
                }
                if picker.filtered.is_empty() {
                    frame.render_widget(
                        Paragraph::new(Span::styled("No matches.", self.theme.dim()))
                            .style(surface),
                        list_area,
                    );
                }
            }
        }
    }
}

// -------------------------------------------------------------------------
// Free helpers
// -------------------------------------------------------------------------

fn rect_contains(rect: Rect, (x, y): (u16, u16)) -> bool {
    x >= rect.x
        && x < rect.x.saturating_add(rect.width)
        && y >= rect.y
        && y < rect.y.saturating_add(rect.height)
}

fn move_selection(state: &mut ListState, len: usize, delta: i32) {
    if len == 0 {
        state.select(None);
        return;
    }
    let current = state.selected().unwrap_or(0) as i32;
    let next = (current + delta).clamp(0, len as i32 - 1) as usize;
    state.select(Some(next));
}

fn gc_index(gc: mc_core::instance::GcPreset) -> usize {
    gc_options()
        .iter()
        .position(|label| label == gc.label())
        .unwrap_or(0)
}

fn split_args(input: &str) -> Vec<String> {
    input.split_whitespace().map(str::to_string).collect()
}

fn settings_field_count() -> usize {
    7
}

/// Truncate a string to `max` display columns, appending `…` when clipped.
fn truncate_str(input: &str, max: usize) -> String {
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

/// A `label: value` line for the build-info panel.
fn info_line(label: &str, value: &str, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<8}"), theme.card_dim()),
        Span::styled(value.to_string(), theme.card()),
    ])
}

/// Keybinding hints for the status bar, as `(key, action)` pairs.
fn footer_hints(nav: Nav) -> &'static [(&'static str, &'static str)] {
    match nav {
        Nav::Instances => &[
            ("Enter", "launch"),
            ("n", "new"),
            ("e", "edit"),
            ("i", "install"),
            ("p", "import"),
            ("v", "versions"),
            ("r", "rename"),
            ("d", "delete"),
        ],
        Nav::Browse => &[
            ("1-4", "type"),
            ("s", "search"),
            ("Enter", "open"),
            ("i", "quick install"),
            ("[/]", "pages"),
            ("f", "compat"),
            ("c", "side"),
            ("o", "sort"),
        ],
        Nav::Mods => &[
            ("t", "pane"),
            ("Space", "toggle"),
            ("s", "search"),
            ("u", "updates"),
            ("d", "delete"),
        ],
        Nav::Modpacks => &[
            ("/", "search"),
            ("Enter", "open"),
            ("i", "install"),
            ("m", "import"),
        ],
        Nav::Versions => &[("c", "change version"), ("r", "reinstall")],
        Nav::Jvm => &[
            ("Enter", "edit"),
            ("j/k", "move"),
            ("s", "save"),
            ("J", "detect Java"),
        ],
        Nav::Logs => &[
            ("j/k", "scroll"),
            ("f", "follow"),
            ("p", "pause"),
            ("c", "clear"),
            ("/", "filter"),
            ("a", "crash"),
        ],
        Nav::Accounts => &[
            ("n", "offline"),
            ("m", "Microsoft"),
            ("Enter", "active"),
            ("c", "skin"),
            ("d", "remove"),
        ],
        Nav::Launcher => &[
            ("Enter", "edit"),
            ("j/k", "move"),
            ("s", "save"),
            ("J", "detect Java"),
        ],
        Nav::ResourcePacks => &[
            ("Space", "toggle"),
            ("d", "delete"),
            ("Enter", "open folder"),
        ],
        Nav::Shaders => &[
            ("Space", "toggle"),
            ("d", "delete"),
            ("Enter", "open folder"),
        ],
        Nav::Worlds => &[
            ("Enter", "open folder"),
            ("d", "delete"),
        ],
        Nav::Screenshots => &[
            ("Enter", "view"),
            ("d", "delete"),
            ("←/→", "prev/next"),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mc_core::instance::LoaderType;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn temp_paths() -> Paths {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Paths::rooted_at(std::env::temp_dir().join(format!("ctm-tui-{nanos}")))
    }

    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[tokio::test]
    async fn renders_all_pages() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();

        // Create a local instance (no network) so build pages have content.
        app.instance_manager
            .create("Demo", "1.21.1", LoaderType::Fabric, Some("0.15.7".into()))
            .await
            .unwrap();
        app.reload_instances();
        app.select_instance(0);

        let mut terminal = Terminal::new(TestBackend::new(160, 44)).unwrap();

        for nav in Nav::all() {
            app.nav = nav;
            terminal.draw(|frame| app.render(frame)).unwrap();
            let content = buffer_text(&terminal);
            assert!(content.contains("CTMLauncher"), "header missing on {nav:?}");
            if nav.number().is_some() {
                assert!(
                    content.contains(nav.menu_label()),
                    "menu label missing on {nav:?}"
                );
            }
        }

        // Build info panel shows the selected build.
        app.nav = Nav::Versions;
        terminal.draw(|frame| app.render(frame)).unwrap();
        assert!(buffer_text(&terminal).contains("Demo"));

        // Empty-state rendering when no instance is selected.
        app.instances.clear();
        app.instance_state.select(None);
        app.nav = Nav::Versions;
        terminal.draw(|frame| app.render(frame)).unwrap();
        assert!(buffer_text(&terminal).contains("No build selected"));

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn help_overlay_opens_and_closes() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();

        app.show_help();
        assert!(matches!(app.overlay, Some(Overlay::Message { .. })));
        app.handle_overlay_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.overlay.is_none());
        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn create_wizard_and_picker_render() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();

        app.open_create_wizard();
        terminal.draw(|frame| app.render(frame)).unwrap();
        let content = buffer_text(&terminal);
        assert!(content.contains("New Build"), "wizard title missing");
        assert!(content.contains("Clean Build"), "kind option missing");

        // Advance to the configure step.
        app.handle_wizard_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        terminal.draw(|frame| app.render(frame)).unwrap();
        let content = buffer_text(&terminal);
        assert!(content.contains("Minecraft"), "game version field missing");
        assert!(content.contains("Loader"), "loader field missing");

        // The version picker renders its own list.
        app.show_version_picker(
            PickerTarget::WizardGame,
            vec!["1.21.1".into(), "1.20.1".into()],
        );
        terminal.draw(|frame| app.render(frame)).unwrap();
        let content = buffer_text(&terminal);
        assert!(
            content.contains("Minecraft Version"),
            "picker title missing"
        );
        assert!(content.contains("1.21.1"), "picker item missing");

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn instances_toolbar_and_log_scroll() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();
        app.instance_manager
            .create("Demo", "1.21.1", LoaderType::Fabric, Some("0.15.7".into()))
            .await
            .unwrap();
        app.reload_instances();
        app.instance_state.select(Some(0));
        let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();

        // Instances page shows the build action toolbar.
        app.nav = Nav::Instances;
        terminal.draw(|frame| app.render(frame)).unwrap();
        let content = buffer_text(&terminal);
        assert!(content.contains("Launch"), "launch button missing");
        assert!(
            content.contains("Install / Repair"),
            "install button missing"
        );

        // Logs: follow pins to the bottom, scrolling up detaches and moves.
        for i in 0..200 {
            app.log_buffer.push_line(&format!("line {i}"));
        }
        app.nav = Nav::Logs;
        terminal.draw(|frame| app.render(frame)).unwrap();
        assert!(app.log_follow, "should follow by default");
        assert!(app.log_visible > 0);

        let bottom = app.log_scroll;
        app.scroll_logs(-5);
        assert!(!app.log_follow, "scrolling up should stop following");
        assert_eq!(app.log_scroll, bottom.saturating_sub(5));
        assert!(bottom > 0);

        app.jump_logs(true);
        assert!(app.log_follow, "jump to bottom re-enables follow");

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn log_lines_stay_inside_viewport() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();
        app.instance_manager
            .create("Demo", "1.21.1", LoaderType::Fabric, Some("0.15.7".into()))
            .await
            .unwrap();
        app.reload_instances();
        app.select_instance(0);

        let mut long_x = String::new();
        for _ in 0..300 {
            long_x.push('x');
        }
        let wide_chunk = "你好世界这是一个测试日志消息 Привет мир! 日本語のログ ".to_string();
        let wide_msg = vec![
            wide_chunk.clone(),
            wide_chunk.clone(),
            wide_chunk.clone(),
            wide_chunk.clone(),
        ]
        .join("");

        app.log_buffer.push_line("[04:05:06] [main/INFO]: tab\tseparated\tvalues");
        app.log_buffer.push_line(&format!(
            "[01:02:03] [A very long thread name that just keeps going on and on]/ERROR]: {long_x}"
        ));
        for i in 0..6 {
            app.log_buffer.push_line(&format!(
                "[00:00:{i:02}] [Render thread/WARN]: {wide_msg}"
            ));
        }
        app.log_buffer.push_line("\x1b[31m\x1b[1m[12:34:56] [main/INFO]: \x1b[0mstarting up");

        let width: u16 = 100;
        let height: u16 = 30;
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        app.nav = Nav::Logs;
        terminal.draw(|frame| app.render(frame)).unwrap();

        // The 1-column gutter between the content column and the sidebar
        // (`[Min(20), Length(1), Length(30)]`) is never painted by any widget;
        // log text must never spill into it, no matter how long or how wide
        // (CJK) the lines are.
        let buffer = terminal.backend().buffer();
        let gutter_x = width as usize - 31;
        let mut leaked: Vec<(usize, String)> = Vec::new();
        for (idx, cell) in buffer.content().iter().enumerate() {
            let x = idx % (width as usize);
            let y = idx / (width as usize);
            if y >= 2
                && y < (height as usize) - 1
                && x == gutter_x
                && cell.symbol() != " "
            {
                leaked.push((y, cell.symbol().to_string()));
            }
        }
        assert!(
            leaked.is_empty(),
            "log text leaked into the gutter column: {leaked:?}"
        );

        // ANSI escapes are stripped and tabs normalised, so the visible log
        // output contains neither.
        let text = buffer_text(&terminal);
        assert!(!text.contains('\x1b'), "escape bytes leaked into the log view");

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn browse_renders_list_and_detail() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();
        app.instance_manager
            .create("Demo", "1.21.1", LoaderType::Fabric, Some("0.15.7".into()))
            .await
            .unwrap();
        app.reload_instances();
        app.select_instance(0);

        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        app.nav = Nav::Browse;

        // Populate search results directly (no network round trip).
        app.browse.results.push(SearchHit {
            project_id: "abc123".into(),
            project_type: "mod".into(),
            slug: "demo-mod".into(),
            author: "Someone".into(),
            title: "Demo Mod".into(),
            description: "A great demo mod".into(),
            categories: vec!["utility".into()],
            display_categories: vec!["utility".into()],
            versions: vec!["1.0.0".into()],
            downloads: 12345,
            follows: 678,
            icon_url: Some("https://img.modrinth.com/demo.png".into()),
            date_created: String::new(),
            date_modified: String::new(),
            latest_version: Some("1.0.0".into()),
            client_side: "".into(),
            server_side: "".into(),
            color: Some(0x00FF00),
        });
        app.browse.total = 1;
        // A cached icon (even without truecolor the row must still render).
        app.browse_images.insert(
            "https://img.modrinth.com/demo.png".into(),
            mc_core::img::RgbaImage {
                width: 2,
                height: 2,
                pixels: vec![
                    255, 0, 0, 255, 0, 255, 0, 255, //
                    0, 0, 255, 255, 255, 255, 255, 255,
                ],
            },
        );
        terminal.draw(|frame| app.render(frame)).unwrap();
        assert!(buffer_text(&terminal).contains("Demo Mod"), "list title missing");

        // Open the detail page with a markdown body, a version and an icon so the
        // widget-based image rendering path is exercised.
        app.browse.detail = Some(Project {
            id: "abc123".into(),
            slug: "demo-mod".into(),
            title: "Demo Mod".into(),
            description: "A great demo mod".into(),
            body: "# Heading\n\nSome **bold** text.".into(),
            project_type: "mod".into(),
            categories: vec!["utility".into()],
            additional_categories: vec!["Someone".into()],
            client_side: "".into(),
            server_side: "".into(),
            downloads: 12345,
            followers: 678,
            icon_url: Some("https://img.modrinth.com/demo.png".into()),
            color: Some(0x00FF00),
            issues_url: None,
            source_url: None,
            wiki_url: None,
            discord_url: None,
            game_versions: vec!["1.21.1".into()],
            loaders: vec!["fabric".into()],
            versions: vec!["v1".into()],
            published: String::new(),
            updated: String::new(),
            license: None,
            gallery: Vec::new(),
        });
        app.browse.versions.push(Version {
            id: "ver1".into(),
            project_id: "abc123".into(),
            name: "Demo Mod 1.0.0".into(),
            version_number: "1.0.0".into(),
            changelog: None,
            date_published: String::new(),
            downloads: 42,
            version_type: "release".into(),
            status: "listed".into(),
            files: Vec::new(),
            dependencies: Vec::new(),
            game_versions: vec!["1.21.1".into()],
            loaders: vec!["fabric".into()],
        });
        app.browse.focus = BrowseFocus::Body;
        terminal.draw(|frame| app.render(frame)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("Demo Mod"), "detail title missing");
        assert!(text.contains("Heading"), "markdown body missing");
        assert!(text.contains("1.0.0"), "version list missing");
        assert!(
            app.browse_protocols.get("https://img.modrinth.com/demo.png").is_some(),
            "icon render state was not created"
        );
        assert!(
            text.contains('▀') || text.contains('▄'),
            "half-block icon did not render"
        );

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn browse_detail_with_realistic_data_small_terminal() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();
        app.instance_manager
            .create("Demo", "1.21.1", LoaderType::Fabric, Some("0.15.7".into()))
            .await
            .unwrap();
        app.reload_instances();
        app.select_instance(0);

        let mut terminal = Terminal::new(TestBackend::new(72, 22)).unwrap();
        app.nav = Nav::Browse;

        // A realistic markdown body: headings, tables, HTML, lists, emoji.
        let mut body = String::new();
        body.push_str("# A Very Long Mod Title That Keeps Going\n\n");
        body.push_str("Some **bold** and `code` with a [link](https://example.com).\n\n");
        body.push_str("| Col A | Col B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n\n");
        body.push_str("- item one\n- item two\n\n");
        body.push_str("```\nfn main() {}\n```\n\n");
        body.push_str("> quoted text here\n\n");
        for i in 0..40 {
            body.push_str(&format!("Paragraph {i} with some longer content that wraps around. 日志日志日志\n\n"));
        }
        body.push_str("<br>html line<br/>");
        app.browse.detail = Some(Project {
            id: "p1".into(),
            slug: "big-mod".into(),
            title: "Big Mod".into(),
            description: "desc".into(),
            body,
            project_type: "mod".into(),
            categories: Vec::new(),
            additional_categories: vec!["Someone".into()],
            client_side: "".into(),
            server_side: "".into(),
            downloads: 1000000,
            followers: 50000,
            icon_url: None,
            color: None,
            issues_url: None,
            source_url: None,
            wiki_url: None,
            discord_url: None,
            game_versions: vec!["1.21.1".into()],
            loaders: vec!["fabric".into()],
            versions: Vec::new(),
            published: String::new(),
            updated: "2026-01-01T00:00:00Z".into(),
            license: None,
            gallery: Vec::new(),
        });
        for i in 0..30 {
            app.browse.versions.push(Version {
                id: format!("v{i}"),
                project_id: "p1".into(),
                name: format!("Big Mod {i}"),
                version_number: format!("1.0.{i}"),
                changelog: None,
                date_published: String::new(),
                downloads: i as u64,
                version_type: "release".into(),
                status: "listed".into(),
                files: Vec::new(),
                dependencies: Vec::new(),
                game_versions: vec!["1.21.1".into()],
                loaders: vec!["fabric".into()],
            });
        }
        app.browse.focus = BrowseFocus::Body;
        for _ in 0..10 {
            terminal.draw(|frame| app.render(frame)).unwrap();
            app.browse_scroll(1);
            app.key_browse(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
            app.browse_version_move(3);
        }
        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn browse_is_reached_from_mods_and_esc_returns() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();
        app.instance_manager
            .create("Demo", "1.21.1", LoaderType::Fabric, Some("0.15.7".into()))
            .await
            .unwrap();
        app.reload_instances();
        app.select_instance(0);

        // The browser has no sidebar entry.
        assert!(
            !Nav::menu().iter().any(|n| *n == Nav::Browse),
            "Browse must not appear in the sidebar menu"
        );

        // Opening it from the Mods page records Mods as the return page.
        app.open_nav(Nav::Mods);
        app.open_nav(Nav::Browse);
        assert_eq!(app.browse_return, Nav::Mods);

        // Esc returns to the origin page (list mode).
        app.handle_view_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.nav, Nav::Mods);

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn mods_scan_and_toggle() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();
        app.instance_manager
            .create("Demo", "1.21.1", LoaderType::Fabric, Some("0.15.7".into()))
            .await
            .unwrap();
        app.reload_instances();
        app.select_instance(0);

        let Some(instance) = app.selected_instance().cloned() else {
            panic!("no instance selected");
        };
        std::fs::create_dir_all(instance.mods_dir()).unwrap();
        std::fs::write(instance.mods_dir().join("sodium.jar"), br#"fake jar"#).unwrap();

        app.reload_mods();
        let mut scanned = false;
        for _ in 0..64 {
            if let Some(event) = app.engine_rx.recv().await {
                app.handle_engine_event(event);
            }
            if app.installed_mods.len() == 1 {
                scanned = true;
                break;
            }
        }
        assert!(scanned, "mod scan never produced results");
        assert_eq!(app.installed_mods[0].file_name, "sodium.jar");
        assert!(app.installed_mods[0].enabled);

        app.mods_state.select(Some(0));
        app.toggle_selected_mod();
        let mut toggled = false;
        for _ in 0..64 {
            if let Some(event) = app.engine_rx.recv().await {
                app.handle_engine_event(event);
            }
            if !app.installed_mods.is_empty() && !app.installed_mods[0].enabled {
                toggled = true;
                break;
            }
        }
        assert!(toggled, "toggle did not disable the mod");

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn wizard_mouse_actions() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();

        app.open_create_wizard();
        assert!(matches!(app.overlay, Some(Overlay::Wizard(_))));

        // Click the Import tab.
        app.dispatch_overlay_action(OverlayAction::WizardTab(BuildKind::Import));
        let Some(Overlay::Wizard(wizard)) = app.overlay.as_ref() else {
            panic!("wizard missing");
        };
        assert_eq!(wizard.kind, BuildKind::Import);

        // Click a field, then the submit button.
        app.dispatch_overlay_action(OverlayAction::WizardField(0));
        let Some(Overlay::Wizard(wizard2)) = app.overlay.as_ref() else {
            panic!("wizard missing");
        };
        assert_eq!(wizard2.field, 0);

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn picker_click_selects() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();

        app.open_create_wizard();
        app.pending_wizard = Some(crate::wizard::CreateWizard::default());
        app.show_version_picker(
            crate::forms::PickerTarget::WizardGame,
            vec!["1.21.1".into(), "1.20.1".into()],
        );
        assert!(matches!(app.overlay, Some(Overlay::Picker { .. })));

        app.dispatch_overlay_action(OverlayAction::PickerItem(1));
        let Some(Overlay::Wizard(wizard)) = app.overlay.as_ref() else {
            panic!("picker did not restore the wizard");
        };
        assert_eq!(wizard.game_version, "1.20.1");

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn rename_build() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();
        app.instance_manager
            .create(
                "Old Name",
                "1.21.1",
                LoaderType::Fabric,
                Some("0.15.7".into()),
            )
            .await
            .unwrap();
        app.reload_instances();
        app.select_instance(0);

        app.rename_instance("New Name".into());
        let mut renamed = false;
        for _ in 0..64 {
            if let Some(event) = app.engine_rx.recv().await {
                app.handle_engine_event(event);
            }
            app.reload_instances();
            if let Some(instance) = app.selected_instance() {
                if instance.name() == "New Name" {
                    renamed = true;
                    break;
                }
            }
        }
        assert!(renamed, "rename did not take effect");

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn loader_picker_flow() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();

        app.open_create_wizard();
        // Click the Loader field (index 2) on the Clean tab.
        app.dispatch_overlay_action(OverlayAction::WizardPick(2));
        assert!(matches!(app.overlay, Some(Overlay::Picker { .. })));

        // Click "Fabric" (index 1) in the loader picker.
        app.dispatch_overlay_action(OverlayAction::PickerItem(1));
        let Some(Overlay::Wizard(wizard)) = app.overlay.as_ref() else {
            panic!("loader picker did not restore the wizard");
        };
        assert_eq!(wizard.loader_idx, 1);
        assert_eq!(wizard.loader(), mc_core::instance::LoaderType::Fabric);

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }
}
