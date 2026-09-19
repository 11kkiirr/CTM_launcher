//! Mod Manager screen: installed mods and Modrinth mod search.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::{App, ButtonId, HitAction};
use crate::views::{buttons_row, hovered_index, jump, move_sel, register_rows, row_style};

impl App {
    pub(crate) fn render_mods(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(5),
                Constraint::Length(10),
            ])
            .split(area);

        buttons_row(
            self,
            frame,
            chunks[0].x + 1,
            chunks[0].y,
            area.x + area.width,
            &[
                ("Toggle", ButtonId::ToggleMod),
                ("Delete", ButtonId::DeleteMod),
                ("Search", ButtonId::ModSearch),
                ("Updates", ButtonId::UpdateMods),
            ],
        );

        self.render_installed_mods(frame, chunks[1]);
        self.render_mod_search(frame, chunks[2]);
    }

    fn render_installed_mods(&mut self, frame: &mut Frame, area: Rect) {
        let instance_name = self
            .selected_instance()
            .map(|i| i.name().to_string())
            .unwrap_or_else(|| "no instance".to_string());

        let focus_marker = if !self.mods_focus_search { "▸ " } else { "" };
        let title = format!(
            " {focus_marker}Installed mods — {instance_name} ({}) ",
            self.installed_mods.len()
        );
        let border = if !self.mods_focus_search {
            self.theme.block_border_focused()
        } else {
            self.theme.block_border()
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(border)
            .title(Line::from(title).style(self.theme.header()));
        let inner = block.inner(area);

        let selected = self.mods_state.selected();
        let hovered = hovered_index(
            self,
            inner,
            self.mods_state.offset(),
            self.installed_mods.len(),
        );
        let items: Vec<ListItem> = self
            .installed_mods
            .iter()
            .enumerate()
            .map(|(idx, module)| {
                let state = if module.enabled {
                    Span::styled("[on]  ", self.theme.accent())
                } else {
                    Span::styled("[off] ", self.theme.dim())
                };
                ListItem::new(Line::from(vec![
                    state,
                    Span::styled(module.file_name.clone(), self.theme.base()),
                    Span::styled(
                        format!("  {:.1} KB", module.size as f64 / 1024.0),
                        self.theme.dim(),
                    ),
                ]))
                .style(row_style(self, idx, selected, hovered))
            })
            .collect();

        let list = List::new(items).block(block).highlight_symbol("▸ ");
        frame.render_stateful_widget(list, area, &mut self.mods_state);
        register_rows(
            &mut self.hitboxes,
            &self.mods_state,
            inner,
            self.installed_mods.len(),
            HitAction::ModRow,
        );
    }

    fn render_mod_search(&mut self, frame: &mut Frame, area: Rect) {
        let focus_marker = if self.mods_focus_search { "▸ " } else { "" };
        let title = if self.mod_search_query.is_empty() {
            format!(" {focus_marker}Modrinth search — press 's' ")
        } else {
            format!(
                " {focus_marker}Modrinth: {} ({} results) ",
                self.mod_search_query,
                self.mod_search_results.len()
            )
        };
        let border = if self.mods_focus_search {
            self.theme.block_border_focused()
        } else {
            self.theme.block_border()
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(border)
            .title(Line::from(title).style(self.theme.header()));
        let inner = block.inner(area);

        let selected = self.mod_search_state.selected();
        let hovered = hovered_index(
            self,
            inner,
            self.mod_search_state.offset(),
            self.mod_search_results.len(),
        );
        let items: Vec<ListItem> = self
            .mod_search_results
            .iter()
            .enumerate()
            .map(|(idx, hit)| {
                ListItem::new(Line::from(vec![
                    Span::styled(hit.title.clone(), self.theme.base()),
                    Span::styled(format!("  by {}", hit.author), self.theme.dim()),
                    Span::styled(format!("  ⤓ {}", hit.downloads), self.theme.accent()),
                ]))
                .style(row_style(self, idx, selected, hovered))
            })
            .collect();

        let list = List::new(items).block(block).highlight_symbol("▸ ");
        frame.render_stateful_widget(list, area, &mut self.mod_search_state);
        register_rows(
            &mut self.hitboxes,
            &self.mod_search_state,
            inner,
            self.mod_search_results.len(),
            HitAction::ModSearchRow,
        );
        if self.mod_search_results.is_empty() {
            let hint = Paragraph::new(Span::styled(
                "Press 's' to search Modrinth, then Enter to install into the current instance.",
                self.theme.dim(),
            ))
            .style(self.theme.base());
            frame.render_widget(hint, inner);
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
