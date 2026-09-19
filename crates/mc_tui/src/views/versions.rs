//! Versions page: shows the build's game version and loader, and lets the user
//! change or reinstall it.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, ButtonId};
use crate::views::buttons_row;

impl App {
    pub(crate) fn render_versions(&mut self, frame: &mut Frame, area: Rect) {
        let Some(instance) = self.selected_instance().cloned() else {
            return;
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(9),
                Constraint::Min(3),
            ])
            .split(area);

        buttons_row(
            self,
            frame,
            chunks[0].x + 1,
            chunks[0].y,
            area.x + area.width,
            &[
                ("Change Version", ButtonId::ChangeVersion),
                ("Reinstall", ButtonId::InstallInstance),
            ],
        );

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(self.theme.block_border())
            .title(Line::from(" Installed Version ").style(self.theme.header()));
        let inner = block.inner(chunks[1]);
        frame.render_widget(block, chunks[1]);

        let lines = vec![
            Line::from(vec![
                Span::styled("  Minecraft      ", self.theme.dim()),
                Span::styled(instance.metadata.game_version.clone(), self.theme.header()),
            ]),
            Line::from(vec![
                Span::styled("  Modloader      ", self.theme.dim()),
                Span::styled(
                    instance.metadata.loader.label().to_string(),
                    self.theme.accent(),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Loader version ", self.theme.dim()),
                Span::styled(
                    instance
                        .metadata
                        .loader_version
                        .clone()
                        .unwrap_or_else(|| "latest".to_string()),
                    self.theme.base(),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Instance id    ", self.theme.dim()),
                Span::styled(instance.id().to_string(), self.theme.dim()),
            ]),
        ];
        frame.render_widget(
            Paragraph::new(lines)
                .style(self.theme.base())
                .wrap(Wrap { trim: false }),
            inner,
        );

        frame.render_widget(
            Paragraph::new(Span::styled(
                "  Changing the game version resets the loader to the latest build for that version.",
                self.theme.dim(),
            ))
            .style(self.theme.base()),
            chunks[2],
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
