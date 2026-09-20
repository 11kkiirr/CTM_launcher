//! Shader pack manager screen.

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
    pub(crate) fn render_shaders(&mut self, frame: &mut Frame, area: Rect) {
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
            chunks[0].x + 1,
            chunks[0].y,
            area.x + area.width,
            &[
                ("Toggle", "Space", ButtonId::ToggleMod),
                ("Delete", "d", ButtonId::DeleteMod),
                ("Browse", "s", ButtonId::ShadersBrowse),
            ],
        );

        self.render_installed_shaders(frame, chunks[2]);
    }

    fn render_installed_shaders(&mut self, frame: &mut Frame, area: Rect) {
        let focused = self.focus == Focus::Content && !self.shaders_search_focused;
        let inner = card(self, frame, area, focused);
        if inner.height == 0 {
            return;
        }

        // ── Inline filter bar ──
        let search_focused = self.shaders_search_focused;
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
        let query_text = if self.shaders_inline_query.is_empty() && !search_focused {
            String::new()
        } else if search_focused {
            format!("{}█", self.shaders_inline_query)
        } else {
            self.shaders_inline_query.clone()
        };
        let query_style = if search_focused {
            Style::default().fg(self.theme.green).bg(search_bg)
        } else if !self.shaders_inline_query.is_empty() {
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

        let query_lower = self.shaders_inline_query.to_lowercase();
        let filtered: Vec<(usize, &String)> = self
            .shaders
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
            format!("{}", self.shaders.len())
        } else {
            format!("{}/{}", filtered.len(), self.shaders.len())
        };
        frame.render_widget(
            Paragraph::new(section_title(
                "Shader Packs",
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
                "No shaders match the filter.".to_string()
            } else {
                "No shader packs found. Press 's' to browse Modrinth.".to_string()
            };
            frame.render_widget(
                Paragraph::new(Span::styled(hint, self.theme.card_dim())).style(self.theme.card()),
                list_area,
            );
            return;
        }

        let selected = self.shaders_state.selected();
        let hovered = hovered_index(self, list_area, self.shaders_state.offset(), filtered.len());

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
        frame.render_stateful_widget(list, list_area, &mut self.shaders_state);
    }

    pub(crate) fn key_shaders(&mut self, key: KeyEvent) {
        // When inline filter bar is focused, handle input directly.
        if self.shaders_search_focused {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.shaders_search_focused = false;
                }
                KeyCode::Backspace => {
                    self.shaders_inline_query.pop();
                }
                KeyCode::Char(c) => {
                    self.shaders_inline_query.push(c);
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Char('t') => self.shaders_search_focused = !self.shaders_search_focused,
            KeyCode::Char('s') => {
                let kind = crate::views::browse::BrowseKind::Shaders;
                self.browse.save_cache();
                self.browse.load_cache(kind);
                self.open_nav(crate::app::Nav::Browse);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self.shaders.len();
                move_sel(&mut self.shaders_state, len, 1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let len = self.shaders.len();
                move_sel(&mut self.shaders_state, len, -1);
            }
            KeyCode::Char('g') => jump(&mut self.shaders_state, self.shaders.len(), false),
            KeyCode::Char('G') => jump(&mut self.shaders_state, self.shaders.len(), true),
            _ => {}
        }
    }
}
