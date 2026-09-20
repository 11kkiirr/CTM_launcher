//! Instance-scoped Settings page (JVM, memory, Java).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, ButtonId, Focus};
use crate::views::{buttons_row, card, section_title};

impl App {
    /// The instance-scoped "Settings" page (JVM, memory, Java).
    pub(crate) fn render_instance_settings(&mut self, frame: &mut Frame, area: Rect) {
        let Some(instance) = self.selected_instance().cloned() else {
            return;
        };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(11),
                Constraint::Length(1),
                Constraint::Min(3),
            ])
            .split(area);

        buttons_row(
            self,
            frame,
            chunks[0].x + 1,
            chunks[0].y,
            area.x + area.width,
            &[("Edit Settings", "e", ButtonId::EditInstance)],
        );

        let jvm = &instance.metadata.jvm;
        let values = [
            ("Min RAM (MB)", jvm.min_memory_mb.to_string()),
            ("Max RAM (MB)", jvm.max_memory_mb.to_string()),
            ("Garbage Collector", jvm.gc.label().to_string()),
            (
                "Java Path",
                jvm.java_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "auto-detect".to_string()),
            ),
            (
                "Fullscreen",
                if jvm.fullscreen { "yes" } else { "no" }.to_string(),
            ),
            ("Custom JVM Args", jvm.custom_jvm_args.join(" ")),
            ("Extra Game Args", jvm.extra_game_args.join(" ")),
        ];

        let focused = self.focus == Focus::Content;
        let inner = card(self, frame, chunks[2], focused);
        if inner.height == 0 {
            return;
        }
        frame.render_widget(
            Paragraph::new(section_title("Instance Settings", "", &self.theme))
                .style(self.theme.card()),
            Rect {
                x: inner.x,
                y: inner.y,
                width: inner.width,
                height: 1,
            },
        );

        for (idx, (label, value)) in values.iter().enumerate() {
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
                    Span::styled(format!("{label:<20}"), self.theme.card_dim()),
                    Span::styled(value.clone(), self.theme.card()),
                ]))
                .style(self.theme.card()),
                row,
            );
        }

        frame.render_widget(
            Paragraph::new(Span::styled(
                "Press Enter or 'e' to edit. Changes apply the next time you launch.",
                self.theme.card_dim(),
            ))
            .style(self.theme.card()),
            chunks[4],
        );
    }

    pub(crate) fn key_instance_settings(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter | KeyCode::Char('e') => self.open_edit_instance_form(),
            _ => {}
        }
    }
}
