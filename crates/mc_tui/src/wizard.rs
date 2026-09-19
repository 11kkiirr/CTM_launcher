//! The "New Build" creation wizard: clean build, `.mrpack` import or a
//! Modrinth modpack.

use crossterm::event::{KeyCode, KeyEvent};
use mc_core::instance::LoaderType;
use mc_core::modrinth::{Project, SearchHit, Version};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::forms::{Overlay, PickerTarget};
use crate::views::row_style;

/// The three ways to create a build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildKind {
    Clean,
    Import,
    Modrinth,
}

impl BuildKind {
    pub fn all() -> [BuildKind; 3] {
        [BuildKind::Clean, BuildKind::Import, BuildKind::Modrinth]
    }

    pub fn label(&self) -> &'static str {
        match self {
            BuildKind::Clean => "Clean Build",
            BuildKind::Import => "Import .mrpack",
            BuildKind::Modrinth => "Modrinth Modpack",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            BuildKind::Clean => "Vanilla or a modloader, configured from scratch.",
            BuildKind::Import => "Import a local Modrinth .mrpack file.",
            BuildKind::Modrinth => "Browse and install a modpack from Modrinth.",
        }
    }
}

/// Wizard steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardStep {
    Kind,
    Configure,
    ModrinthSearch,
    ModrinthProject,
}

/// State for the create-build wizard.
#[derive(Debug, Clone)]
pub struct CreateWizard {
    pub kind: BuildKind,
    pub step: WizardStep,
    pub name: String,
    pub game_version: String,
    pub loader_idx: usize,
    pub loader_version: String,
    pub path: String,
    pub query: String,
    pub results: Vec<SearchHit>,
    pub selected: usize,
    pub project: Option<Project>,
    pub project_versions: Vec<Version>,
    pub field: usize,
}

impl Default for CreateWizard {
    fn default() -> Self {
        Self {
            kind: BuildKind::Clean,
            step: WizardStep::Kind,
            name: "New Build".to_string(),
            game_version: "1.21.1".to_string(),
            loader_idx: 0,
            loader_version: String::new(),
            path: String::new(),
            query: String::new(),
            results: Vec::new(),
            selected: 0,
            project: None,
            project_versions: Vec::new(),
            field: 0,
        }
    }
}

impl CreateWizard {
    pub fn loaders() -> [LoaderType; 6] {
        [
            LoaderType::Vanilla,
            LoaderType::Fabric,
            LoaderType::Quilt,
            LoaderType::Forge,
            LoaderType::NeoForge,
            LoaderType::Paper,
        ]
    }

    pub fn loader(&self) -> LoaderType {
        Self::loaders()[self.loader_idx % Self::loaders().len()]
    }

    pub fn cycle_loader(&mut self, forward: bool) {
        let count = Self::loaders().len();
        self.loader_idx = if forward {
            (self.loader_idx + 1) % count
        } else {
            (self.loader_idx + count - 1) % count
        };
    }

    fn field_count(&self) -> usize {
        match self.kind {
            BuildKind::Clean => 5,
            BuildKind::Import => 2,
            BuildKind::Modrinth => 1,
        }
    }
}

impl App {
    pub(crate) fn open_create_wizard(&mut self) {
        self.overlay = Some(Overlay::Wizard(CreateWizard::default()));
    }

    pub(crate) fn handle_wizard_key(&mut self, key: KeyEvent) {
        let step = match self.overlay.as_ref() {
            Some(Overlay::Wizard(wizard)) => wizard.step,
            _ => return,
        };
        match step {
            WizardStep::Kind => self.handle_wizard_kind(key),
            WizardStep::Configure => self.handle_wizard_configure(key),
            WizardStep::ModrinthSearch => self.handle_wizard_search(key),
            WizardStep::ModrinthProject => self.handle_wizard_project(key),
        }
    }

    fn handle_wizard_kind(&mut self, key: KeyEvent) {
        let Some(Overlay::Wizard(mut wizard)) = self.overlay.take() else {
            return;
        };
        match key.code {
            KeyCode::Esc => return,
            KeyCode::Up | KeyCode::Char('k') => {
                wizard.kind = BuildKind::all()[(wizard.kind as usize + 2) % 3];
            }
            KeyCode::Down | KeyCode::Char('j') => {
                wizard.kind = BuildKind::all()[(wizard.kind as usize + 1) % 3];
            }
            KeyCode::Char('1') => wizard.kind = BuildKind::Clean,
            KeyCode::Char('2') => wizard.kind = BuildKind::Import,
            KeyCode::Char('3') => wizard.kind = BuildKind::Modrinth,
            KeyCode::Enter => {
                wizard.step = match wizard.kind {
                    BuildKind::Modrinth => WizardStep::ModrinthSearch,
                    _ => WizardStep::Configure,
                };
                wizard.field = 0;
            }
            _ => {}
        }
        self.overlay = Some(Overlay::Wizard(wizard));
    }

    fn handle_wizard_configure(&mut self, key: KeyEvent) {
        let Some(Overlay::Wizard(mut wizard)) = self.overlay.take() else {
            return;
        };
        let count = wizard.field_count();
        let mut restore = true;

        match key.code {
            KeyCode::Esc => {
                wizard.step = WizardStep::Kind;
            }
            KeyCode::Up | KeyCode::BackTab => wizard.field = (wizard.field + count - 1) % count,
            KeyCode::Down | KeyCode::Tab => wizard.field = (wizard.field + 1) % count,
            KeyCode::Left if wizard.kind == BuildKind::Clean && wizard.field == 2 => {
                wizard.cycle_loader(false)
            }
            KeyCode::Right if wizard.kind == BuildKind::Clean && wizard.field == 2 => {
                wizard.cycle_loader(true)
            }
            KeyCode::Backspace => match (wizard.kind, wizard.field) {
                (BuildKind::Clean, 0) => {
                    wizard.name.pop();
                }
                (BuildKind::Clean, 3) => {
                    wizard.loader_version.pop();
                }
                (BuildKind::Import, 0) => {
                    wizard.path.pop();
                }
                _ => {}
            },
            KeyCode::Char(c) => match (wizard.kind, wizard.field) {
                (BuildKind::Clean, 0) => wizard.name.push(c),
                (BuildKind::Clean, 3) => wizard.loader_version.push(c),
                (BuildKind::Import, 0) => wizard.path.push(c),
                _ => {}
            },
            KeyCode::Enter => match (wizard.kind, wizard.field) {
                (BuildKind::Clean, 1) => {
                    self.pending_wizard = Some(wizard.clone());
                    self.request_game_versions();
                    restore = false;
                }
                (BuildKind::Clean, 3) => {
                    self.pending_wizard = Some(wizard.clone());
                    self.request_loader_versions();
                    restore = false;
                }
                (BuildKind::Clean, 4) => {
                    self.submit_clean_build(&wizard);
                    restore = false;
                }
                (BuildKind::Import, 1) => {
                    self.submit_import(&wizard);
                    restore = false;
                }
                _ => {}
            },
            _ => {}
        }
        if restore {
            self.overlay = Some(Overlay::Wizard(wizard));
        }
    }

    fn handle_wizard_search(&mut self, key: KeyEvent) {
        let Some(Overlay::Wizard(mut wizard)) = self.overlay.take() else {
            return;
        };
        match key.code {
            KeyCode::Esc => {
                wizard.step = WizardStep::Kind;
                self.overlay = Some(Overlay::Wizard(wizard));
            }
            KeyCode::Backspace => {
                wizard.query.pop();
                self.overlay = Some(Overlay::Wizard(wizard));
            }
            KeyCode::Down | KeyCode::Char('j') if !wizard.results.is_empty() => {
                wizard.selected = (wizard.selected + 1).min(wizard.results.len() - 1);
                self.overlay = Some(Overlay::Wizard(wizard));
            }
            KeyCode::Up | KeyCode::Char('k') if !wizard.results.is_empty() => {
                wizard.selected = wizard.selected.saturating_sub(1);
                self.overlay = Some(Overlay::Wizard(wizard));
            }
            KeyCode::Char(c) => {
                wizard.query.push(c);
                self.overlay = Some(Overlay::Wizard(wizard));
            }
            KeyCode::Enter => {
                if wizard.results.is_empty() {
                    self.pending_wizard = Some(wizard.clone());
                    self.wizard_search();
                } else {
                    self.pending_wizard = Some(wizard.clone());
                    self.wizard_open_project();
                }
            }
            _ => self.overlay = Some(Overlay::Wizard(wizard)),
        }
    }

    fn handle_wizard_project(&mut self, key: KeyEvent) {
        let Some(Overlay::Wizard(mut wizard)) = self.overlay.take() else {
            return;
        };
        match key.code {
            KeyCode::Esc => {
                wizard.step = WizardStep::ModrinthSearch;
                self.overlay = Some(Overlay::Wizard(wizard));
            }
            KeyCode::Down | KeyCode::Char('j') if !wizard.project_versions.is_empty() => {
                wizard.selected = (wizard.selected + 1).min(wizard.project_versions.len() - 1);
                self.overlay = Some(Overlay::Wizard(wizard));
            }
            KeyCode::Up | KeyCode::Char('k') if !wizard.project_versions.is_empty() => {
                wizard.selected = wizard.selected.saturating_sub(1);
                self.overlay = Some(Overlay::Wizard(wizard));
            }
            KeyCode::Enter => self.submit_modrinth(&wizard),
            _ => self.overlay = Some(Overlay::Wizard(wizard)),
        }
    }

    // -----------------------------------------------------------------
    // Wizard actions
    // -----------------------------------------------------------------

    pub(crate) fn wizard_search(&mut self) {
        let Some(wizard) = self.pending_wizard.as_ref() else {
            return;
        };
        let query = wizard.query.trim().to_string();
        if query.is_empty() {
            return;
        }
        let modrinth = self.modrinth.clone();
        let tx = self.engine_tx.clone();
        self.progress = Some((None, format!("Searching '{query}'...")));
        tokio::spawn(async move {
            let result = modrinth
                .search(&query, Some("modpack"), None, None, 30, 0)
                .await;
            let _ = tx.send(crate::engine::EngineEvent::ProgressDone);
            match result {
                Ok(results) => {
                    let _ = tx.send(crate::engine::EngineEvent::WizardSearch(results));
                }
                Err(err) => {
                    let _ = tx.send(crate::engine::EngineEvent::Error(format!(
                        "Search failed: {err}"
                    )));
                }
            }
        });
    }

    fn wizard_open_project(&mut self) {
        let Some(wizard) = self.pending_wizard.as_ref() else {
            return;
        };
        let Some(hit) = wizard.results.get(wizard.selected).cloned() else {
            return;
        };
        let modrinth = self.modrinth.clone();
        let tx = self.engine_tx.clone();
        self.progress = Some((None, "Loading project...".into()));
        tokio::spawn(async move {
            let result = async {
                let project = modrinth.project(&hit.project_id).await?;
                let versions = modrinth.versions_filtered(&project.id, None, None).await?;
                Ok::<_, mc_core::CoreError>((project, versions))
            }
            .await;
            let _ = tx.send(crate::engine::EngineEvent::ProgressDone);
            match result {
                Ok((project, versions)) => {
                    let _ = tx.send(crate::engine::EngineEvent::WizardProject {
                        project: Box::new(project),
                        versions,
                    });
                }
                Err(err) => {
                    let _ = tx.send(crate::engine::EngineEvent::Error(format!(
                        "Load failed: {err}"
                    )));
                }
            }
        });
    }

    fn submit_clean_build(&mut self, wizard: &CreateWizard) {
        let name = wizard.name.trim().to_string();
        if name.is_empty() {
            self.set_toast("Build name cannot be empty", true);
            self.overlay = Some(Overlay::Wizard(wizard.clone()));
            return;
        }
        let loader_version = wizard.loader_version.trim().to_string();
        self.overlay = None;
        self.create_instance_async(
            name,
            wizard.game_version.clone(),
            wizard.loader(),
            (!loader_version.is_empty()).then_some(loader_version),
        );
    }

    fn submit_import(&mut self, wizard: &CreateWizard) {
        let path = wizard.path.trim().to_string();
        if path.is_empty() {
            self.set_toast("Enter a path to a .mrpack file", true);
            self.overlay = Some(Overlay::Wizard(wizard.clone()));
            return;
        }
        self.overlay = None;
        self.import_modpack(std::path::PathBuf::from(path));
    }

    fn submit_modrinth(&mut self, wizard: &CreateWizard) {
        let Some(version) = wizard.project_versions.get(wizard.selected).cloned() else {
            self.overlay = Some(Overlay::Wizard(wizard.clone()));
            return;
        };
        let name = wizard
            .project
            .as_ref()
            .map(|p| p.title.clone())
            .unwrap_or_else(|| "Modrinth Pack".to_string());
        self.overlay = None;
        self.install_modrinth_modpack(name, version);
    }

    /// Handle the async version-list event by showing the picker.
    pub(crate) fn show_version_picker(&mut self, target: PickerTarget, versions: Vec<String>) {
        let title = match target {
            PickerTarget::WizardGame | PickerTarget::ChangeGameVersion => "Minecraft Version",
            PickerTarget::WizardLoader => "Loader Version",
        };
        self.overlay = Some(Overlay::Picker(crate::forms::VersionPicker::new(
            title, versions, target,
        )));
    }

    /// Apply a picker selection, restoring the wizard when applicable.
    pub(crate) fn apply_picker_value(&mut self, target: PickerTarget, value: String) {
        match target {
            PickerTarget::WizardGame => {
                if let Some(mut wizard) = self.pending_wizard.take() {
                    wizard.game_version = value;
                    wizard.step = WizardStep::Configure;
                    self.overlay = Some(Overlay::Wizard(wizard));
                }
            }
            PickerTarget::WizardLoader => {
                if let Some(mut wizard) = self.pending_wizard.take() {
                    wizard.loader_version = value;
                    wizard.step = WizardStep::Configure;
                    self.overlay = Some(Overlay::Wizard(wizard));
                }
            }
            PickerTarget::ChangeGameVersion => self.change_instance_version(value),
        }
    }

    /// Restore a stashed wizard when the picker is dismissed.
    pub(crate) fn cancel_picker(&mut self) {
        if let Some(wizard) = self.pending_wizard.take() {
            self.overlay = Some(Overlay::Wizard(wizard));
        } else {
            self.overlay = None;
        }
    }

    // -----------------------------------------------------------------
    // Rendering
    // -----------------------------------------------------------------

    pub(crate) fn render_wizard(&mut self, frame: &mut Frame, area: Rect) {
        let Some(Overlay::Wizard(wizard)) = self.overlay.clone() else {
            return;
        };
        let popup = crate::widgets::centered_rect(76, 76, area);
        frame.render_widget(ratatui::widgets::Clear, popup);
        let surface = Style::default().fg(self.theme.fg).bg(self.theme.panel_alt);
        frame.render_widget(ratatui::widgets::Block::default().style(surface), popup);
        crate::views::accent_bar(frame, popup, &self.theme);

        let content = Rect {
            x: popup.x + 2,
            y: popup.y + 1,
            width: popup.width.saturating_sub(3),
            height: popup.height.saturating_sub(2),
        };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(4),
                Constraint::Length(1),
            ])
            .split(content);

        frame.render_widget(
            Paragraph::new(Span::styled("New Build", self.theme.header())).style(surface),
            chunks[0],
        );

        let mut tabs = Vec::new();
        for kind in BuildKind::all() {
            let selected = kind == wizard.kind;
            let style = if selected {
                self.theme.row_selected()
            } else {
                self.theme.card_dim()
            };
            tabs.push(Span::styled(format!(" {} ", kind.label()), style));
            tabs.push(Span::raw("  "));
        }
        frame.render_widget(Paragraph::new(Line::from(tabs)).style(surface), chunks[1]);

        match wizard.step {
            WizardStep::Kind => self.render_wizard_kinds(frame, chunks[2]),
            WizardStep::Configure => self.render_wizard_configure(frame, chunks[2]),
            WizardStep::ModrinthSearch => self.render_wizard_search(frame, chunks[2]),
            WizardStep::ModrinthProject => self.render_wizard_project(frame, chunks[2]),
        }

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Enter", self.theme.accent()),
                Span::styled(" continue", self.theme.card_dim()),
                Span::styled("   Esc", self.theme.accent()),
                Span::styled(" back", self.theme.card_dim()),
                Span::styled("   Tab/↑↓", self.theme.accent()),
                Span::styled(" move", self.theme.card_dim()),
            ]))
            .style(surface),
            chunks[3],
        );
    }

    fn render_wizard_kinds(&mut self, frame: &mut Frame, area: Rect) {
        let Some(Overlay::Wizard(wizard)) = self.overlay.as_ref() else {
            return;
        };
        let surface = Style::default().fg(self.theme.fg).bg(self.theme.panel_alt);
        for (idx, kind) in BuildKind::all().iter().enumerate() {
            let row = Rect {
                x: area.x,
                y: area.y + idx as u16 * 2,
                width: area.width,
                height: 1,
            };
            let selected = *kind == wizard.kind;
            let style = if selected {
                self.theme.row_selected()
            } else {
                surface
            };
            let marker = if selected { "▸ " } else { "  " };
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    format!("{marker}{}", kind.label()),
                    style,
                )))
                .style(style),
                row,
            );
            frame.render_widget(
                Paragraph::new(Span::styled(
                    format!("    {}", kind.description()),
                    self.theme.card_dim(),
                ))
                .style(surface),
                Rect {
                    y: row.y + 1,
                    height: 1,
                    ..row
                },
            );
        }
    }

    fn render_wizard_configure(&mut self, frame: &mut Frame, area: Rect) {
        let Some(Overlay::Wizard(wizard)) = self.overlay.clone() else {
            return;
        };
        match wizard.kind {
            BuildKind::Clean => {
                let rows = [
                    ("Name", wizard.name.clone()),
                    ("Minecraft", wizard.game_version.clone()),
                    ("Loader", wizard.loader().label().to_string()),
                    (
                        "Loader Version",
                        if wizard.loader_version.is_empty() {
                            "latest".to_string()
                        } else {
                            wizard.loader_version.clone()
                        },
                    ),
                    ("", "[ Create ]".to_string()),
                ];
                self.render_wizard_rows(frame, area, &rows, wizard.field);
            }
            BuildKind::Import => {
                let rows = [
                    ("Archive", wizard.path.clone()),
                    ("", "[ Import ]".to_string()),
                ];
                self.render_wizard_rows(frame, area, &rows, wizard.field);
            }
            _ => {}
        }
    }

    fn render_wizard_rows(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        rows: &[(&str, String)],
        active: usize,
    ) {
        let surface = Style::default().fg(self.theme.fg).bg(self.theme.panel_alt);
        for (idx, (label, value)) in rows.iter().enumerate() {
            let row = Rect {
                x: area.x,
                y: area.y + idx as u16,
                width: area.width,
                height: 1,
            };
            let selected = idx == active;
            let style = if selected {
                self.theme.row_selected()
            } else {
                surface
            };
            let marker = if selected { "▸ " } else { "  " };
            let line = if label.is_empty() {
                Line::from(Span::styled(format!("{marker}{value}"), style))
            } else {
                Line::from(vec![
                    Span::styled(format!("{marker}{label:<16}"), self.theme.card_dim()),
                    Span::styled(value.clone(), style),
                ])
            };
            frame.render_widget(Paragraph::new(line).style(style), row);
        }
    }

    fn render_wizard_search(&mut self, frame: &mut Frame, area: Rect) {
        let Some(Overlay::Wizard(wizard)) = self.overlay.clone() else {
            return;
        };
        let surface = Style::default().fg(self.theme.fg).bg(self.theme.panel_alt);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(3)])
            .split(area);

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Search  ", self.theme.card_dim()),
                Span::styled(format!("{}█", wizard.query), self.theme.accent()),
            ]))
            .style(surface),
            chunks[0],
        );

        let selected = Some(wizard.selected);
        let items: Vec<ListItem> = wizard
            .results
            .iter()
            .enumerate()
            .map(|(idx, hit)| {
                ListItem::new(Line::from(vec![
                    Span::styled(hit.title.clone(), surface),
                    Span::styled(format!("  by {}", hit.author), self.theme.card_dim()),
                    Span::styled(format!("  ⤓ {}", hit.downloads), self.theme.accent()),
                ]))
                .style(row_style(self, idx, selected, None))
            })
            .collect();
        frame.render_widget(List::new(items).style(surface), chunks[1]);
    }

    fn render_wizard_project(&mut self, frame: &mut Frame, area: Rect) {
        let Some(Overlay::Wizard(wizard)) = self.overlay.clone() else {
            return;
        };
        let surface = Style::default().fg(self.theme.fg).bg(self.theme.panel_alt);
        let title = wizard
            .project
            .as_ref()
            .map(|p| p.title.clone())
            .unwrap_or_default();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(3)])
            .split(area);
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(title, self.theme.header())),
                Line::from(Span::styled(
                    "Choose a version to install:",
                    self.theme.card_dim(),
                )),
            ])
            .style(surface),
            chunks[0],
        );

        let selected = Some(wizard.selected);
        let items: Vec<ListItem> = wizard
            .project_versions
            .iter()
            .enumerate()
            .map(|(idx, version)| {
                let game = version.game_versions.first().cloned().unwrap_or_default();
                ListItem::new(Line::from(vec![
                    Span::styled(version.version_number.clone(), surface),
                    Span::styled(
                        format!("  {game} {}", version.loaders.join(", ")),
                        self.theme.card_dim(),
                    ),
                ]))
                .style(row_style(self, idx, selected, None))
            })
            .collect();
        frame.render_widget(
            List::new(items).style(surface).highlight_symbol("▸ "),
            chunks[1],
        );
    }
}
