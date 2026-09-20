//! Resource pack manager screen.

use crossterm::event::KeyEvent;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::{App, ButtonId, Focus};
use crate::views::{buttons_row, card, hovered_index, jump, move_sel, row_style, section_title};

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
            chunks[0].x + 1,
            chunks[0].y,
            area.x + area.width,
            &[
                ("Toggle", "Space", ButtonId::ToggleMod),
                ("Delete", "d", ButtonId::DeleteMod),
                ("Open Folder", "Enter", ButtonId::BrowseMods),
            ],
        );

        frame.render_widget(
            Paragraph::new(section_title(
                "Resource Packs",
                &format!("{}", self.resource_packs.len()),
                &self.theme,
            ))
            .style(self.theme.card()),
            chunks[1],
        );

        let focused = self.focus == Focus::Content;
        let inner = card(self, frame, chunks[2], focused);
        if inner.height == 0 {
            return;
        }

        if self.resource_packs.is_empty() {
            let lines = vec![
                Line::from(""),
                Line::from(Span::styled("No resource packs found.", self.theme.header())),
                Line::from(Span::styled(
                    "Place .zip files in the resourcepacks folder.",
                    self.theme.card_dim(),
                )),
            ];
            frame.render_widget(Paragraph::new(lines).style(self.theme.card()), inner);
            return;
        }

        let selected = self.resource_packs_state.selected();
        let hovered = hovered_index(self, inner, 0, self.resource_packs.len());
        let items: Vec<ListItem> = self
            .resource_packs
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let style = row_style(self, i, selected, hovered);
                ListItem::new(Line::from(Span::styled(name.as_str(), style)))
            })
            .collect();
        let list = List::new(items).style(self.theme.card());
        frame.render_widget(list, inner);
    }

    pub(crate) fn key_resource_packs(&mut self, key: KeyEvent) {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                move_sel(&mut self.resource_packs_state, self.resource_packs.len(), 1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                move_sel(&mut self.resource_packs_state, self.resource_packs.len(), -1);
            }
            KeyCode::Char('g') => jump(&mut self.resource_packs_state, self.resource_packs.len(), false),
            KeyCode::Char('G') => jump(&mut self.resource_packs_state, self.resource_packs.len(), true),
            _ => {}
        }
    }
}
