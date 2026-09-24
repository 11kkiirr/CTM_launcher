//! Versions page: shows the build's game version and loader, and lets the user
//! change or reinstall it.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, ButtonId, Focus};
use crate::views::{buttons_row, card, section_title};

impl App {
    pub(crate) fn render_versions(&mut self, frame: &mut Frame, area: Rect) {
        let Some(instance) = self.selected_instance().cloned() else {
            return;
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(9),
                Constraint::Length(1),
                Constraint::Min(3),
            ])
            .split(area);

        let change = self.tr("btn.change_version");
        let reinstall = self.tr("btn.reinstall");
        buttons_row(
            self,
            frame,
            chunks[0].x + 2,
            chunks[0].y,
            area.x + area.width,
            &[
                (change, "c", ButtonId::ChangeVersion),
                (reinstall, "r", ButtonId::InstallInstance),
            ],
        );

        let focused = self.focus == Focus::Content;
        let inner = card(self, frame, chunks[2], focused);
        if inner.height == 0 {
            return;
        }
        let title = self.tr("versions.title").to_string();
        frame.render_widget(
            Paragraph::new(section_title(&title, "", &self.theme))
                .style(self.theme.card()),
            Rect {
                x: inner.x,
                y: inner.y,
                width: inner.width,
                height: 1,
            },
        );

        let rows = [
            (
                self.tr("versions.minecraft").to_string(),
                instance.metadata.game_version.clone(),
            ),
            (
                self.tr("versions.modloader").to_string(),
                instance.metadata.loader.label().to_string(),
            ),
            (
                self.tr("versions.loader_version").to_string(),
                instance
                    .metadata
                    .loader_version
                    .clone()
                    .unwrap_or_else(|| self.tr("common.latest").to_string()),
            ),
            (
                self.tr("versions.instance_id").to_string(),
                instance.id().to_string(),
            ),
        ];
        for (idx, (label, value)) in rows.iter().enumerate() {
            let y = inner.y + 2 + idx as u16;
            if y >= inner.y + inner.height {
                break;
            }
            let row = Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: 1,
            };
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(format!("{label:<16}"), self.theme.card_dim()),
                    Span::styled(value.clone(), self.theme.card()),
                ]))
                .style(self.theme.card()),
                row,
            );
        }

        frame.render_widget(
            Paragraph::new(Span::styled(
                self.tr("versions.reset_hint"),
                self.theme.card_dim(),
            ))
            .style(self.theme.card()),
            chunks[4],
        );
    }

    pub(crate) fn key_versions(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('c') => self.open_change_version_picker(),
            KeyCode::Char('r') => self.install_selected_instance(),
            _ => {}
        }
    }
}
