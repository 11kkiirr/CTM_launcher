//! Resource pack manager screen.

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
    /// Indices into `resource_packs` that pass the current filter, in display
    /// order. Selection and actions use these display positions.
    pub(crate) fn visible_rp_indices(&self) -> Vec<usize> {
        let query_lower = self.rp_inline_query.to_lowercase();
        self.resource_packs
            .iter()
            .enumerate()
            .filter(|(_, name)| {
                query_lower.is_empty() || name.to_lowercase().contains(&query_lower)
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub(crate) fn render_resource_packs(&mut self, frame: &mut Frame, area: Rect) {
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
            (browse, "s", ButtonId::RpBrowse),
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
        let query = self.rp_inline_query.clone();
        crate::views::render_filter_pill(
            self,
            frame,
            filter_rect,
            &query,
            self.rp_search_focused,
            HitAction::RPSearchBar,
        );

        self.render_installed_resource_packs(frame, crate::views::left_gutter(chunks[1]));
    }

    fn render_installed_resource_packs(&mut self, frame: &mut Frame, area: Rect) {
        let focused = self.focus == Focus::Content;
        let inner = card(self, frame, area, focused);
        if inner.height == 0 {
            return;
        }

        let query_lower = self.rp_inline_query.to_lowercase();
        let filtered: Vec<usize> = self.visible_rp_indices();

        // ── List ──
        let list_area = Rect {
            x: inner.x,
            y: inner.y + 1,
            width: inner.width,
            height: inner.height.saturating_sub(1),
        };
        self.resource_packs_visible = list_area.height as usize;

        if filtered.is_empty() {
            let hint = if !query_lower.is_empty() {
                self.tr("empty.no_rp_filter").to_string()
            } else {
                self.tr("empty.no_rp").to_string()
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
            .map(|(display_idx, &orig_idx)| {
                let name = &self.resource_packs[orig_idx];
                let style = row_style(self, display_idx, selected, hovered);
                ListItem::new(Line::from(Span::styled(format!("  {name}"), style))).style(style)
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
                    self.edit.end_drag();
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Char('t') => {
                self.rp_search_focused = !self.rp_search_focused;
                if self.rp_search_focused {
                    self.edit.reset_with(&self.rp_inline_query);
                }
            }
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
                let len = self.visible_rp_indices().len();
                move_sel(&mut self.resource_packs_state, len, 1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let len = self.visible_rp_indices().len();
                move_sel(&mut self.resource_packs_state, len, -1);
            }
            KeyCode::PageDown => {
                let len = self.visible_rp_indices().len();
                move_sel(&mut self.resource_packs_state, len, 10);
            }
            KeyCode::PageUp => {
                let len = self.visible_rp_indices().len();
                move_sel(&mut self.resource_packs_state, len, -10);
            }
            KeyCode::Char('g') => {
                let len = self.visible_rp_indices().len();
                jump(&mut self.resource_packs_state, len, false);
            }
            KeyCode::Char('G') => {
                let len = self.visible_rp_indices().len();
                jump(&mut self.resource_packs_state, len, true);
            }
            _ => {}
        }
    }
}
