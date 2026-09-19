//! Console / Logs screen: real-time game output, filtering, scrolling and
//! crash analysis.
//!
//! The console uses a simple viewport model instead of a selectable list: the
//! whole text scrolls by several lines at a time, and "follow" mode keeps the
//! view pinned to the newest output.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::{App, ButtonId};
use crate::views::buttons_row;
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
        let follow_label = if self.log_follow {
            "Unfollow"
        } else {
            "Follow"
        };
        buttons_row(
            self,
            frame,
            chunks[0].x + 1,
            chunks[0].y,
            area.x + area.width,
            &[
                (pause_label, ButtonId::PauseLogs),
                (follow_label, ButtonId::FollowLogs),
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
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(self.theme.block_border());
        let inner = block.inner(area);

        let visible = inner.height as usize;
        self.log_visible = visible;
        let max_scroll = total.saturating_sub(visible);
        if self.log_follow {
            self.log_scroll = max_scroll;
        } else {
            self.log_scroll = self.log_scroll.min(max_scroll);
        }
        let start = self.log_scroll;
        let shown = visible.min(total.saturating_sub(start));
        let end = start + shown;
        let new_below = total.saturating_sub(end);

        let follow = if self.log_follow {
            " · FOLLOW".to_string()
        } else if new_below > 0 {
            format!(" · ↓{new_below} new")
        } else {
            " · PAUSED".to_string()
        };
        let title = if total == 0 {
            format!(" Console {running}{paused}{follow} ")
        } else {
            format!(
                " Console {running}{paused}{follow} — lines {}-{} / {total} · min {} ",
                start + 1,
                end,
                self.log_buffer.filter.min_level.label()
            )
        };
        let block = block.title(Line::from(title).style(self.theme.header()));
        frame.render_widget(block, area);

        if total == 0 {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "No log output yet. Launch a build or open a saved latest.log.",
                    self.theme.dim(),
                ))
                .style(self.theme.base()),
                inner,
            );
            return;
        }

        let lines: Vec<Line> = self
            .log_buffer
            .visible()
            .skip(start)
            .take(visible)
            .map(|entry| {
                let color = self.theme.log_level_color(entry.level);
                let time = entry.timestamp.clone().unwrap_or_default();
                let thread = entry.thread.clone().unwrap_or_default();
                Line::from(vec![
                    Span::styled(format!("{time:>8} "), self.theme.dim()),
                    Span::styled(
                        format!("{:>5} ", entry.level.label()),
                        ratatui::style::Style::default().fg(color),
                    ),
                    Span::styled(format!("[{thread}] "), self.theme.dim()),
                    Span::styled(entry.message.clone(), self.theme.base()),
                ])
            })
            .collect();

        // No wrapping: each entry occupies exactly one row and long lines are
        // truncated, so the viewport never overflows the panel.
        frame.render_widget(Paragraph::new(lines).style(self.theme.base()), inner);
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
                    format!("   buffered: {}", self.log_buffer.len()),
                    self.theme.dim(),
                ),
            ]),
            Line::from(Span::styled(command, self.theme.dim())),
        ];
        frame.render_widget(Paragraph::new(lines).style(self.theme.base()), area);
    }

    pub(crate) fn key_logs(&mut self, key: KeyEvent) {
        let page = self.log_visible.max(1) as i32;
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => self.scroll_logs(1),
            KeyCode::Up | KeyCode::Char('k') => self.scroll_logs(-1),
            KeyCode::PageDown => self.scroll_logs(page),
            KeyCode::PageUp => self.scroll_logs(-page),
            KeyCode::Home | KeyCode::Char('g') => self.jump_logs(false),
            KeyCode::End | KeyCode::Char('G') => self.jump_logs(true),
            KeyCode::Char('f') => self.log_follow = !self.log_follow,
            KeyCode::Char('p') => self.log_buffer.paused = !self.log_buffer.paused,
            KeyCode::Char('c') => {
                self.log_buffer.clear();
                self.log_scroll = 0;
                self.log_follow = true;
            }
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
