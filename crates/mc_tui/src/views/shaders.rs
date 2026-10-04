//! Shader pack manager screen.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::{App, ButtonId, Focus, HitAction};
use crate::views::{
    buttons_row, card, hovered_index, jump, move_sel, row_style,
};

impl App {
    /// Indices into `shaders` that pass the current filter, in display order.
    pub(crate) fn visible_shader_indices(&self) -> Vec<usize> {
        let query_lower = self.shaders_inline_query.to_lowercase();
        self.shaders
            .iter()
            .enumerate()
            .filter(|(_, name)| {
                query_lower.is_empty() || name.to_lowercase().contains(&query_lower)
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub(crate) fn render_shaders(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(5)])
            .split(area);

        let toggle = self.tr("btn.toggle");
        let delete = self.tr("btn.delete");
        let browse = self.tr("btn.browse");
        let buttons = [
            (toggle, "Space", ButtonId::ToggleSelected),
            (delete, "d", ButtonId::DeleteSelected),
            (browse, "s", ButtonId::ShadersBrowse),
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
        let query = self.shaders_inline_query.clone();
        crate::views::render_filter_pill(
            self,
            frame,
            filter_rect,
            &query,
            self.shaders_search_focused,
            HitAction::ShadersSearchBar,
        );

        self.render_installed_shaders(frame, crate::views::left_gutter(chunks[1]));
    }

    fn render_installed_shaders(&mut self, frame: &mut Frame, area: Rect) {
        let focused = self.focus == Focus::Content;
        let inner = card(self, frame, area, focused);
        if inner.height == 0 {
            return;
        }

        let query_lower = self.shaders_inline_query.to_lowercase();
        let filtered: Vec<usize> = self.visible_shader_indices();

        // ── List ──
        let list_area = Rect {
            x: inner.x,
            y: inner.y + 1,
            width: inner.width,
            height: inner.height.saturating_sub(1),
        };
        self.shaders_visible = list_area.height as usize;

        if filtered.is_empty() {
            let hint = if !query_lower.is_empty() {
                self.tr("empty.no_shaders_filter").to_string()
            } else {
                self.tr("empty.no_shaders").to_string()
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
            .map(|(display_idx, &orig_idx)| {
                let name = &self.shaders[orig_idx];
                let style = row_style(self, display_idx, selected, hovered);
                ListItem::new(Line::from(Span::styled(format!("  {name}"), style))).style(style)
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
                    self.edit.end_drag();
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Char('t') => {
                self.shaders_search_focused = !self.shaders_search_focused;
                if self.shaders_search_focused {
                    self.edit.reset_with(&self.shaders_inline_query);
                }
            }
            KeyCode::Char(' ') => self.toggle_selected_entry(),
            KeyCode::Char('d') => self.confirm_delete_selected(),
            KeyCode::Enter => self.open_current_folder(),
            KeyCode::Char('s') => {
                let kind = crate::views::browse::BrowseKind::Shaders;
                self.browse.save_cache();
                self.browse.load_cache(kind);
                self.open_nav(crate::app::Nav::Browse);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self.visible_shader_indices().len();
                move_sel(&mut self.shaders_state, len, 1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let len = self.visible_shader_indices().len();
                move_sel(&mut self.shaders_state, len, -1);
            }
            KeyCode::PageDown => {
                let len = self.visible_shader_indices().len();
                move_sel(&mut self.shaders_state, len, 10);
            }
            KeyCode::PageUp => {
                let len = self.visible_shader_indices().len();
                move_sel(&mut self.shaders_state, len, -10);
            }
            KeyCode::Char('g') => {
                let len = self.visible_shader_indices().len();
                jump(&mut self.shaders_state, len, false);
            }
            KeyCode::Char('G') => {
                let len = self.visible_shader_indices().len();
                jump(&mut self.shaders_state, len, true);
            }
            _ => {}
        }
    }
}
