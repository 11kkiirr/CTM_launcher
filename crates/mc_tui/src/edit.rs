//! Inline text editing: cursor, selection, and familiar key bindings for the
//! single-line input fields (search bars, settings values).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Style};
use ratatui::text::Span;

/// Outcome of feeding a key press into [`EditState::handle_key`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum EditSignal {
    /// Not an editing key — let the caller dispatch it further.
    #[default]
    Unhandled,
    /// Key consumed (possibly a no-op for this field).
    Handled,
    /// CTRL+C with a non-empty selection: copy text to the clipboard.
    Copy(String),
    /// CTRL+X with a non-empty selection: copy and remove the selection.
    Cut(String),
}

/// Cursor + selection state for one logical line edit. Positions are
/// `char` indices, not bytes, so Unicode text stays consistent.
#[derive(Debug, Clone, Default)]
pub(crate) struct EditState {
    pub cursor: usize,
    pub anchor: Option<usize>,
    drag_anchor: Option<usize>,
}

fn char_len(text: &str) -> usize {
    text.chars().count()
}

fn char_to_byte(text: &str, ci: usize) -> usize {
    text.char_indices().nth(ci).map(|(b, _)| b).unwrap_or(text.len())
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.')
}

fn remove_range(text: &mut String, start: usize, end: usize) {
    let b0 = char_to_byte(text, start);
    let b1 = char_to_byte(text, end);
    if b0 <= b1 && b1 <= text.len() {
        text.replace_range(b0..b1, "");
    }
}

fn word_left(text: &str, from: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut i = from.min(chars.len());
    while i > 0 && !is_word(chars[i - 1]) {
        i -= 1;
    }
    while i > 0 && is_word(chars[i - 1]) {
        i -= 1;
    }
    i
}

fn word_right(text: &str, from: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut i = from.min(n);
    while i < n && !is_word(chars[i]) {
        i += 1;
    }
    while i < n && is_word(chars[i]) {
        i += 1;
    }
    i
}

impl EditState {
    pub fn reset_with(&mut self, text: &str) {
        self.cursor = char_len(text);
        self.anchor = None;
        self.drag_anchor = None;
    }

    pub fn set_cursor(&mut self, text: &str, pos: usize) {
        self.cursor = pos.min(char_len(text));
        self.anchor = None;
        self.drag_anchor = None;
    }

    /// Begin a mouse selection: place the cursor and remember the origin.
    pub fn start_drag(&mut self, text: &str, pos: usize) {
        self.set_cursor(text, pos);
        self.drag_anchor = Some(self.cursor);
    }

    /// Extend the selection from the drag origin to `pos`.
    pub fn drag_to(&mut self, text: &str, pos: usize) {
        let len = char_len(text);
        self.cursor = pos.min(len);
        self.anchor = self.drag_anchor;
    }

    pub fn end_drag(&mut self) {
        self.drag_anchor = None;
    }

    pub fn selection(&self, text: &str) -> Option<(usize, usize)> {
        let len = char_len(text);
        let a = self.anchor?.min(len);
        let c = self.cursor.min(len);
        let (lo, hi) = if a <= c { (a, c) } else { (c, a) };
        if lo == hi {
            None
        } else {
            Some((lo, hi))
        }
    }

    pub fn selected_text(&self, text: &str) -> String {
        match self.selection(text) {
            Some((a, b)) => text.chars().skip(a).take(b - a).collect(),
            None => String::new(),
        }
    }

    fn delete_selection(&mut self, text: &mut String) -> bool {
        if let Some((a, b)) = self.selection(text) {
            remove_range(text, a, b);
            self.cursor = a;
            self.anchor = None;
            true
        } else {
            false
        }
    }

    /// Insert `s` at the cursor, replacing any selection. Content after the
    /// first newline is dropped (single-line fields only).
    pub fn insert_str(&mut self, text: &mut String, s: &str) {
        let s = match s.find(['\n', '\r']) {
            Some(i) => &s[..i],
            None => s,
        };
        self.delete_selection(text);
        if s.is_empty() {
            return;
        }
        let byte = char_to_byte(text, self.cursor);
        text.insert_str(byte, s);
        self.cursor += s.chars().count();
        self.anchor = None;
    }

    fn select_all(&mut self, text: &str) {
        self.anchor = Some(0);
        self.cursor = char_len(text);
    }

    fn move_left(&mut self, text: &str, extend: bool, ctrl: bool) {
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor);
            }
            self.cursor = if ctrl {
                word_left(text, self.cursor)
            } else {
                self.cursor.saturating_sub(1)
            };
        } else if let Some((lo, _hi)) = self.selection(text) {
            self.cursor = lo;
            self.anchor = None;
        } else {
            self.cursor = if ctrl {
                word_left(text, self.cursor)
            } else {
                self.cursor.saturating_sub(1)
            };
        }
    }

    fn move_right(&mut self, text: &str, extend: bool, ctrl: bool) {
        let len = char_len(text);
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor);
            }
            self.cursor = if ctrl {
                word_right(text, self.cursor)
            } else {
                (self.cursor + 1).min(len)
            };
        } else if let Some((_lo, hi)) = self.selection(text) {
            self.cursor = hi;
            self.anchor = None;
        } else {
            self.cursor = if ctrl {
                word_right(text, self.cursor)
            } else {
                (self.cursor + 1).min(len)
            };
        }
    }

    fn move_to(&mut self, text: &str, pos: usize, extend: bool) {
        let len = char_len(text);
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor);
            }
            self.cursor = pos.min(len);
        } else {
            self.cursor = pos.min(len);
            self.anchor = None;
        }
    }

    pub fn handle_key(&mut self, text: &mut String, key: KeyEvent) -> EditSignal {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        self.cursor = self.cursor.min(char_len(text));

        match key.code {
            KeyCode::Char('a') if ctrl => {
                self.select_all(text);
                EditSignal::Handled
            }
            KeyCode::Char('c') if ctrl => match self.selection(text) {
                Some(_) => EditSignal::Copy(self.selected_text(text)),
                None => EditSignal::Handled,
            },
            KeyCode::Char('x') if ctrl => match self.selection(text) {
                Some(_) => {
                    let s = self.selected_text(text);
                    self.delete_selection(text);
                    EditSignal::Cut(s)
                }
                None => EditSignal::Handled,
            },
            // Paste arrives as a bracketed-paste terminal event, not a key.
            KeyCode::Char('v') if ctrl => EditSignal::Handled,
            KeyCode::Char('u') if ctrl => {
                let c = self.cursor;
                if c > 0 {
                    remove_range(text, 0, c);
                    self.cursor = 0;
                }
                self.anchor = None;
                EditSignal::Handled
            }
            KeyCode::Char('k') if ctrl => {
                let c = self.cursor;
                let len = char_len(text);
                if c < len {
                    remove_range(text, c, len);
                }
                self.anchor = None;
                EditSignal::Handled
            }
            KeyCode::Char('w') if ctrl | shift => {
                let c = self.cursor;
                let start = word_left(text, c);
                if start < c {
                    remove_range(text, start, c);
                    self.cursor = start;
                }
                self.anchor = None;
                EditSignal::Handled
            }
            KeyCode::Left => {
                self.move_left(text, shift, ctrl);
                EditSignal::Handled
            }
            KeyCode::Right => {
                self.move_right(text, shift, ctrl);
                EditSignal::Handled
            }
            KeyCode::Home => {
                self.move_to(text, 0, shift);
                EditSignal::Handled
            }
            KeyCode::End => {
                self.move_to(text, char_len(text), shift);
                EditSignal::Handled
            }
            KeyCode::Backspace if ctrl => {
                let c = self.cursor;
                let start = word_left(text, c);
                if start < c {
                    remove_range(text, start, c);
                    self.cursor = start;
                }
                self.anchor = None;
                EditSignal::Handled
            }
            KeyCode::Backspace => {
                if self.delete_selection(text) {
                } else if self.cursor > 0 {
                    let c = self.cursor;
                    remove_range(text, c - 1, c);
                    self.cursor -= 1;
                }
                EditSignal::Handled
            }
            KeyCode::Delete if ctrl => {
                let c = self.cursor;
                let end = word_right(text, c);
                if end > c {
                    remove_range(text, c, end);
                }
                self.anchor = None;
                EditSignal::Handled
            }
            KeyCode::Delete => {
                if self.delete_selection(text) {
                } else {
                    let c = self.cursor;
                    if c < char_len(text) {
                        remove_range(text, c, c + 1);
                    }
                }
                EditSignal::Handled
            }
            // Do not scroll lists while an input field has focus.
            KeyCode::Up | KeyCode::Down => EditSignal::Handled,
            KeyCode::Char(_) if ctrl || alt => EditSignal::Handled,
            KeyCode::Char(c) => {
                self.insert_str(text, &c.to_string());
                EditSignal::Handled
            }
            _ => EditSignal::Unhandled,
        }
    }

    /// Build the spans for the edited line: the text split around the
    /// selection, with the caret rendered as `█` at the cursor position.
    /// When not focused, returns the plain text as a single span.
    pub fn edit_spans(
        &self,
        text: &str,
        focused: bool,
        base: Style,
        cursor_style: Style,
        sel_bg: Color,
    ) -> Vec<Span<'static>> {
        if !focused {
            return if text.is_empty() {
                vec![]
            } else {
                vec![Span::styled(text.to_string(), base)]
            };
        }
        let len = char_len(text);
        let c = self.cursor.min(len);
        let (a, b) = self.selection(text).unwrap_or((c, c));
        let pieces = [(0usize, a, false), (a, b, true), (b, len, false)];
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut placed = false;
        for (s, e, is_sel) in pieces {
            if !placed && s == c {
                spans.push(Span::styled("█", cursor_style));
                placed = true;
            }
            if s < e {
                let chunk: String = text.chars().skip(s).take(e - s).collect();
                let style = if is_sel { base.bg(sel_bg) } else { base };
                spans.push(Span::styled(chunk, style));
            }
            if !placed && e == c {
                spans.push(Span::styled("█", cursor_style));
                placed = true;
            }
        }
        if !placed {
            spans.push(Span::styled("█", cursor_style));
        }
        spans
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    fn shift(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::SHIFT)
    }

    fn edit(s: &str, cursor: usize) -> (EditState, String) {
        (EditState { cursor, anchor: None, drag_anchor: None }, s.to_string())
    }

    #[test]
    fn typing_inserts_at_cursor() {
        let (mut e, mut t) = edit("abc", 1);
        e.handle_key(&mut t, key(KeyCode::Char('x')));
        assert_eq!(t, "axbc");
        assert_eq!(e.cursor, 2);
    }

    #[test]
    fn typing_replaces_selection() {
        let (mut e, mut t) = edit("hello", 0);
        e.anchor = Some(4);
        e.handle_key(&mut t, key(KeyCode::Char('y')));
        assert_eq!(t, "yo");
        assert_eq!(e.cursor, 1);
        assert!(e.anchor.is_none());
    }

    #[test]
    fn backspace_removes_left_char_and_selection() {
        let (mut e, mut t) = edit("abc", 3);
        e.handle_key(&mut t, key(KeyCode::Backspace));
        assert_eq!(t, "ab");
        assert_eq!(e.cursor, 2);

        let (mut e, mut t) = edit("hello", 4);
        e.anchor = Some(1);
        e.handle_key(&mut t, key(KeyCode::Backspace));
        assert_eq!(t, "ho");
        assert_eq!(e.cursor, 1);
    }

    #[test]
    fn arrows_home_end_and_shift_selection() {
        let (mut e, mut t) = edit("abcde", 2);
        e.handle_key(&mut t, key(KeyCode::Left));
        assert_eq!(e.cursor, 1);
        e.handle_key(&mut t, key(KeyCode::Right));
        assert_eq!(e.cursor, 2);
        e.handle_key(&mut t, key(KeyCode::End));
        assert_eq!(e.cursor, 5);
        e.handle_key(&mut t, key(KeyCode::Home));
        assert_eq!(e.cursor, 0);

        e.handle_key(&mut t, shift(KeyCode::Right));
        e.handle_key(&mut t, shift(KeyCode::Right));
        assert_eq!(e.selection(&t), Some((0, 2)));
        e.handle_key(&mut t, key(KeyCode::Right));
        assert_eq!(e.cursor, 2);
        assert!(e.anchor.is_none());
    }

    #[test]
    fn ctrl_select_all_copy_cut() {
        let (mut e, mut t) = edit("world", 2);
        let sig = e.handle_key(&mut t, ctrl(KeyCode::Char('a')));
        assert_eq!(sig, EditSignal::Handled);
        assert_eq!(e.selection(&t), Some((0, 5)));

        let sig = e.handle_key(&mut t, ctrl(KeyCode::Char('c')));
        assert_eq!(sig, EditSignal::Copy("world".into()));

        let sig = e.handle_key(&mut t, ctrl(KeyCode::Char('x')));
        assert_eq!(sig, EditSignal::Cut("world".into()));
        assert_eq!(t, "");
        assert_eq!(e.cursor, 0);
    }

    #[test]
    fn ctrl_w_deletes_word_left() {
        let (mut e, mut t) = edit("sodium fabric", 13);
        e.handle_key(&mut t, ctrl(KeyCode::Char('w')));
        assert_eq!(t, "sodium ");
        e.handle_key(&mut t, ctrl(KeyCode::Backspace));
        assert_eq!(t, "");
        assert_eq!(e.cursor, 0);
    }

    #[test]
    fn ctrl_u_kills_to_start() {
        let (mut e, mut t) = edit("abcdef", 3);
        e.handle_key(&mut t, ctrl(KeyCode::Char('u')));
        assert_eq!(t, "def");
        assert_eq!(e.cursor, 0);
    }

    #[test]
    fn word_motion_with_ctrl() {
        let (mut e, mut t) = edit("foo bar baz", 11);
        e.handle_key(&mut t, ctrl(KeyCode::Left));
        assert_eq!(e.cursor, 8);
        e.handle_key(&mut t, ctrl(KeyCode::Left));
        assert_eq!(e.cursor, 4);
        e.handle_key(&mut t, ctrl(KeyCode::Right));
        assert_eq!(e.cursor, 7);
    }

    #[test]
    fn drag_selects_range() {
        let (mut e, mut t) = edit("abcd", 0);
        e.start_drag(&t, 1);
        e.drag_to(&t, 3);
        assert_eq!(e.selection(&t), Some((1, 3)));
        assert_eq!(e.selected_text(&t), "bc");
        e.end_drag();
        e.drag_to(&t, 0);
        assert_eq!(e.selection(&t), None);
    }

    #[test]
    fn insert_truncates_at_newline() {
        let (mut e, mut t) = edit("", 0);
        e.insert_str(&mut t, "first\nsecond");
        assert_eq!(t, "first");
        assert_eq!(e.cursor, 5);
    }

    #[test]
    fn control_chars_are_consumed_not_inserted() {
        let (mut e, mut t) = edit("", 0);
        let sig = e.handle_key(&mut t, ctrl(KeyCode::Char('s')));
        assert_eq!(sig, EditSignal::Handled);
        assert_eq!(t, "");
    }

    #[test]
    fn unicode_positions_are_char_based() {
        let (mut e, mut t) = edit("привет", 6);
        e.handle_key(&mut t, key(KeyCode::Backspace));
        assert_eq!(t, "приве");
        assert_eq!(e.cursor, 5);
        e.handle_key(&mut t, key(KeyCode::Char('т')));
        assert_eq!(t, "привет");
    }

    #[test]
    fn edit_spans_render_caret_and_selection() {
        let e = EditState { cursor: 2, anchor: Some(1), drag_anchor: None };
        let base = Style::default().fg(Color::Green);
        let cur = Style::default().fg(Color::White);
        let spans = e.edit_spans("abcde", true, base, cur, Color::DarkGray);
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "ab█cde");

        let (mut e2, mut t) = edit("abcde", 0);
        e2.handle_key(&mut t, shift(KeyCode::Right));
        e2.handle_key(&mut t, shift(KeyCode::Right));
        let spans = e2.edit_spans(&t, true, base, cur, Color::DarkGray);
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content.as_ref(), "ab");
        assert_eq!(spans[0].style.bg, Some(Color::DarkGray));
        assert_eq!(spans[1].content.as_ref(), "█");
    }
}

