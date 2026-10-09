mod actions;
mod events;
mod input;
mod render;

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use mc_core::auth::AccountStore;
use mc_core::import::ExternalInstance;
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
use crate::i18n::{self, Lang};
use crate::settings::LauncherSettings;
use crate::theme::Theme;
use crate::views::browse::Browse;
use crate::wizard::BuildKind;

/// The Azure application (client) id used for Microsoft device-code auth.
pub const CLIENT_ID: &str = mc_core::auth::microsoft::DEFAULT_CLIENT_ID;

/// Discord Application id for Rich Presence ("Playing CTMLauncher").
///
/// Register an application in the Discord Developer Portal and use its
/// Application id here. Presence stays inert while the id is empty.
pub const DISCORD_APP_ID: &str = "1557789779439915018";

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

    #[allow(dead_code)]
    pub fn label(&self) -> &'static str {
        i18n::en_static(self.nav_key())
    }

    #[allow(dead_code)]
    pub fn label_lang(&self, lang: Lang) -> String {
        i18n::tr_string(lang, self.nav_key())
    }

    #[allow(dead_code)]
    pub fn menu_label(&self) -> &'static str {
        match self {
            Nav::Jvm => "JVM",
            Nav::ResourcePacks => "Res Packs",
            other => other.label(),
        }
    }

    pub fn menu_label_lang(&self, lang: Lang) -> String {
        match self {
            Nav::Jvm => i18n::tr_string(lang, "nav.jvm_short"),
            Nav::ResourcePacks => i18n::tr_string(lang, "nav.resource_packs_short"),
            other => i18n::tr_string(lang, other.nav_key()),
        }
    }

    fn nav_key(&self) -> &'static str {
        match self {
            Nav::Instances => "nav.instances",
            Nav::Browse => "nav.browse",
            Nav::Mods => "nav.mods",
            Nav::Modpacks => "nav.modpacks",
            Nav::Versions => "nav.versions",
            Nav::Jvm => "nav.jvm",
            Nav::Logs => "nav.logs",
            Nav::Accounts => "nav.accounts",
            Nav::Launcher => "nav.launcher",
            Nav::ResourcePacks => "nav.resource_packs",
            Nav::Shaders => "nav.shaders",
            Nav::Worlds => "nav.worlds",
            Nav::Screenshots => "nav.screenshots",
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
    SearchRow(usize),
    ProjectVersionRow(usize),
    ModRow(usize),
    ModsSearchBar,
    RPSearchBar,
    ShadersSearchBar,
    AccountRow(usize),
    SettingsRow(usize),
    /// Open dropdown option click: `(field_index, option_index)`.
    SettingsOption(usize, usize),
    /// Click on a RAM slider track: `(field_index)`.
    SettingsSlider(usize),
    WorldRow(usize),
    ScreenshotTile(usize),
    Button(ButtonId),
    BrowseResult(usize),
    BrowseVersion(usize),
    BrowseDetailTab(usize),
    BrowseGallery(usize),
    BrowseLink(usize),
    /// Click on a linked image inside the description body: index into
    /// `browse.body_images`.
    BrowseBodyLink(usize),
    /// Click on an inline text link inside the description body: index into
    /// `browse.body_links`.
    BrowseTextLink(usize),
    /// Click on an inline text link inside the changelog tab: index into
    /// `browse.changelog_links`.
    BrowseChangelogLink(usize),
    /// Click on an inline code span in the description body — copy it:
    /// index into `browse.body_codes`.
    CopyBodyCode(usize),
    /// Click on a code block panel in the description body — copy it:
    /// index into `browse.body_code_blocks`.
    CopyBodyCodeBlock(usize),
    BrowseInstall,
    BrowseQuickInstall(usize),
    BrowseSearchBar,
    BrowseFilter(crate::views::browse::FilterItem),
    BrowsePagePrev,
    BrowsePageNext,
    GroupHeader(usize),
    Overlay(OverlayAction),
    Crumb(Crumb),
}

/// A clickable breadcrumb segment in the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Crumb {
    Page(Nav),
    BrowseList,
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
    StopGame,
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
    UpdateMods,
    BrowseMods,
    ShadersBrowse,
    RpBrowse,
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
    DetectJava,
    OpenAsciiBgFolder,
    OpenFolder,
    DeleteSelected,
    ToggleSelected,
    NewGroup,
    RenameGroup,
    DeleteGroup,
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
    /// Unix time (seconds) when the process was started, for presence elapsed.
    pub started_unix: u64,
    /// Instance display name (for presence / toasts).
    pub instance: String,
    /// Second presence line, e.g. `Fabric 1.21.1 · Pack Name`.
    pub state: String,
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
    pub tile_target: usize,
    pub tile_max: usize,
    pub tile_view_h: usize,
    pub tile_columns: usize,
    pub tile_body_w: u16,
    pub tile_visible_rows: usize,
    pub screenshot_scroll: usize,
    pub screenshot_cols: usize,
    pub screenshot_visible_rows: usize,
    pub world_scroll: usize,
    pub world_visible_rows: usize,
    pub sidebar_area: Rect,
    pub toolbar_area: Rect,
    pub toolbar_max: u16,
    pub toolbar_scroll: u16,
    pub toolbar_target: u16,
    pub nav_viewport: Rect,
    pub nav_max: u16,
    pub nav_scroll: u16,
    pub nav_target: u16,

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

    pub groups: Vec<String>,
    /// Active section header for group rename/delete (empty = none).
    pub selected_group: String,
    /// Instance counts keyed by group name (`""` = ungrouped / All).
    pub group_counts: std::collections::HashMap<String, usize>,
    /// Group names whose panel is collapsed (only the header strip shows).
    pub collapsed_groups: HashSet<String>,
    /// After the `g` picker's "[New Group]", the text prompt also moves
    /// the selected instance into the new group.
    pub pending_move_on_new_group: bool,

    pub external_instances: Vec<ExternalInstance>,

    pub accounts: AccountStore,
    pub account_state: ListState,
    /// Visible rows in the accounts list (for wheel view-scroll).
    pub accounts_visible: usize,

    pub modrinth: ModrinthClient,
    pub search_query: String,
    pub search_results: Vec<SearchHit>,
    pub search_state: ListState,
    /// Visible rows in the modpack search list.
    pub search_visible: usize,
    pub selected_project: Option<Box<Project>>,
    pub project_versions: Vec<Version>,
    pub project_state: ListState,
    /// Visible rows in the modpack versions list.
    pub project_visible: usize,

    /// Inline search bar query for filtering installed mods.
    pub mods_search_query: String,
    /// Whether the installed mods search bar is focused.
    pub mods_search_focused: bool,

    pub installed_mods: Vec<InstalledMod>,
    pub mods_state: ListState,
    pub mods_scanning: bool,
    /// Visible rows in the installed mods list (for wheel view-scroll).
    pub mods_visible: usize,

    pub resource_packs: Vec<String>,
    pub resource_packs_state: ListState,
    /// Visible rows in the resource packs list.
    pub resource_packs_visible: usize,
    /// Inline filter for installed resource packs.
    pub rp_search_focused: bool,
    pub rp_inline_query: String,

    pub shaders: Vec<String>,
    pub shaders_state: ListState,
    /// Visible rows in the shaders list.
    pub shaders_visible: usize,
    /// Inline filter for installed shaders.
    pub shaders_search_focused: bool,
    pub shaders_inline_query: String,
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
    pub local_image_requested: HashSet<String>,

    pub crash_analysis: Option<CrashAnalysis>,

    pub running: Option<RunningProcess>,
    pub stop_requested: bool,
    pub last_command: Option<String>,

    /// Discord Rich Presence worker; `None` when the app id is not configured.
    pub presence: Option<mc_core::presence::PresenceHandle>,
    /// Last activity pushed to the presence worker (dedup).
    pub presence_last: Option<mc_core::presence::PresenceActivity>,

    pub java_installations: Vec<JavaInstallation>,
    pub settings_field: usize,
    /// Inline settings editor: `(field_index, text_buffer)` while typing.
    pub settings_edit: Option<(usize, String)>,
    /// Cursor and selection state for the currently active inline edit field.
    pub edit: crate::edit::EditState,
    /// Active mouse selection drag: `(field hit action, field rect)`.
    pub edit_drag: Option<(HitAction, Rect)>,
    /// Open choice dropdown: `(field_index, highlighted_option)`.
    pub settings_dropdown: Option<(usize, usize)>,
    /// Vertical scroll offset for the settings body (sections may overflow).
    pub settings_scroll: u16,
    /// Slider track hitboxes rebuilt each frame: `(field, rect, min, max)`.
    pub settings_sliders: Vec<(usize, Rect, u32, u32)>,

    pub engine_tx: EngineSender,
    pub engine_rx: EngineReceiver,
}

impl App {
    pub async fn new(paths: Paths, client: reqwest::Client) -> anyhow::Result<Self> {
        paths.ensure_layout()?;
        let settings = LauncherSettings::load(&paths).await;
        let instance_manager = InstanceManager::new(paths.clone());
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
            tile_target: 0,
            tile_max: 0,
            tile_view_h: 0,
            tile_columns: 2,
            tile_body_w: 0,
            tile_visible_rows: 1,
            screenshot_scroll: 0,
            screenshot_cols: 1,
            screenshot_visible_rows: 1,
            world_scroll: 0,
            world_visible_rows: 1,
            sidebar_area: Rect::default(),
            toolbar_area: Rect::default(),
            toolbar_max: 0,
            toolbar_scroll: 0,
            toolbar_target: 0,
            nav_viewport: Rect::default(),
            nav_max: 0,
            nav_scroll: 0,
            nav_target: 0,
            tick: 0,
            status: "Ready".to_string(),
            toast: None,
            progress: None,
            overlay: None,
            pending_wizard: None,
            hitboxes: Vec::new(),
            instance_manager,
            instances: Vec::new(),
            instance_state: ListState::default(),
            groups: Vec::new(),
            selected_group: String::new(),
            group_counts: std::collections::HashMap::new(),
            collapsed_groups: HashSet::new(),
            pending_move_on_new_group: false,
            external_instances: Vec::new(),
            accounts,
            account_state: ListState::default(),
            accounts_visible: 0,
            search_query: String::new(),
            search_results: Vec::new(),
            search_state: ListState::default(),
            search_visible: 0,
            selected_project: None,
            project_versions: Vec::new(),
            project_state: ListState::default(),
            project_visible: 0,
            mods_search_query: String::new(),
            mods_search_focused: false,
            installed_mods: Vec::new(),
            mods_state: ListState::default(),
            mods_scanning: false,
            mods_visible: 0,

            resource_packs: Vec::new(),
            resource_packs_state: ListState::default(),
            resource_packs_visible: 0,
            rp_search_focused: false,
            rp_inline_query: String::new(),

            shaders: Vec::new(),
            shaders_state: ListState::default(),
            shaders_visible: 0,
            shaders_search_focused: false,
            shaders_inline_query: String::new(),
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
            local_image_requested: HashSet::new(),
            crash_analysis: None,
            running: None,
            stop_requested: false,
            last_command: None,
            presence: if DISCORD_APP_ID.is_empty() {
                None
            } else {
                Some(mc_core::presence::spawn(DISCORD_APP_ID))
            },
            presence_last: None,
            java_installations: Vec::new(),
            settings_field: 0,
            settings_edit: None,
            edit: crate::edit::EditState::default(),
            edit_drag: None,
            settings_dropdown: None,
            settings_scroll: 0,
            settings_sliders: Vec::new(),
            engine_tx,
            engine_rx,
        };

        if !app.instances.is_empty() {
            app.instance_state.select(Some(0));
        }
        if !app.accounts.accounts().is_empty() {
            app.account_state.select(Some(0));
        }
        app.reload_instances();
        app.log_buffer.filter.min_level = logs::LogLevel::Info;
        app.spawn_java_discovery();
        app.reload_mods();
        app.reload_resource_packs();
        app.reload_shaders();
        app.reload_worlds();
        app.reload_screenshots();
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
                    if self.on_tick() {
                        needs_draw = true;
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn on_tick(&mut self) -> bool {
        self.tick = self.tick.wrapping_add(1);
        let mut changed = self.animate_scrolls();
        changed |= self.drain_process();
        if let Some(toast) = &self.toast {
            if toast.at.elapsed() > Duration::from_secs(6) {
                self.toast = None;
                changed = true;
            }
        }
        changed |= self.progress.is_some();
        changed |= self.browse_link_tick();
        changed |= self.running.is_some() && self.tick % 30 == 0;
        self.sync_presence();
        changed
    }

    /// Current Discord presence activity, or `None` when it must be cleared.
    pub(crate) fn presence_activity(&self) -> Option<mc_core::presence::PresenceActivity> {
        match &self.running {
            Some(running) => {
                self.playing_presence(&running.instance, &running.state, running.started_unix)
            }
            None => {
                if self.settings.discord_presence {
                    // Discord handles are public; keep the idle line fixed in
                    // English and drop the current-page state line.
                    Some(mc_core::presence::PresenceActivity::new(
                        "Idle in CTMLauncher",
                        "",
                    ))
                } else {
                    None
                }
            }
        }
    }

    /// Activity for a running instance: `Playing {instance}` + loader line.
    pub(crate) fn playing_presence(
        &self,
        instance: &str,
        state: &str,
        started_unix: u64,
    ) -> Option<mc_core::presence::PresenceActivity> {
        if !self.settings.discord_presence {
            return None;
        }
        let details = self.trs("presence.playing").replacen("{}", instance, 1);
        Some(
            mc_core::presence::PresenceActivity::new(details, state.to_string())
                .started_at(started_unix),
        )
    }

    /// Push the current activity to the presence worker (deduplicated).
    pub(crate) fn sync_presence(&mut self) {
        let Some(handle) = self.presence.as_ref() else {
            return;
        };
        match self.presence_activity() {
            Some(activity) => {
                if self.presence_last.as_ref() != Some(&activity) {
                    self.presence_last = Some(activity.clone());
                    handle.set(activity);
                }
            }
            None => {
                if self.presence_last.take().is_some() {
                    handle.clear();
                }
            }
        }
    }

    /// Tell the presence worker to clear the activity and exit.
    pub(crate) fn shutdown_presence(&mut self) {
        if let Some(handle) = self.presence.take() {
            handle.shutdown();
        }
    }

    fn animate_scrolls(&mut self) -> bool {
        let mut moved = false;
        moved |= step_toward(&mut self.toolbar_scroll, self.toolbar_target);
        moved |= step_toward(&mut self.nav_scroll, self.nav_target);
        moved |= advance(&mut self.tile_scroll, self.tile_target);
        moved
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
            if self.stop_requested {
                self.stop_requested = false;
                self.set_toast(
                    crate::i18n::tr_string(self.lang(), "toast.game_stopped")
                        .replace("{}", &version),
                    false,
                );
            } else {
                self.on_process_exit(&version, code);
            }
            changed = true;
        }

        changed
    }

    pub(crate) fn on_process_exit(&mut self, version: &str, code: Option<i32>) {
        if code == Some(0) {
            self.set_toast(crate::i18n::tr_string(self.lang(), "toast.game_exited").replace("{}", &version), false);
            return;
        }
        if let Some(instance) = self.selected_instance() {
            match logs::analyze_latest(instance) {
                Ok(Some(analysis)) => {
                    let headline = analysis.headline.clone();
                    self.crash_analysis = Some(analysis);
                    self.set_toast(
                        crate::i18n::tr_string(self.lang(), "toast.game_crashed")
                            .replace("{}", version)
                            .replace("{}", &headline),
                        true,
                    );
                }
                _ => {
                    self.set_toast(
                        crate::i18n::tr_string(self.lang(), "toast.game_exit_code")
                            .replace("{}", &code.map_or("?".into(), |c| c.to_string()))
                            .replace("{}", version),
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

pub(crate) fn step_toward(cur: &mut u16, target: u16) -> bool {
    const SPEED: u16 = 3;
    if *cur < target {
        *cur = (*cur + SPEED).min(target);
        true
    } else if *cur > target {
        *cur = cur.saturating_sub(SPEED);
        true
    } else {
        false
    }
}

fn advance(cur: &mut usize, target: usize) -> bool {
    let diff = target.abs_diff(*cur);
    if diff == 0 {
        return false;
    }
    let speed = (diff / 4).clamp(3, 12);
    if *cur < target {
        *cur = (*cur + speed).min(target);
    } else {
        *cur = cur.saturating_sub(speed);
    }
    true
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

pub(crate) fn settings_field_count(nav: Nav) -> usize {
    match nav {
        Nav::Jvm => 7,
        _ => 10,
    }
}

/// Current unix time in whole seconds (for presence timestamps).
pub(crate) fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl App {
    /// Translate `key` using the current UI language.
    pub fn tr(&self, key: &str) -> &'static str {
        match i18n::tr(self.settings.language, key) {
            std::borrow::Cow::Borrowed(s) => s,
            // Unknown key: fall back to the English table, then the key.
            std::borrow::Cow::Owned(_) => i18n::en_static(key),
        }
    }

    /// Translate `key` into an owned string (for `format!` labels).
    #[allow(dead_code)]
    pub fn trs(&self, key: &str) -> String {
        i18n::tr_string(self.settings.language, key)
    }

    /// Current UI language.
    pub fn lang(&self) -> Lang {
        self.settings.language
    }
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
            ("Enter", "hint.launch"),
            ("n", "hint.new"),
            ("e", "hint.edit"),
            ("i", "hint.install"),
            ("p", "hint.import"),
            ("v", "hint.versions"),
            ("g", "hint.move_group"),
            ("c", "hint.collapse"),
            ("y", "hint.new_group"),
            ("Y", "hint.ren_group"),
            ("D", "hint.del_group"),
            ("r", "hint.rename"),
            ("d", "hint.delete"),
            ("wheel", "hint.scroll"),
        ],
        Nav::Browse => &[
            ("s", "hint.search"),
            ("Enter", "hint.open"),
            ("i", "hint.quick_install"),
            ("[/]", "hint.pages"),
            ("f", "hint.compat"),
            ("c", "hint.side"),
            ("o", "hint.sort"),
        ],
        Nav::Mods => &[
            ("t", "hint.pane"),
            ("Space", "hint.toggle"),
            ("s", "hint.search"),
            ("u", "hint.updates"),
            ("d", "hint.delete"),
            ("wheel", "hint.scroll"),
        ],
        Nav::Modpacks => &[
            ("/", "hint.search"),
            ("Enter", "hint.open"),
            ("i", "hint.install"),
            ("m", "hint.import"),
            ("wheel", "hint.scroll"),
        ],
        Nav::Versions => &[("c", "hint.change_version"), ("r", "hint.reinstall")],
        Nav::Jvm => &[
            ("j/k", "hint.move"),
            ("←/→", "hint.change"),
            ("Enter", "hint.edit"),
            ("Space", "hint.toggle"),
            ("Esc", "hint.cancel"),
        ],
        Nav::Logs => &[
            ("j/k", "hint.scroll"),
            ("f", "hint.follow"),
            ("p", "hint.pause"),
            ("c", "hint.clear"),
            ("/", "hint.filter"),
            ("a", "hint.crash"),
        ],
        Nav::Accounts => &[
            ("n", "hint.offline"),
            ("m", "Microsoft"),
            ("Enter", "hint.active"),
            ("c", "hint.skin"),
            ("d", "hint.remove"),
            ("wheel", "hint.scroll"),
        ],
        Nav::Launcher => &[
            ("j/k", "hint.move"),
            ("←/→", "hint.change"),
            ("Enter", "hint.open_list"),
            ("Space", "hint.toggle"),
            ("J", "hint.detect_java"),
        ],
        Nav::ResourcePacks => &[
            ("Space", "hint.toggle"),
            ("d", "hint.delete"),
            ("Enter", "hint.open_folder"),
            ("t", "hint.search"),
            ("s", "hint.store"),
            ("wheel", "hint.scroll"),
        ],
        Nav::Shaders => &[
            ("Space", "hint.toggle"),
            ("d", "hint.delete"),
            ("Enter", "hint.open_folder"),
            ("t", "hint.search"),
            ("s", "hint.store"),
            ("wheel", "hint.scroll"),
        ],
        Nav::Worlds => &[
            ("Enter", "hint.open_folder"),
            ("d", "hint.delete"),
            ("wheel", "hint.scroll"),
        ],
        Nav::Screenshots => &[
            ("Enter", "hint.open_folder"),
            ("d", "hint.delete"),
            ("\u{2190}/\u{2192}", "hint.prev_next"),
            ("wheel", "hint.scroll"),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use mc_core::instance::LoaderType;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use crate::engine::EngineEvent;
    use crate::forms::ConfirmAction;
    use crate::views::browse::{BrowseFocus, BrowseKind, DetailTab};

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

    fn raster(width: u32, height: u32) -> mc_core::img::RgbaImage {
        mc_core::img::RgbaImage {
            width,
            height,
            pixels: vec![120u8, 100, 200, 255].repeat((width * height) as usize),
        }
    }

    fn buffer_row(terminal: &Terminal<TestBackend>, y: u16, x0: u16, x1: u16) -> String {
        let buf = terminal.backend().buffer();
        (x0..x1).map(|x| buf[(x, y)].symbol()).collect()
    }

    #[tokio::test]
    async fn idle_tick_reports_no_change() {
        let mut app = App::new(temp_paths(), reqwest::Client::new()).await.unwrap();
        app.toast = None;
        assert!(!app.on_tick());
        assert!(!app.on_tick());

        app.progress = Some((None, "busy".to_string()));
        assert!(app.on_tick());
        app.progress = None;

        app.tile_target = app.tile_scroll + 50;
        assert!(app.on_tick());
        for _ in 0..32 {
            app.on_tick();
        }
        assert!(!app.on_tick());

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn presence_activity_idle_game_and_toggle() {
        let mut app = App::new(temp_paths(), reqwest::Client::new()).await.unwrap();
        app.toast = None;

        let idle = app.presence_activity().expect("on by default");
        assert_eq!(idle.details, "Idle in CTMLauncher");
        assert!(idle.state.is_empty());
        assert!(idle.started_unix.is_none());

        app.nav = Nav::Browse;
        let idle = app.presence_activity().unwrap();
        assert_eq!(idle.details, "Idle in CTMLauncher");
        assert!(idle.state.is_empty());

        let playing = app
            .playing_presence("My Pack", "Fabric 1.21.1 · FO", 1_700_000_000)
            .unwrap();
        assert_eq!(playing.details, "Playing My Pack");
        assert_eq!(playing.state, "Fabric 1.21.1 · FO");
        assert_eq!(playing.started_unix, Some(1_700_000_000));

        app.settings.discord_presence = false;
        assert!(app.presence_activity().is_none());
        assert!(app.playing_presence("x", "y", 0).is_none());
        app.settings.discord_presence = true;

        // Empty DISCORD_APP_ID means no worker; sync must stay a quiet no-op.
        app.presence = None;
        app.sync_presence();
        app.sync_presence();
        assert!(!app.on_tick());

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn settings_screen_shows_discord_presence_row() {
        let mut app = App::new(temp_paths(), reqwest::Client::new()).await.unwrap();
        app.nav = Nav::Launcher;
        app.settings_field = 9;
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        let text = buffer_text(&terminal);
        assert!(
            text.contains("Discord Presence"),
            "interface section must list the new toggle"
        );

        app.toggle_setting(9);
        assert!(!app.settings.discord_presence);
        terminal.draw(|frame| app.render(frame)).unwrap();

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn sidebar_power_button_launches_when_idle() {
        let mut app = App::new(temp_paths(), reqwest::Client::new()).await.unwrap();
        app.instance_manager
            .create("Demo", "1.21.1", LoaderType::Fabric, Some("0.15.7".into()))
            .await
            .unwrap();
        app.reload_instances();
        app.select_instance(0);

        let mut terminal = Terminal::new(TestBackend::new(160, 44)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        assert!(
            buffer_text(&terminal).contains("▶"),
            "sidebar must show the launch glyph"
        );
        let hit = app
            .hitboxes
            .iter()
            .find(|h| h.action == HitAction::Button(ButtonId::Launch));
        assert!(hit.is_some(), "sidebar launch hitbox missing");
        let rect = hit.unwrap().rect;
        assert_eq!(rect.y, app.sidebar_area.y);
        assert_eq!(rect.height, 3);
        assert_eq!(rect.width, 5);

        app.stop_selected_game();
        assert!(!app.stop_requested);

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn header_breadcrumbs_navigate() {
        let mut app = App::new(temp_paths(), reqwest::Client::new()).await.unwrap();
        app.instance_manager
            .create("industrial", "1.21.1", LoaderType::Fabric, Some("0.15.7".into()))
            .await
            .unwrap();
        app.reload_instances();
        app.select_instance(0);

        app.nav = Nav::Mods;
        let crumbs = app.breadcrumbs();
        assert_eq!(crumbs.len(), 3);
        assert_eq!(crumbs[0].0, "CTMLauncher");
        assert_eq!(crumbs[1].0, "industrial");
        assert_eq!(crumbs[1].1, Some(Crumb::Page(Nav::Instances)));

        app.nav = Nav::Browse;
        app.browse_return = Nav::Mods;
        app.browse.detail = Some(Project {
            id: "s".into(),
            slug: "sodium".into(),
            title: "Sodium".into(),
            description: String::new(),
            body: String::new(),
            project_type: "mod".into(),
            categories: Vec::new(),
            additional_categories: Vec::new(),
            client_side: String::new(),
            server_side: String::new(),
            downloads: 0,
            followers: 0,
            icon_url: None,
            color: None,
            issues_url: None,
            source_url: None,
            wiki_url: None,
            discord_url: None,
            game_versions: Vec::new(),
            loaders: Vec::new(),
            versions: Vec::new(),
            published: String::new(),
            updated: String::new(),
            license: None,
            gallery: Vec::new(),
        });
        let crumbs = app.breadcrumbs();
        assert!(crumbs.iter().any(|(l, _)| l == "Sodium"));
        let kind = crumbs
            .iter()
            .find(|(_, a)| *a == Some(Crumb::BrowseList))
            .unwrap();
        assert_eq!(
            kind.0,
            format!("{} {}", app.tr("nav.browse"), app.browse.kind.label_lang(app.lang()))
        );

        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        let header = buffer_row(&terminal, 0, 0, 120);
        assert!(header.contains("Sodium"), "header must show the project: {header}");
        assert!(
            app.hitboxes
                .iter()
                .any(|h| h.action == HitAction::Crumb(Crumb::BrowseList)),
            "kind segment must be clickable back to the list"
        );

        app.dispatch_hit(HitAction::Crumb(Crumb::BrowseList));
        assert!(app.browse.detail.is_none());
        app.dispatch_hit(HitAction::Crumb(Crumb::Page(Nav::Mods)));
        assert_eq!(app.nav, Nav::Mods);

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn browse_detail_tabs_layout() {
        let mut app = App::new(temp_paths(), reqwest::Client::new()).await.unwrap();
        app.instance_manager
            .create("Demo", "1.21.1", LoaderType::Fabric, Some("0.15.7".into()))
            .await
            .unwrap();
        app.reload_instances();
        app.select_instance(0);
        app.nav = Nav::Browse;
        for i in 0..3 {
            app.browse_images.insert(
                format!("https://img.modrinth.com/g{i}.png"),
                mc_core::img::RgbaImage {
                    width: 2,
                    height: 2,
                    pixels: vec![
                        255, 0, 0, 255, 0, 255, 0, 255,
                        0, 0, 255, 255, 255, 255, 255, 255,
                    ],
                },
            );
        }
        app.browse.detail = Some(Project {
            id: "p1".into(),
            slug: "sodium".into(),
            title: "Sodium".into(),
            description: "A high-performance rendering engine.".into(),
            body: "# Sodium\n\nFast **renderer**.".into(),
            project_type: "mod".into(),
            categories: vec!["optimization".into()],
            additional_categories: Vec::new(),
            client_side: "required".into(),
            server_side: "unsupported".into(),
            downloads: 232_300_000,
            followers: 41_100,
            icon_url: Some("https://img.modrinth.com/g0.png".into()),
            color: Some(0x00FF00),
            issues_url: Some("https://github.com/issues".into()),
            source_url: None,
            wiki_url: None,
            discord_url: None,
            game_versions: vec![
                "1.21.4".into(),
                "1.21.5".into(),
                "1.21.6".into(),
                "1.21.7".into(),
                "1.21.8".into(),
                "24w14a".into(),
            ],
            loaders: vec!["fabric".into()],
            versions: vec!["v1".into()],
            published: String::new(),
            updated: String::new(),
            license: None,
            gallery: vec![
                mc_core::modrinth::GalleryImage {
                    url: "https://img.modrinth.com/g1.png".into(),
                    raw_url: None,
                    description: None,
                    featured: true,
                },
                mc_core::modrinth::GalleryImage {
                    url: "https://img.modrinth.com/g2.png".into(),
                    raw_url: None,
                    description: None,
                    featured: false,
                },
            ],
        });
        app.browse.versions.push(Version {
            id: "ver1".into(),
            project_id: "p1".into(),
            name: "Sodium 0.9".into(),
            version_number: "0.9".into(),
            changelog: Some("## Changes\n\n- faster".into()),
            date_published: "2026-01-01T00:00:00Z".into(),
            downloads: 42,
            version_type: "release".into(),
            status: "listed".into(),
            files: Vec::new(),
            dependencies: Vec::new(),
            game_versions: vec!["1.21.1".into()],
            loaders: vec!["fabric".into()],
        });

        let mut terminal = Terminal::new(TestBackend::new(120, 34)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("Sodium"), "header title missing");
        assert!(text.contains("232.3M"), "compact stats missing");
        assert!(text.contains("Download"), "download pill missing");
        assert!(text.contains("Compatibility"), "sidebar missing");
        assert!(text.contains("Issues"), "sidebar links missing");
        assert!(
            text.contains("1.21.4-1.21.8"),
            "compact version range missing: {text}"
        );
        assert!(!text.contains("24w14a"), "snapshots must be hidden");
        assert!(
            app.hitboxes
                .iter()
                .any(|h| h.action == HitAction::BrowseInstall),
            "download hitbox missing"
        );
        assert!(
            app.hitboxes
                .iter()
                .any(|h| matches!(h.action, HitAction::BrowseLink(_))),
            "link hitbox missing"
        );

        app.browse.members.push(mc_core::modrinth::Member {
            role: "Owner".into(),
            user: mc_core::modrinth::MemberUser {
                username: "jellysquid3".into(),
                avatar_url: None,
            },
        });
        terminal.draw(|frame| app.render(frame)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("Creators"), "creators section missing");
        assert!(text.contains("jellysquid3"), "creator name missing");

        app.browse_select_tab(2);
        terminal.draw(|frame| app.render(frame)).unwrap();
        assert!(
            buffer_text(&terminal).contains("faster"),
            "changelog tab missing content"
        );

        app.browse_select_tab(3);
        terminal.draw(|frame| app.render(frame)).unwrap();
        assert!(
            buffer_text(&terminal).contains("0.9"),
            "versions tab missing list"
        );

        app.browse_select_tab(1);
        terminal.draw(|frame| app.render(frame)).unwrap();
        assert_eq!(
            app.hitboxes
                .iter()
                .filter(|h| matches!(h.action, HitAction::BrowseGallery(_)))
                .count(),
            2,
            "gallery thumbs missing hitboxes"
        );
        let text = buffer_text(&terminal);
        assert!(text.contains("#1/2"), "gallery caption missing: {text}");
        assert!(
            app.browse_protocols
                .contains_key("preview::https://img.modrinth.com/g1.png"),
            "gallery preview protocol missing"
        );
        assert!(
            app.browse_protocols
                .contains_key("thumb::https://img.modrinth.com/g1.png"),
            "gallery thumb protocol missing"
        );
        app.dispatch_hit(HitAction::BrowseGallery(1));
        assert_eq!(app.browse.gallery_selected, 1);
        terminal.draw(|frame| app.render(frame)).unwrap();
        let sel_thumb = app
            .hitboxes
            .iter()
            .find(|h| h.action == HitAction::BrowseGallery(1))
            .map(|h| h.rect)
            .expect("selected thumb hitbox");
        assert_eq!(
            terminal.backend().buffer()[(sel_thumb.x, sel_thumb.y)].bg,
            app.theme.selection_bg,
            "selected thumb must be highlighted"
        );

        app.dispatch_hit(HitAction::BrowseDetailTab(0));
        assert_eq!(
            app.browse.detail_tab,
            crate::views::browse::DetailTab::Description
        );

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn gallery_strip_scrolls_horizontally() {
        let mut app = App::new(temp_paths(), reqwest::Client::new()).await.unwrap();
        app.nav = Nav::Browse;
        for i in 0..8 {
            app.browse_images
                .insert(format!("https://img.modrinth.com/big{i}.png"), raster(64, 36));
        }
        app.browse.detail = Some(Project {
            id: "big".into(),
            slug: "big".into(),
            title: "Big Gallery".into(),
            description: "gallery".into(),
            body: "body".into(),
            project_type: "mod".into(),
            categories: Vec::new(),
            additional_categories: Vec::new(),
            client_side: "".into(),
            server_side: "".into(),
            downloads: 1,
            followers: 0,
            icon_url: None,
            color: None,
            issues_url: None,
            source_url: None,
            wiki_url: None,
            discord_url: None,
            game_versions: Vec::new(),
            loaders: Vec::new(),
            versions: Vec::new(),
            published: String::new(),
            updated: String::new(),
            license: None,
            gallery: (0..8)
                .map(|i| mc_core::modrinth::GalleryImage {
                    url: format!("https://img.modrinth.com/big{i}.png"),
                    raw_url: None,
                    description: None,
                    featured: false,
                })
                .collect(),
        });
        app.browse_select_tab(1);
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();

        let cols = app.browse.gallery_cols;
        assert!(
            (2..8).contains(&cols),
            "unexpected thumb column count: {cols}"
        );
        let visible = |app: &App| -> Vec<usize> {
            app.hitboxes
                .iter()
                .filter_map(|h| match h.action {
                    HitAction::BrowseGallery(idx) => Some(idx),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(visible(&app).len(), cols, "strip must show one full row");
        assert_eq!(app.browse.gallery_scroll, 0);
        assert!(buffer_text(&terminal).contains("#1/8"), "caption missing");

        app.dispatch_hit(HitAction::BrowseGallery(7));
        assert_eq!(app.browse.gallery_selected, 7);
        assert_eq!(app.browse.gallery_scroll, 8 - cols, "strip must follow");
        terminal.draw(|frame| app.render(frame)).unwrap();
        let shown = visible(&app);
        assert_eq!(shown.len(), cols, "strip row must stay full");
        assert_eq!(shown.last().copied(), Some(7), "selected thumb visible");

        app.browse_scroll(-1);
        assert_eq!(app.browse.gallery_selected, 6, "wheel moves selection");
        app.browse_scroll(1);
        assert_eq!(app.browse.gallery_selected, 7);

        let mut tiny = Terminal::new(TestBackend::new(60, 20)).unwrap();
        tiny.draw(|frame| app.render(frame)).unwrap();

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn browse_link_marquee_on_hover() {
        let mut app = App::new(temp_paths(), reqwest::Client::new()).await.unwrap();
        app.instance_manager
            .create("Demo", "1.21.1", LoaderType::Fabric, Some("0.15.7".into()))
            .await
            .unwrap();
        app.reload_instances();
        app.select_instance(0);
        app.nav = Nav::Browse;
        app.browse.detail = Some(Project {
            id: "p1".into(),
            slug: "sodium".into(),
            title: "Sodium".into(),
            description: String::new(),
            body: String::new(),
            project_type: "mod".into(),
            categories: Vec::new(),
            additional_categories: Vec::new(),
            client_side: String::new(),
            server_side: String::new(),
            downloads: 0,
            followers: 0,
            icon_url: None,
            color: None,
            issues_url: Some("https://github.com/very/long/path/that/does/not/fit".into()),
            source_url: None,
            wiki_url: None,
            discord_url: None,
            game_versions: Vec::new(),
            loaders: Vec::new(),
            versions: Vec::new(),
            published: String::new(),
            updated: String::new(),
            license: None,
            gallery: Vec::new(),
        });

        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        let text = buffer_text(&terminal);
        assert!(!text.contains("https://"), "scheme must be stripped");
        assert!(text.contains("github.com"), "short url missing");
        let chip = app
            .hitboxes
            .iter()
            .filter(|h| matches!(h.action, HitAction::BrowseLink(0)))
            .min_by_key(|h| h.rect.x)
            .unwrap()
            .rect;
        let before: String = buffer_row(&terminal, chip.y, chip.right() + 1, chip.right() + 21);

        app.mouse_pos = Some((chip.x + chip.width + 1, chip.y));
        terminal.draw(|frame| app.render(frame)).unwrap();
        assert_eq!(app.browse.link_hover, Some(0));
        assert!(app.on_tick());
        assert!(app.browse.link_marquee > 0);
        app.tick += 400;
        terminal.draw(|frame| app.render(frame)).unwrap();
        let after: String = buffer_row(&terminal, chip.y, chip.right() + 1, chip.right() + 21);
        assert_ne!(before, after, "marquee must scroll the url");

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
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
    async fn groups_sections_render_and_manage() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();

        app.instance_manager
            .create("Alpha", "1.21.1", LoaderType::Fabric, None)
            .await
            .unwrap();
        app.instance_manager
            .create("Beta", "1.20.1", LoaderType::Vanilla, None)
            .await
            .unwrap();
        app.reload_instances();
        app.nav = Nav::Instances;
        app.focus = Focus::Content;

        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        let content = buffer_text(&terminal);
        assert!(content.contains("Ungrouped"), "ungrouped section header missing");
        assert!(
            !content.contains("Groups"),
            "groups side panel should be gone"
        );

        // Create a group via the core API and verify its section header shows.
        app.instance_manager.create_group("Pack").await.unwrap();
        app.reload_instances();
        terminal.draw(|frame| app.render(frame)).unwrap();
        let content = buffer_text(&terminal);
        assert!(content.contains("Pack"), "created group section missing");
        assert!(content.contains("Alpha"), "instance tiles missing");

        // Click-selecting the Pack section marks it active without filtering.
        app.set_selected_group("Pack".into());
        assert_eq!(app.selected_group, "Pack");
        assert_eq!(app.instances.len(), 2, "all builds stay visible");
        terminal.draw(|frame| app.render(frame)).unwrap();

        // Rename: registry updates, section follows after reload.
        app.instance_manager
            .rename_group("Pack", "Renamed")
            .await
            .unwrap();
        app.selected_group = "Renamed".into();
        app.reload_instances();
        assert!(app.groups.contains(&"Renamed".to_string()));
        assert!(!app.groups.contains(&"Pack".to_string()));
        terminal.draw(|frame| app.render(frame)).unwrap();
        let content = buffer_text(&terminal);
        assert!(content.contains("Renamed"), "renamed section missing");

        // Delete removes the empty group; builds remain.
        app.instance_manager.delete_group("Renamed").await.unwrap();
        app.selected_group.clear();
        app.reload_instances();
        assert!(!app.groups.contains(&"Renamed".to_string()));
        assert!(app.selected_group.is_empty());
        assert_eq!(app.instances.len(), 2);

        // Tab cycles Sidebar ↔ Content only (no Groups focus).
        app.focus = Focus::Sidebar;
        app.toggle_focus();
        assert_eq!(app.focus, Focus::Content);
        app.toggle_focus();
        assert_eq!(app.focus, Focus::Sidebar);

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn group_panels_collapse_and_expand() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();

        app.instance_manager
            .create("Alpha", "1.21.1", LoaderType::Fabric, None)
            .await
            .unwrap();
        app.reload_instances();
        app.nav = Nav::Instances;
        app.focus = Focus::Content;

        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        let content = buffer_text(&terminal);
        // "Ready to play" is unique to the instance tile body (not the sidebar).
        assert!(
            content.contains("Ready to play"),
            "tile should be visible expanded"
        );
        assert!(content.contains("▼"), "expanded chevron missing");

        app.toggle_group_collapsed("");
        assert!(app.collapsed_groups.contains(""));
        terminal.draw(|frame| app.render(frame)).unwrap();
        let content = buffer_text(&terminal);
        assert!(
            !content.contains("Ready to play"),
            "collapsed panel must hide tiles"
        );
        assert!(content.contains("▶"), "collapsed chevron missing");
        assert!(content.contains("Ungrouped"), "header must stay visible");

        app.toggle_selected_group_collapsed();
        assert!(!app.collapsed_groups.contains(""));
        terminal.draw(|frame| app.render(frame)).unwrap();
        let content = buffer_text(&terminal);
        assert!(
            content.contains("Ready to play"),
            "expanded panel must show tiles"
        );

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn digit_keys_open_all_menu_navs_and_skip_while_typing() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();

        for (i, expected) in Nav::menu().into_iter().enumerate() {
            let digit = (b'1' + i as u8) as char;
            app.handle_key(KeyEvent::new(KeyCode::Char(digit), KeyModifiers::NONE));
            assert_eq!(app.nav, expected, "digit {digit} should open {expected:?}");
        }

        app.mods_search_focused = true;
        app.nav = Nav::Mods;
        app.handle_key(KeyEvent::new(KeyCode::Char('9'), KeyModifiers::NONE));
        assert_eq!(app.nav, Nav::Mods, "digits must not switch tabs while typing");
        assert!(app.mods_search_focused, "search focus must stay open");

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn mods_filter_inline_editing_navigation_and_select_all() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();
        app.nav = Nav::Mods;

        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        assert!(app.mods_search_focused, "t must open the filter");
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
        assert_eq!(app.mods_search_query, "so");

        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert_eq!(app.mods_search_query, "sxo", "cursor must move with arrows");

        app.handle_key(KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::CONTROL,
        ));
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        assert_eq!(app.mods_search_query, "z", "ctrl+a must select all");

        app.handle_key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        ));
        assert!(!app.should_quit, "ctrl+c must copy, not quit, while typing");

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!app.mods_search_focused, "escape must close the filter");

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn clicking_and_dragging_mods_filter_moves_cursor_and_selection() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();
        app.instance_manager
            .create("Alpha", "1.21.1", LoaderType::Vanilla, None)
            .await
            .unwrap();
        app.reload_instances();
        app.select_instance(0);
        app.nav = Nav::Mods;
        app.mods_search_query = "sodium".to_string();

        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        let hit = app
            .hitboxes
            .iter()
            .find(|h| matches!(h.action, HitAction::ModsSearchBar))
            .copied()
            .expect("filter hitbox must be registered");

        // The text starts one column right of the field edge (left inset).
        let click = (hit.rect.x + 3, hit.rect.y);
        app.dispatch_click(hit, click);
        assert!(app.mods_search_focused, "click must focus the filter");
        assert_eq!(app.edit.cursor, 2, "cursor must follow the click column");

        // Drag to extend the selection (accent bar shifts columns by 1 once focused).
        app.drag_edit(hit.action, hit.rect, (hit.rect.x + 5, hit.rect.y));
        assert_eq!(app.edit.selection(&app.mods_search_query), Some((2, 4)));

        // The focused filter renders the caret.
        terminal.draw(|frame| app.render(frame)).unwrap();
        let content = buffer_text(&terminal);
        assert!(content.contains('█'), "caret must be rendered while typing");

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn filter_pill_left_inset_is_panel_colored() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();
        app.instance_manager
            .create("Alpha", "1.21.1", LoaderType::Vanilla, None)
            .await
            .unwrap();
        app.reload_instances();
        app.select_instance(0);
        app.nav = Nav::Mods;
        app.mods_search_focused = false;
        app.mods_search_query.clear();

        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        let hit = app
            .hitboxes
            .iter()
            .find(|h| matches!(h.action, HitAction::ModsSearchBar))
            .copied()
            .expect("filter hitbox must be registered");
        let panel = app.theme.panel;
        let cell = terminal.backend().buffer().get(hit.rect.x, hit.rect.y + 1);
        assert_eq!(
            cell.bg,
            panel,
            "left inset cell must be panel-colored, not the dark page background"
        );

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
    async fn hover_scrolls_toolbar_and_sidebar() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();
        app.instance_manager
            .create("Demo", "1.21.1", LoaderType::Fabric, Some("0.15.7".into()))
            .await
            .unwrap();
        app.reload_instances();
        app.instance_state.select(Some(0));

        let mut terminal = Terminal::new(TestBackend::new(70, 14)).unwrap();
        app.nav = Nav::Instances;
        terminal.draw(|frame| app.render(frame)).unwrap();

        assert!(app.toolbar_max > 0, "toolbar must overflow on a narrow window");
        assert!(app.nav_max > 0, "nav must overflow on a short window");

        app.mouse_pos = Some((app.toolbar_area.x + 1, app.toolbar_area.y));
        let nav_before = app.nav_target;
        app.hover_scroll(1);
        assert!(app.toolbar_target > 0, "toolbar should scroll right");
        assert_eq!(
            app.nav_target, nav_before,
            "hovering the toolbar must not scroll the sidebar"
        );

        let scroll_before = app.toolbar_scroll;
        app.on_tick();
        assert!(
            app.toolbar_scroll > scroll_before && app.toolbar_scroll <= app.toolbar_target,
            "toolbar scroll should advance gradually"
        );

        for _ in 0..100 {
            app.hover_scroll(1);
        }
        app.toolbar_scroll = app.toolbar_target;
        terminal.draw(|frame| app.render(frame)).unwrap();
        let (tx, ty, tr) = (
            app.toolbar_area.x,
            app.toolbar_area.y + 1,
            app.toolbar_area.right(),
        );
        let row = buffer_row(&terminal, ty, tx, tr);
        assert!(!row.contains("Launch"), "first pill should scroll out: {row}");
        assert!(row.contains("Grp Del"), "last pill should scroll in: {row}");

        app.mouse_pos = Some((app.sidebar_area.x + 5, app.sidebar_area.y + 5));
        let toolbar_before = app.toolbar_target;
        app.hover_scroll(1);
        assert!(app.nav_target > 0, "hovering the sidebar should scroll the nav");
        assert_eq!(
            app.toolbar_target, toolbar_before,
            "hovering the sidebar must not scroll the toolbar"
        );

        app.nav = Nav::Logs;
        for i in 0..300 {
            app.log_buffer.push_line(&format!("line {i}"));
        }
        terminal.draw(|frame| app.render(frame)).unwrap();
        app.scroll_logs(-10_000);
        let log_before = app.log_scroll;
        app.mouse_pos = Some((10, 8));
        app.hover_scroll(1);
        assert!(app.log_scroll > log_before, "plain content wheel should still scroll the view");

        app.nav_target = 0;
        app.nav_scroll = 0;
        app.open_nav(Nav::Logs);
        assert!(
            app.nav_target > 0,
            "keyboard navigation should scroll the sidebar back to the selection"
        );

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn instances_wheel_scrolls_grid_to_reach_cards() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();
        for (name, version) in [
            ("A1", "1.21.1"),
            ("A2", "1.21.1"),
            ("B1", "1.20.6"),
            ("B2", "1.20.6"),
            ("C1", "1.19.4"),
            ("C2", "1.19.4"),
        ] {
            app.instance_manager
                .create(name, version, LoaderType::Fabric, Some("0.15.7".into()))
                .await
                .unwrap();
        }
        app.reload_instances();
        app.select_instance(0);

        let mut terminal = Terminal::new(TestBackend::new(88, 24)).unwrap();
        app.nav = Nav::Instances;
        terminal.draw(|frame| app.render(frame)).unwrap();

        assert!(app.tile_max > 0, "grid must overflow the viewport");
        let text = buffer_text(&terminal);
        assert!(text.contains("1.21.1"), "first row of cards must render");
        assert!(text.contains("B1"), "a clipped card must still render");
        assert!(
            !text.contains("1.19.4"),
            "the last row of cards must stay below the fold"
        );

        app.mouse_pos = Some((10, 8));
        let target_before = app.tile_target;
        let scroll_before = app.tile_scroll;
        app.hover_scroll(1);
        assert!(
            app.tile_target > target_before,
            "wheel over the grid should scroll it"
        );
        assert_eq!(
            app.tile_scroll, scroll_before,
            "scroll must animate instead of jumping"
        );
        assert_eq!(
            app.instance_state.selected(),
            Some(0),
            "the wheel must not move the selection"
        );

        app.on_tick();
        assert!(
            app.tile_scroll > scroll_before,
            "grid scroll should advance one tick at a time"
        );

        for _ in 0..40 {
            app.hover_scroll(1);
        }
        for _ in 0..100 {
            if app.tile_scroll == app.tile_max {
                break;
            }
            app.on_tick();
        }
        assert_eq!(app.tile_scroll, app.tile_max, "animation must settle");
        terminal.draw(|frame| app.render(frame)).unwrap();
        assert!(
            buffer_text(&terminal).contains("1.19.4"),
            "scrolling must reveal the last row of cards"
        );

        app.tile_scroll = 0;
        app.tile_target = 0;
        app.select_instance(2);
        assert!(
            app.tile_target > 0,
            "selecting a hidden card must scroll it into view"
        );

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
        assert!(
            app.browse_protocols.get("https://img.modrinth.com/demo.png").is_some(),
            "icon render state was not created"
        );
        assert!(
            text.contains('\u{2580}') || text.contains('\u{2584}'),
            "half-block icon did not render"
        );

        app.browse_select_tab(3);
        terminal.draw(|frame| app.render(frame)).unwrap();
        assert!(
            buffer_text(&terminal).contains("1.0.0"),
            "version list missing"
        );

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn browse_description_renders_markdown_images() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        app.nav = Nav::Browse;

        let url = "https://cdn.example/pic.png";
        let pixels: Vec<u8> = (0..32 * 16)
            .flat_map(|i| [i as u8, 100, 200, 255])
            .collect();
        app.browse_images.insert(
            url.to_string(),
            mc_core::img::RgbaImage {
                width: 32,
                height: 16,
                pixels,
            },
        );
        app.browse.detail = Some(Project {
            id: "imgproj".into(),
            slug: "img-proj".into(),
            title: "Img Proj".into(),
            description: "with image".into(),
            body: "Hello\n\n![pic](https://cdn.example/pic.png)\n\nAfter".into(),
            project_type: "mod".into(),
            categories: Vec::new(),
            additional_categories: Vec::new(),
            client_side: "".into(),
            server_side: "".into(),
            downloads: 1,
            followers: 0,
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
            updated: String::new(),
            license: None,
            gallery: Vec::new(),
        });
        app.browse.focus = BrowseFocus::Body;
        terminal.draw(|frame| app.render(frame)).unwrap();

        assert_eq!(app.browse.body_images.len(), 1, "image block not reserved");
        let block = app.browse.body_images[0].clone();
        assert!(block.height >= 1 && block.width >= 1);
        let placeholder_line = &app.browse.body[block.line];
        assert!(
            placeholder_line
                .spans
                .iter()
                .all(|s| s.content.to_string().trim().is_empty()),
            "placeholder should make room for the image"
        );
        assert!(
            app.browse.body.len() >= block.height as usize + 4,
            "image rows should extend the body"
        );
        assert!(
            app.browse_protocols.contains_key(url),
            "image protocol not created"
        );
        let text = buffer_text(&terminal);
        assert!(text.contains("Hello"), "text before image missing");
        assert!(text.contains("After"), "text after image missing");
        assert!(
            text.contains('\u{2580}') || text.contains('\u{2584}'),
            "half-block image did not render"
        );

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn browse_description_packs_adjacent_images_into_centered_rows() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        app.nav = Nav::Browse;

        let wide = "https://cdn.example/wide.png";
        let a = "https://cdn.example/a.png";
        let b = "https://cdn.example/b.png";
        app.browse_images.insert(wide.to_string(), raster(2000, 400));
        app.browse_images.insert(a.to_string(), raster(20, 20));
        app.browse_images.insert(b.to_string(), raster(20, 20));

        app.browse.detail = Some(Project {
            id: "pack".into(),
            slug: "pack".into(),
            title: "Pack".into(),
            description: "pack".into(),
            body: "Intro\n\n![wide](https://cdn.example/wide.png)\n\n<p align=\"center\">\n![a](https://cdn.example/a.png)\n![b](https://cdn.example/b.png)\n</p>\n\nOutro".into(),
            project_type: "mod".into(),
            categories: Vec::new(),
            additional_categories: Vec::new(),
            client_side: "".into(),
            server_side: "".into(),
            downloads: 1,
            followers: 0,
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
            updated: String::new(),
            license: None,
            gallery: Vec::new(),
        });
        app.browse.focus = BrowseFocus::Body;
        terminal.draw(|frame| app.render(frame)).unwrap();

        let body_w = app.browse.body_width as u16;
        let imgs = &app.browse.body_images;
        assert_eq!(imgs.len(), 3, "all images reserved");
        let (im_wide, im_a, im_b) = (&imgs[0], &imgs[1], &imgs[2]);
        assert_eq!(im_wide.url, wide);
        assert!(im_wide.line < im_a.line, "wide image sits on its own row");
        assert_eq!(im_a.line, im_b.line, "badges share one row");
        assert_eq!(
            im_a.line as u16,
            im_wide.line as u16 + im_wide.height + 1,
            "one blank row between wrapped rows"
        );
        assert_eq!(im_wide.x, 0, "full-width row starts at the left edge");
        assert!(im_a.x > 0, "badge row must be centred, not left-aligned");
        assert_eq!(im_b.x, im_a.x + im_a.width + 2, "gap between badges");
        assert!(
            im_a.x + im_a.width + 2 + im_b.width <= body_w,
            "badge row fits the body width"
        );
        for line in app.browse.body.iter() {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert!(!text.contains('▣'), "no placeholders when loaded: {text:?}");
        }
        let text = buffer_text(&terminal);
        assert!(text.contains("Intro"), "text before images missing");
        assert!(text.contains("Outro"), "text after images missing");

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn browse_description_keeps_images_separated_by_text_apart() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        app.nav = Nav::Browse;

        app.browse_images
            .insert("https://cdn.example/a.png".to_string(), raster(20, 20));
        app.browse_images
            .insert("https://cdn.example/b.png".to_string(), raster(20, 20));
        app.browse.detail = Some(Project {
            id: "split".into(),
            slug: "split".into(),
            title: "Split".into(),
            description: "split".into(),
            body: "![a](https://cdn.example/a.png)\n\nmiddle text\n\n![b](https://cdn.example/b.png)".into(),
            project_type: "mod".into(),
            categories: Vec::new(),
            additional_categories: Vec::new(),
            client_side: "".into(),
            server_side: "".into(),
            downloads: 1,
            followers: 0,
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
            updated: String::new(),
            license: None,
            gallery: Vec::new(),
        });
        app.browse.focus = BrowseFocus::Body;
        terminal.draw(|frame| app.render(frame)).unwrap();

        let imgs = &app.browse.body_images;
        assert_eq!(imgs.len(), 2, "both images reserved");
        assert!(
            imgs[0].line + imgs[0].height as usize <= imgs[1].line,
            "text between images keeps them in separate strips"
        );
        assert_eq!(
            imgs[0].x, imgs[1].x,
            "standalone images without a wrapper share the left edge"
        );

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn browse_description_adjacent_images_stay_left_by_default() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        app.nav = Nav::Browse;

        app.browse_images
            .insert("https://cdn.example/a.png".to_string(), raster(20, 20));
        app.browse_images
            .insert("https://cdn.example/b.png".to_string(), raster(20, 20));
        app.browse.detail = Some(Project {
            id: "leftrow".into(),
            slug: "leftrow".into(),
            title: "Left Row".into(),
            description: "left row".into(),
            body: "Intro\n\n![a](https://cdn.example/a.png)\n![b](https://cdn.example/b.png)\n\nOutro".into(),
            project_type: "mod".into(),
            categories: Vec::new(),
            additional_categories: Vec::new(),
            client_side: "".into(),
            server_side: "".into(),
            downloads: 1,
            followers: 0,
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
            updated: String::new(),
            license: None,
            gallery: Vec::new(),
        });
        app.browse.focus = BrowseFocus::Body;
        terminal.draw(|frame| app.render(frame)).unwrap();

        let imgs = &app.browse.body_images;
        assert_eq!(imgs.len(), 2, "both images reserved");
        assert_eq!(imgs[0].line, imgs[1].line, "adjacent images share one row");
        assert_eq!(imgs[0].x, 0, "default alignment is the left edge");
        assert_eq!(imgs[1].x, imgs[0].width + 2, "packed tightly from the left");

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn browse_description_entityculling_layout_modes() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        app.nav = Nav::Browse;

        let banner = "https://tr7zw.github.io/uikit/banner/header_entity_culling.png";
        let discord = "https://tr7zw.github.io/uikit/social_buttons_icon/Discord-Button-64.png";
        let github = "https://tr7zw.github.io/uikit/social_buttons_icon/Github-Button-64.png";
        let divider = "https://tr7zw.github.io/uikit/divider_faded/Divider_01.png";
        let about = "https://tr7zw.github.io/uikit/headlines/large/About.png";
        app.browse_images.insert(banner.to_string(), raster(900, 300));
        app.browse_images.insert(discord.to_string(), raster(60, 64));
        app.browse_images.insert(github.to_string(), raster(60, 64));
        app.browse_images.insert(divider.to_string(), raster(2074, 52));
        app.browse_images.insert(about.to_string(), raster(196, 52));

        app.browse.detail = Some(Project {
            id: "ec".into(),
            slug: "ec".into(),
            title: "EC".into(),
            description: "ec".into(),
            body: format!(
                concat!(
                    "![banner]({})\n\n",
                    "<p align=\"center\">\n",
                    "<a href=\"https://discord.gg/x\"><img src=\"{}\" alt=\"Discord\"></a>\n",
                    "<a href=\"https://github.com/x\"><img src=\"{}\" alt=\"GitHub\"></a>\n",
                    "</p>\n\n",
                    "<br>![Divider]({})\n\n",
                    "<img src=\"{}\" alt=\"About\">"
                ),
                banner, discord, github, divider, about
            ),
            project_type: "mod".into(),
            categories: Vec::new(),
            additional_categories: Vec::new(),
            client_side: "".into(),
            server_side: "".into(),
            downloads: 1,
            followers: 0,
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
            updated: String::new(),
            license: None,
            gallery: Vec::new(),
        });
        app.browse.focus = BrowseFocus::Body;
        terminal.draw(|frame| app.render(frame)).unwrap();

        let body_w = app.browse.body_width as u16;
        let imgs = &app.browse.body_images;
        assert_eq!(imgs.len(), 5, "banner, 2 badges, divider, headline");
        let (im_banner, im_dc, im_gh, im_div, im_about) =
            (&imgs[0], &imgs[1], &imgs[2], &imgs[3], &imgs[4]);

        assert_eq!(im_banner.x, 0, "banner stretches from the left edge");
        assert_eq!(im_banner.width, body_w, "banner fills the body width");

        assert_eq!(im_dc.line, im_gh.line, "badges share one row");
        assert!(im_dc.x > 0, "wrapper row is centred, not left-aligned");
        assert_eq!(im_gh.x, im_dc.x + im_dc.width + 2, "gap between badges");
        assert!(
            im_gh.x + im_gh.width <= body_w,
            "centred badge row fits the body"
        );

        assert_eq!(im_div.x, 0, "divider stretches from the left edge");
        assert_eq!(im_div.width, body_w, "divider fills the body width");
        assert!(
            im_div.line >= im_gh.line + im_gh.height as usize,
            "divider sits below the badge row"
        );

        assert_eq!(im_about.x, 0, "small headline stays on the left edge");
        assert!(
            im_about.width < body_w,
            "small headline keeps its natural size"
        );
        assert!(im_about.line > im_div.line + im_div.height as usize);

        assert_eq!(im_banner.link, None, "plain banner is not a link");
        assert_eq!(
            im_dc.link.as_deref(),
            Some("https://discord.gg/x"),
            "badge keeps its anchor target"
        );
        assert_eq!(
            im_gh.link.as_deref(),
            Some("https://github.com/x"),
            "badge keeps its anchor target"
        );
        assert_eq!(im_div.link, None);
        assert_eq!(im_about.link, None);
        let linked_hits: Vec<_> = app
            .hitboxes
            .iter()
            .filter(|h| matches!(h.action, HitAction::BrowseBodyLink(_)))
            .collect();
        assert_eq!(linked_hits.len(), 2, "only linked badges are clickable");
        assert!(
            linked_hits.iter().any(|h| h.rect.width == im_dc.width),
            "hitbox matches the badge rect"
        );

        let badge_rect = linked_hits[0].rect;
        app.mouse_pos = Some((
            badge_rect.x + badge_rect.width / 2,
            badge_rect.y + badge_rect.height / 2,
        ));
        terminal.draw(|frame| app.render(frame)).unwrap();
        assert_eq!(
            app.browse.hover_hint.as_deref(),
            Some("↗ https://discord.gg/x"),
            "hover exposes the link target"
        );
        let text = buffer_text(&terminal);
        assert!(
            text.contains("↗ https://discord.gg/x"),
            "footer shows the hovered target: {text:?}"
        );

        app.mouse_pos = None;
        terminal.draw(|frame| app.render(frame)).unwrap();
        assert_eq!(
            app.browse.hover_hint, None,
            "hover hint clears when the pointer leaves"
        );

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn browse_description_text_links_are_clickable_and_hinted() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        app.nav = Nav::Browse;

        app.browse.detail = Some(Project {
            id: "textlinks".into(),
            slug: "textlinks".into(),
            title: "Text Links".into(),
            description: "text links".into(),
            body: "Intro one.\n\nRead the [docs](https://example.com/docs) and the [guide](https://wiki.example/guide).".into(),
            project_type: "mod".into(),
            categories: Vec::new(),
            additional_categories: Vec::new(),
            client_side: "".into(),
            server_side: "".into(),
            downloads: 1,
            followers: 0,
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
            updated: String::new(),
            license: None,
            gallery: Vec::new(),
        });
        app.browse.focus = BrowseFocus::Body;
        terminal.draw(|frame| app.render(frame)).unwrap();

        let links = &app.browse.body_links;
        assert_eq!(links.len(), 2, "both inline links are tracked");
        assert_eq!(links[0].url, "https://example.com/docs");
        assert_eq!(links[1].url, "https://wiki.example/guide");
        assert_eq!(
            (links[0].line, links[0].start, links[0].end),
            (2, 9, 13),
            "line remapped past the intro paragraph, range covers `docs`"
        );
        assert_eq!((links[1].start, links[1].end), (22, 27));

        let text_hits: Vec<_> = app
            .hitboxes
            .iter()
            .filter(|h| matches!(h.action, HitAction::BrowseTextLink(_)))
            .map(|h| h.rect)
            .collect();
        assert_eq!(text_hits.len(), 2, "each link has a hitbox");
        assert_eq!(text_hits[0].width, 4, "hitbox covers exactly `docs`");
        assert_eq!(text_hits[1].width, 5, "hitbox covers exactly `guide`");

        app.mouse_pos = Some((
            text_hits[0].x + text_hits[0].width / 2,
            text_hits[0].y + text_hits[0].height / 2,
        ));
        terminal.draw(|frame| app.render(frame)).unwrap();
        assert_eq!(
            app.browse.hover_hint.as_deref(),
            Some("↗ https://example.com/docs")
        );
        let text = buffer_text(&terminal);
        assert!(
            text.contains("↗ https://example.com/docs"),
            "footer shows the hovered text link: {text:?}"
        );

        app.mouse_pos = Some((
            text_hits[1].x + text_hits[1].width / 2,
            text_hits[1].y,
        ));
        terminal.draw(|frame| app.render(frame)).unwrap();
        assert_eq!(
            app.browse.hover_hint.as_deref(),
            Some("↗ https://wiki.example/guide"),
            "second link has its own target"
        );

        app.mouse_pos = None;
        terminal.draw(|frame| app.render(frame)).unwrap();
        assert_eq!(app.browse.hover_hint, None);

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn browse_body_code_is_clickable_and_hinted() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        app.nav = Nav::Browse;

        app.browse.detail = Some(Project {
            id: "code".into(),
            slug: "code".into(),
            title: "Code".into(),
            description: "code".into(),
            body: "Run `cargo build` now.\n\n```\nfn main() {}\n```".into(),
            project_type: "mod".into(),
            categories: Vec::new(),
            additional_categories: Vec::new(),
            client_side: "".into(),
            server_side: "".into(),
            downloads: 1,
            followers: 0,
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
            updated: String::new(),
            license: None,
            gallery: Vec::new(),
        });
        app.browse.focus = BrowseFocus::Body;
        terminal.draw(|frame| app.render(frame)).unwrap();

        assert_eq!(app.browse.body_codes.len(), 1, "inline span is tracked");
        assert_eq!(app.browse.body_codes[0].text, "cargo build");
        assert_eq!(app.browse.body_code_blocks.len(), 1, "block is tracked");
        assert_eq!(app.browse.body_code_blocks[0].text, "fn main() {}");

        let code_hits: Vec<_> = app
            .hitboxes
            .iter()
            .filter(|h| matches!(h.action, HitAction::CopyBodyCode(_)))
            .map(|h| h.rect)
            .collect();
        let block_hits: Vec<_> = app
            .hitboxes
            .iter()
            .filter(|h| matches!(h.action, HitAction::CopyBodyCodeBlock(_)))
            .map(|h| h.rect)
            .collect();
        assert_eq!(code_hits.len(), 1, "inline code has a hitbox");
        assert_eq!(code_hits[0].width, 11, "hitbox covers `cargo build`");
        assert_eq!(block_hits.len(), 1, "code block has a hitbox");
        assert_eq!(block_hits[0].height, 3, "pad row + code row + pad row");

        app.mouse_pos = Some((code_hits[0].x + code_hits[0].width / 2, code_hits[0].y));
        terminal.draw(|frame| app.render(frame)).unwrap();
        assert_eq!(app.browse.hover_hint.as_deref(), Some("⎘ cargo build"));
        let text = buffer_text(&terminal);
        assert!(
            text.contains("⎘ cargo build"),
            "footer shows the code hint: {text:?}"
        );

        app.dispatch_hit(HitAction::CopyBodyCode(0));
        let toast = app.toast.clone().expect("copy reports via toast");
        assert!(!toast.error, "copy succeeded: {toast:?}");
        assert_eq!(toast.message, "Copied to clipboard");

        app.mouse_pos = None;
        terminal.draw(|frame| app.render(frame)).unwrap();
        assert_eq!(app.browse.hover_hint, None);

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn browse_changelog_links_are_clickable() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        app.nav = Nav::Browse;

        app.browse.detail = Some(Project {
            id: "cl".into(),
            slug: "cl".into(),
            title: "CL".into(),
            description: "cl".into(),
            body: String::new(),
            project_type: "mod".into(),
            categories: Vec::new(),
            additional_categories: Vec::new(),
            client_side: "".into(),
            server_side: "".into(),
            downloads: 1,
            followers: 0,
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
            updated: String::new(),
            license: None,
            gallery: Vec::new(),
        });
        app.browse.versions = vec![Version {
            id: "v1".into(),
            project_id: "cl".into(),
            name: "v1".into(),
            version_number: "1.0.0".into(),
            changelog: Some("See [notes](https://example.com/notes) for details.".into()),
            date_published: "2026-01-01T00:00:00Z".into(),
            downloads: 1,
            version_type: "release".into(),
            status: "listed".into(),
            files: Vec::new(),
            dependencies: Vec::new(),
            game_versions: vec!["1.21.1".into()],
            loaders: vec!["fabric".into()],
        }];
        app.browse.detail_tab = DetailTab::Changelog;
        app.browse.focus = BrowseFocus::Body;
        terminal.draw(|frame| app.render(frame)).unwrap();

        let links = &app.browse.changelog_links;
        assert_eq!(links.len(), 1, "changelog link is tracked");
        assert_eq!(links[0].url, "https://example.com/notes");
        assert_eq!(
            (links[0].line, links[0].start, links[0].end),
            (0, 4, 9),
            "range covers `notes` after `See `"
        );

        let hit = app
            .hitboxes
            .iter()
            .find(|h| matches!(h.action, HitAction::BrowseChangelogLink(_)))
            .map(|h| h.rect);
        let hit = hit.expect("changelog link has a hitbox");
        assert_eq!(hit.width, 5, "hitbox covers exactly `notes`");

        app.mouse_pos = Some((hit.x + hit.width / 2, hit.y));
        terminal.draw(|frame| app.render(frame)).unwrap();
        assert_eq!(
            app.browse.hover_hint.as_deref(),
            Some("↗ https://example.com/notes")
        );
        let text = buffer_text(&terminal);
        assert!(
            text.contains("↗ https://example.com/notes"),
            "footer shows the hovered changelog link: {text:?}"
        );

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn browse_description_partial_run_keeps_placeholder_for_missing() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        app.nav = Nav::Browse;

        app.browse_images
            .insert("https://cdn.example/a.png".to_string(), raster(20, 20));
        app.browse.detail = Some(Project {
            id: "partial".into(),
            slug: "partial".into(),
            title: "Partial".into(),
            description: "partial".into(),
            body: "![a](https://cdn.example/a.png)\n![missing](https://cdn.example/nope.png)".into(),
            project_type: "mod".into(),
            categories: Vec::new(),
            additional_categories: Vec::new(),
            client_side: "".into(),
            server_side: "".into(),
            downloads: 1,
            followers: 0,
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
            updated: String::new(),
            license: None,
            gallery: Vec::new(),
        });
        app.browse.focus = BrowseFocus::Body;
        terminal.draw(|frame| app.render(frame)).unwrap();

        assert_eq!(app.browse.body_images.len(), 1, "only the loaded one reserves");
        assert_eq!(app.browse.body_images[0].url, "https://cdn.example/a.png");
        let has_placeholder = app.browse.body.iter().any(|line| {
            line.spans
                .iter()
                .any(|s| s.content.starts_with('▣'))
        });
        assert!(has_placeholder, "missing image keeps its placeholder");

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
    async fn browse_shows_installed_state() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();
        app.instance_manager
            .create("Demo", "1.21.1", LoaderType::Fabric, Some("0.15.7".into()))
            .await
            .unwrap();
        app.reload_instances();
        app.select_instance(0);

        app.browse.kind = BrowseKind::Mods;
        app.browse.results.push(SearchHit {
            project_id: "abc123".into(),
            project_type: "mod".into(),
            slug: "sodium".into(),
            author: "Someone".into(),
            title: "Sodium".into(),
            description: "Engine".into(),
            categories: vec![],
            display_categories: vec![],
            versions: vec![],
            downloads: 1,
            follows: 1,
            icon_url: None,
            date_created: String::new(),
            date_modified: String::new(),
            latest_version: Some("0.6.0".into()),
            client_side: "".into(),
            server_side: "".into(),
            color: None,
        });
        app.browse.results.push(SearchHit {
            project_id: "def456".into(),
            project_type: "mod".into(),
            slug: "lithium".into(),
            author: "Someone".into(),
            title: "Lithium".into(),
            description: "Engine".into(),
            categories: vec![],
            display_categories: vec![],
            versions: vec![],
            downloads: 1,
            follows: 1,
            icon_url: None,
            date_created: String::new(),
            date_modified: String::new(),
            latest_version: Some("0.3.0".into()),
            client_side: "".into(),
            server_side: "".into(),
            color: None,
        });
        app.browse.total = 2;

        // Uninstalled: both show Install
        assert!(!app.is_hit_installed(&app.browse.results[0].clone()));
        assert!(!app.is_hit_installed(&app.browse.results[1].clone()));

        app.installed_mods.push(InstalledMod {
            path: std::path::PathBuf::from("sodium.jar"),
            file_name: "sodium.jar".into(),
            enabled: true,
            sha1: String::new(),
            size: 1,
            mod_name: "Sodium".into(),
            version: "0.6.0".into(),
            mod_id: "sodium".into(),
            install_date: String::new(),
        });

        assert!(app.is_hit_installed(&app.browse.results[0].clone()));
        assert!(!app.is_hit_installed(&app.browse.results[1].clone()));

        // Resource-pack style name matching
        app.browse.kind = BrowseKind::ResourcePacks;
        app.resource_packs.push("Faithful_32x.zip".into());
        let rp_hit = SearchHit {
            project_id: "rp1".into(),
            project_type: "resourcepack".into(),
            slug: "faithful-32x".into(),
            author: "Someone".into(),
            title: "Faithful 32x".into(),
            description: String::new(),
            categories: vec![],
            display_categories: vec![],
            versions: vec![],
            downloads: 1,
            follows: 1,
            icon_url: None,
            date_created: String::new(),
            date_modified: String::new(),
            latest_version: None,
            client_side: "".into(),
            server_side: "".into(),
            color: None,
        };
        assert!(app.is_hit_installed(&rp_hit));

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn browse_wheel_scrolls_view_not_selection() {
        let mut app = App::new(temp_paths(), reqwest::Client::new()).await.unwrap();
        app.instance_manager
            .create("Demo", "1.21.1", LoaderType::Fabric, Some("0.15.7".into()))
            .await
            .unwrap();
        app.reload_instances();
        app.select_instance(0);
        app.nav = Nav::Browse;
        for i in 0..10 {
            app.browse.results.push(SearchHit {
                project_id: format!("id{i}"),
                project_type: "mod".into(),
                slug: format!("mod-{i}"),
                author: "Someone".into(),
                title: format!("Mod {i}"),
                description: String::new(),
                categories: vec![],
                display_categories: vec![],
                versions: vec![],
                downloads: 1,
                follows: 1,
                icon_url: None,
                date_created: String::new(),
                date_modified: String::new(),
                latest_version: None,
                client_side: "".into(),
                server_side: "".into(),
                color: None,
            });
        }
        app.browse.total = 10;
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        let vis = app.browse.list_visible;
        assert!(vis > 0 && vis < 10, "expected partial viewport, got {vis}");

        // Wheel scrolls the view, selection stays.
        app.browse_wheel(1);
        assert_eq!(app.browse.list_scroll, 1);
        assert_eq!(app.browse.selected, 0);
        app.browse_wheel(-1);
        assert_eq!(app.browse.list_scroll, 0);

        // Scroll clamps at the end (no page turn: total fits in offset window).
        for _ in 0..20 {
            app.browse_wheel(1);
        }
        assert_eq!(app.browse.list_scroll, 10 - vis);
        assert_eq!(app.browse.selected, 0);

        // Wheel up at the top with no prev page stays.
        app.browse.list_scroll = 0;
        app.browse_wheel(-1);
        assert_eq!(app.browse.list_scroll, 0);

        // Keyboard moves selection and pulls the view along.
        app.browse.list_scroll = 0;
        app.browse_move(1);
        assert_eq!(app.browse.selected, 1);
        assert_eq!(app.browse.list_scroll, 0);
        app.browse.selected = 9;
        app.browse_ensure_selected_visible();
        assert_eq!(app.browse.list_scroll, 10 - vis);

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

    #[tokio::test]
    async fn inline_filter_closes_on_navigation_and_input_works() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();
        app.instance_manager
            .create("Alpha", "1.21.1", LoaderType::Vanilla, None)
            .await
            .unwrap();
        app.reload_instances();
        app.select_instance(0);

        app.open_nav(Nav::ResourcePacks);
        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        assert!(app.rp_search_focused, "t must open the rp filter");

        app.open_nav(Nav::Shaders);
        assert!(
            !app.rp_search_focused,
            "navigation must close the previous page's inline filter"
        );
        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        assert!(app.shaders_search_focused, "t must open the shaders filter");
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        assert_eq!(app.shaders_inline_query, "a", "typing must insert");

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!app.shaders_search_focused);
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(
            app.should_quit,
            "ctrl+c with no active field must quit the launcher"
        );

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn filter_text_scrolls_horizontally_when_too_long() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();
        app.instance_manager
            .create("Alpha", "1.21.1", LoaderType::Vanilla, None)
            .await
            .unwrap();
        app.reload_instances();
        app.select_instance(0);
        app.open_nav(Nav::Mods);

        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();

        // Typing: caret sits after a long string, it must stay visible.
        let long: String = "x".repeat(80);
        app.mods_search_focused = true;
        app.mods_search_query = long.clone();
        app.edit.reset_with(&long);
        terminal.draw(|frame| app.render(frame)).unwrap();
        let content = buffer_text(&terminal);
        assert!(
            content.contains('█'),
            "caret must stay visible while typing a too-long filter"
        );

        // Idle: the tail of the query must be visible, not the head.
        app.mods_search_focused = false;
        app.mods_search_query = format!("START{}TAIL99", "a".repeat(80));
        terminal.draw(|frame| app.render(frame)).unwrap();
        let content = buffer_text(&terminal);
        assert!(
            content.contains("TAIL99"),
            "the tail of a too-long filter must be scrolled into view"
        );
        assert!(
            !content.contains("START"),
            "the head must be scrolled out of view"
        );

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn installed_mods_rows_align_into_columns() {
        fn char_pos(haystack: &str, needle: &str) -> Option<usize> {
            let h: Vec<char> = haystack.chars().collect();
            let n: Vec<char> = needle.chars().collect();
            if n.is_empty() {
                return Some(0);
            }
            h.windows(n.len()).position(|w| w == n.as_slice())
        }

        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();
        app.instance_manager
            .create("Alpha", "1.21.1", LoaderType::Vanilla, None)
            .await
            .unwrap();
        app.reload_instances();
        app.select_instance(0);
        app.open_nav(Nav::Mods);

        app.installed_mods.push(InstalledMod {
            path: std::path::PathBuf::from("short.jar"),
            file_name: "short.jar".into(),
            enabled: true,
            sha1: String::new(),
            size: 314 * 1024,
            mod_name: "Short".into(),
            version: "1.0".into(),
            mod_id: "short".into(),
            install_date: "2026-05-30 16:35".into(),
        });
        app.installed_mods.push(InstalledMod {
            path: std::path::PathBuf::from("long.jar"),
            file_name: "long.jar".into(),
            enabled: true,
            sha1: String::new(),
            size: 71_300_000,
            mod_name: "A Much Longer Mod Name".into(),
            version: "2.0.0-beta.3".into(),
            mod_id: "long".into(),
            install_date: "2026-01-31 15:06".into(),
        });

        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        let rows: Vec<String> = (0..40)
            .map(|y| {
                (0..120)
                    .map(|x| terminal.backend().buffer()[(x, y)].symbol().to_string())
                    .collect()
            })
            .collect();
        let header = rows
            .iter()
            .find(|l| l.contains("Version"))
            .expect("header row");
        let short_line = rows.iter().find(|l| l.contains("Short")).expect("short row");
        let long_line = rows
            .iter()
            .find(|l| l.contains("A Much Longer"))
            .expect("long row");

        let hv = char_pos(header, "Version").expect("Version in header");
        assert_eq!(char_pos(short_line, "1.0"), Some(hv), "short version");
        assert_eq!(char_pos(long_line, "2.0.0"), Some(hv), "long version");

        let hd = char_pos(header, "Date").expect("Date in header");
        assert_eq!(char_pos(short_line, "2026"), Some(hd), "short date");
        assert_eq!(char_pos(long_line, "2026"), Some(hd), "long date");

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    fn mouse_down(x: u16, y: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        }
    }

    /// First screen point that is not covered by any hitbox.
    fn empty_point(app: &App) -> (u16, u16) {
        (0..40)
            .flat_map(|y| (0..120).map(move |x| (x, y)))
            .find(|&p| app.hit_action_at(p).is_none())
            .expect("some point must be free of hitboxes")
    }

    fn test_project(id: &str, title: &str) -> Project {
        Project {
            id: id.into(),
            slug: id.into(),
            title: title.into(),
            description: "desc".into(),
            body: String::new(),
            project_type: "mod".into(),
            categories: Vec::new(),
            additional_categories: Vec::new(),
            client_side: "".into(),
            server_side: "".into(),
            downloads: 1,
            followers: 1,
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
            updated: String::new(),
            license: None,
            gallery: Vec::new(),
        }
    }

    fn test_version(num: &str, game_versions: &[&str], loaders: &[&str]) -> Version {
        Version {
            id: num.into(),
            project_id: "proj".into(),
            name: num.into(),
            version_number: num.into(),
            changelog: None,
            date_published: String::new(),
            downloads: 1,
            version_type: "release".into(),
            status: "listed".into(),
            files: Vec::new(),
            dependencies: Vec::new(),
            game_versions: game_versions.iter().map(|s| s.to_string()).collect(),
            loaders: loaders.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[tokio::test]
    async fn filtered_mod_actions_use_display_selection() {
        let paths = temp_paths();
        let mut app = App::new(paths, reqwest::Client::new()).await.unwrap();
        app.instance_manager
            .create("Demo", "1.21.1", LoaderType::Fabric, Some("0.15.7".into()))
            .await
            .unwrap();
        app.reload_instances();
        app.select_instance(0);
        let instance = app.selected_instance().cloned().expect("instance");
        std::fs::create_dir_all(instance.mods_dir()).unwrap();
        for name in ["alpha.jar", "beta.jar", "gamma.jar"] {
            std::fs::write(instance.mods_dir().join(name), b"fake jar").unwrap();
        }
        app.installed_mods = ["alpha", "beta", "gamma"]
            .iter()
            .map(|n| InstalledMod {
                path: instance.mods_dir().join(format!("{n}.jar")),
                file_name: format!("{n}.jar"),
                enabled: true,
                sha1: String::new(),
                size: 1,
                mod_name: format!("{n} mod"),
                version: "1.0".into(),
                mod_id: (*n).into(),
                install_date: String::new(),
            })
            .collect();
        app.open_nav(Nav::Mods);
        app.mods_search_query = "gamma".to_string();
        app.mods_state.select(Some(0));

        app.confirm_delete_mod();
        match &app.overlay {
            Some(Overlay::Confirm {
                action: ConfirmAction::DeleteMod(path),
                message,
                ..
            }) => {
                assert!(
                    path.ends_with("gamma.jar"),
                    "delete must target the filtered mod, got {path:?}"
                );
                assert!(message.contains("gamma.jar"), "message: {message}");
            }
            other => panic!("expected a delete confirm, got {other:?}"),
        }
        app.overlay = None;

        app.toggle_selected_mod();
        assert!(
            !app.installed_mods[2].enabled,
            "the filtered mod must be toggled"
        );
        assert!(
            app.installed_mods[0].enabled && app.installed_mods[1].enabled,
            "hidden mods must stay untouched"
        );
        let mut reloaded = false;
        for _ in 0..64 {
            if let Some(EngineEvent::ModsChanged) = app.engine_rx.recv().await {
                reloaded = true;
                break;
            }
        }
        assert!(reloaded, "toggle must rename the file on disk");
        assert!(instance.mods_dir().join("gamma.jar.disabled").exists());
        assert!(instance.mods_dir().join("alpha.jar").exists());

        // Navigation is clamped to the filtered list, not the full one.
        app.key_mods(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.mods_state.selected(), Some(0), "list has one entry");
        app.key_mods(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE));
        assert_eq!(app.mods_state.selected(), Some(0), "jump lands on the only entry");

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn click_outside_search_bar_drops_focus() {
        let paths = temp_paths();
        let mut app = App::new(paths, reqwest::Client::new()).await.unwrap();
        app.instance_manager
            .create("Demo", "1.21.1", LoaderType::Fabric, Some("0.15.7".into()))
            .await
            .unwrap();
        app.reload_instances();
        app.select_instance(0);
        app.installed_mods.push(InstalledMod {
            path: std::path::PathBuf::from("alpha.jar"),
            file_name: "alpha.jar".into(),
            enabled: true,
            sha1: String::new(),
            size: 1,
            mod_name: "Alpha".into(),
            version: "1.0".into(),
            mod_id: "alpha".into(),
            install_date: String::new(),
        });
        app.open_nav(Nav::Mods);
        app.mods_search_focused = true;

        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        let bar = app
            .hitboxes
            .iter()
            .find(|h| h.action == HitAction::ModsSearchBar)
            .copied()
            .expect("search bar hitbox");
        let row = app
            .hitboxes
            .iter()
            .find(|h| matches!(h.action, HitAction::ModRow(_)))
            .copied()
            .expect("mod row hitbox");

        // Clicking the bar itself keeps the focus.
        app.handle_mouse(mouse_down(bar.rect.x + 2, bar.rect.y));
        assert!(app.mods_search_focused, "bar click must keep focus");

        // Clicking a mod row unfocuses and selects the row.
        app.handle_mouse(mouse_down(row.rect.x + 3, row.rect.y));
        assert!(!app.mods_search_focused, "row click must drop focus");
        assert_eq!(app.mods_state.selected(), Some(0));

        // Clicking empty space unfocuses too.
        app.mods_search_focused = true;
        terminal.draw(|frame| app.render(frame)).unwrap();
        let (ex, ey) = empty_point(&app);
        app.handle_mouse(mouse_down(ex, ey));
        assert!(!app.mods_search_focused, "empty click must drop focus");

        // Browse search bar behaves the same way.
        app.open_nav(Nav::Browse);
        app.browse.focus = BrowseFocus::Search;
        terminal.draw(|frame| app.render(frame)).unwrap();
        let browse_bar = app
            .hitboxes
            .iter()
            .find(|h| h.action == HitAction::BrowseSearchBar)
            .copied()
            .expect("browse search hitbox");
        app.handle_mouse(mouse_down(browse_bar.rect.x + 2, browse_bar.rect.y));
        assert_eq!(
            app.browse.focus,
            BrowseFocus::Search,
            "browse bar click must keep focus"
        );
        let (ex, ey) = empty_point(&app);
        app.handle_mouse(mouse_down(ex, ey));
        assert_ne!(
            app.browse.focus,
            BrowseFocus::Search,
            "outside click must leave the browse search"
        );

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn confirm_popup_confirms_only_via_yes_button() {
        let paths = temp_paths();
        let mut app = App::new(paths, reqwest::Client::new()).await.unwrap();
        app.instance_manager
            .create("Demo", "1.21.1", LoaderType::Fabric, Some("0.15.7".into()))
            .await
            .unwrap();
        app.reload_instances();
        app.select_instance(0);

        let file = app.paths.data_dir.join("victim.jar");
        std::fs::create_dir_all(&app.paths.data_dir).unwrap();
        std::fs::write(&file, b"fake jar").unwrap();
        let open = || {
            Overlay::confirm(
                "Delete Mod",
                "Delete 'victim.jar'?",
                ConfirmAction::DeleteMod(file.clone()),
            )
        };

        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        let frame_area = Rect::new(0, 0, 120, 40);
        let popup = crate::widgets::centered_rect(52, 22, frame_area);
        assert!(popup.height > 5, "test popup too small: {popup:?}");

        // Only the two buttons are clickable — never the whole dialog.
        app.overlay = Some(open());
        terminal.draw(|frame| app.render(frame)).unwrap();
        let overlay_hits: Vec<_> = app
            .hitboxes
            .iter()
            .filter(|h| matches!(h.action, HitAction::Overlay(_)))
            .copied()
            .collect();
        assert_eq!(overlay_hits.len(), 2, "exactly Yes and No hitboxes");
        let yes = overlay_hits
            .iter()
            .find(|h| h.action == HitAction::Overlay(OverlayAction::ConfirmYes))
            .copied()
            .expect("Yes hitbox");
        let no = overlay_hits
            .iter()
            .find(|h| h.action == HitAction::Overlay(OverlayAction::ConfirmNo))
            .copied()
            .expect("No hitbox");
        assert!(
            yes.rect.width <= 12 && no.rect.width <= 12,
            "buttons must be small, got yes={} no={}",
            yes.rect.width,
            no.rect.width
        );

        // Click far outside the popup: cancels, file survives.
        app.handle_mouse(mouse_down(1, 1));
        assert!(app.overlay.is_none(), "outside click must close the dialog");
        assert!(file.exists(), "outside click must not delete the file");

        // Click the message text inside the popup: still just a cancel.
        app.overlay = Some(open());
        terminal.draw(|frame| app.render(frame)).unwrap();
        app.handle_mouse(mouse_down(popup.x + popup.width / 2, popup.y + 3));
        assert!(app.overlay.is_none(), "message click must close the dialog");
        assert!(file.exists(), "message click must not delete the file");

        // Click "No": cancels.
        app.overlay = Some(open());
        terminal.draw(|frame| app.render(frame)).unwrap();
        app.handle_mouse(mouse_down(no.rect.x + 1, no.rect.y));
        assert!(app.overlay.is_none(), "No must close the dialog");
        assert!(file.exists(), "No must not delete the file");

        // Click "Yes": confirms and the file goes away.
        app.overlay = Some(open());
        terminal.draw(|frame| app.render(frame)).unwrap();
        app.handle_mouse(mouse_down(yes.rect.x + 1, yes.rect.y));
        assert!(app.overlay.is_none(), "Yes must close the dialog");
        let mut deleted = false;
        for _ in 0..400 {
            if !file.exists() {
                deleted = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(deleted, "Yes must delete the file");

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn mod_hover_highlights_row_under_mouse() {
        let paths = temp_paths();
        let mut app = App::new(paths, reqwest::Client::new()).await.unwrap();
        app.instance_manager
            .create("Demo", "1.21.1", LoaderType::Fabric, Some("0.15.7".into()))
            .await
            .unwrap();
        app.reload_instances();
        app.select_instance(0);
        for n in ["alpha", "beta"] {
            app.installed_mods.push(InstalledMod {
                path: std::path::PathBuf::from(format!("{n}.jar")),
                file_name: format!("{n}.jar"),
                enabled: true,
                sha1: String::new(),
                size: 1,
                mod_name: n.to_string(),
                version: "1.0".into(),
                mod_id: n.to_string(),
                install_date: String::new(),
            });
        }
        app.open_nav(Nav::Mods);
        app.mods_state.select(None);

        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        let mut rows: Vec<_> = app
            .hitboxes
            .iter()
            .filter(|h| matches!(h.action, HitAction::ModRow(_)))
            .copied()
            .collect();
        rows.sort_by_key(|h| h.rect.y);
        assert!(rows.len() >= 2, "need two mod rows");
        let x = rows[0].rect.x + 4;
        let (y0, y1) = (rows[0].rect.y, rows[1].rect.y);

        app.mouse_pos = Some((x, y0));
        terminal.draw(|frame| app.render(frame)).unwrap();
        let buf = terminal.backend().buffer();
        assert_eq!(
            buf[(x, y0)].bg,
            app.theme.hover_bg,
            "the row under the mouse must be hovered"
        );
        assert_ne!(
            buf[(x, y1)].bg,
            app.theme.hover_bg,
            "the next row must not be hovered"
        );

        app.mouse_pos = Some((x, y1));
        terminal.draw(|frame| app.render(frame)).unwrap();
        let buf = terminal.backend().buffer();
        assert_eq!(
            buf[(x, y1)].bg,
            app.theme.hover_bg,
            "second row hover must track the mouse"
        );
        assert_ne!(
            buf[(x, y0)].bg,
            app.theme.hover_bg,
            "first row must lose the hover"
        );

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn browse_versions_default_to_first_compatible() {
        let paths = temp_paths();
        let mut app = App::new(paths, reqwest::Client::new()).await.unwrap();
        app.instance_manager
            .create("Demo", "1.21.1", LoaderType::Fabric, Some("0.15.7".into()))
            .await
            .unwrap();
        app.reload_instances();
        app.select_instance(0);
        app.nav = Nav::Browse;

        app.handle_engine_event(EngineEvent::BrowseProject {
            project: Box::new(test_project("demo", "Demo")),
            versions: vec![
                test_version("26.3.0", &["26.3"], &["fabric"]),
                test_version("0.5.0", &["1.21.1"], &["fabric"]),
                test_version("0.4.0", &["1.21.1"], &["quilt"]),
            ],
            members: Vec::new(),
        });
        assert_eq!(
            app.browse.version_selected, 1,
            "the build for the instance's Minecraft must be preselected"
        );

        // Arrow navigation walks the displayed (compatible-first) order.
        app.browse_version_move(1);
        assert_eq!(app.browse.version_selected, 0, "ordered: [1, 0, 2]");
        app.browse_version_move(1);
        assert_eq!(app.browse.version_selected, 2);
        app.browse_version_move(-1);
        assert_eq!(app.browse.version_selected, 0);

        // Scrolling follows the display position, so a compatible version
        // sitting last in raw order still opens as the first visible row.
        app.handle_engine_event(EngineEvent::BrowseProject {
            project: Box::new(test_project("demo", "Demo")),
            versions: (0..30)
                .map(|i| {
                    if i == 29 {
                        test_version("0.1.0", &["1.21.1"], &["fabric"])
                    } else {
                        test_version(&format!("9.{i}.0"), &["26.3"], &["fabric"])
                    }
                })
                .collect(),
            members: Vec::new(),
        });
        assert_eq!(app.browse.version_selected, 29);
        app.browse_select_tab(3);
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        let mut hits: Vec<_> = app
            .hitboxes
            .iter()
            .filter_map(|h| match h.action {
                HitAction::BrowseVersion(idx) => Some((h.rect, idx)),
                _ => None,
            })
            .collect();
        hits.sort_by_key(|(rect, _)| rect.y);
        let (top_rect, top_idx) = hits.first().expect("version rows").to_owned();
        assert_eq!(
            top_idx, 29,
            "the compatible version must be the top visible row"
        );
        assert_eq!(
            terminal.backend().buffer()[(top_rect.x + 1, top_rect.y)].bg,
            app.theme.selection_bg,
            "top row carries the selection"
        );

        // Wizard picks the version matching its own configured MC + loader.
        let wizard = crate::wizard::CreateWizard {
            game_version: "1.21.1".into(),
            loader_idx: 1,
            ..crate::wizard::CreateWizard::default()
        };
        app.pending_wizard = Some(wizard);
        app.handle_engine_event(EngineEvent::WizardProject {
            project: Box::new(test_project("wiz", "Wizard Mod")),
            versions: vec![
                test_version("26.3.0", &["26.3"], &["fabric"]),
                test_version("0.5.0", &["1.21.1"], &["fabric"]),
                test_version("0.4.0", &["1.21.1"], &["quilt"]),
            ],
        });
        match &app.overlay {
            Some(Overlay::Wizard(w)) => assert_eq!(
                w.selected, 1,
                "wizard must preselect the build for 1.21.1 + fabric"
            ),
            other => panic!("expected wizard overlay, got {other:?}"),
        }

        // Modpack browse: the list selection follows compatibility too.
        app.overlay = None;
        app.handle_engine_event(EngineEvent::Project {
            project: Box::new(test_project("pack", "Pack")),
            versions: vec![
                test_version("26.3.0", &["26.3"], &["fabric"]),
                test_version("0.5.0", &["1.21.1"], &["fabric"]),
            ],
        });
        assert_eq!(
            app.project_state.selected(),
            Some(1),
            "modpack version list must start on the compatible build"
        );

        // Pure helper fallbacks.
        let versions = vec![
            test_version("a", &["1.20.1"], &["fabric"]),
            test_version("b", &["1.21.1"], &["fabric"]),
        ];
        assert_eq!(
            crate::views::browse::first_compatible_index(&versions, Some("1.21.1"), Some("fabric")),
            1
        );
        assert_eq!(
            crate::views::browse::first_compatible_index(&versions, Some("1.21.1"), Some("quilt")),
            1,
            "game-version-only fallback when no loader matches"
        );
        assert_eq!(
            crate::views::browse::first_compatible_index(&versions, Some("1.19"), Some("fabric")),
            0,
            "nothing matches: fall back to the newest build"
        );

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn device_code_overlay_tracks_status_and_shows_errors() {
        let mut app = App::new(temp_paths(), reqwest::Client::new()).await.unwrap();
        let prompt = mc_core::auth::DeviceCodePrompt {
            user_code: "ABCD-EFGH".into(),
            device_code: "device".into(),
            verification_uri: "https://www.microsoft.com/link".into(),
            expires_in: 900,
            interval: 5,
            message: "To sign in, open the page and enter the code.".into(),
        };
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
        let prompt_reopen = || mc_core::auth::DeviceCodePrompt {
            user_code: "IJKL-MNOP".into(),
            device_code: "device".into(),
            verification_uri: "https://www.microsoft.com/link".into(),
            expires_in: 900,
            interval: 5,
            message: String::new(),
        };

        app.handle_engine_event(EngineEvent::DeviceCode(Box::new(prompt)));
        terminal.draw(|frame| app.render(frame)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("ABCD-EFGH"), "user code must be visible");
        assert!(
            text.contains(app.tr("dialog.ms_waiting")),
            "waiting status must be visible"
        );

        app.handle_engine_event(EngineEvent::DeviceCodeWorking);
        terminal.draw(|frame| app.render(frame)).unwrap();
        assert!(
            buffer_text(&terminal).contains(app.tr("dialog.ms_working")),
            "status must flip once the browser step finishes"
        );

        app.handle_engine_event(EngineEvent::Error(
            "Login failed: XSTS 2148916233".into(),
        ));
        assert!(
            matches!(app.overlay, Some(Overlay::Message { .. })),
            "a failed login must replace the waiting popup with a persistent error"
        );
        terminal.draw(|frame| app.render(frame)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("XSTS 2148916233"), "error text must stay visible");
        assert!(text.contains(app.tr("dialog.ms_failed")));
        assert!(text.contains(app.tr("dialog.close")));

        app.overlay = None;
        app.handle_engine_event(EngineEvent::DeviceCode(Box::new(prompt_reopen())));
        app.handle_engine_event(EngineEvent::Error(
            "Login failed: authentication error: Minecraft services login failed: { \"errorMessage\" : \"Invalid app registration, see https://aka.ms/AppRegInfo for more information\"}".into(),
        ));
        terminal.draw(|frame| app.render(frame)).unwrap();
        let text = buffer_text(&terminal);
        assert!(
            text.contains("aka.ms/mce-reviewappid"),
            "unapproved client id must point at the Mojang review form"
        );

        app.handle_engine_event(EngineEvent::DeviceCode(Box::new(prompt_reopen())));
        assert!(
            matches!(app.overlay, Some(Overlay::DeviceCode { .. })),
            "a later attempt must be able to reopen the prompt"
        );

        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn detail_sidebar_clamps_creators_panel() {
        let mut app = App::new(temp_paths(), reqwest::Client::new()).await.unwrap();
        app.instance_manager
            .create("Demo", "1.21.1", LoaderType::Fabric, Some("0.15.7".into()))
            .await
            .unwrap();
        app.reload_instances();
        app.select_instance(0);
        app.nav = Nav::Browse;
        app.browse.detail = Some(Project {
            id: "p1".into(),
            slug: "demo".into(),
            title: "Demo".into(),
            description: "Short summary".into(),
            body: "Hello".into(),
            project_type: "mod".into(),
            categories: Vec::new(),
            additional_categories: Vec::new(),
            client_side: String::new(),
            server_side: String::new(),
            downloads: 1,
            followers: 0,
            icon_url: None,
            color: None,
            issues_url: None,
            source_url: None,
            wiki_url: None,
            discord_url: None,
            game_versions: Vec::new(),
            loaders: Vec::new(),
            versions: Vec::new(),
            published: String::new(),
            updated: String::new(),
            license: None,
            gallery: Vec::new(),
        });
        app.browse.members = (0..6)
            .map(|i| mc_core::modrinth::Member {
                role: "Member".into(),
                user: mc_core::modrinth::MemberUser {
                    username: format!("user{i}"),
                    avatar_url: Some(format!("https://example.com/a{i}.png")),
                },
            })
            .collect();
        // 100x20: the sidebar fits only 4 of the 6 two-row member blocks; the
        // unclamped loop used to draw the 6th avatar at y == 20 → buffer panic.
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("user0"), "first member must render");
        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }

    #[tokio::test]
    async fn detail_renders_lone_pipe_body() {
        let mut app = App::new(temp_paths(), reqwest::Client::new()).await.unwrap();
        app.instance_manager
            .create("Demo", "1.21.1", LoaderType::Fabric, Some("0.15.7".into()))
            .await
            .unwrap();
        app.reload_instances();
        app.select_instance(0);
        app.nav = Nav::Browse;
        let body = "[CurseForge](https://legacy.curseforge.com/minecraft/mc-mods/simple-voice-chat)\n|\n[Discord](https://discord.gg/4dH2zwTmyX)"
            .to_string();
        app.browse.detail = Some(Project {
            id: "p1".into(),
            slug: "simple-voice-chat".into(),
            title: "Simple Voice Chat".into(),
            description: "Short summary".into(),
            body,
            project_type: "mod".into(),
            categories: Vec::new(),
            additional_categories: Vec::new(),
            client_side: String::new(),
            server_side: String::new(),
            downloads: 1,
            followers: 0,
            icon_url: None,
            color: None,
            issues_url: None,
            source_url: None,
            wiki_url: None,
            discord_url: None,
            game_versions: Vec::new(),
            loaders: Vec::new(),
            versions: Vec::new(),
            published: String::new(),
            updated: String::new(),
            license: None,
            gallery: Vec::new(),
        });
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("CurseForge"), "body must render");
        let _ = std::fs::remove_dir_all(&app.paths.data_dir);
    }
}


