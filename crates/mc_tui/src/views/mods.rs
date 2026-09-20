//! Mod Manager screen: installed mods and Modrinth mod search.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::{App, ButtonId, Focus, HitAction};
use crate::views::{
    buttons_row, card, hovered_index, jump, move_sel, register_rows, row_style, section_title,
    truncate,
};

/// Column offsets for the installed mods table.
struct ModColumns {
    toggle: u16,
    name: u16,
    version: u16,
    size: u16,
    date: u16,
}

impl ModColumns {
    fn compute(width: u16) -> Self {
        let w = width as usize;
        let toggle = 0;
        let name = 3;
        let version = (w * 3 / 10).max(name + 6);
        let size = (w * 6 / 10).max(version + 8);
        let date = (w * 8 / 10).max(size + 10);
        Self {
            toggle: toggle as u16,
            name: name as u16,
            version: version as u16,
            size: size as u16,
            date: date as u16,
        }
    }
}

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
            chunks[0].x + 1,
            chunks[0].y,
            area.x + area.width,
            &[
                ("Toggle", "Space", ButtonId::ToggleMod),
                ("Delete", "d", ButtonId::DeleteMod),
                ("Browse", "s", ButtonId::BrowseMods),
                ("Search", "/", ButtonId::ModSearch),
                ("Updates", "u", ButtonId::UpdateMods),
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

        if self.installed_mods.is_empty() {
            let hint = if self.mods_scanning {
                "Scanning mods...".to_string()
            } else {
                "No mods installed. Press 's' to browse Modrinth.".to_string()
            };
            frame.render_widget(
                Paragraph::new(Span::styled(hint, self.theme.card_dim())).style(self.theme.card()),
                list_area,
            );
            return;
        }

        let cols = ModColumns::compute(list_area.width);
        let selected = self.mods_state.selected();
        let hovered = hovered_index(
            self,
            list_area,
            self.mods_state.offset(),
            self.installed_mods.len(),
        );

        // Header row
        let header_rect = Rect {
            x: list_area.x,
            y: list_area.y,
            width: list_area.width,
            height: 1,
        };
        let header_style = self.theme.card_comment();
        let header_line = Line::from(vec![
            Span::styled(
                truncate("  On/Off", (cols.name - cols.toggle) as usize),
                header_style,
            ),
            Span::styled(
                truncate("  Name", (cols.version - cols.name) as usize),
                header_style,
            ),
            Span::styled(
                truncate("  Version", (cols.size - cols.version) as usize),
                header_style,
            ),
            Span::styled(
                truncate("  Size", (cols.date - cols.size) as usize),
                header_style,
            ),
            Span::styled("  Date", header_style),
        ]);
        frame.render_widget(
            Paragraph::new(header_line).style(self.theme.card()),
            header_rect,
        );

        // Data rows
        let data_area = Rect {
            x: list_area.x,
            y: list_area.y + 1,
            width: list_area.width,
            height: list_area.height.saturating_sub(1),
        };

        let items: Vec<ListItem> = self
            .installed_mods
            .iter()
            .enumerate()
            .map(|(idx, module)| {
                let toggle = if module.enabled {
                    Span::styled("  \u{25CF} ", self.theme.accent())
                } else {
                    Span::styled("  \u{25CB} ", self.theme.dim())
                };

                let display_name = if module.mod_name.is_empty() {
                    // Fall back to filename without extension
                    module
                        .file_name
                        .strip_suffix(".jar")
                        .unwrap_or(&module.file_name)
                        .strip_suffix(".disabled")
                        .unwrap_or(
                            module
                                .file_name
                                .strip_suffix(".jar")
                                .unwrap_or(&module.file_name),
                        )
                        .to_string()
                } else {
                    module.mod_name.clone()
                };

                let version_text = if module.version.is_empty() {
                    "-".to_string()
                } else {
                    module.version.clone()
                };

                let size_text = if module.size >= 1_048_576 {
                    format!("{:.1} MB", module.size as f64 / 1_048_576.0)
                } else {
                    format!("{:.0} KB", module.size as f64 / 1024.0)
                };

                let date_text = if module.install_date.is_empty() {
                    "-".to_string()
                } else {
                    module.install_date.clone()
                };

                let row_style = row_style(self, idx, selected, hovered);
                let dim_style = if selected == Some(idx) {
                    Style::default()
                        .fg(self.theme.green)
                        .bg(self.theme.selection_bg)
                } else if hovered == Some(idx) {
                    Style::default()
                        .fg(self.theme.muted)
                        .bg(self.theme.hover_bg)
                } else {
                    self.theme.card_dim()
                };

                ListItem::new(Line::from(vec![
                    toggle,
                    Span::styled(
                        format!("  {}", truncate(&display_name, (cols.version - cols.name - 2) as usize)),
                        row_style,
                    ),
                    Span::styled(
                        format!("  {}", truncate(&version_text, (cols.size - cols.version - 2) as usize)),
                        dim_style,
                    ),
                    Span::styled(
                        format!("  {}", truncate(&size_text, (cols.date - cols.size - 2) as usize)),
                        dim_style,
                    ),
                    Span::styled(format!("  {date_text}"), dim_style),
                ]))
                .style(row_style)
            })
            .collect();

        let list = List::new(items)
            .highlight_symbol("\u{25B8} ")
            .highlight_style(self.theme.row_selected());
        frame.render_stateful_widget(list, data_area, &mut self.mods_state);
        register_rows(
            &mut self.hitboxes,
            &self.mods_state,
            data_area,
            self.installed_mods.len(),
            HitAction::ModRow,
        );
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
                    Span::styled(format!("  \u{2913} {}", hit.downloads), self.theme.accent()),
                ]))
                .style(row_style(self, idx, selected, hovered))
            })
            .collect();

        let list = List::new(items)
            .highlight_symbol("\u{25B8} ")
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
                    "Press 's' to browse Modrinth, then Enter to install.",
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
