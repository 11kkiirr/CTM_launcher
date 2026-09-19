//! Console / Logs screen: real-time game output, filtering and crash analysis.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::{App, ButtonId, HitAction};
use crate::views::{buttons_row, hovered_index, jump, move_sel, register_rows, row_style};
use mc_core::logs::LogLevel;

impl App {
    pub(crate) fn render_logs(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(5),
                Constraint::Length(2),
            ])
            .split(area);

        let pause_label = if self.log_buffer.paused {
            "Resume"
        } else {
            "Pause"
        };
        buttons_row(
            self,
            frame,
            chunks[0].x + 1,
            chunks[0].y,
            area.x + area.width,
            &[
                (pause_label, ButtonId::PauseLogs),
                ("Clear", ButtonId::ClearLogs),
                ("Analyze Crash", ButtonId::AnalyzeCrash),
            ],
        );

        self.render_log_list(frame, chunks[1]);
        self.render_log_status(frame, chunks[2]);
    }

    fn render_log_list(&mut self, frame: &mut Frame, area: Rect) {
        let total = self.log_buffer.visible().count();
        let running = if self.running.is_some() {
            "● live"
        } else {
            "○ idle"
        };
        let paused = if self.log_buffer.paused {
            " · PAUSED"
        } else {
            ""
        };
        let title = format!(
            " Console {running}{paused} — {total} lines · min {} ",
            self.log_buffer.filter.min_level.label()
        );
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(self.theme.block_border())
            .title(Line::from(title).style(self.theme.header()));
        let inner = block.inner(area);

        let selected = self.log_state.selected();
        let hovered = hovered_index(self, inner, self.log_state.offset(), total);
        let items: Vec<ListItem> = self
            .log_buffer
            .visible()
            .enumerate()
            .map(|(idx, entry)| {
                let color = self.theme.log_level_color(entry.level);
                let time = entry.timestamp.clone().unwrap_or_default();
                let thread = entry.thread.clone().unwrap_or_default();
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{time:>8} "), self.theme.dim()),
                    Span::styled(
                        format!("{:>5} ", entry.level.label()),
                        ratatui::style::Style::default().fg(color),
                    ),
                    Span::styled(format!("[{thread}] "), self.theme.dim()),
                    Span::styled(entry.message.clone(), self.theme.base()),
                ]))
                .style(row_style(self, idx, selected, hovered))
            })
            .collect();

        let list = List::new(items).block(block);
        frame.render_stateful_widget(list, area, &mut self.log_state);
        register_rows(
            &mut self.hitboxes,
            &self.log_state,
            inner,
            total,
            HitAction::LogRow,
        );

        if total == 0 {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "No log output yet. Launch an instance or open a saved latest.log.",
                    self.theme.dim(),
                ))
                .style(self.theme.base()),
                inner,
            );
        }
    }

    fn render_log_status(&mut self, frame: &mut Frame, area: Rect) {
        let errors = self.log_buffer.error_count();
        let filter = if self.log_search.is_empty() {
            "no filter".to_string()
        } else {
            format!("filter: '{}'", self.log_search)
        };
        let command = self
            .last_command
            .as_ref()
            .map(|c| format!("  cmd: {c}"))
            .unwrap_or_default();

        let lines = vec![
            Line::from(vec![
                Span::styled("  Errors: ", self.theme.dim()),
                Span::styled(
                    errors.to_string(),
                    if errors > 0 {
                        self.theme.error_style()
                    } else {
                        self.theme.accent()
                    },
                ),
                Span::styled(format!("   {filter}"), self.theme.dim()),
                Span::styled(
                    format!("   total buffered: {}", self.log_buffer.len()),
                    self.theme.dim(),
                ),
            ]),
            Line::from(Span::styled(command, self.theme.dim())),
        ];
        frame.render_widget(Paragraph::new(lines).style(self.theme.base()), area);
    }

    pub(crate) fn key_logs(&mut self, key: KeyEvent) {
        let len = self.log_buffer.visible().count();
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => move_sel(&mut self.log_state, len, 1),
            KeyCode::Up | KeyCode::Char('k') => move_sel(&mut self.log_state, len, -1),
            KeyCode::PageDown => move_sel(&mut self.log_state, len, 15),
            KeyCode::PageUp => move_sel(&mut self.log_state, len, -15),
            KeyCode::Char('g') => jump(&mut self.log_state, len, false),
            KeyCode::Char('G') => jump(&mut self.log_state, len, true),
            KeyCode::Char('p') => self.log_buffer.paused = !self.log_buffer.paused,
            KeyCode::Char('c') => self.log_buffer.clear(),
            KeyCode::Char('/') => {
                self.overlay = Some(crate::forms::Overlay::text(
                    "Filter Logs",
                    "Substring: ",
                    crate::forms::TextAction::SearchLogs,
                ));
            }
            KeyCode::Char('a') => self.analyze_crash(),
            KeyCode::Char('l') => {
                let next = match self.log_buffer.filter.min_level {
                    LogLevel::Trace => LogLevel::Debug,
                    LogLevel::Debug => LogLevel::Info,
                    LogLevel::Info => LogLevel::Warn,
                    LogLevel::Warn => LogLevel::Error,
                    LogLevel::Error | LogLevel::Fatal | LogLevel::Unknown => LogLevel::Trace,
                };
                self.log_buffer.filter.min_level = next;
            }
            _ => {}
        }
    }
}
