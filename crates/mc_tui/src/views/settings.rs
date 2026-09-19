//! Settings screen: Java paths, default RAM/GC and UI toggles.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::{App, ButtonId, HitAction};
use crate::views::buttons_row;

const FIELD_COUNT: usize = 7;

impl App {
    pub(crate) fn render_settings(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(FIELD_COUNT as u16 + 2),
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
                ("Edit", ButtonId::EditSettings),
                ("Save", ButtonId::SaveSettings),
                ("Detect Java", ButtonId::DetectJava),
            ],
        );

        self.render_settings_fields(frame, chunks[1]);
        self.render_java_list(frame, chunks[2]);
    }

    fn render_settings_fields(&mut self, frame: &mut Frame, area: Rect) {
        let s = &self.settings;
        let java = s
            .java_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "auto-detect".to_string());
        let values = [
            ("Java Path", java),
            ("Default Min RAM", format!("{} MB", s.default_min_memory_mb)),
            ("Default Max RAM", format!("{} MB", s.default_max_memory_mb)),
            ("Default GC", s.default_gc.label().to_string()),
            (
                "Show Progress",
                if s.show_progress { "yes" } else { "no" }.to_string(),
            ),
            (
                "Confirm Quit",
                if s.confirm_quit { "yes" } else { "no" }.to_string(),
            ),
            (
                "Auto-scroll Logs",
                if s.log_auto_scroll { "yes" } else { "no" }.to_string(),
            ),
        ];

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(self.theme.block_border())
            .title(Line::from(" Launcher Settings ").style(self.theme.header()));
        let inner = block.inner(area);
        frame.render_widget(block, area);

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
            let selected = idx == self.settings_field;
            let style = if selected {
                self.theme.selection()
            } else {
                self.theme.base()
            };
            let marker = if selected { "▸" } else { " " };
            let line = Line::from(vec![
                Span::styled(format!(" {marker} "), style),
                Span::styled(format!("{label:<20}"), self.theme.dim()),
                Span::styled(value.clone(), style),
            ]);
            frame.render_widget(Paragraph::new(line).style(style), row);
            self.hitboxes.push(crate::app::Hitbox {
                rect: row,
                action: HitAction::SettingsRow(idx),
            });
        }
    }

    fn render_java_list(&mut self, frame: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .java_installations
            .iter()
            .map(|java| {
                ListItem::new(Line::from(vec![
                    Span::styled(format!("Java {:<3} ", java.major), self.theme.accent()),
                    Span::styled(java.version.clone(), self.theme.base()),
                    Span::styled(format!("  {}", java.path.display()), self.theme.dim()),
                ]))
            })
            .collect();

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(self.theme.block_border())
            .title(
                Line::from(format!(
                    " Detected Java Runtimes ({}) — press 'J' to rescan ",
                    self.java_installations.len()
                ))
                .style(self.theme.header()),
            );
        let inner = block.inner(area);
        let list = List::new(items).block(block);
        frame.render_widget(list, area);

        if self.java_installations.is_empty() {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "Scanning for Java runtimes...",
                    self.theme.dim(),
                ))
                .style(self.theme.base()),
                inner,
            );
        }
    }

    pub(crate) fn key_settings(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                self.settings_field = (self.settings_field + 1).min(FIELD_COUNT - 1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.settings_field = self.settings_field.saturating_sub(1);
            }
            KeyCode::Enter => self.open_settings_form(),
            KeyCode::Char(' ') => self.toggle_setting(self.settings_field),
            KeyCode::Left => self.cycle_setting_gc(false),
            KeyCode::Right => self.cycle_setting_gc(true),
            KeyCode::Char('s') => self.save_settings(),
            KeyCode::Char('J') => self.spawn_java_discovery(),
            _ => {}
        }
    }
}
