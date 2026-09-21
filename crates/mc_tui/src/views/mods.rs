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
            ])
            .split(area);

        buttons_row(
            self,
            frame,
            chunks[0].x + 2,
            chunks[0].y,
            area.x + area.width,
            &[
                ("Toggle", "Space", ButtonId::ToggleMod),
                ("Delete", "d", ButtonId::DeleteMod),
                ("Browse", "s", ButtonId::BrowseMods),
                ("Updates", "u", ButtonId::UpdateMods),
            ],
        );

        self.render_installed_mods(frame, chunks[2]);
    }

    fn render_installed_mods(&mut self, frame: &mut Frame, area: Rect) {
        let focused = self.focus == Focus::Content;
        let inner = card(self, frame, area, focused);
        if inner.height == 0 {
            return;
        }

        // ── Inline search bar ──
        let search_focused = self.mods_search_focused;
        let search_bg = if search_focused {
            self.theme.panel_alt
        } else {
            self.theme.panel
        };
        let search_rect = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 1,
        };
        if search_focused && search_rect.width > 0 {
            let bar = Rect { x: search_rect.x, y: search_rect.y, width: 1, height: 1 };
            frame.render_widget(
                Paragraph::new(Span::styled(" ", self.theme.accent()))
                    .style(Style::default().bg(search_bg)),
                bar,
            );
        }
        let search_label = " Filter  ";
        let query_text = if self.mods_search_query.is_empty() && !search_focused {
            String::new()
        } else if search_focused {
            format!("{}█", self.mods_search_query)
        } else {
            self.mods_search_query.clone()
        };
        let query_style = if search_focused {
            Style::default().fg(self.theme.green).bg(search_bg)
        } else if !self.mods_search_query.is_empty() {
            Style::default().fg(self.theme.fg).bg(search_bg)
        } else {
            Style::default().fg(self.theme.muted).bg(search_bg)
        };
        let search_line = Line::from(vec![
            Span::styled(
                search_label,
                Style::default()
                    .fg(if search_focused { self.theme.green } else { self.theme.muted })
                    .bg(search_bg),
            ),
            Span::styled(query_text, query_style),
        ]);
        let bar_x = if search_focused { search_rect.x + 1 } else { search_rect.x };
        let bar_w = if search_focused { search_rect.width.saturating_sub(1) } else { search_rect.width };
        frame.render_widget(
            Paragraph::new(search_line).style(Style::default().bg(search_bg)),
            Rect { x: bar_x, y: search_rect.y, width: bar_w, height: 1 },
        );
        self.push_hitbox(search_rect, HitAction::ModsSearchBar);

        // ── Title row ──
        let instance_name = self
            .selected_instance()
            .map(|i| i.name().to_string())
            .unwrap_or_else(|| "no instance".to_string());

        // Filter mods by search query
        let query_lower = self.mods_search_query.to_lowercase();
        let filtered_indices: Vec<usize> = self
            .installed_mods
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                if query_lower.is_empty() {
                    return true;
                }
                let name = if m.mod_name.is_empty() {
                    m.file_name.to_lowercase()
                } else {
                    m.mod_name.to_lowercase()
                };
                name.contains(&query_lower)
                    || m.version.to_lowercase().contains(&query_lower)
                    || m.file_name.to_lowercase().contains(&query_lower)
            })
            .map(|(i, _)| i)
            .collect();

        let title = Rect {
            x: inner.x,
            y: inner.y + 1,
            width: inner.width,
            height: 1,
        };
        let count_text = if query_lower.is_empty() {
            format!("{}", self.installed_mods.len())
        } else {
            format!("{}/{}", filtered_indices.len(), self.installed_mods.len())
        };
        frame.render_widget(
            Paragraph::new(section_title(
                "Installed mods",
                &format!("{instance_name}  ·  {count_text}"),
                &self.theme,
            ))
            .style(self.theme.card()),
            title,
        );

        // ── Mod list ──
        let list_area = Rect {
            x: inner.x,
            y: inner.y + 2,
            width: inner.width,
            height: inner.height.saturating_sub(2),
        };

        if filtered_indices.is_empty() {
            let hint = if self.mods_scanning {
                "Scanning mods...".to_string()
            } else if !query_lower.is_empty() {
                "No mods match the filter.".to_string()
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
            filtered_indices.len(),
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

        let items: Vec<ListItem> = filtered_indices
            .iter()
            .map(|&idx| {
                let module = &self.installed_mods[idx];
                let toggle = if module.enabled {
                    Span::styled("  \u{25CF} ", self.theme.accent())
                } else {
                    Span::styled("  \u{25CB} ", self.theme.dim())
                };

                let display_name = if module.mod_name.is_empty() {
                    module
                        .file_name
                        .strip_suffix(".disabled")
                        .unwrap_or(&module.file_name)
                        .strip_suffix(".jar")
                        .unwrap_or(
                            module
                                .file_name
                                .strip_suffix(".disabled")
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

                let display_idx = filtered_indices.iter().position(|&i| i == idx).unwrap_or(0);
                let row_style = row_style(self, display_idx, selected, hovered);
                let dim_style = if selected == Some(display_idx) {
                    Style::default()
                        .fg(self.theme.green)
                        .bg(self.theme.selection_bg)
                } else if hovered == Some(display_idx) {
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
            filtered_indices.len(),
            HitAction::ModRow,
        );
    }

    pub(crate) fn key_mods(&mut self, key: KeyEvent) {
        // When the search bar is focused, handle input directly.
        if self.mods_search_focused {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.mods_search_focused = false;
                }
                KeyCode::Backspace => {
                    self.mods_search_query.pop();
                }
                KeyCode::Char(c) => {
                    self.mods_search_query.push(c);
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Char('t') => self.mods_search_focused = !self.mods_search_focused,
            KeyCode::Char('s') => {
                self.browse.kind = crate::views::browse::BrowseKind::Mods;
                self.browse.save_cache();
                self.browse.load_cache(crate::views::browse::BrowseKind::Mods);
                self.open_nav(crate::app::Nav::Browse);
            }
            KeyCode::Char('u') => self.check_mod_updates(),
            KeyCode::Char('r') => self.reload_mods(),
            _ => {}
        }

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
