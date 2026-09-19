//! Instance Overview and instance-scoped Settings pages.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::{App, ButtonId};
use crate::views::buttons_row;

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
                Constraint::Length(10),
                Constraint::Min(3),
            ])
            .split(area);

        buttons_row(
            self,
            frame,
            chunks[0].x + 1,
            chunks[0].y,
            area.x + area.width,
            &[("Edit Settings", ButtonId::EditInstance)],
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

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(self.theme.block_border())
            .title(Line::from(" Instance Settings ").style(self.theme.header()));
        let inner = block.inner(chunks[1]);
        frame.render_widget(block, chunks[1]);
        for (idx, (label, value)) in values.iter().enumerate() {
            if idx as u16 >= inner.height {
                break;
            }
            let row = Rect {
                x: inner.x,
                y: inner.y + idx as u16,
                width: inner.width,
                height: 1,
            };
            let line = Line::from(vec![
                Span::styled(format!("  {label:<20}"), self.theme.dim()),
                Span::styled(value.clone(), self.theme.base()),
            ]);
            frame.render_widget(Paragraph::new(line).style(self.theme.base()), row);
        }

        frame.render_widget(
            Paragraph::new(Span::styled(
                "  Press Enter or 'e' to edit. Changes apply the next time you launch.",
                self.theme.dim(),
            ))
            .style(self.theme.base()),
            chunks[2],
        );
    }

    pub(crate) fn key_instance_settings(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter | KeyCode::Char('e') => self.open_edit_instance_form(),
            _ => {}
        }
    }
}
