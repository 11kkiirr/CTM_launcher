//! Mod Manager screen: installed mods and Modrinth mod search.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::{App, ButtonId, Focus, HitAction};
use crate::views::{
    buttons_row, card, hovered_index, jump, move_sel, register_rows, row_style,
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
        let name = 8;
        let version = (w * 3 / 10).max(name + 8);
        // Date is anchored to the right edge of the card: 16 cells for
        // "YYYY-MM-DD HH:MM".
        let date = w.saturating_sub(16);
        let size = (w * 6 / 10)
            .max(version + 6)
            .min(date.saturating_sub(6));
        Self {
            toggle: toggle as u16,
            name: name as u16,
            version: version as u16,
            size: size as u16,
            date: date as u16,
        }
    }
}

/// Format `text` into a fixed-width table cell: truncate when too long,
/// pad with spaces (right-aligned when `right` is set) so every column
/// starts at exactly the same offset on every row.
fn cell(text: &str, width: u16, right: bool) -> String {
    use unicode_width::UnicodeWidthStr;
    let w = width as usize;
    if w == 0 {
        return String::new();
    }
    let t = truncate(text, w);
    let pad = w.saturating_sub(UnicodeWidthStr::width(t.as_str()));
    if right {
        format!("{}{}", " ".repeat(pad), t)
    } else {
        format!("{}{}", t, " ".repeat(pad))
    }
}

impl App {
    /// Indices into `installed_mods` that pass the current search filter,
    /// in display order. All list state (selection, hitboxes, actions) is
    /// expressed in these display positions.
    pub(crate) fn visible_mod_indices(&self) -> Vec<usize> {
        let query_lower = self.mods_search_query.to_lowercase();
        self.installed_mods
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
            .collect()
    }

    pub(crate) fn render_mods(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(5)])
            .split(area);

        let toggle = self.tr("btn.toggle");
        let delete = self.tr("btn.delete");
        let browse = self.tr("btn.browse");
        let updates = self.tr("btn.updates");
        let buttons: [(&str, &str, ButtonId); 4] = [
            (toggle, "Space", ButtonId::ToggleMod),
            (delete, "d", ButtonId::DeleteMod),
            (browse, "s", ButtonId::BrowseMods),
            (updates, "u", ButtonId::UpdateMods),
        ];
        let buttons_w: u16 = crate::views::row_widths(buttons.iter().map(|(l, k, _)| (*l, *k)))
            .iter()
            .sum::<u16>()
            + buttons.len().saturating_sub(1) as u16;
        let filter_w = area
            .width
            .saturating_sub(chunks[0].x + 2 + buttons_w + 1)
            .clamp(12, 36);
        let filter_rect = Rect {
            x: area.x + area.width.saturating_sub(filter_w),
            y: chunks[0].y,
            width: filter_w,
            height: 3,
        };
        buttons_row(
            self,
            frame,
            chunks[0].x + 2,
            chunks[0].y,
            filter_rect.x.saturating_sub(1),
            &buttons,
        );
        let query = self.mods_search_query.clone();
        crate::views::render_filter_pill(
            self,
            frame,
            filter_rect,
            &query,
            self.mods_search_focused,
            HitAction::ModsSearchBar,
        );

        self.render_installed_mods(frame, crate::views::left_gutter(chunks[1]));
    }

    fn render_installed_mods(&mut self, frame: &mut Frame, area: Rect) {
        let focused = self.focus == Focus::Content;
        let inner = card(self, frame, area, focused);
        if inner.height == 0 {
            return;
        }

        // Filter mods by search query
        let query_lower = self.mods_search_query.to_lowercase();
        let filtered_indices: Vec<usize> = self.visible_mod_indices();

        // ── Mod list ──
        let list_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: inner.height,
        };

        if filtered_indices.is_empty() {
            let hint = if self.mods_scanning {
                self.tr("empty.scanning_mods").to_string()
            } else if !query_lower.is_empty() {
                self.tr("empty.no_mods_filter").to_string()
            } else {
                self.tr("empty.no_mods").to_string()
            };
            frame.render_widget(
                Paragraph::new(Span::styled(hint, self.theme.card_dim())).style(self.theme.card()),
                list_area,
            );
            return;
        }

        let cols = ModColumns::compute(list_area.width);
        let selected = self.mods_state.selected();

        // Header row
        let header_rect = Rect {
            x: list_area.x,
            y: list_area.y,
            width: list_area.width,
            height: 1,
        };
        let header_style = self.theme.card_comment();
        let toggle_w = cols.name - cols.toggle;
        let name_w = cols.version - cols.name;
        let version_w = cols.size - cols.version;
        let size_w = cols.date - cols.size;
        let date_w = list_area.width.saturating_sub(cols.date);
        let header_line = Line::from(vec![
            Span::styled(cell("On/Off", toggle_w, false), header_style),
            Span::styled(cell("Name", name_w, false), header_style),
            Span::styled(cell("Version", version_w, false), header_style),
            Span::styled(
                if size_w > 0 {
                    format!("{} ", cell("Size", size_w - 1, true))
                } else {
                    String::new()
                },
                header_style,
            ),
            Span::styled(cell("Date", date_w, false), header_style),
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
        self.mods_visible = data_area.height as usize;
        // Hover is measured against the data area (below the header row):
        // using `list_area` would map the mouse to the row one below.
        let hovered = hovered_index(
            self,
            data_area,
            self.mods_state.offset(),
            filtered_indices.len(),
        );

        let items: Vec<ListItem> = filtered_indices
            .iter()
            .map(|&idx| {
                let module = &self.installed_mods[idx];
                let display_idx = filtered_indices.iter().position(|&i| i == idx).unwrap_or(0);
                let marker = if selected == Some(display_idx) {
                    "\u{25B8} "
                } else {
                    "  "
                };
                let toggle = if module.enabled {
                    Span::styled(
                        cell(&format!("{marker}\u{25CF}"), toggle_w, false),
                        self.theme.accent(),
                    )
                } else {
                    Span::styled(
                        cell(&format!("{marker}\u{25CB}"), toggle_w, false),
                        self.theme.dim(),
                    )
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
                    Span::styled(cell(&display_name, name_w, false), row_style),
                    Span::styled(cell(&version_text, version_w, false), dim_style),
                    Span::styled(
                        if size_w > 0 {
                            format!("{} ", cell(&size_text, size_w - 1, true))
                        } else {
                            String::new()
                        },
                        dim_style,
                    ),
                    Span::styled(cell(&date_text, date_w, true), dim_style),
                ]))
                .style(row_style)
            })
            .collect();

        let list = List::new(items).highlight_style(self.theme.row_selected());
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
                    self.edit.end_drag();
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Char('t') => {
                self.mods_search_focused = !self.mods_search_focused;
                if self.mods_search_focused {
                    self.edit.reset_with(&self.mods_search_query);
                }
            }
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

        let len = self.visible_mod_indices().len();
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => move_sel(&mut self.mods_state, len, 1),
            KeyCode::Up | KeyCode::Char('k') => move_sel(&mut self.mods_state, len, -1),
            KeyCode::PageDown => move_sel(&mut self.mods_state, len, 10),
            KeyCode::PageUp => move_sel(&mut self.mods_state, len, -10),
            KeyCode::Char('g') => jump(&mut self.mods_state, len, false),
            KeyCode::Char('G') => jump(&mut self.mods_state, len, true),
            KeyCode::Char(' ') | KeyCode::Enter => self.toggle_selected_mod(),
            KeyCode::Char('d') => self.confirm_delete_mod(),
            _ => {}
        }
    }
}
