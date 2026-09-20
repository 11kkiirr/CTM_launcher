//! Shader pack manager screen.

use crossterm::event::KeyEvent;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::{App, ButtonId, Focus};
use crate::views::{buttons_row, card, hovered_index, jump, move_sel, row_style, section_title};

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
                ("Open Folder", "Enter", ButtonId::BrowseMods),
            ],
        );

        frame.render_widget(
            Paragraph::new(section_title(
                "Shader Packs",
                &format!("{}", self.shaders.len()),
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

        if self.shaders.is_empty() {
            let lines = vec![
                Line::from(""),
                Line::from(Span::styled("No shader packs found.", self.theme.header())),
                Line::from(Span::styled(
                    "Place .zip files in the shaderpacks folder.",
                    self.theme.card_dim(),
                )),
            ];
            frame.render_widget(Paragraph::new(lines).style(self.theme.card()), inner);
            return;
        }

        let selected = self.shaders_state.selected();
        let hovered = hovered_index(self, inner, 0, self.shaders.len());
        let items: Vec<ListItem> = self
            .shaders
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

    pub(crate) fn key_shaders(&mut self, key: KeyEvent) {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                move_sel(&mut self.shaders_state, self.shaders.len(), 1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                move_sel(&mut self.shaders_state, self.shaders.len(), -1);
            }
            KeyCode::Char('g') => jump(&mut self.shaders_state, self.shaders.len(), false),
            KeyCode::Char('G') => jump(&mut self.shaders_state, self.shaders.len(), true),
            _ => {}
        }
    }
}
