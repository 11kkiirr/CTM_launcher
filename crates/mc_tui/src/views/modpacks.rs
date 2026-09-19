//! Modpacks screen: Modrinth modpack search, project details and `.mrpack`
//! import.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, ButtonId, HitAction};
use crate::views::{buttons_row, hovered_index, jump, move_sel, register_rows, row_style};

impl App {
    pub(crate) fn render_modpacks(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(5),
                Constraint::Length(7),
            ])
            .split(area);

        buttons_row(
            self,
            frame,
            chunks[0].x + 1,
            chunks[0].y,
            area.x + area.width,
            &[
                ("Search", ButtonId::Search),
                ("Import .mrpack", ButtonId::ImportModpack),
                ("Install", ButtonId::InstallProject),
            ],
        );

        if self.selected_project.is_some() {
            self.render_project(frame, chunks[1]);
            self.render_project_info(frame, chunks[2]);
        } else {
            self.render_search_results(frame, chunks[1]);
            self.render_search_info(frame, chunks[2]);
        }
    }

    fn render_search_results(&mut self, frame: &mut Frame, area: Rect) {
        let title = if self.search_query.is_empty() {
            " Modpack Search ".to_string()
        } else {
            format!(
                " Search: {} ({} results) ",
                self.search_query,
                self.search_results.len()
            )
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.block_border())
            .title(Line::from(title).style(self.theme.header()));
        let inner = block.inner(area);

        let selected = self.search_state.selected();
        let hovered = hovered_index(
            self,
            inner,
            self.search_state.offset(),
            self.search_results.len(),
        );
        let items: Vec<ListItem> = self
            .search_results
            .iter()
            .enumerate()
            .map(|(idx, hit)| {
                ListItem::new(Line::from(vec![
                    Span::styled(hit.title.clone(), self.theme.base()),
                    Span::styled(format!("  by {}", hit.author), self.theme.dim()),
                    Span::styled(
                        format!("  ⤓ {}", format_count(hit.downloads)),
                        self.theme.accent(),
                    ),
                ]))
                .style(row_style(self, idx, selected, hovered))
            })
            .collect();

        let list = List::new(items).block(block).highlight_symbol("▸ ");
        frame.render_stateful_widget(list, area, &mut self.search_state);
        register_rows(
            &mut self.hitboxes,
            &self.search_state,
            inner,
            self.search_results.len(),
            HitAction::SearchRow,
        );
    }

    fn render_project(&mut self, frame: &mut Frame, area: Rect) {
        let title = self
            .selected_project
            .as_ref()
            .map(|p| format!(" {} — versions ", p.title))
            .unwrap_or_else(|| " Versions ".to_string());
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.block_border())
            .title(Line::from(title).style(self.theme.header()));
        let inner = block.inner(area);

        let selected = self.project_state.selected();
        let hovered = hovered_index(
            self,
            inner,
            self.project_state.offset(),
            self.project_versions.len(),
        );
        let items: Vec<ListItem> = self
            .project_versions
            .iter()
            .enumerate()
            .map(|(idx, version)| {
                let loaders = version.loaders.join(", ");
                let game = version.game_versions.first().cloned().unwrap_or_default();
                ListItem::new(Line::from(vec![
                    Span::styled(version.version_number.clone(), self.theme.base()),
                    Span::styled(format!("  {} {}", game, loaders), self.theme.dim()),
                    Span::styled(format!("  [{}]", version.version_type), self.theme.accent()),
                ]))
                .style(row_style(self, idx, selected, hovered))
            })
            .collect();

        let list = List::new(items).block(block).highlight_symbol("▸ ");
        frame.render_stateful_widget(list, area, &mut self.project_state);
        register_rows(
            &mut self.hitboxes,
            &self.project_state,
            inner,
            self.project_versions.len(),
            HitAction::ProjectVersionRow,
        );
    }

    fn render_search_info(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.block_border())
            .title(Line::from(" Details ").style(self.theme.header()));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let Some(hit) = self
            .search_state
            .selected()
            .and_then(|idx| self.search_results.get(idx))
        else {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "Press '/' to search Modrinth modpacks, or 'm' to import a local .mrpack file.",
                    self.theme.dim(),
                ))
                .style(self.theme.base()),
                inner,
            );
            return;
        };
        let lines = vec![
            Line::from(Span::styled(hit.title.clone(), self.theme.header())),
            Line::from(Span::styled(hit.description.clone(), self.theme.base())),
            Line::from(""),
            Line::from(vec![
                Span::styled("Downloads: ", self.theme.dim()),
                Span::styled(format_count(hit.downloads), self.theme.accent()),
                Span::styled("   Followers: ", self.theme.dim()),
                Span::styled(format_count(hit.follows), self.theme.accent()),
            ]),
            Line::from(vec![
                Span::styled("Categories: ", self.theme.dim()),
                Span::styled(hit.categories.join(", "), self.theme.info_style()),
            ]),
        ];
        frame.render_widget(
            Paragraph::new(lines)
                .style(self.theme.base())
                .wrap(Wrap { trim: true }),
            inner,
        );
    }

    fn render_project_info(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.block_border())
            .title(Line::from(" Project ").style(self.theme.header()));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let Some(project) = self.selected_project.as_ref() else {
            return;
        };
        let lines = vec![
            Line::from(Span::styled(project.title.clone(), self.theme.header())),
            Line::from(Span::styled(project.description.clone(), self.theme.base())),
            Line::from(""),
            Line::from(vec![
                Span::styled("Downloads: ", self.theme.dim()),
                Span::styled(format_count(project.downloads), self.theme.accent()),
                Span::styled("   Versions: ", self.theme.dim()),
                Span::styled(project.versions.len().to_string(), self.theme.accent()),
            ]),
            Line::from(Span::styled(
                "Enter/i installs the selected version into the current instance. Esc goes back.",
                self.theme.dim(),
            )),
        ];
        frame.render_widget(
            Paragraph::new(lines)
                .style(self.theme.base())
                .wrap(Wrap { trim: true }),
            inner,
        );
    }

    pub(crate) fn key_modpacks(&mut self, key: KeyEvent) {
        if self.selected_project.is_some() {
            let len = self.project_versions.len();
            match key.code {
                KeyCode::Esc => {
                    self.selected_project = None;
                    self.project_versions.clear();
                }
                KeyCode::Down | KeyCode::Char('j') => move_sel(&mut self.project_state, len, 1),
                KeyCode::Up | KeyCode::Char('k') => move_sel(&mut self.project_state, len, -1),
                KeyCode::Char('g') => jump(&mut self.project_state, len, false),
                KeyCode::Char('G') => jump(&mut self.project_state, len, true),
                KeyCode::Enter | KeyCode::Char('i') => self.install_selected_project(),
                _ => {}
            }
            return;
        }

        let len = self.search_results.len();
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => move_sel(&mut self.search_state, len, 1),
            KeyCode::Up | KeyCode::Char('k') => move_sel(&mut self.search_state, len, -1),
            KeyCode::Char('g') => jump(&mut self.search_state, len, false),
            KeyCode::Char('G') => jump(&mut self.search_state, len, true),
            KeyCode::Char('/') => self.open_search_prompt(),
            KeyCode::Char('m') => self.open_import_prompt(),
            KeyCode::Enter => {
                if let Some(hit) = self
                    .search_state
                    .selected()
                    .and_then(|idx| self.search_results.get(idx))
                    .cloned()
                {
                    self.open_project(&hit);
                }
            }
            _ => {}
        }
    }
}

fn format_count(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}K", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}
