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
use ratatui::widgets::{Paragraph, Clear};
use ratatui::Frame;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

use crate::app::{App, ButtonId, Focus};
use crate::views::tab_row;
use mc_core::logs::{LogEntry, LogLevel};

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
            chunks[0].x + 1,
            chunks[0].y,
            area.x + area.width,
            &[
                (pause_label, "p", ButtonId::PauseLogs, self.log_buffer.paused),
                (follow_label, "f", ButtonId::FollowLogs, self.log_follow),
                ("Clear", "c", ButtonId::ClearLogs, false),
                ("Analyze Crash", "a", ButtonId::AnalyzeCrash, false),
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

        let viewport = rows[1];
        let visible = viewport.height as usize;
        self.log_visible = visible;
        let wrap_w = (viewport.width as usize).saturating_sub(2).max(1);

        // Flatten the filtered log into wrapped display rows so every message
        // stays inside the panel no matter how long it is. Every fragment is
        // width-aware (wide CJK chars count as two columns) and leaves room
        // for the `time level [thread] ` prefix, so no line ever exceeds the
        // viewport and wide characters never misalign the terminal buffer.
        let entries: Vec<&LogEntry> = self.log_buffer.visible().collect();
        let mut total_height: usize = 0;
        for entry in &entries {
            let (_, _, _, _, height) = entry_layout(entry, wrap_w);
            total_height += height;
        }
        let max_scroll = total_height.saturating_sub(visible);
        if self.log_follow {
            self.log_scroll = max_scroll;
        } else {
            self.log_scroll = self.log_scroll.min(max_scroll);
        }
        let start = self.log_scroll;
        let end = (start + visible).min(total_height.max(1));

        let range = if total == 0 {
            "no output".to_string()
        } else {
            format!("lines {}-{} / {total_height}", start + 1, end)
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

        if total == 0 {
            frame.render_widget(Clear, viewport);
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

        // Render only the wrapped rows inside the viewport.
        let mut lines: Vec<Line> = Vec::new();
        let mut cursor = 0usize;
        for entry in entries {
            let (time, level, thread, prefix_w, height) = entry_layout(entry, wrap_w);
            if cursor + height > start {
                let frags = display_chunks(&entry.message, wrap_w.saturating_sub(prefix_w).max(1));
                let color = self.theme.log_level_color(entry.level);
                for (fi, frag) in frags.iter().enumerate() {
                    let line_no = cursor + fi;
                    if line_no >= end {
                        break;
                    }
                    if line_no >= start {
                        let line = if fi == 0 {
                            Line::from(vec![
                                Span::styled(format!("{time:>8} "), self.theme.card_comment()),
                                Span::styled(
                                    format!("{:>5} ", level),
                                    Style::default().fg(color).bg(self.theme.panel),
                                ),
                                Span::styled(format!("[{thread}] "), self.theme.card_dim()),
                                Span::styled(frag.clone(), self.theme.card()),
                            ])
                        } else {
                            let indent: String = (0..prefix_w).map(|_| ' ').collect::<String>();
                            Line::from(vec![
                                Span::styled(indent, self.theme.card_dim()),
                                Span::styled(frag.clone(), self.theme.card()),
                            ])
                        };
                        lines.push(line);
                    }
                }
            }
            cursor += height;
            if cursor >= end {
                break;
            }
        }
        // Reset the viewport first: Paragraph only overwrites the cells its
        // text touches, so short lines would otherwise leave leftovers from
        // longer lines (or previous pages) behind.
        frame.render_widget(Clear, viewport);
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

/// Display columns occupied by a single character. Tabs are normalised to one
/// space so they never shift the cursor unpredictably.
fn char_cols(c: char) -> usize {
    if c == '\t' {
        1
    } else {
        c.width().unwrap_or(1)
    }
}

/// Truncate `text` to at most `max` display columns.
fn truncate_cols(text: &str, max: usize) -> String {
    let mut out = String::new();
    let mut cols = 0;
    for c in text.chars() {
        let w = char_cols(c);
        if cols + w > max {
            break;
        }
        out.push(c);
        cols += w;
    }
    out
}

/// Layout of a log entry: the `time level [thread] ` prefix (with the thread
/// clamped so it never crowds out the message) and the number of wrapped rows
/// the entry occupies.
///
/// Returns `(time, level, thread, prefix_cols, wrapped_rows)`.
fn entry_layout(entry: &LogEntry, wrap_w: usize) -> (String, &'static str, String, usize, usize) {
    let time = truncate_cols(entry.timestamp.clone().unwrap_or_default().as_str(), 8);
    let level = entry.level.label();
    let mut thread = entry.thread.clone().unwrap_or_default();
    // 8 (time) + 1 + 5 (level) + 1 + `[` + `] ` = 18 columns of fixed prefix.
    let max_thread = wrap_w.saturating_sub(18).max(1);
    if thread.as_str().width() > max_thread {
        thread = truncate_cols(thread.as_str(), max_thread.saturating_sub(1).max(1));
        thread.push('…');
    }
    let prefix = format!("{time:>8} {level:>5} [{thread}] ");
    let prefix_w = prefix.as_str().width();
    let width = wrap_w.saturating_sub(prefix_w).max(1);
    (time, level, thread, prefix_w, chunk_count(&entry.message, width))
}

/// Number of display rows `text` occupies when wrapped at `width` columns.
fn chunk_count(text: &str, width: usize) -> usize {
    if text.is_empty() {
        return 1;
    }
    let width = width.max(1);
    let mut count = 1;
    let mut cols = 0;
    for c in text.chars() {
        let w = char_cols(c);
        if cols > 0 && cols + w > width {
            count += 1;
            cols = 0;
        }
        cols += w;
    }
    count
}

/// Split `text` into fragments no wider than `width` display columns, counting
/// wide characters at their real width and replacing tabs with spaces, so the
/// fragments always match what the terminal renders.
fn display_chunks(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    let mut cols = 0;
    let mut frag = String::new();
    for c in text.chars() {
        let w = char_cols(c);
        if cols > 0 && cols + w > width {
            out.push(frag);
            frag = String::new();
            cols = 0;
        }
        frag.push(if c == '\t' { ' ' } else { c });
        cols += w;
    }
    if !frag.is_empty() {
        out.push(frag);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}
