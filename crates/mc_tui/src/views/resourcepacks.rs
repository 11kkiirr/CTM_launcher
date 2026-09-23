//! Resource pack manager screen.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::{App, ButtonId, Focus};
use crate::views::{
    buttons_row, card, hovered_index, jump, move_sel, row_style, section_title,
};

impl App {
    pub(crate) fn render_resource_packs(&mut self, frame: &mut Frame, area: Rect) {
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
                ("Toggle", "Space", ButtonId::ToggleSelected),
                ("Delete", "d", ButtonId::DeleteSelected),
                ("Browse", "s", ButtonId::RpBrowse),
            ],
        );

        self.render_installed_resource_packs(frame, chunks[2]);
    }

    fn render_installed_resource_packs(&mut self, frame: &mut Frame, area: Rect) {
        let focused = self.focus == Focus::Content && !self.rp_search_focused;
        let inner = card(self, frame, area, focused);
        if inner.height == 0 {
            return;
        }

        // ── Inline filter bar ──
        let search_focused = self.rp_search_focused;
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
        let query_text = if self.rp_inline_query.is_empty() && !search_focused {
            String::new()
        } else if search_focused {
            format!("{}█", self.rp_inline_query)
        } else {
            self.rp_inline_query.clone()
        };
        let query_style = if search_focused {
            Style::default().fg(self.theme.green).bg(search_bg)
        } else if !self.rp_inline_query.is_empty() {
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

        // ── Title ──
        let instance_name = self
            .selected_instance()
            .map(|i| i.name().to_string())
            .unwrap_or_else(|| "no instance".to_string());

        let query_lower = self.rp_inline_query.to_lowercase();
        let filtered: Vec<(usize, &String)> = self
            .resource_packs
            .iter()
            .enumerate()
            .filter(|(_, name)| {
                if query_lower.is_empty() {
                    true
                } else {
                    name.to_lowercase().contains(&query_lower)
                }
            })
            .collect();

        let title = Rect {
            x: inner.x,
            y: inner.y + 1,
            width: inner.width,
            height: 1,
        };
        let count_text = if query_lower.is_empty() {
            format!("{}", self.resource_packs.len())
        } else {
            format!("{}/{}", filtered.len(), self.resource_packs.len())
        };
        frame.render_widget(
            Paragraph::new(section_title(
                "Resource Packs",
                &format!("{instance_name}  ·  {count_text}"),
                &self.theme,
            ))
            .style(self.theme.card()),
            title,
        );

        // ── List ──
        let list_area = Rect {
            x: inner.x,
            y: inner.y + 2,
            width: inner.width,
            height: inner.height.saturating_sub(2),
        };

        if filtered.is_empty() {
            let hint = if !query_lower.is_empty() {
                "No resource packs match the filter.".to_string()
            } else {
                "No resource packs found. Press 's' to browse Modrinth.".to_string()
            };
            frame.render_widget(
                Paragraph::new(Span::styled(hint, self.theme.card_dim())).style(self.theme.card()),
                list_area,
            );
            return;
        }

        let selected = self.resource_packs_state.selected();
        let hovered = hovered_index(
            self,
            list_area,
            self.resource_packs_state.offset(),
            filtered.len(),
        );

        let items: Vec<ListItem> = filtered
            .iter()
            .enumerate()
            .map(|(display_idx, &(_orig_idx, name))| {
                let style = row_style(self, display_idx, selected, hovered);
                ListItem::new(Line::from(Span::styled(
                    format!("  {name}"),
                    style,
                )))
                .style(style)
            })
            .collect();

        let list = List::new(items)
            .highlight_symbol("\u{25B8} ")
            .highlight_style(self.theme.row_selected());
        frame.render_stateful_widget(list, list_area, &mut self.resource_packs_state);
    }

    pub(crate) fn key_resource_packs(&mut self, key: KeyEvent) {
        // When inline filter bar is focused, handle input directly.
        if self.rp_search_focused {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.rp_search_focused = false;
                }
                KeyCode::Backspace => {
                    self.rp_inline_query.pop();
                }
                KeyCode::Char(c) => {
                    self.rp_inline_query.push(c);
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Char('t') => self.rp_search_focused = !self.rp_search_focused,
            KeyCode::Char(' ') => self.toggle_selected_entry(),
            KeyCode::Char('d') => self.confirm_delete_selected(),
            KeyCode::Enter => self.open_current_folder(),
            KeyCode::Char('s') => {
                let kind = crate::views::browse::BrowseKind::ResourcePacks;
                self.browse.save_cache();
                self.browse.load_cache(kind);
                self.open_nav(crate::app::Nav::Browse);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self.resource_packs.len();
                move_sel(&mut self.resource_packs_state, len, 1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let len = self.resource_packs.len();
                move_sel(&mut self.resource_packs_state, len, -1);
            }
            KeyCode::Char('g') => {
                let len = self.resource_packs.len();
                jump(&mut self.resource_packs_state, len, false);
            }
            KeyCode::Char('G') => {
                let len = self.resource_packs.len();
                jump(&mut self.resource_packs_state, len, true);
            }
            _ => {}
        }
    }
}
