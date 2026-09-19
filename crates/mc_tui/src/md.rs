//! A small CommonMark renderer for project descriptions.
//!
//! It renders a practical subset (headings, paragraphs, lists, blockquotes,
//! fenced/indented code, tables, inline code/bold/italic/strikethrough/links)
//! into wrapped, styled [`Line`]s that fit a fixed terminal width.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

use crate::theme::Theme;

/// A run of text with a single style.
#[derive(Debug, Clone)]
struct Frag {
    text: String,
    style: Style,
}

/// Render a markdown document into wrapped, styled lines.
pub fn render_md(text: &str, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let clean = strip_html(text);
    let lines: Vec<&str> = clean.lines().collect();
    let base = theme.card();
    let mut out: Vec<Line> = Vec::new();
    let mut i = 0usize;
    let mut prev_blank = false;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        if trimmed.is_empty() {
            prev_blank = true;
            i += 1;
            continue;
        }
        if is_hr(&trimmed) {
            if prev_blank {
                out.push(Line::default());
            }
            out.push(Line::from(Span::styled(
                (0..width).map(|_| '─').collect::<String>(),
                Style::default().fg(theme.green_dim),
            )));
            prev_blank = true;
            i += 1;
            continue;
        }
        if let Some(fence) = fence_of(&trimmed) {
            if prev_blank {
                out.push(Line::default());
            }
            let mut code: Vec<String> = Vec::new();
            let mut j = i + 1;
            while j < lines.len() {
                let t = lines[j].trim();
                if !t.is_empty() && fence_of(&t).is_some() && t.ends_with(fence.as_str()) {
                    break;
                }
                code.push(lines[j].to_string());
                j += 1;
            }
            out.extend(code_block(&code, width, theme));
            prev_blank = true;
            i = (j + 1).min(lines.len());
            continue;
        }
        if let Some(level) = heading_level(&trimmed) {
            if prev_blank {
                out.push(Line::default());
            }
            let text = trimmed.chars().skip(level).collect::<String>().trim_start().to_string();
            let heading_style = theme.header();
            out.extend(wrap_inline(parse_inline(&text, heading_style, theme), width, heading_style));
            prev_blank = true;
            i += 1;
            continue;
        }
        if trimmed.starts_with(">") {
            let mut quote: Vec<String> = Vec::new();
            let mut j = i;
            while j < lines.len() && lines[j].trim_start().starts_with(">") {
                let mut body = lines[j]
                    .trim_start()
                    .chars()
                    .skip(1)
                    .collect::<String>()
                    .trim_start()
                    .to_string();
                while body.starts_with(">") {
                    body = body
                        .chars()
                        .skip(1)
                        .collect::<String>()
                        .trim_start()
                        .to_string();
                }
                quote.push(body);
                j += 1;
            }
            if prev_blank {
                out.push(Line::default());
            }
            out.extend(render_blockquote(&quote, width, theme));
            prev_blank = true;
            i = j;
            continue;
        }
        if let Some((marker, indent)) = list_marker(&trimmed) {
            let mut items: Vec<String> = Vec::new();
            let mut j = i;
            while j < lines.len() {
                let t = lines[j].trim();
                if t.is_empty() {
                    break;
                }
                if let Some((m, _)) = list_marker(&t) {
                    if m != marker {
                        break;
                    }
                } else {
                    break;
                }
                items.push(t.chars().skip(indent).collect::<String>().to_string());
                j += 1;
            }
            if prev_blank {
                out.push(Line::default());
            }
            out.extend(render_list(&items, marker, width, theme));
            prev_blank = true;
            i = j;
            continue;
        }
        if is_table_separator(&trimmed) {
            i += 1;
            continue;
        }
        if trimmed.contains('|') && i + 1 < lines.len() && is_table_separator(&lines[i + 1].trim()) {
            let mut rows: Vec<String> = vec![trimmed.to_string()];
            let mut j = i + 2;
            while j < lines.len() {
                let t = lines[j].trim();
                if t.is_empty() || !t.contains('|') {
                    break;
                }
                rows.push(t.to_string());
                j += 1;
            }
            if prev_blank {
                out.push(Line::default());
            }
            out.extend(render_table(&rows, width, theme));
            prev_blank = true;
            i = j;
            continue;
        }
        if lines[i].starts_with("    ") || lines[i].starts_with("\t") {
            let mut code: Vec<String> = Vec::new();
            let mut j = i;
            while j < lines.len() {
                let l = lines[j];
                if l.trim().is_empty() {
                    code.push(String::new());
                } else if l.starts_with("    ") || l.starts_with("\t") {
                    code.push(l.chars().skip(4).collect::<String>().to_string());
                } else {
                    break;
                }
                j += 1;
            }
            if prev_blank {
                out.push(Line::default());
            }
            out.extend(code_block(&code, width, theme));
            prev_blank = true;
            i = j;
            continue;
        }
        // Paragraph: collect consecutive plain lines.
        let mut para: Vec<String> = Vec::new();
        let mut j = i;
        while j < lines.len() {
            let t = lines[j].trim();
            if t.is_empty()
                || is_hr(&t)
                || fence_of(&t).is_some()
                || heading_level(&t).is_some()
                || t.starts_with(">")
                || list_marker(&t).is_some()
                || t.contains('|')
            {
                break;
            }
            para.push(t.to_string());
            j += 1;
        }
        if prev_blank {
            out.push(Line::default());
        }
        let joined = para.join(" ");
        out.extend(wrap_inline(parse_inline(&joined, base, theme), width, base));
        prev_blank = false;
        i = j;
    }

    while !out.is_empty() && line_blank(out.last().unwrap()) {
        out.pop();
    }
    out
}

// ---------------------------------------------------------------------------
// Blocks
// ---------------------------------------------------------------------------

fn code_block(lines: &[String], width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let style = Style::default().fg(theme.muted).bg(theme.panel_alt);
    let mut out = Vec::new();
    for line in lines {
        let text = line.trim_end().to_string();
        let shown = truncate_cols(&text, width);
        out.push(Line::from(Span::styled(shown, style)));
    }
    if out.is_empty() {
        out.push(Line::from(Span::styled(String::new(), style)));
    }
    out
}

fn render_blockquote(lines: &[String], width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let style = Style::default().fg(theme.comment);
    let inner_w = width.saturating_sub(2).max(1);
    let mut out = Vec::new();
    for line in lines {
        let wrapped = wrap_inline(parse_inline(line, style, theme), inner_w, style);
        for (idx, wrapped_line) in wrapped.iter().enumerate() {
            let prefix = if idx == 0 { "│ " } else { "  " };
            let mut spans = vec![Span::styled(prefix.to_string(), style)];
            for s in wrapped_line.spans.iter() {
                spans.push(Span::styled(s.content.to_string(), s.style));
            }
            out.push(Line::from(spans));
        }
    }
    out
}

fn render_list(items: &[String], marker: &str, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let ordered = marker != "-" && marker != "*" && marker != "+";
    let indent = 3usize;
    let inner_w = width.saturating_sub(indent).max(1);
    let mut out = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        let marker_text = if ordered {
            let mut m = format!("{}", idx + 1);
            m.push_str(". ");
            m
        } else {
            "• ".to_string()
        };
        let wrapped = wrap_inline(parse_inline(item, theme.card(), theme), inner_w, theme.card());
        for (wi, wrapped_line) in wrapped.iter().enumerate() {
            let prefix = if wi == 0 {
                marker_text.clone()
            } else {
                (0..indent).map(|_| ' ').collect::<String>()
            };
            let marker_style = if wi == 0 { theme.accent() } else { theme.card_dim() };
            let mut spans = vec![Span::styled(prefix, marker_style)];
            for s in wrapped_line.spans.iter() {
                spans.push(Span::styled(s.content.to_string(), s.style));
            }
            out.push(Line::from(spans));
        }
    }
    out
}

fn render_table(rows: &[String], width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let mut cells: Vec<Vec<String>> = Vec::new();
    let mut max_cols = 0usize;
    for row in rows {
        let parts: Vec<String> = row
            .split('|')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        max_cols = max_cols.max(parts.len());
        cells.push(parts);
    }
    if cells.is_empty() {
        return Vec::new();
    }
    let cols = max_cols.max(1);
    let gutter = 2usize;
    let cell_w = (width.saturating_sub(cols.saturating_sub(1) * gutter) / cols).max(3);
    let mut sep_parts: Vec<String> = Vec::new();
    for _ in 0..cols {
        sep_parts.push((0..cell_w).map(|_| '─').collect::<String>());
    }
    let sep = sep_parts.join("  ");
    let mut out = Vec::new();
    for (ri, row) in cells.iter().enumerate() {
        if ri == 1 {
            out.push(Line::from(Span::styled(sep.clone(), Style::default().fg(theme.green_dim))));
        }
        let mut line_spans: Vec<Span> = Vec::new();
        for ci in 0..cols {
            let text = row.get(ci).cloned().unwrap_or_default();
            let shown = truncate_cols(&text, cell_w);
            let cell_style = if ri == 0 { theme.header() } else { theme.card() };
            line_spans.push(Span::styled(shown, cell_style));
            if ci + 1 < cols {
                line_spans.push(Span::styled(
                    (0..gutter).map(|_| ' ').collect::<String>(),
                    theme.card_dim(),
                ));
            }
        }
        out.push(Line::from(line_spans));
    }
    out
}

// ---------------------------------------------------------------------------
// Inline
// ---------------------------------------------------------------------------

/// Parse inline markdown (bold, italic, code, strike, links, images) into
/// styled fragments with consecutive same-style runs merged.
fn parse_inline(text: &str, base: Style, theme: &Theme) -> Vec<Frag> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<Frag> = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            push_frag(&mut out, format!("{}", chars[i + 1]), base);
            i += 2;
            continue;
        }
        if c == '!' && i + 1 < chars.len() && chars[i + 1] == '[' {
            if let Some((alt, rest)) = link_target(&chars, i + 1) {
                push_frag(&mut out, format!("▣ {alt}"), Style::default().fg(theme.green_dim));
                i = rest;
                continue;
            }
        }
        if c == '[' {
            if let Some((label, rest)) = link_target(&chars, i) {
                let inner = parse_inline(&label, base, theme);
                for frag in inner {
                    push_frag(
                        &mut out,
                        frag.text.clone(),
                        frag.style.clone().add_modifier(Modifier::UNDERLINED),
                    );
                }
                i = rest;
                continue;
            }
        }
        if c == '`' {
            if let Some((code, rest)) = code_span(&chars, i) {
                push_frag(&mut out, code, Style::default().fg(theme.info));
                i = rest;
                continue;
            }
        }
        if matches!(c, '*' | '_' | '~') {
            let run = count_run(&chars, i, c);
            if let Some(closer) = find_closer(&chars, i + run, c, run) {
                let inner_text = chars
                    .iter()
                    .skip(i + run)
                    .take(closer - i - run)
                    .map(|ch| *ch)
                    .collect::<String>();
                let inner_style = if c == '~' {
                    base.clone().add_modifier(Modifier::CROSSED_OUT)
                } else if run >= 2 {
                    base.clone().add_modifier(Modifier::BOLD)
                } else {
                    base.clone().add_modifier(Modifier::ITALIC)
                };
                let inner = parse_inline(&inner_text, inner_style, theme);
                for frag in inner {
                    push_frag(&mut out, frag.text.clone(), frag.style.clone());
                }
                i = closer + run;
                continue;
            }
        }
        push_frag(&mut out, format!("{}", c), base);
        i += 1;
    }
    out
}

/// Push a fragment, merging with the previous when the style matches.
fn push_frag(out: &mut Vec<Frag>, text: String, style: Style) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = out.last_mut() {
        if last.style == style {
            last.text.push_str(text.as_str());
            return;
        }
    }
    out.push(Frag { text, style });
}

/// For a `[` at `i`, find the closing `](url)` and return `(label, next_index)`.
fn link_target(chars: &[char], i: usize) -> Option<(String, usize)> {
    let mut depth = 0;
    let mut j = i;
    while j < chars.len() {
        let c = chars[j];
        if c == '[' {
            depth += 1;
        } else if c == ']' {
            depth -= 1;
            if depth == 0 {
                if j + 1 < chars.len() && chars[j + 1] == '(' {
                    let mut k = j + 2;
                    while k < chars.len() && chars[k] != ')' {
                        k += 1;
                    }
                    if k < chars.len() {
                        let label = chars
                            .iter()
                            .skip(i + 1)
                            .take(j - i - 1)
                            .map(|ch| *ch)
                            .collect::<String>();
                        return Some((label, k + 1));
                    }
                }
                return None;
            }
        }
        j += 1;
    }
    None
}

/// For a backtick run starting at `i`, find the matching closing run and
/// return the raw code text plus the index just past the closer.
fn code_span(chars: &[char], i: usize) -> Option<(String, usize)> {
    let run = count_run(chars, i, '`');
    let mut j = i + run;
    while j + run <= chars.len() {
        if chars[j] == '`' {
            let again = count_run(chars, j, '`');
            if again == run {
                let code = chars
                    .iter()
                    .skip(i + run)
                    .take(j - i - run)
                    .map(|ch| *ch)
                    .collect::<String>();
                return Some((code, j + run));
            }
            j += again;
        } else {
            j += 1;
        }
    }
    None
}

fn count_run(chars: &[char], i: usize, c: char) -> usize {
    let mut n = 0;
    while i + n < chars.len() && chars[i + n] == c {
        n += 1;
    }
    n
}

/// Find the index of a run of `run` copies of `c` starting at or after `from`.
fn find_closer(chars: &[char], from: usize, c: char, run: usize) -> Option<usize> {
    let mut j = from;
    while j < chars.len() {
        if chars[j] == '\\' {
            j += 2;
            continue;
        }
        if chars[j] == c {
            let n = count_run(chars, j, c);
            if n == run {
                return Some(j);
            }
            j += n;
        } else {
            j += 1;
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Wrapping
// ---------------------------------------------------------------------------

/// Wrap inline fragments to `width` columns using word wrapping. Returns styled
/// lines (each ≤ `width` display columns).
fn wrap_inline(frags: Vec<Frag>, width: usize, line_style: Style) -> Vec<Line<'static>> {
    if width == 0 {
        return vec![Line::default()];
    }
    let mut stream: Vec<(char, Style)> = Vec::new();
    for frag in frags {
        for c in frag.text.chars() {
            stream.push((c, frag.style.clone()));
        }
    }
    if stream.is_empty() {
        return vec![Line::from(Span::styled(String::new(), line_style))];
    }

    // Split into words.
    let mut words: Vec<Vec<(char, Style)>> = Vec::new();
    let mut word: Vec<(char, Style)> = Vec::new();
    for item in stream {
        if item.0 == ' ' {
            if !word.is_empty() {
                words.push(word);
                word = Vec::new();
            }
        } else {
            word.push(item);
        }
    }
    if !word.is_empty() {
        words.push(word);
    }

    let mut lines: Vec<Vec<(char, Style)>> = Vec::new();
    let mut line: Vec<(char, Style)> = Vec::new();
    let mut cols = 0usize;
    for word in words {
        let w: usize = word.iter().map(|(c, _)| char_cols(*c)).sum();
        if w > width {
            if !line.is_empty() {
                lines.push(line);
                line = Vec::new();
                cols = 0;
            }
            for item in word {
                let cw = char_cols(item.0);
                if cols > 0 && cols + cw > width {
                    lines.push(line);
                    line = Vec::new();
                    cols = 0;
                }
                line.push(item);
                cols += cw;
            }
            if !line.is_empty() {
                lines.push(line);
                line = Vec::new();
                cols = 0;
            }
            continue;
        }
        if !line.is_empty() {
            if cols + 1 + w <= width {
                line.push((' ', line.last().unwrap().1.clone()));
                cols += 1;
            } else {
                lines.push(line);
                line = Vec::new();
                cols = 0;
            }
        }
        for item in word {
            line.push(item);
            cols += char_cols(item.0);
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }

    let mut out: Vec<Line> = Vec::new();
    for chars in lines {
        let mut spans: Vec<Span> = Vec::new();
        let mut style = line_style.clone();
        let mut run = String::new();
        for (c, st) in chars {
            if st != style {
                if !run.is_empty() {
                    spans.push(Span::styled(run, style));
                    run = String::new();
                }
                style = st.clone();
            }
            run.push(c);
        }
        if !run.is_empty() {
            spans.push(Span::styled(run, style));
        }
        out.push(Line::from(spans));
    }
    out
}

// ---------------------------------------------------------------------------
// Block classification helpers
// ---------------------------------------------------------------------------

fn is_hr(trimmed: &str) -> bool {
    let chars: Vec<char> = trimmed.chars().collect();
    chars.len() >= 3
        && chars.iter().all(|c| matches!(*c, '-' | '*' | '_' | ' '))
        && chars.iter().any(|c| *c != ' ')
}

fn fence_of(trimmed: &str) -> Option<String> {
    if trimmed.starts_with("```") {
        Some("```".to_string())
    } else if trimmed.starts_with("~~~") {
        Some("~~~".to_string())
    } else {
        None
    }
}

fn heading_level(trimmed: &str) -> Option<usize> {
    let chars: Vec<char> = trimmed.chars().collect();
    let mut n = 0;
    while n < chars.len() && n < 6 && chars[n] == '#' {
        n += 1;
    }
    if n > 0 && n < chars.len() && chars[n] == ' ' {
        Some(n)
    } else {
        None
    }
}

/// If `trimmed` is a list item, return the marker and the prefix width
/// (`"- "`, `"* "`, `"+ "`, `"1. "`, `"12. "`).
fn list_marker(trimmed: &str) -> Option<(&str, usize)> {
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.is_empty() {
        return None;
    }
    if matches!(chars[0], '-' | '*' | '+') && chars.len() > 1 && chars[1] == ' ' {
        return Some(("-", 2));
    }
    if chars[0] >= '0' && chars[0] <= '9' {
        let mut n = 0;
        while n < chars.len() && chars[n] >= '0' && chars[n] <= '9' {
            n += 1;
        }
        if n > 0 && n + 1 < chars.len() && chars[n] == '.' && chars[n + 1] == ' ' {
            return Some(("1", n + 2));
        }
    }
    None
}

fn is_table_separator(trimmed: &str) -> bool {
    let chars: Vec<char> = trimmed.chars().collect();
    let mut any_dash = false;
    for c in chars {
        match c {
            '-' | ':' => any_dash = true,
            '|' | ' ' => {}
            _ => return false,
        }
    }
    any_dash
}

fn line_blank(line: &Line) -> bool {
    line.spans.is_empty()
        || line.spans.iter().all(|s| s.content.to_string().trim().is_empty())
}

// ---------------------------------------------------------------------------
// Preprocessing
// ---------------------------------------------------------------------------

/// Strip raw HTML, converting `<br>` into newlines so inline HTML in Modrinth
/// descriptions degrades to readable text.
fn strip_html(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '<' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != '>' {
                j += 1;
            }
            if j >= chars.len() {
                break;
            }
            let tag = chars
                .iter()
                .skip(i + 1)
                .take(j - i - 1)
                .map(|c| *c)
                .collect::<String>();
            let lower = tag.to_ascii_lowercase();
            if lower.starts_with("br") || lower.starts_with("/p") || lower.starts_with("/h") {
                out.push('\n');
                out.push('\n');
            }
            i = j + 1;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// Display columns occupied by a character (tabs as one column).
fn char_cols(c: char) -> usize {
    if c == '\t' {
        1
    } else {
        c.width().unwrap_or(1)
    }
}

/// Truncate `text` to `max` display columns.
fn truncate_cols(text: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    fn theme() -> Theme {
        Theme::default()
    }

    fn plain(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|s| s.content.to_string())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .collect()
    }

    #[test]
    fn renders_headings_and_emphasis() {
        let md = "# Title\n\nSome **bold** and *italic* text with `code`.";
        let lines = render_md(md, 40, &theme());
        let text = plain(&lines);
        assert_eq!(text[0], "Title");
        assert!(
            lines[0].spans[0].style.add_modifier == Modifier::BOLD,
            "headings should be bold"
        );
        assert!(text.iter().any(|l| l.contains("bold")));
        assert!(text.iter().any(|l| l.contains("code")));
    }

    #[test]
    fn wraps_long_paragraphs() {
        let md = "word word word word word word word word word word word word word word word";
        let lines = render_md(md, 12, &theme());
        let text = plain(&lines);
        for line in &text {
            assert!(line.chars().count() <= 12, "line too wide: {line:?}");
        }
        assert!(text.len() > 1);
    }

    #[test]
    fn renders_lists_and_code() {
        let md = "- one\n- two\n\n```\ncode line\n```\n";
        let lines = render_md(md, 40, &theme());
        let text = plain(&lines);
        assert!(text.iter().any(|l| l.contains("•")));
        assert!(text.iter().any(|l| l.contains("code line")));
    }

    #[test]
    fn strips_html() {
        let md = "hello<br>world";
        let lines = render_md(md, 40, &theme());
        let text = plain(&lines);
        assert!(text.len() >= 2, "br should create a line break");
        assert!(!text.iter().any(|l| l.contains('<')), "tags must be stripped");
    }

    #[test]
    fn respects_wide_chars() {
        let md = "日志日志日志日志日志日志日志日志日志日志日志日志日志日志";
        let lines = render_md(md, 16, &theme());
        let text = plain(&lines);
        for line in &text {
            let w = line.as_str().width();
            assert!(w <= 16, "line too wide: {line:?} ({w} cols)");
        }
    }
}