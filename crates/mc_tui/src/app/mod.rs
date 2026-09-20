mod actions;
mod events;
mod input;
mod render;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use mc_core::auth::AccountStore;
use mc_core::instance::{Instance, InstanceManager};
use mc_core::launch::{JavaInstallation, LogReceiver, ProcessHandle};
use mc_core::logs::{self, CrashAnalysis, LogBuffer};
use mc_core::modrinth::{InstalledMod, ModrinthClient, Project, SearchHit, Version};
use mc_core::util::Paths;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::ListState;
use tokio::sync::mpsc;

use crate::engine::{EngineReceiver, EngineSender};
use crate::forms::{gc_options, Overlay, PickerTarget};
use crate::settings::LauncherSettings;
use crate::theme::Theme;
use crate::views::browse::Browse;
use crate::wizard::BuildKind;

/// The Azure application (client) id used for Microsoft device-code auth.
pub const CLIENT_ID: &str = mc_core::auth::microsoft::DEFAULT_CLIENT_ID;

/// Height in rows of a chunky sidebar navigation block button.
pub(crate) const NAV_BUTTON_HEIGHT: u16 = 3;

/// Navigation entries shown in the right-hand panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Instances,
    Browse,
    Mods,
    Modpacks,
    Versions,
    Jvm,
    Logs,
    Accounts,
    Launcher,
    ResourcePacks,
    Shaders,
    Worlds,
    Screenshots,
}

impl Nav {
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
            Nav::Instances => "\u{2302}",
            Nav::Browse => "\u{25A6}",
            Nav::Mods => "\u{2726}",
            Nav::Modpacks => "\u{26C1}",
            Nav::Versions => "\u{2756}",
            Nav::Jvm => "\u{2699}",
            Nav::Logs => "\u{2263}",
            Nav::Accounts => "\u{263A}",
            Nav::Launcher => "\u{2692}",
            Nav::ResourcePacks => "\u{25E7}",
            Nav::Shaders => "\u{2600}",
            Nav::Worlds => "\u{1F310}",
            Nav::Screenshots => "\u{1F4F7}",
        }
    }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    Content,
}

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
    BrowseResult(usize),
    BrowseVersion(usize),
    BrowseInstall,
    BrowseQuickInstall(usize),
    BrowseFilter(crate::views::browse::FilterItem),
    BrowsePagePrev,
    BrowsePageNext,
    Overlay(OverlayAction),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayAction {
    WizardTab(BuildKind),
    WizardField(usize),
    WizardPick(usize),
    WizardSubmit,
    WizardBrowseImport,
    WizardResult(usize),
    WizardResultOpen,
    WizardVersion(usize),
    WizardVersionInstall,
    PickerItem(usize),
    FormField(usize),
    FormSubmit,
    TextDone,
    ConfirmYes,
    ConfirmNo,
    MessageClose,
}

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

#[derive(Debug, Clone, Copy)]
pub struct Hitbox {
    pub rect: Rect,
    pub action: HitAction,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub message: String,
    pub error: bool,
    pub at: Instant,
}

pub(crate) enum PickerKey {
    None,
    Cancel,
    Select(PickerTarget, String),
}

pub struct RunningProcess {
    pub handle: ProcessHandle,
    pub logs: LogReceiver,
    pub version: String,
    pub started: Instant,
}

pub struct App {
    pub paths: Paths,
    pub client: reqwest::Client,
    pub theme: Theme,
    pub settings: LauncherSettings,

    pub nav: Nav,
    pub focus: Focus,
    pub should_quit: bool,

    pub mouse_pos: Option<(u16, u16)>,
    pub tile_scroll: usize,
    pub tile_columns: usize,
    pub sidebar_area: Rect,

    pub tick: u64,
    pub status: String,
    pub toast: Option<Toast>,
    pub progress: Option<(Option<f64>, String)>,
    pub overlay: Option<Overlay>,
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
    pub log_scroll: usize,
    pub log_follow: bool,
    pub log_visible: usize,
    pub log_search: String,

    pub browse: Browse,
    pub browse_images: HashMap<String, mc_core::img::RgbaImage>,
    pub browse_return: Nav,
    pub picker: ratatui_image::picker::Picker,
    pub browse_protocols: HashMap<String, ratatui_image::protocol::StatefulProtocol>,

    pub local_images: HashMap<String, mc_core::img::RgbaImage>,
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

    pub async fn run(
        &mut self,
        terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    ) -> anyhow::Result<()> {
        use futures::StreamExt;
        let mut reader = crossterm::event::EventStream::new();
        let mut ticker = tokio::time::interval(Duration::from_millis(33));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

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
        if let Some(instance) = self.selected_instance() {
            match logs::analyze_latest(instance) {
                Ok(Some(analysis)) => {
                    let headline = analysis.headline.clone();
                    self.crash_analysis = Some(analysis);
                    self.set_toast(
                        format!("Game crashed ({version}): {headline} \u{2014} press 'a' in Logs"),
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
}

// -------------------------------------------------------------------------
// Free helpers
// -------------------------------------------------------------------------

pub(crate) fn rect_contains(rect: Rect, (x, y): (u16, u16)) -> bool {
    x >= rect.x
        && x < rect.x.saturating_add(rect.width)
        && y >= rect.y
        && y < rect.y.saturating_add(rect.height)
}

pub(crate) fn move_selection(state: &mut ListState, len: usize, delta: i32) {
    if len == 0 {
        state.select(None);
        return;
    }
    let current = state.selected().unwrap_or(0) as i32;
    let next = (current + delta).clamp(0, len as i32 - 1) as usize;
    state.select(Some(next));
}

pub(crate) fn gc_index(gc: mc_core::instance::GcPreset) -> usize {
    gc_options()
        .iter()
        .position(|label| label == gc.label())
        .unwrap_or(0)
}

pub(crate) fn split_args(input: &str) -> Vec<String> {
    input.split_whitespace().map(str::to_string).collect()
}

pub(crate) fn settings_field_count() -> usize {
    7
}

pub(crate) fn truncate_str(input: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = input.chars().collect();
    if chars.len() <= max {
        return input.to_string();
    }
    let mut out: String = chars.into_iter().take(max.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

pub(crate) fn info_line(label: &str, value: &str, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<8}"), theme.card_dim()),
        Span::styled(value.to_string(), theme.card()),
    ])
}

pub(crate) fn footer_hints(nav: Nav) -> &'static [(&'static str, &'static str)] {
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
            ("\u{2190}/\u{2192}", "prev/next"),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use mc_core::instance::LoaderType;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use crate::views::browse::BrowseFocus;

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

        app.nav = Nav::Versions;
        terminal.draw(|frame| app.render(frame)).unwrap();
        assert!(buffer_text(&terminal).contains("Demo"));

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

        app.handle_wizard_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        terminal.draw(|frame| app.render(frame)).unwrap();
        let content = buffer_text(&terminal);
        assert!(content.contains("Minecraft"), "game version field missing");
        assert!(content.contains("Loader"), "loader field missing");

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

        app.nav = Nav::Instances;
        terminal.draw(|frame| app.render(frame)).unwrap();
        let content = buffer_text(&terminal);
        assert!(content.contains("Launch"), "launch button missing");
        assert!(
            content.contains("Install / Repair"),
            "install button missing"
        );

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
        let wide_chunk = "\u{4F60}\u{597D}\u{4E16}\u{754C}\u{8FD9}\u{662F}\u{4E00}\u{4E2A}\u{6D4B}\u{8BD5}\u{65E5}\u{5FD7}\u{6D88}\u{606F} \u{041F}\u{0440}\u{0438}\u{0432}\u{0435}\u{0442} \u{043C}\u{0438}\u{0440}! \u{65E5}\u{672C}\u{8A9E}\u{306E}\u{30ED}\u{30B0} ".to_string();
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
        app.browse_images.insert(
            "https://img.modrinth.com/demo.png".into(),
            mc_core::img::RgbaImage {
                width: 2,
                height: 2,
                pixels: vec![
                    255, 0, 0, 255, 0, 255, 0, 255,
                    0, 0, 255, 255, 255, 255, 255, 255,
                ],
            },
        );
        terminal.draw(|frame| app.render(frame)).unwrap();
        assert!(buffer_text(&terminal).contains("Demo Mod"), "list title missing");

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
            text.contains('\u{2580}') || text.contains('\u{2584}'),
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

        let mut body = String::new();
        body.push_str("# A Very Long Mod Title That Keeps Going\n\n");
        body.push_str("Some **bold** and `code` with a [link](https://example.com).\n\n");
        body.push_str("| Col A | Col B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n\n");
        body.push_str("- item one\n- item two\n\n");
        body.push_str("```\nfn main() {}\n```\n\n");
        body.push_str("> quoted text here\n\n");
        for i in 0..40 {
            body.push_str(&format!("Paragraph {i} with some longer content that wraps around. \u{65E5}\u{5FD7}\u{65E5}\u{5FD7}\u{65E5}\u{5FD7}\n\n"));
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

        assert!(
            !Nav::menu().iter().any(|n| *n == Nav::Browse),
            "Browse must not appear in the sidebar menu"
        );

        app.open_nav(Nav::Mods);
        app.open_nav(Nav::Browse);
        assert_eq!(app.browse_return, Nav::Mods);

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

        app.dispatch_overlay_action(OverlayAction::WizardTab(BuildKind::Import));
        let Some(Overlay::Wizard(wizard)) = app.overlay.as_ref() else {
            panic!("wizard missing");
        };
        assert_eq!(wizard.kind, BuildKind::Import);

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
        app.dispatch_overlay_action(OverlayAction::WizardPick(2));
        assert!(matches!(app.overlay, Some(Overlay::Picker { .. })));

        app.dispatch_overlay_action(OverlayAction::PickerItem(1));
        let Some(Overlay::Wizard(wizard)) = app.overlay.as_ref() else {
            panic!("loader picker did not restore the wizard");
        };
        assert_eq!(wizard.loader_idx, 1);
        assert_eq!(wizard.loader(), mc_core::instance::LoaderType::Fabric);

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }
}
