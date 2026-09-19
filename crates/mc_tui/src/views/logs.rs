//! Console / Logs screen: a borderless log stream with a minimal tab bar and a
//! bottom status line.
//!
//! The console uses a simple viewport model instead of a selectable list: the
//! whole text scrolls by several lines at a time, and "follow" mode keeps the
//! view pinned to the newest output.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, ButtonId, Focus};
use crate::views::{tab_row, truncate};
use mc_core::logs::LogLevel;

impl App {
    pub(crate) fn render_logs(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(5),
                Constraint::Length(1),
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
        tab_row(
            self,
            frame,
            chunks[0].x,
            chunks[0].y,
            area.x + area.width,
            &[
                (pause_label, ButtonId::PauseLogs, self.log_buffer.paused),
                (follow_label, ButtonId::FollowLogs, self.log_follow),
                ("Clear", ButtonId::ClearLogs, false),
                ("Analyze Crash", ButtonId::AnalyzeCrash, false),
            ],
        );

        self.render_log_list(frame, chunks[2]);
        self.render_log_status(frame, chunks[3]);
    }

    fn render_log_list(&mut self, frame: &mut Frame, area: Rect) {
        let total = self.log_buffer.visible().count();
        let focused = self.focus == Focus::Content;
        let inner = crate::views::card(self, frame, area, focused);
        if inner.height == 0 {
            return;
        }

        let running = if self.running.is_some() {
            ("● live", self.theme.accent())
        } else {
            ("○ idle", self.theme.card_dim())
        };
        let state = if self.log_buffer.paused {
            ("PAUSED", self.theme.warning_style())
        } else if self.log_follow {
            ("FOLLOW", self.theme.accent())
        } else {
            ("UNFOLLOW", self.theme.card_dim())
        };

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(inner);

        let range = if total == 0 {
            "no output".to_string()
        } else {
            let visible = rows[1].height as usize;
            let max_scroll = total.saturating_sub(visible);
            if self.log_follow {
                self.log_scroll = max_scroll;
            } else {
                self.log_scroll = self.log_scroll.min(max_scroll);
            }
            let start = self.log_scroll;
            let end = (start + visible).min(total);
            format!("lines {}-{} / {total}", start + 1, end)
        };

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Console", self.theme.header()),
                Span::styled("   ", self.theme.card()),
                Span::styled(running.0, running.1),
                Span::styled("   ", self.theme.card()),
                Span::styled(state.0, state.1),
                Span::styled(format!("   {range}"), self.theme.card_dim()),
                Span::styled(
                    format!("   min {}", self.log_buffer.filter.min_level.label()),
                    self.theme.card_comment(),
                ),
            ]))
            .style(self.theme.card()),
            rows[0],
        );

        let viewport = rows[1];
        let visible = viewport.height as usize;
        self.log_visible = visible;

        if total == 0 {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "No log output yet. Launch a build or open a saved latest.log.",
                    self.theme.card_dim(),
                ))
                .style(self.theme.card()),
                viewport,
            );
            return;
        }

        let max_scroll = total.saturating_sub(visible);
        if self.log_follow {
            self.log_scroll = max_scroll;
        } else {
            self.log_scroll = self.log_scroll.min(max_scroll);
        }
        let start = self.log_scroll;

        let lines: Vec<Line> = self
            .log_buffer
            .visible()
            .skip(start)
            .take(visible)
            .map(|entry| {
                let color = self.theme.log_level_color(entry.level);
                let time = entry.timestamp.clone().unwrap_or_default();
                let thread = entry.thread.clone().unwrap_or_default();
                let prefix = format!("{time:>8} {:>5} [{thread}] ", entry.level.label());
                let prefix_width = prefix.chars().count();
                let message_width = (viewport.width as usize).saturating_sub(prefix_width);
                Line::from(vec![
                    Span::styled(format!("{time:>8} "), self.theme.card_comment()),
                    Span::styled(
                        format!("{:>5} ", entry.level.label()),
                        Style::default().fg(color).bg(self.theme.panel),
                    ),
                    Span::styled(format!("[{thread}] "), self.theme.card_dim()),
                    Span::styled(truncate(&entry.message, message_width), self.theme.card()),
                ])
            })
            .collect();

        // No wrapping: each entry occupies exactly one row and long lines are
        // truncated, so the viewport never overflows the panel.
        frame.render_widget(Paragraph::new(lines).style(self.theme.card()), viewport);
    }

    fn render_log_status(&mut self, frame: &mut Frame, area: Rect) {
        let errors = self.log_buffer.error_count();
        let filter = if self.log_search.is_empty() {
            "no filter".to_string()
        } else {
            format!("filter '{}'", self.log_search)
        };
        let ready = if self.log_buffer.paused {
            ("Paused", self.theme.warning_style())
        } else if self.running.is_some() {
            ("Running", self.theme.accent())
        } else {
            ("Ready", self.theme.accent())
        };

        let runtime = self
            .running
            .as_ref()
            .map(|r| format!("    running {:.0}s", r.started.elapsed().as_secs()))
            .unwrap_or_default();
        let line = Line::from(vec![
            Span::styled("● ", ready.1),
            Span::styled(ready.0, ready.1),
            Span::styled(runtime, self.theme.dim()),
            Span::styled("    errors ", self.theme.dim()),
            Span::styled(
                errors.to_string(),
                if errors > 0 {
                    self.theme.error_style()
                } else {
                    self.theme.accent()
                },
            ),
            Span::styled(format!("    {filter}"), self.theme.dim()),
            Span::styled(
                format!("    buffered {}", self.log_buffer.len()),
                self.theme.comment_style(),
            ),
        ]);
        frame.render_widget(Paragraph::new(line).style(self.theme.base()), area);
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
