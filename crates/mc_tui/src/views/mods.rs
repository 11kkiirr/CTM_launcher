//! Mod Manager screen: installed mods and Modrinth mod search.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::{App, ButtonId, Focus, HitAction};
use crate::views::{
    buttons_row, card, hovered_index, jump, move_sel, register_rows, row_style, section_title,
};

impl App {
    pub(crate) fn render_mods(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(5),
                Constraint::Length(1),
                Constraint::Length(10),
            ])
            .split(area);

        buttons_row(
            self,
            frame,
            chunks[0].x,
            chunks[0].y,
            area.x + area.width,
            &[
                ("Toggle", ButtonId::ToggleMod),
                ("Delete", ButtonId::DeleteMod),
                ("Store", ButtonId::BrowseMods),
                ("Search", ButtonId::ModSearch),
                ("Updates", ButtonId::UpdateMods),
            ],
        );

        self.render_installed_mods(frame, chunks[2]);
        self.render_mod_search(frame, chunks[4]);
    }

    fn render_installed_mods(&mut self, frame: &mut Frame, area: Rect) {
        let focused = self.focus == Focus::Content && !self.mods_focus_search;
        let inner = card(self, frame, area, focused);
        if inner.height == 0 {
            return;
        }

        let instance_name = self
            .selected_instance()
            .map(|i| i.name().to_string())
            .unwrap_or_else(|| "no instance".to_string());
        let title = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(section_title(
                "Installed mods",
                &format!("{instance_name}  ·  {}", self.installed_mods.len()),
                &self.theme,
            ))
            .style(self.theme.card()),
            title,
        );

        let list_area = Rect {
            x: inner.x,
            y: inner.y + 1,
            width: inner.width,
            height: inner.height.saturating_sub(1),
        };

        let selected = self.mods_state.selected();
        let hovered = hovered_index(
            self,
            list_area,
            self.mods_state.offset(),
            self.installed_mods.len(),
        );
        let items: Vec<ListItem> = self
            .installed_mods
            .iter()
            .enumerate()
            .map(|(idx, module)| {
                let state = if module.enabled {
                    Span::styled("● ", self.theme.accent())
                } else {
                    Span::styled("○ ", self.theme.dim())
                };
                ListItem::new(Line::from(vec![
                    state,
                    Span::styled(module.file_name.clone(), self.theme.card()),
                    Span::styled(
                        format!("  {:.1} KB", module.size as f64 / 1024.0),
                        self.theme.card_dim(),
                    ),
                ]))
                .style(row_style(self, idx, selected, hovered))
            })
            .collect();

        let list = List::new(items)
            .highlight_symbol("▸ ")
            .highlight_style(self.theme.row_selected());
        frame.render_stateful_widget(list, list_area, &mut self.mods_state);
        register_rows(
            &mut self.hitboxes,
            &self.mods_state,
            list_area,
            self.installed_mods.len(),
            HitAction::ModRow,
        );

        if self.installed_mods.is_empty() {
            let hint = if self.mods_scanning {
                "Scanning mods…".to_string()
            } else {
                "No mods installed. Press 's' to search Modrinth.".to_string()
            };
            frame.render_widget(
                Paragraph::new(Span::styled(hint, self.theme.card_dim())).style(self.theme.card()),
                list_area,
            );
        }
    }

    fn render_mod_search(&mut self, frame: &mut Frame, area: Rect) {
        let focused = self.focus == Focus::Content && self.mods_focus_search;
        let inner = card(self, frame, area, focused);
        if inner.height == 0 {
            return;
        }

        let suffix = if self.mod_search_query.is_empty() {
            "press 's' to search".to_string()
        } else {
            format!(
                "{}  ·  {} results",
                self.mod_search_query,
                self.mod_search_results.len()
            )
        };
        let title = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(section_title("Modrinth", &suffix, &self.theme))
                .style(self.theme.card()),
            title,
        );

        let list_area = Rect {
            x: inner.x,
            y: inner.y + 1,
            width: inner.width,
            height: inner.height.saturating_sub(1),
        };

        let selected = self.mod_search_state.selected();
        let hovered = hovered_index(
            self,
            list_area,
            self.mod_search_state.offset(),
            self.mod_search_results.len(),
        );
        let items: Vec<ListItem> = self
            .mod_search_results
            .iter()
            .enumerate()
            .map(|(idx, hit)| {
                ListItem::new(Line::from(vec![
                    Span::styled(hit.title.clone(), self.theme.card()),
                    Span::styled(format!("  by {}", hit.author), self.theme.card_dim()),
                    Span::styled(format!("  ⤓ {}", hit.downloads), self.theme.accent()),
                ]))
                .style(row_style(self, idx, selected, hovered))
            })
            .collect();

        let list = List::new(items)
            .highlight_symbol("▸ ")
            .highlight_style(self.theme.row_selected());
        frame.render_stateful_widget(list, list_area, &mut self.mod_search_state);
        register_rows(
            &mut self.hitboxes,
            &self.mod_search_state,
            list_area,
            self.mod_search_results.len(),
            HitAction::ModSearchRow,
        );
        if self.mod_search_results.is_empty() {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "Press 's' to search Modrinth, then Enter to install into the current instance.",
                    self.theme.card_dim(),
                ))
                .style(self.theme.card()),
                list_area,
            );
        }
    }

    pub(crate) fn key_mods(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('t') => self.mods_focus_search = !self.mods_focus_search,
            KeyCode::Char('s') => self.open_mod_search_prompt(),
            KeyCode::Char('u') => self.check_mod_updates(),
            KeyCode::Char('r') => self.reload_mods(),
            _ => {}
        }

        if self.mods_focus_search {
            let len = self.mod_search_results.len();
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => move_sel(&mut self.mod_search_state, len, 1),
                KeyCode::Up | KeyCode::Char('k') => move_sel(&mut self.mod_search_state, len, -1),
                KeyCode::Char('g') => jump(&mut self.mod_search_state, len, false),
                KeyCode::Char('G') => jump(&mut self.mod_search_state, len, true),
                KeyCode::Enter | KeyCode::Char('i') => self.install_selected_mod_search(),
                _ => {}
            }
        } else {
            let len = self.installed_mods.len();
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => move_sel(&mut self.mods_state, len, 1),
                KeyCode::Up | KeyCode::Char('k') => move_sel(&mut self.mods_state, len, -1),
                KeyCode::Char('g') => jump(&mut self.mods_state, len, false),
                KeyCode::Char('G') => jump(&mut self.mods_state, len, true),
                KeyCode::Char(' ') | KeyCode::Enter => self.toggle_selected_mod(),
                KeyCode::Char('d') => self.confirm_delete_mod(),
                _ => {}
            }
        }
    }
}
