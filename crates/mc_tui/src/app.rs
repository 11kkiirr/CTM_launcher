//! Application state, event routing and background-task orchestration.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use mc_core::auth::microsoft::MicrosoftAuth;
use mc_core::auth::{Account, AccountKind, AccountStore};
use mc_core::install::Installer;
use mc_core::instance::{Instance, InstanceManager, JvmConfig};
use mc_core::launch::{
    java, resolve_instance_version, select_java, version_id_for, JavaInstallation, Launcher,
    LogReceiver, ProcessHandle,
};
use mc_core::logs::{self, CrashAnalysis, LogBuffer};
use mc_core::modpack::ModpackInstaller;
use mc_core::modrinth::{self, InstalledMod, ModrinthClient, Project, SearchHit, Version};
use mc_core::skins::{SkinClient, SkinVariant};
use mc_core::util::{Paths, Progress, ProgressCallback};
use mc_core::CoreError;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, ListState, Paragraph};
use ratatui::Frame;
use tokio::sync::mpsc;

use crate::engine::{progress_event, EngineEvent, EngineReceiver, EngineSender};
use crate::forms::{
    gc_options, loader_options, parse_gc, parse_loader, ConfirmAction, Form, FormAction, Overlay,
    TextAction,
};
use crate::settings::LauncherSettings;
use crate::theme::Theme;

/// The Azure application (client) id used for Microsoft device-code auth.
pub const CLIENT_ID: &str = mc_core::auth::microsoft::DEFAULT_CLIENT_ID;

/// Top-level navigation tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Instances,
    Modpacks,
    Mods,
    Accounts,
    Logs,
    Settings,
}

impl Tab {
    pub fn all() -> [Tab; 6] {
        [
            Tab::Instances,
            Tab::Modpacks,
            Tab::Mods,
            Tab::Accounts,
            Tab::Logs,
            Tab::Settings,
        ]
    }

    pub fn title(&self) -> &'static str {
        match self {
            Tab::Instances => "Instances",
            Tab::Modpacks => "Modpacks",
            Tab::Mods => "Mod Manager",
            Tab::Accounts => "Accounts & Skins",
            Tab::Logs => "Console / Logs",
            Tab::Settings => "Settings",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Tab::Instances => "▣",
            Tab::Modpacks => "▤",
            Tab::Mods => "✦",
            Tab::Accounts => "☺",
            Tab::Logs => "≣",
            Tab::Settings => "⚙",
        }
    }

    pub fn index(&self) -> usize {
        Tab::all().iter().position(|t| t == self).unwrap_or(0)
    }
}

/// Which region currently owns arrow-key navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    Content,
}

/// A clickable region registered during rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitAction {
    Tab(Tab),
    InstanceRow(usize),
    SearchRow(usize),
    ProjectVersionRow(usize),
    ModRow(usize),
    ModSearchRow(usize),
    AccountRow(usize),
    SettingsRow(usize),
    LogRow(usize),
    Button(ButtonId),
}

/// Identifiers for on-screen action buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonId {
    Launch,
    NewInstance,
    EditInstance,
    DeleteInstance,
    InstallInstance,
    Search,
    ImportModpack,
    InstallProject,
    ToggleMod,
    DeleteMod,
    ModSearch,
    UpdateMods,
    OfflineLogin,
    MicrosoftLogin,
    SetActiveAccount,
    DeleteAccount,
    ChangeSkin,
    ClearLogs,
    PauseLogs,
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

    pub tab: Tab,
    pub focus: Focus,
    pub should_quit: bool,

    pub status: String,
    pub toast: Option<Toast>,
    pub progress: Option<(Option<f64>, String)>,
    pub overlay: Option<Overlay>,
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

    pub log_buffer: LogBuffer,
    pub log_state: ListState,
    pub log_search: String,

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
            tab: Tab::Instances,
            focus: Focus::Content,
            should_quit: false,
            status: "Ready".to_string(),
            toast: None,
            progress: None,
            overlay: None,
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
            log_buffer: LogBuffer::new(5000),
            log_state: ListState::default(),
            log_search: String::new(),
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
        let mut ticker = tokio::time::interval(Duration::from_millis(80));

        while !self.should_quit {
            terminal.draw(|frame| self.render(frame))?;

            tokio::select! {
                maybe_event = reader.next() => {
                    if let Some(Ok(event)) = maybe_event {
                        self.handle_terminal_event(event);
                    }
                }
                maybe_msg = self.engine_rx.recv() => {
                    if let Some(msg) = maybe_msg {
                        self.handle_engine_event(msg);
                    }
                }
                _ = ticker.tick() => self.on_tick(),
            }
        }
        Ok(())
    }

    pub(crate) fn handle_terminal_event(&mut self, event: crossterm::event::Event) {
        match event {
            crossterm::event::Event::Key(key) => {
                if key.kind == crossterm::event::KeyEventKind::Press {
                    self.handle_key(key);
                }
            }
            crossterm::event::Event::Mouse(mouse) => self.handle_mouse(mouse),
            crossterm::event::Event::Resize(_, _) => {}
            _ => {}
        }
    }

    pub(crate) fn on_tick(&mut self) {
        self.drain_process();
        if let Some(toast) = &self.toast {
            if toast.at.elapsed() > Duration::from_secs(6) {
                self.toast = None;
            }
        }
    }

    pub(crate) fn drain_process(&mut self) {
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

        for line in lines {
            self.log_buffer.push_line(&line);
        }

        if let Some(code) = exited {
            let version = self
                .running
                .as_ref()
                .map(|r| r.version.clone())
                .unwrap_or_default();
            self.running = None;
            self.progress = None;
            self.on_process_exit(&version, code);
        }

        if self.settings.log_auto_scroll {
            let len = self.log_buffer.visible().count();
            if len > 0 {
                self.log_state.select(Some(len - 1));
            }
        }
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
                self.tab = Tab::Logs;
            }
            EngineEvent::InstancesChanged => {
                self.reload_instances();
                self.reload_mods();
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
            KeyCode::Char(c @ '1'..='6') => {
                let idx = (c as u8 - b'1') as usize;
                self.select_tab(Tab::all()[idx]);
            }
            _ => self.handle_view_key(key),
        }
    }

    pub(crate) fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Sidebar => Focus::Content,
            Focus::Content => Focus::Sidebar,
        };
    }

    pub(crate) fn select_tab(&mut self, tab: Tab) {
        self.tab = tab;
        self.focus = Focus::Content;
        match tab {
            Tab::Mods => self.reload_mods(),
            Tab::Logs => {
                if self.running.is_none() && self.log_buffer.is_empty() {
                    self.load_latest_log();
                }
            }
            Tab::Accounts => self.reload_accounts(),
            _ => {}
        }
    }

    pub(crate) fn handle_view_key(&mut self, key: KeyEvent) {
        if self.focus == Focus::Sidebar {
            self.handle_sidebar_key(key);
            return;
        }
        match self.tab {
            Tab::Instances => self.key_instances(key),
            Tab::Modpacks => self.key_modpacks(key),
            Tab::Mods => self.key_mods(key),
            Tab::Accounts => self.key_accounts(key),
            Tab::Logs => self.key_logs(key),
            Tab::Settings => self.key_settings(key),
        }
    }

    pub(crate) fn handle_sidebar_key(&mut self, key: KeyEvent) {
        let tabs = Tab::all();
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                let idx = (self.tab.index() + 1).min(tabs.len() - 1);
                self.select_tab(tabs[idx]);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let idx = self.tab.index().saturating_sub(1);
                self.select_tab(tabs[idx]);
            }
            KeyCode::Enter => self.focus = Focus::Content,
            _ => {}
        }
    }

    pub(crate) fn handle_mouse(&mut self, mouse: MouseEvent) {
        if self.overlay.is_some() {
            if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
                self.handle_overlay_click();
            }
            return;
        }
        match mouse.kind {
            MouseEventKind::ScrollDown => self.scroll_active(1),
            MouseEventKind::ScrollUp => self.scroll_active(-1),
            MouseEventKind::Down(MouseButton::Left) => {
                let pos = (mouse.column, mouse.row);
                if let Some(hit) = self
                    .hitboxes
                    .iter()
                    .find(|h| rect_contains(h.rect, pos))
                    .copied()
                {
                    self.dispatch_hit(hit.action);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn dispatch_hit(&mut self, action: HitAction) {
        match action {
            HitAction::Tab(tab) => self.select_tab(tab),
            HitAction::InstanceRow(idx) => {
                self.instance_state.select(Some(idx));
                self.focus = Focus::Content;
            }
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
            HitAction::LogRow(idx) => {
                self.log_state.select(Some(idx));
                self.focus = Focus::Content;
            }
            HitAction::Button(button) => self.dispatch_button(button),
        }
    }

    pub(crate) fn dispatch_button(&mut self, button: ButtonId) {
        match button {
            ButtonId::Launch => self.launch_selected(),
            ButtonId::NewInstance => self.open_create_instance_form(),
            ButtonId::EditInstance => self.open_edit_instance_form(),
            ButtonId::DeleteInstance => self.confirm_delete_instance(),
            ButtonId::InstallInstance => self.install_selected_instance(),
            ButtonId::Search => self.open_search_prompt(),
            ButtonId::ImportModpack => self.open_import_prompt(),
            ButtonId::InstallProject => self.install_selected_project(),
            ButtonId::ToggleMod => self.toggle_selected_mod(),
            ButtonId::DeleteMod => self.confirm_delete_mod(),
            ButtonId::ModSearch => self.open_mod_search_prompt(),
            ButtonId::UpdateMods => self.check_mod_updates(),
            ButtonId::OfflineLogin => self.open_offline_login(),
            ButtonId::MicrosoftLogin => self.start_microsoft_login(),
            ButtonId::SetActiveAccount => self.set_active_account(),
            ButtonId::DeleteAccount => self.confirm_delete_account(),
            ButtonId::ChangeSkin => self.open_skin_prompt(),
            ButtonId::ClearLogs => self.log_buffer.clear(),
            ButtonId::PauseLogs => self.log_buffer.paused = !self.log_buffer.paused,
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
        match self.tab {
            Tab::Instances => move_selection(&mut self.instance_state, self.instances.len(), delta),
            Tab::Modpacks => {
                if self.selected_project.is_some() {
                    move_selection(&mut self.project_state, self.project_versions.len(), delta);
                } else {
                    move_selection(&mut self.search_state, self.search_results.len(), delta);
                }
            }
            Tab::Mods => move_selection(&mut self.mods_state, self.installed_mods.len(), delta),
            Tab::Accounts => move_selection(
                &mut self.account_state,
                self.accounts.accounts().len(),
                delta,
            ),
            Tab::Logs => {
                let len = self.log_buffer.visible().count();
                move_selection(&mut self.log_state, len, delta);
            }
            Tab::Settings => {
                let len = settings_field_count();
                let next = (self.settings_field as i32 + delta).clamp(0, len as i32 - 1) as usize;
                self.settings_field = next;
            }
        }
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

    pub(crate) fn submit_text(&mut self, action: TextAction, text: String) {
        match action {
            TextAction::SearchModrinth => self.run_search(text),
            TextAction::SearchMods => self.run_mod_search(text),
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
            TextAction::None => {}
        }
    }

    pub(crate) fn submit_form(&mut self, form: Form) {
        match form.action {
            FormAction::CreateInstance => self.create_instance_from_form(&form),
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
        let form = Form::new("Create Instance", FormAction::CreateInstance)
            .push_text("Name", "New Instance")
            .push_text("Minecraft Version", "1.20.1")
            .push_choice("Loader", loader_options(), 0)
            .push_text("Loader Version", "")
            .with_hint("blank = latest")
            .push_number("Min RAM (MB)", self.settings.default_min_memory_mb)
            .push_number("Max RAM (MB)", self.settings.default_max_memory_mb)
            .push_choice(
                "Garbage Collector",
                gc_options(),
                gc_index(self.settings.default_gc),
            );
        self.overlay = Some(Overlay::Form(form));
    }

    pub(crate) fn create_instance_from_form(&mut self, form: &Form) {
        let name = form.text_value("Name").unwrap_or("").trim().to_string();
        if name.is_empty() {
            self.set_toast("Instance name cannot be empty", true);
            return;
        }
        let game_version = form
            .text_value("Minecraft Version")
            .unwrap_or("")
            .trim()
            .to_string();
        let loader = parse_loader(form.choice_value("Loader").unwrap_or("Vanilla"));
        let loader_version = form
            .text_value("Loader Version")
            .unwrap_or("")
            .trim()
            .to_string();
        let min = form.number_value("Min RAM (MB)").unwrap_or(512);
        let max = form.number_value("Max RAM (MB)").unwrap_or(4096);
        let gc = parse_gc(form.choice_value("Garbage Collector").unwrap_or("G1GC"));

        let manager = self.instance_manager.clone();
        let installer = Installer::new(
            self.client.clone(),
            self.paths.clone(),
            self.progress_callback(),
        );
        let tx = self.engine_tx.clone();
        self.progress = Some((None, format!("Creating {name}")));

        tokio::spawn(async move {
            let result: Result<Instance, CoreError> = async {
                let instance = manager
                    .create(
                        &name,
                        &game_version,
                        loader,
                        Some(loader_version.clone()).filter(|v| !v.is_empty()),
                    )
                    .await?;
                let mut instance = instance;
                instance.metadata.jvm.min_memory_mb = min;
                instance.metadata.jvm.max_memory_mb = max;
                instance.metadata.jvm.gc = gc;
                instance.save().await?;

                let _ = tx.send(EngineEvent::Status(format!(
                    "Installing {} ...",
                    instance.metadata.descriptor()
                )));
                installer
                    .install_loader(
                        &game_version,
                        loader,
                        Some(loader_version.as_str()).filter(|v| !v.is_empty()),
                    )
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
                let version_id = version_id_for(&instance);
                let json = paths
                    .versions_dir()
                    .join(&version_id)
                    .join(format!("{version_id}.json"));
                if !json.exists() {
                    let _ = tx.send(EngineEvent::Status(format!("Installing {version_id} ...")));
                    installer
                        .install_loader(
                            &instance.metadata.game_version,
                            instance.metadata.loader,
                            instance.metadata.loader_version.as_deref(),
                        )
                        .await?;
                }

                let resolved = resolve_instance_version(&paths, &instance).await?;
                let required = resolved.details.required_java_major();
                let java = match instance.metadata.jvm.java_path.clone() {
                    Some(path) => java::probe(&path).await?,
                    None => select_java(&instance, required).await?,
                };

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

    pub(crate) fn run_search(&mut self, query: String) {
        self.search_query = query.clone();
        self.selected_project = None;
        let modrinth = self.modrinth.clone();
        let tx = self.engine_tx.clone();
        self.progress = Some((None, format!("Searching '{query}'...")));
        tokio::spawn(async move {
            let result = modrinth
                .search(&query, Some("modpack"), None, None, 30, 0)
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
            return;
        };
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
                    30,
                    0,
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

    pub(crate) fn toggle_selected_mod(&mut self) {
        let Some(idx) = self.mods_state.selected() else {
            return;
        };
        let Some(module) = self.installed_mods.get(idx).cloned() else {
            return;
        };
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
                if module.sha1.is_empty() {
                    continue;
                }
                if let Ok(Some((installed, latest))) = modrinth
                    .check_update(&module.sha1, Some(&game_version), Some(&loader))
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
            "Global".to_string(),
            "  1-6          switch tab".to_string(),
            "  Tab          toggle sidebar/content focus".to_string(),
            "  j/k, ↑/↓     move selection".to_string(),
            "  g/G          jump to top/bottom".to_string(),
            "  /            search (context sensitive)".to_string(),
            "  Enter        primary action".to_string(),
            "  q / Ctrl-C   quit".to_string(),
            String::new(),
            "Instances".to_string(),
            "  n new · i install · e edit · d delete · l launch".to_string(),
            String::new(),
            "Mods".to_string(),
            "  Space toggle · d delete · s search · u updates · r refresh".to_string(),
            String::new(),
            "Logs".to_string(),
            "  p pause · c clear · / filter · a analyze crash".to_string(),
            String::new(),
            "Mouse: click tabs/lists/buttons, scroll to navigate.".to_string(),
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

    // ---------------------------------------------------------------------
    // Rendering
    // ---------------------------------------------------------------------

    pub(crate) fn render(&mut self, frame: &mut Frame) {
        self.hitboxes.clear();
        let area = frame.area();
        frame.render_widget(Block::default().style(self.theme.base()), area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(3),
                Constraint::Length(2),
            ])
            .split(area);

        self.render_header(frame, chunks[0]);

        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(22), Constraint::Min(10)])
            .split(chunks[1]);

        self.render_sidebar(frame, body[0]);

        match self.tab {
            Tab::Instances => self.render_instances(frame, body[1]),
            Tab::Modpacks => self.render_modpacks(frame, body[1]),
            Tab::Mods => self.render_mods(frame, body[1]),
            Tab::Accounts => self.render_accounts(frame, body[1]),
            Tab::Logs => self.render_logs(frame, body[1]),
            Tab::Settings => self.render_settings(frame, body[1]),
        }

        self.render_footer(frame, chunks[2]);
        self.render_overlay(frame, area);
    }

    pub(crate) fn render_header(&mut self, frame: &mut Frame, area: Rect) {
        let active = self
            .accounts
            .active()
            .map(|a| {
                format!(
                    "{} ({})",
                    a.username,
                    match a.kind {
                        AccountKind::Microsoft => "Microsoft",
                        AccountKind::Offline => "Offline",
                    }
                )
            })
            .unwrap_or_else(|| "no account".to_string());

        let left = Line::from(vec![
            Span::styled(" CTMLauncher ", self.theme.header()),
            Span::styled(format!("· {} ", self.tab.title()), self.theme.dim()),
        ]);
        let right = Span::styled(format!("{active} "), self.theme.accent());
        let width = area.width as usize;
        let left_len = 2 + self.tab.title().len();
        let pad = width.saturating_sub(left_len + active.len() + 3);
        let line = Line::from(vec![
            Span::styled(" CTMLauncher ", self.theme.header()),
            Span::styled(format!("· {} ", self.tab.title()), self.theme.dim()),
            Span::raw(" ".repeat(pad)),
            right,
        ]);
        frame.render_widget(
            Paragraph::new(if pad == 0 { left } else { line }).style(self.theme.base()),
            area,
        );
    }

    pub(crate) fn render_sidebar(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(ratatui::widgets::Borders::ALL)
            .border_style(self.theme.block_border())
            .style(self.theme.base());
        let inner = block.inner(area);
        frame.render_widget(block, area);

        for (idx, tab) in Tab::all().iter().enumerate() {
            let row = Rect {
                x: inner.x,
                y: inner.y + idx as u16,
                width: inner.width,
                height: 1,
            };
            if row.y >= inner.y + inner.height {
                break;
            }
            let selected = *tab == self.tab;
            let focused = selected && self.focus == Focus::Sidebar;
            let marker = if selected { "▸" } else { " " };
            let style = if focused {
                self.theme.selection()
            } else if selected {
                self.theme.accent()
            } else {
                self.theme.base()
            };
            let line = Line::from(vec![
                Span::styled(format!(" {marker} "), style),
                Span::styled(format!("{} ", tab.icon()), style),
                Span::styled(tab.title().to_string(), style),
            ]);
            frame.render_widget(Paragraph::new(line).style(style), row);
            self.push_hitbox(row, HitAction::Tab(*tab));
        }
    }

    pub(crate) fn render_footer(&mut self, frame: &mut Frame, area: Rect) {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(area);

        let hint = match self.tab {
            Tab::Instances => "n new · Enter/l launch · i install · e edit · d delete",
            Tab::Modpacks => "/ search · Enter open · i install · m import .mrpack · Esc back",
            Tab::Mods => "Space toggle · s search · u updates · d delete · r refresh",
            Tab::Accounts => "n offline · m Microsoft · Enter set active · c skin · d remove",
            Tab::Logs => "p pause · c clear · / filter · a crash · g/G top/bottom",
            Tab::Settings => "Enter edit · j/k move · s save · J detect Java",
        };

        let status_style = if let Some(toast) = &self.toast {
            if toast.error {
                self.theme.error_style()
            } else {
                self.theme.accent()
            }
        } else {
            self.theme.dim()
        };
        let status_text = self
            .toast
            .as_ref()
            .map(|t| t.message.clone())
            .unwrap_or_else(|| self.status.clone());

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!(" {status_text}  "), status_style),
                Span::styled(format!("│  {hint}"), self.theme.dim()),
            ]))
            .style(self.theme.base()),
            rows[0],
        );

        if self.settings.show_progress {
            if let Some((ratio, label)) = &self.progress {
                let gauge =
                    crate::widgets::ProgressBar::new(ratio.unwrap_or(0.0), label, &self.theme);
                frame.render_widget(gauge, rows[1]);
            } else {
                let runtime = self
                    .running
                    .as_ref()
                    .map(|r| {
                        format!(
                            "running: {} ({:.0}s)",
                            r.version,
                            r.started.elapsed().as_secs()
                        )
                    })
                    .unwrap_or_default();
                frame.render_widget(
                    Paragraph::new(Span::styled(format!(" {runtime}"), self.theme.dim()))
                        .style(self.theme.base()),
                    rows[1],
                );
            }
        }
    }

    pub(crate) fn render_overlay(&mut self, frame: &mut Frame, area: Rect) {
        let Some(overlay) = self.overlay.clone() else {
            return;
        };
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
            }
            Overlay::Form(form) => {
                let height = (form.fields.len() as u16 * 2 + 4).min(area.height.saturating_sub(2));
                let popup = crate::widgets::centered_rect(
                    64,
                    (height * 100 / area.height.max(1)).max(20),
                    area,
                );
                let mut lines = Vec::new();
                for (idx, field) in form.fields.iter().enumerate() {
                    let active = idx == form.active;
                    let label_style = if active {
                        self.theme.accent()
                    } else {
                        self.theme.dim()
                    };
                    lines.push(Line::from(Span::styled(
                        format!("{} {}", if active { "▸" } else { " " }, field.label),
                        label_style,
                    )));
                    let value_style = if active {
                        self.theme.selection()
                    } else {
                        self.theme.base()
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
                    lines.push(Line::from(vec![
                        Span::styled(format!("    {display}"), value_style),
                        Span::styled(hint, self.theme.dim()),
                    ]));
                }
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "Tab/↑↓ field · ←→ change · Space toggle · Enter submit · Esc cancel",
                    self.theme.dim(),
                )));
                crate::widgets::render_popup(frame, popup, &form.title, lines, &self.theme);
            }
            Overlay::Confirm { title, message, .. } => {
                let popup = crate::widgets::centered_rect(52, 22, area);
                let lines = vec![
                    Line::from(Span::styled(message, self.theme.warning_style())),
                    Line::from(""),
                    Line::from(Span::styled("[Y]es   [N]o", self.theme.accent())),
                ];
                crate::widgets::render_popup(frame, popup, &title, lines, &self.theme);
            }
            Overlay::Message { title, lines } => {
                let popup = crate::widgets::centered_rect(66, 60, area);
                let mut rendered: Vec<Line> = lines.into_iter().map(Line::from).collect();
                rendered.push(Line::from(""));
                rendered.push(Line::from(Span::styled(
                    "Enter/Esc to close",
                    self.theme.dim(),
                )));
                crate::widgets::render_popup(frame, popup, &title, rendered, &self.theme);
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn temp_paths() -> Paths {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Paths::rooted_at(std::env::temp_dir().join(format!("ctm-tui-{nanos}")))
    }

    #[tokio::test]
    async fn renders_every_tab() {
        let paths = temp_paths();
        let client = reqwest::Client::new();
        let mut app = App::new(paths, client).await.unwrap();

        let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
        for tab in Tab::all() {
            app.tab = tab;
            terminal.draw(|frame| app.render(frame)).unwrap();
            let content: String = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(content.contains("CTMLauncher"), "header missing on {tab:?}");
            assert!(
                content.contains(tab.title()),
                "title missing on {tab:?}: {content}"
            );
        }
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
}
