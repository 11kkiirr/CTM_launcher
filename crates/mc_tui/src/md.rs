//! A small CommonMark renderer for project descriptions.
//!
//! It renders a practical subset (headings, paragraphs, lists, blockquotes,
//! fenced/indented code, tables, inline code/bold/italic/strikethrough/links)
//! into wrapped, styled [`Line`]s that fit a fixed terminal width.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::theme::Theme;

/// A run of text with a single style.
#[derive(Debug, Clone)]
struct Frag {
    text: String,
    style: Style,
    link: Option<String>,
    code: Option<String>,
}

/// One wrapped character with its style and optional link/code payloads.
type StreamItem = (char, Style, Option<String>, Option<String>);

/// A clickable inline link found while rendering markdown, in output
/// coordinates of [`render_md_full`].
#[derive(Debug, Clone)]
pub struct MdLink {
    /// Index of the output line containing the link text.
    pub line: usize,
    /// First column of the link text (0-based, exclusive end in `end`).
    pub start: usize,
    /// Column just past the last character of the link text.
    pub end: usize,
    /// Link target URL.
    pub url: String,
}

/// An inline code span (`\`code\``) found while rendering markdown.
#[derive(Debug, Clone)]
pub struct MdCode {
    /// Index of the output line containing the span (one entry per wrapped
    /// line when the span crosses a line break).
    pub line: usize,
    /// First column of the span.
    pub start: usize,
    /// Column just past the last character of the span.
    pub end: usize,
    /// Full code text (the whole span, even when wrapped across lines).
    pub text: String,
}

/// A fenced/indented code block region in the rendered output.
#[derive(Debug, Clone)]
pub struct MdCodeBlock {
    /// Index of the first line of the block (its top padding row).
    pub line: usize,
    /// Total height in rows, including the padding rows.
    pub height: usize,
    /// Full block text (lines joined with newlines) — the copy payload.
    pub text: String,
}

/// Link/code range inside a single [`wrap_inline`] result, offset-adjusted by
/// the block renderer that prepends prefixes (quote bars, list markers).
#[derive(Debug, Clone)]
struct WrapMark {
    line: usize,
    start: usize,
    end: usize,
    url: Option<String>,
    code: Option<String>,
}

/// Append wrapped lines and their link/code ranges to the block output,
/// rebasing line indices onto `out`.
fn push_wrapped(
    out: &mut Vec<Line<'static>>,
    marks: &mut Vec<WrapMark>,
    wrapped: Vec<Line<'static>>,
    wm: Vec<WrapMark>,
) {
    let base = out.len();
    for mut m in wm {
        m.line += base;
        marks.push(m);
    }
    out.extend(wrapped);
}

/// Horizontal alignment of a block inside the description body, taken from
/// HTML wrapper tags (`<p align="center">`, `<center>`, `text-align` styles).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MdAlign {
    Left,
    Center,
    Right,
}

/// A standalone image block found while rendering markdown.
#[derive(Debug, Clone)]
pub struct MdImage {
    /// Index of the placeholder line in the rendered output.
    pub line: usize,
    /// Absolute URL of the image.
    pub url: String,
    /// Alignment requested by the surrounding HTML wrapper, if any.
    pub align: MdAlign,
    /// Link target when the image is wrapped in a link
    /// (`[![alt](url)](target)` or `<a href="target"><img/></a>`).
    pub link: Option<String>,
}

/// Render markdown to styled lines, ignoring standalone image blocks.
#[allow(dead_code)]
pub fn render_md(text: &str, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    render_md_ex(text, width, theme).0
}

/// Render markdown, also reporting standalone `![alt](url)` blocks so the
/// caller can replace their placeholder lines with real images.
#[allow(dead_code)]
pub fn render_md_ex(text: &str, width: usize, theme: &Theme) -> (Vec<Line<'static>>, Vec<MdImage>) {
    let (lines, images, _, _, _) = render_md_full(text, width, theme);
    (lines, images)
}

/// Full markdown render result: wrapped lines, image blocks, clickable links,
/// inline code spans and code block regions.
pub type MdFull = (
    Vec<Line<'static>>,
    Vec<MdImage>,
    Vec<MdLink>,
    Vec<MdCode>,
    Vec<MdCodeBlock>,
);

/// Like [`render_md_ex`], also reporting clickable inline links
/// (`[label](url)`, `<a href="…">`), inline code spans and code block
/// regions with their line/column coordinates.
pub fn render_md_full(text: &str, width: usize, theme: &Theme) -> MdFull {
    let clean = strip_html(text);
    let lines: Vec<&str> = clean.iter().map(|l| l.text.as_str()).collect();
    let aligns: Vec<MdAlign> = clean.iter().map(|l| l.align).collect();
    let base = theme.card();
    let mut out: Vec<Line> = Vec::new();
    let mut images: Vec<MdImage> = Vec::new();
    let mut raw_marks: Vec<WrapMark> = Vec::new();
    let mut code_blocks: Vec<MdCodeBlock> = Vec::new();
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
        if is_hr(trimmed) {
            if prev_blank {
                out.push(Line::default());
            }
            out.push(Line::from(Span::styled(
                (0..width).map(|_| '─').collect::<String>(),
                Style::default().fg(theme.panel_alt),
            )));
            prev_blank = true;
            i += 1;
            continue;
        }
        if let Some(fence) = fence_of(trimmed) {
            if prev_blank {
                out.push(Line::default());
            }
            let mut code: Vec<String> = Vec::new();
            let mut j = i + 1;
            while j < lines.len() {
                let t = lines[j].trim();
                if !t.is_empty() && fence_of(t).is_some() && t.ends_with(fence.as_str()) {
                    break;
                }
                code.push(lines[j].to_string());
                j += 1;
            }
            let block_line = out.len();
            out.extend(code_block(&code, width, theme));
            code_blocks.push(MdCodeBlock {
                line: block_line,
                height: out.len() - block_line,
                text: code.join("\n"),
            });
            prev_blank = true;
            i = (j + 1).min(lines.len());
            continue;
        }
        if let Some(level) = heading_level(trimmed) {
            if prev_blank {
                out.push(Line::default());
            }
            let text = trimmed.chars().skip(level).collect::<String>().trim_start().to_string();
            let heading_style = theme.header();
            let (wrapped, wl) = wrap_inline(parse_inline(&text, heading_style, theme), width, heading_style);
            push_wrapped(&mut out, &mut raw_marks, wrapped, wl);
            prev_blank = true;
            i += 1;
            continue;
        }
        if let Some(block_imgs) = block_images(trimmed) {
            if prev_blank {
                out.push(Line::default());
            }
            let align = aligns[i];
            for (alt, url, link) in block_imgs {
                let label = if alt.trim().is_empty() {
                    image_label(&url)
                } else {
                    alt
                };
                images.push(MdImage {
                    line: out.len(),
                    url,
                    align,
                    link,
                });
                out.push(Line::from(Span::styled(
                    format!("▣ {label}"),
                    Style::default().fg(theme.green_dim),
                )));
            }
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
            let (quoted, ql) = render_blockquote(&quote, width, theme);
            push_wrapped(&mut out, &mut raw_marks, quoted, ql);
            prev_blank = true;
            i = j;
            continue;
        }
        if let Some((marker, indent)) = list_marker(trimmed) {
            let mut items: Vec<String> = Vec::new();
            let mut j = i;
            while j < lines.len() {
                let t = lines[j].trim();
                if t.is_empty() {
                    break;
                }
                if let Some((m, _)) = list_marker(t) {
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
            let (listed, ll) = render_list(&items, marker, width, theme);
            push_wrapped(&mut out, &mut raw_marks, listed, ll);
            prev_blank = true;
            i = j;
            continue;
        }
        if is_table_separator(trimmed) {
            i += 1;
            continue;
        }
        if trimmed.contains('|') && i + 1 < lines.len() && is_table_separator(lines[i + 1].trim()) {
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
            let block_line = out.len();
            out.extend(code_block(&code, width, theme));
            code_blocks.push(MdCodeBlock {
                line: block_line,
                height: out.len() - block_line,
                text: code.join("\n"),
            });
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
                || is_hr(t)
                || fence_of(t).is_some()
                || heading_level(t).is_some()
                || t.starts_with(">")
                || list_marker(t).is_some()
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
        let (wrapped, wl) = wrap_inline(parse_inline(&joined, base, theme), width, base);
        push_wrapped(&mut out, &mut raw_marks, wrapped, wl);
        prev_blank = false;
        i = j;
    }

    let code_end = code_blocks
        .last()
        .map(|b| b.line + b.height)
        .unwrap_or(0);
    while out.len() > code_end && line_blank(out.last().unwrap()) {
        out.pop();
    }
    let mut links = Vec::new();
    let mut codes = Vec::new();
    for m in raw_marks {
        let WrapMark {
            line,
            start,
            end,
            url,
            code,
        } = m;
        if let Some(url) = url {
            links.push(MdLink {
                line,
                start,
                end,
                url,
            });
        }
        if let Some(text) = code {
            codes.push(MdCode {
                line,
                start,
                end,
                text,
            });
        }
    }
    (out, images, links, codes, code_blocks)
}

// ---------------------------------------------------------------------------
// Blocks
// ---------------------------------------------------------------------------

fn code_block(lines: &[String], width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let style = Style::default().fg(theme.comment).bg(theme.panel_alt);
    let w = width.max(4);
    let mut out = Vec::new();
    let pad_row = Line::from(Span::styled(" ".repeat(w), style));
    out.push(pad_row.clone());
    for line in lines {
        let text = line.trim_end();
        let mut shown = String::from("  ");
        shown.push_str(&truncate_cols(text, w.saturating_sub(4)));
        while shown.width() < w {
            shown.push(' ');
        }
        out.push(Line::from(Span::styled(shown, style)));
    }
    out.push(pad_row);
    out
}

fn render_blockquote(lines: &[String], width: usize, theme: &Theme) -> (Vec<Line<'static>>, Vec<WrapMark>) {
    let style = Style::default().fg(theme.comment);
    let inner_w = width.saturating_sub(2).max(1);
    let mut out = Vec::new();
    let mut links = Vec::new();
    for line in lines {
        let (wrapped, wl) = wrap_inline(parse_inline(line, style, theme), inner_w, style);
        let base = out.len();
        for mut l in wl {
            l.line += base;
            l.start += 2;
            l.end += 2;
            links.push(l);
        }
        for (idx, wrapped_line) in wrapped.iter().enumerate() {
            let prefix = if idx == 0 { "│ " } else { "  " };
            let mut spans = vec![Span::styled(prefix.to_string(), style)];
            for s in wrapped_line.spans.iter() {
                spans.push(Span::styled(s.content.to_string(), s.style));
            }
            out.push(Line::from(spans));
        }
    }
    (out, links)
}

fn render_list(items: &[String], marker: &str, width: usize, theme: &Theme) -> (Vec<Line<'static>>, Vec<WrapMark>) {
    let ordered = marker != "-" && marker != "*" && marker != "+";
    let indent = 3usize;
    let inner_w = width.saturating_sub(indent).max(1);
    let mut out = Vec::new();
    let mut links = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        let marker_text = if ordered {
            let mut m = format!("{}", idx + 1);
            m.push_str(". ");
            m
        } else {
            "• ".to_string()
        };
        let (wrapped, wl) = wrap_inline(parse_inline(item, theme.card(), theme), inner_w, theme.card());
        let base = out.len();
        for mut l in wl {
            l.line += base;
            let offset = if l.line == base { marker_text.width() } else { indent };
            l.start += offset;
            l.end += offset;
            links.push(l);
        }
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
    (out, links)
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
    let min_w = 3usize;

    let mut col_w = vec![0usize; cols];
    for row in &cells {
        for (ci, cell) in row.iter().enumerate().take(cols) {
            col_w[ci] = col_w[ci].max(cell.width());
        }
    }
    let table_w = |col_w: &[usize]| -> usize {
        col_w.iter().sum::<usize>() + gutter * cols.saturating_sub(1)
    };
    while table_w(&col_w) > width {
        let widest = col_w
            .iter()
            .enumerate()
            .filter(|(_, w)| **w > min_w)
            .max_by_key(|(_, w)| **w)
            .map(|(ci, _)| ci);
        match widest {
            Some(ci) => col_w[ci] -= 1,
            None => break,
        }
    }

    let sep: String = col_w
        .iter()
        .map(|w| (0..*w).map(|_| '─').collect::<String>())
        .collect::<Vec<_>>()
        .join(&" ".repeat(gutter));

    let mut out = Vec::new();
    for (ri, row) in cells.iter().enumerate() {
        if ri == 1 {
            out.push(Line::from(Span::styled(sep.clone(), Style::default().fg(theme.panel_alt))));
        }
        let mut line_spans: Vec<Span> = Vec::new();
        for (ci, w) in col_w.iter().enumerate() {
            let text = row.get(ci).cloned().unwrap_or_default();
            let mut shown = truncate_cols(&text, *w);
            let pad = w.saturating_sub(shown.width());
            shown.push_str(&" ".repeat(pad));
            let cell_style = if ri == 0 { theme.header() } else { theme.card() };
            line_spans.push(Span::styled(shown, cell_style));
            if ci + 1 < cols {
                line_spans.push(Span::styled(
                    " ".repeat(gutter),
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
            push_frag(&mut out, format!("{}", chars[i + 1]), base, None, None);
            i += 2;
            continue;
        }
        if c == '!' && i + 1 < chars.len() && chars[i + 1] == '[' {
            if let Some((alt, _url, rest)) = link_target(&chars, i + 1) {
                push_frag(
                    &mut out,
                    format!("▣ {alt}"),
                    Style::default().fg(theme.green_dim),
                    None,
                    None,
                );
                i = rest;
                continue;
            }
        }
        if c == '[' {
            if let Some((label, url, rest)) = link_target(&chars, i) {
                let url = normalize_img_url(&url);
                let inner = parse_inline(&label, base, theme);
                for frag in inner {
                    push_frag(
                        &mut out,
                        frag.text.clone(),
                        frag.style.add_modifier(Modifier::UNDERLINED),
                        frag.link.clone().or_else(|| Some(url.clone())),
                        frag.code.clone(),
                    );
                }
                i = rest;
                continue;
            }
        }
        if c == '`' {
            if let Some((code, rest)) = code_span(&chars, i) {
                push_frag(
                    &mut out,
                    code.clone(),
                    Style::default().fg(theme.comment).bg(theme.panel_alt),
                    None,
                    Some(code),
                );
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
                    .copied()
                    .collect::<String>();
                let inner_style = if c == '~' {
                    base.add_modifier(Modifier::CROSSED_OUT)
                } else if run >= 2 {
                    base.add_modifier(Modifier::BOLD)
                } else {
                    base.add_modifier(Modifier::ITALIC)
                };
                let inner = parse_inline(&inner_text, inner_style, theme);
                for frag in inner {
                    push_frag(
                        &mut out,
                        frag.text.clone(),
                        frag.style,
                        frag.link.clone(),
                        frag.code.clone(),
                    );
                }
                i = closer + run;
                continue;
            }
        }
        push_frag(&mut out, format!("{}", c), base, None, None);
        i += 1;
    }
    out
}

/// Push a fragment, merging with the previous when style, link and code all
/// match.
fn push_frag(
    out: &mut Vec<Frag>,
    text: String,
    style: Style,
    link: Option<String>,
    code: Option<String>,
) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = out.last_mut() {
        if last.style == style && last.link == link && last.code == code {
            last.text.push_str(text.as_str());
            return;
        }
    }
    out.push(Frag {
        text,
        style,
        link,
        code,
    });
}

/// For a `[` at `i`, find the closing `](url)` and return
/// `(label, url, next_index)`.
fn link_target(chars: &[char], i: usize) -> Option<(String, String, usize)> {
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
                            .copied()
                            .collect::<String>();
                        let url: String = chars[j + 2..k].iter().collect();
                        return Some((label, url, k + 1));
                    }
                }
                return None;
            }
        }
        j += 1;
    }
    None
}

/// Parse `![alt](url)` starting at `start` (index of `!`). Returns
/// `(alt, url, next_index_after_closing_paren)`.
fn parse_image_at(chars: &[char], start: usize) -> Option<(String, String, usize)> {
    if start + 1 >= chars.len() || chars[start] != '!' || chars[start + 1] != '[' {
        return None;
    }
    let mut depth = 0i32;
    let mut j = start + 1;
    while j < chars.len() {
        match chars[j] {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        j += 1;
    }
    if depth != 0 || j + 1 >= chars.len() || chars[j + 1] != '(' {
        return None;
    }
    let alt: String = chars[start + 2..j].iter().collect();
    let mut k = j + 2;
    while k < chars.len() && chars[k] != ')' {
        k += 1;
    }
    if k >= chars.len() {
        return None;
    }
    let url: String = chars[j + 2..k].iter().collect();
    let url = normalize_img_url(&url);
    if url.is_empty() {
        return None;
    }
    Some((alt, url, k + 1))
}

/// Parse a line consisting solely of image tokens — `![alt](url)`, optionally
/// wrapped in a link (`[![alt](url)](target)`), separated by whitespace — so
/// badge rows become image blocks. Returns `(alt, url, link)` triples; `link`
/// is the wrapping link target when present. Returns `None` when any non-image
/// text is present, leaving the line to normal inline rendering.
fn block_images(line: &str) -> Option<Vec<(String, String, Option<String>)>> {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0usize;
    let mut out: Vec<(String, String, Option<String>)> = Vec::new();
    loop {
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }
        let linked = chars[i] == '[';
        if linked {
            i += 1;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
        }
        let (alt, url, next) = parse_image_at(&chars, i)?;
        i = next;
        let mut link = None;
        if linked {
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            if i >= chars.len() || chars[i] != ']' {
                return None;
            }
            i += 1;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            if i < chars.len() && chars[i] == '(' {
                let open = i;
                let mut depth = 0i32;
                let mut close = usize::MAX;
                while i < chars.len() {
                    match chars[i] {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                close = i;
                                i += 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                if depth != 0 || close == usize::MAX {
                    return None;
                }
                let target: String = chars[open + 1..close].iter().collect();
                let target = target.trim().to_string();
                if !target.is_empty() {
                    link = Some(target);
                }
            }
        }
        out.push((alt, url, link));
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Placeholder label for an image with an empty alt: the URL's file name
/// without query string or extension (`…/banner.png?x=1` → `banner`).
fn image_label(url: &str) -> String {
    let path = url.split('?').next().unwrap_or(url);
    let name = path.rsplit('/').next().unwrap_or(path);
    let stem = name.split('.').next().unwrap_or(name);
    if stem.trim().is_empty() {
        "image".to_string()
    } else {
        stem.to_string()
    }
}

/// Normalize an image URL for fetching (`//host/…` → `https://host/…`).
fn normalize_img_url(url: &str) -> String {
    let url = url.trim().replace(['[', ']'], "");
    match url.strip_prefix("//") {
        Some(rest) => format!("https://{rest}"),
        None => url,
    }
}

/// Extract `name="value"` (or `name='value'`) from an HTML tag body,
/// matching the attribute name case-insensitively at a word boundary.
fn html_attr(tag: &str, name: &str) -> Option<String> {
    let t = tag.as_bytes();
    let n = name.as_bytes();
    let mut i = 0usize;
    while i + n.len() <= t.len() {
        let is_match = t[i..i + n.len()].eq_ignore_ascii_case(n)
            && (i == 0 || t[i - 1].is_ascii_whitespace());
        if is_match {
            let mut k = i + n.len();
            while k < t.len() && t[k].is_ascii_whitespace() {
                k += 1;
            }
            if k < t.len() && t[k] == b'=' {
                k += 1;
                while k < t.len() && t[k].is_ascii_whitespace() {
                    k += 1;
                }
                if k >= t.len() {
                    return None;
                }
                let quote = t[k];
                if quote == b'"' || quote == b'\'' {
                    let start = k + 1;
                    let end = t[start..].iter().position(|&c| c == quote)? + start;
                    return Some(tag[start..end].to_string());
                }
                let end = t[k..]
                    .iter()
                    .position(|&c| c.is_ascii_whitespace() || c == b'/')
                    .map(|p| p + k)
                    .unwrap_or(t.len());
                return Some(tag[k..end].to_string());
            }
        }
        i += 1;
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
                    .copied()
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
fn wrap_inline(
    frags: Vec<Frag>,
    width: usize,
    line_style: Style,
) -> (Vec<Line<'static>>, Vec<WrapMark>) {
    if width == 0 {
        return (vec![Line::default()], Vec::new());
    }
    let mut stream: Vec<StreamItem> = Vec::new();
    for frag in frags {
        for c in frag.text.chars() {
            stream.push((c, frag.style, frag.link.clone(), frag.code.clone()));
        }
    }
    if stream.is_empty() {
        return (
            vec![Line::from(Span::styled(String::new(), line_style))],
            Vec::new(),
        );
    }

    // Split into words.
    let mut words: Vec<Vec<StreamItem>> = Vec::new();
    let mut word: Vec<StreamItem> = Vec::new();
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

    let mut lines: Vec<Vec<StreamItem>> = Vec::new();
    let mut line: Vec<StreamItem> = Vec::new();
    let mut cols = 0usize;
    for word in words {
        let w: usize = word.iter().map(|(c, _, _, _)| char_cols(*c)).sum();
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
                let last = line.last().unwrap();
                let last_style = last.1;
                let last_link = last.2.clone();
                let word_link = word.first().unwrap().2.clone();
                let space_link = if last_link == word_link { last_link } else { None };
                let last_code = last.3.clone();
                let word_code = word.first().unwrap().3.clone();
                let space_code = if last_code == word_code { last_code } else { None };
                line.push((' ', last_style, space_link, space_code));
                cols += 1;
            } else {
                lines.push(line);
                line = Vec::new();
                cols = 0;
            }
        }
        for item in word {
            cols += char_cols(item.0);
            line.push(item);
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }

    let mut out: Vec<Line> = Vec::new();
    let mut marks: Vec<WrapMark> = Vec::new();
    for (line_idx, chars) in lines.into_iter().enumerate() {
        let mut spans: Vec<Span> = Vec::new();
        let mut style = line_style;
        let mut link: Option<String> = None;
        let mut code: Option<String> = None;
        let mut run = String::new();
        let mut run_start = 0usize;
        let mut col = 0usize;
        let flush = |run: &mut String,
                     style: Style,
                     link: Option<String>,
                     code: Option<String>,
                     run_start: usize,
                     col: usize,
                     spans: &mut Vec<Span>,
                     marks: &mut Vec<WrapMark>| {
            if run.is_empty() {
                return;
            }
            spans.push(Span::styled(std::mem::take(run), style));
            if link.is_some() || code.is_some() {
                marks.push(WrapMark {
                    line: line_idx,
                    start: run_start,
                    end: col,
                    url: link,
                    code,
                });
            }
        };
        for (c, st, ln, cd) in chars {
            let cw = char_cols(c);
            if !run.is_empty() && (st != style || ln != link || cd != code) {
                flush(
                    &mut run,
                    style,
                    link.clone(),
                    code.clone(),
                    run_start,
                    col,
                    &mut spans,
                    &mut marks,
                );
                run_start = col;
            }
            if run.is_empty() {
                style = st;
                link = ln;
                code = cd;
            }
            run.push(c);
            col += cw;
        }
        flush(
            &mut run,
            style,
            link,
            code,
            run_start,
            col,
            &mut spans,
            &mut marks,
        );
        out.push(Line::from(spans));
    }
    (out, marks)
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

/// One preprocessed source line with the block alignment in effect for it.
struct CleanLine {
    text: String,
    align: MdAlign,
}

/// Strip raw HTML, converting `<br>` into newlines so inline HTML in Modrinth
/// descriptions degrades to readable text. `<img>` tags become markdown images
/// in place. `<a href="…">…</a>` wrappers become markdown links (`[…](target)`)
/// so linked images keep their target; line breaks inside an anchor join into
/// one line instead of splitting the link apart.
///
/// Block-level wrappers carrying `align="…"` / `text-align: …` (and `<center>`)
/// push an alignment that applies to every line until the matching close tag,
/// so image rows can follow Modrinth's per-paragraph alignment.
fn strip_html(text: &str) -> Vec<CleanLine> {
    let chars: Vec<char> = text.chars().collect();
    let mut lines: Vec<CleanLine> = Vec::new();
    let mut cur = String::new();
    let mut stack = vec![MdAlign::Left];
    let mut anchors: Vec<String> = Vec::new();
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
                .copied()
                .collect::<String>();
            let lower = tag.to_ascii_lowercase();
            if lower.starts_with("img") {
                if let Some(src) = html_attr(&tag, "src") {
                    let alt = html_attr(&tag, "alt").unwrap_or_default();
                    let alt = alt.replace(['[', ']'], "");
                    let url = normalize_img_url(&src);
                    if !url.is_empty() {
                        cur.push_str(&format!("![{alt}]({url})"));
                    }
                }
            } else if lower.starts_with("br") {
                if anchors.is_empty() {
                    flush_line(&mut cur, &mut lines, &stack);
                    flush_line(&mut cur, &mut lines, &stack);
                } else {
                    cur.push(' ');
                }
            } else if let Some(name) = closing_tag_name(&lower) {
                if name == "a" {
                    if let Some(href) = anchors.pop() {
                        if !href.is_empty() {
                            cur.push_str(&format!("]({href})"));
                        }
                    }
                }
                if is_block_tag(&name) && stack.len() > 1 {
                    stack.pop();
                }
                if name.starts_with('p') || name.starts_with('h') {
                    flush_line(&mut cur, &mut lines, &stack);
                    flush_line(&mut cur, &mut lines, &stack);
                }
            } else if let Some(name) = opening_tag_name(&lower) {
                if name == "a" {
                    let href = html_attr(&tag, "href")
                        .map(|h| h.trim().to_string())
                        .filter(|h| !h.is_empty());
                    match href {
                        Some(href) => {
                            anchors.push(href);
                            cur.push('[');
                        }
                        None => anchors.push(String::new()),
                    }
                } else if name == "center" {
                    stack.push(MdAlign::Center);
                } else if is_block_tag(&name) {
                    let align = tag_align(&tag).unwrap_or(MdAlign::Left);
                    stack.push(align);
                }
            }
            i = j + 1;
        } else if chars[i] == '\n' {
            if anchors.is_empty() {
                flush_line(&mut cur, &mut lines, &stack);
            } else {
                cur.push(' ');
            }
            i += 1;
        } else {
            cur.push(chars[i]);
            i += 1;
        }
    }
    if !cur.is_empty() {
        flush_line(&mut cur, &mut lines, &stack);
    }
    lines
}

fn flush_line(cur: &mut String, lines: &mut Vec<CleanLine>, stack: &[MdAlign]) {
    let align = stack.last().copied().unwrap_or(MdAlign::Left);
    lines.push(CleanLine {
        text: std::mem::take(cur),
        align,
    });
}

/// Tag name of a `</tag …>` closing tag, lowercased.
fn closing_tag_name(lower: &str) -> Option<String> {
    let rest = lower.strip_prefix('/')?;
    let name = rest.split_whitespace().next().unwrap_or("");
    let name = name.trim_end_matches('/');
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// Tag name of an opening `<tag …>` / `<tag/>` tag, lowercased.
fn opening_tag_name(lower: &str) -> Option<String> {
    if lower.starts_with('/') || lower.starts_with('!') {
        return None;
    }
    let name = lower.split(|c: char| c.is_ascii_whitespace() || c == '/').next()?;
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn is_block_tag(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "center"
            | "blockquote"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "figure"
            | "figcaption"
            | "ul"
            | "ol"
            | "li"
            | "dl"
            | "dt"
            | "dd"
            | "table"
            | "thead"
            | "tbody"
            | "tr"
            | "td"
            | "th"
            | "pre"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
    )
}

/// Alignment declared on a tag via `align="…"` or `style="text-align: …"`.
fn tag_align(tag: &str) -> Option<MdAlign> {
    if let Some(v) = html_attr(tag, "align") {
        if let Some(a) = parse_align(&v) {
            return Some(a);
        }
    }
    let style = html_attr(tag, "style")?;
    let idx = style.to_ascii_lowercase().find("text-align")?;
    let rest = style[idx + "text-align".len()..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let value: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    parse_align(&value)
}

fn parse_align(value: &str) -> Option<MdAlign> {
    match value.trim().to_ascii_lowercase().as_str() {
        "center" => Some(MdAlign::Center),
        "right" | "end" => Some(MdAlign::Right),
        "left" | "start" => Some(MdAlign::Left),
        _ => None,
    }
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

    #[test]
    fn standalone_images_yield_markers() {
        let md = "text\n\n![cool pic](https://cdn.example/x.png)\n\nmore";
        let (lines, images) = render_md_ex(md, 40, &theme());
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].url, "https://cdn.example/x.png");
        let text = plain(&lines);
        assert_eq!(text[images[0].line], "▣ cool pic");
    }

    #[test]
    fn inline_images_stay_placeholders() {
        let (lines, images) = render_md_ex("see ![shot](https://c.e/s.png) here", 40, &theme());
        assert!(images.is_empty(), "inline image is not a block");
        let text = plain(&lines);
        assert!(text[0].contains("▣ shot"));
    }

    #[test]
    fn converts_img_tags_to_image_blocks() {
        let md = "hello\n\n<img src=\"https://cdn.example/a.png\" alt=\"A pic\">\n\nbye";
        let (lines, images) = render_md_ex(md, 40, &theme());
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].url, "https://cdn.example/a.png");
        let text = plain(&lines);
        assert_eq!(text[images[0].line], "▣ A pic");
    }

    #[test]
    fn normalizes_protocol_relative_urls() {
        let (_, images) = render_md_ex("![](//cdn.example/b.png)", 40, &theme());
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].url, "https://cdn.example/b.png");
    }

    #[test]
    fn html_attr_extraction() {
        assert_eq!(
            html_attr("img src=\"a.png\" alt='x'", "src").as_deref(),
            Some("a.png")
        );
        assert_eq!(html_attr("img alt=x src=b.png", "alt").as_deref(), Some("x"));
        assert_eq!(html_attr("img data-src=nope", "src"), None);
    }

    #[test]
    fn linked_images_yield_blocks_without_bracket_junk() {
        let md = "[<img src=\"https://img.shields.io/badge/Discord-5865F2\" alt=\"discord\"/>](https://discord.gg/x)";
        let (lines, images) = render_md_ex(md, 60, &theme());
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].url, "https://img.shields.io/badge/Discord-5865F2");
        assert_eq!(images[0].link.as_deref(), Some("https://discord.gg/x"));
        let text = plain(&lines);
        assert_eq!(text[images[0].line], "▣ discord");
        for line in &text {
            assert!(!line.contains('[') && !line.contains(']'), "bracket junk: {line:?}");
        }
    }

    #[test]
    fn linked_markdown_images_yield_blocks() {
        let md = "[![shot](https://c.e/s.png)](https://example.com)";
        let (lines, images) = render_md_ex(md, 60, &theme());
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].url, "https://c.e/s.png");
        assert_eq!(images[0].link.as_deref(), Some("https://example.com"));
        let text = plain(&lines);
        assert_eq!(text[images[0].line], "▣ shot");
    }

    #[test]
    fn badge_row_of_linked_images_yields_blocks_in_order() {
        let md = "[![a](https://c.e/a.png)](https://x/1) [![b](https://c.e/b.png)](https://x/2)";
        let (_, images) = render_md_ex(md, 60, &theme());
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].url, "https://c.e/a.png");
        assert_eq!(images[0].link.as_deref(), Some("https://x/1"));
        assert_eq!(images[1].url, "https://c.e/b.png");
        assert_eq!(images[1].link.as_deref(), Some("https://x/2"));
        assert_eq!(images[1].line, images[0].line + 1, "placeholders stack tightly");
    }

    #[test]
    fn html_anchor_image_keeps_link_target() {
        let md = "<a href=\"https://discord.gg/x\"><img src=\"https://c.e/badge.png\" alt=\"badge\"/></a>";
        let (lines, images) = render_md_ex(md, 60, &theme());
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].url, "https://c.e/badge.png");
        assert_eq!(images[0].link.as_deref(), Some("https://discord.gg/x"));
        let text = plain(&lines);
        assert_eq!(text[images[0].line], "▣ badge");
        for line in &text {
            assert!(!line.contains('[') && !line.contains(']'), "bracket junk: {line:?}");
        }
    }

    #[test]
    fn multiline_html_anchor_joins_into_one_block() {
        let md = "<a href=\"https://example.com/more\">\n<img src=\"https://c.e/m.png\" alt=\"m\">\n</a><br><br>tail";
        let (lines, images) = render_md_ex(md, 60, &theme());
        assert_eq!(images.len(), 1, "anchor image is one block");
        assert_eq!(images[0].link.as_deref(), Some("https://example.com/more"));
        let text = plain(&lines);
        assert!(
            !text.iter().any(|l| l.contains('[') || l.contains(']')),
            "no stray anchor brackets: {text:?}"
        );
        assert!(
            text.iter().any(|l| l.contains("tail")),
            "text after anchor survives"
        );
    }

    #[test]
    fn html_anchor_text_link_stays_inline() {
        let (lines, images) =
            render_md_ex("see <a href=\"https://c.e/d\">docs</a> now", 60, &theme());
        assert!(images.is_empty());
        let text = plain(&lines);
        assert!(text[0].contains("docs"));
        assert!(!text[0].contains('['), "no stray brackets: {:?}", text[0]);
    }

    #[test]
    fn text_plus_link_falls_back_to_inline() {
        let (lines, images) =
            render_md_ex("see [docs](https://c.e/docs) for details", 60, &theme());
        assert!(images.is_empty());
        assert!(plain(&lines)[0].contains("docs"));
    }

    #[test]
    fn text_link_reports_line_and_columns() {
        let (lines, _images, links, _, _) =
            render_md_full("see [docs](https://c.e/x) now", 80, &theme());
        assert_eq!(links.len(), 1);
        let l = &links[0];
        assert_eq!(l.line, 0);
        assert_eq!(l.start, 4, "link starts right after `see `");
        assert_eq!(l.end, 8, "link ends after `docs`");
        assert_eq!(l.url, "https://c.e/x");
        let text = plain(&lines);
        assert_eq!(text[0], "see docs now");
    }

    #[test]
    fn wrapped_text_link_reports_range_per_line() {
        let (_lines, _images, links, _, _) = render_md_full(
            "alpha [one two three four](https://c.e/x) omega",
            16,
            &theme(),
        );
        assert_eq!(links.len(), 2, "one range per wrapped line");
        assert_eq!((links[0].line, links[0].start, links[0].end), (0, 6, 13));
        assert_eq!((links[1].line, links[1].start, links[1].end), (1, 0, 10));
        assert_eq!(links[0].url, "https://c.e/x");
        assert_eq!(links[1].url, "https://c.e/x");
    }

    #[test]
    fn list_and_quote_links_offset_by_prefix() {
        let (_l, _i, links, _, _) = render_md_full("- see [docs](https://c.e/a)", 80, &theme());
        assert_eq!(links.len(), 1);
        assert_eq!(
            (links[0].line, links[0].start, links[0].end),
            (0, 6, 10),
            "marker prefix shifts the range"
        );
        let (_l, _i, qlinks, _, _) = render_md_full("> [docs](https://c.e/a)", 80, &theme());
        assert_eq!(qlinks.len(), 1);
        assert_eq!((qlinks[0].start, qlinks[0].end), (2, 6));
    }

    #[test]
    fn html_text_anchor_reports_range() {
        let (_l, _i, links, _, _) =
            render_md_full("see <a href=\"https://c.e/g\">guide</a> now", 80, &theme());
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].url, "https://c.e/g");
        assert_eq!((links[0].line, links[0].start, links[0].end), (0, 4, 9));
    }

    #[test]
    fn heading_and_bold_links_are_tracked() {
        let (_l, _i, hlinks, _, _) = render_md_full("# [Title](https://c.e/t)", 80, &theme());
        assert_eq!(hlinks.len(), 1);
        assert_eq!((hlinks[0].start, hlinks[0].end), (0, 5), "hash prefix stripped");
        let (_l, _i, blinks, _, _) = render_md_full("**[bold](https://c.e/b)**", 80, &theme());
        assert_eq!(blinks.len(), 1);
        assert_eq!((blinks[0].start, blinks[0].end), (0, 4));
    }

    #[test]
    fn table_columns_align_across_rows() {
        let md = concat!(
            "Other mods | Without IF | With IF | Improvement\n",
            "| --- | --- | --- | --- |\n",
            "None | 16 FPS | 60 FPS | 3.75x\n",
            "Sodium | 21 FPS | 82 FPS | 3.90x"
        );
        let (lines, _i, _l, _, _) = render_md_full(md, 80, &theme());
        let text = plain(&lines);
        assert!(text[1].contains('─'), "separator sits under the header");
        for (col, (head, cell)) in [
            ("Without IF", "16 FPS"),
            ("With IF", "60 FPS"),
            ("Improvement", "3.75x"),
        ]
        .into_iter()
        .enumerate()
        {
            let head_col = text[0].find(head).unwrap_or_else(|| panic!("header `{head}` missing"));
            let cell_col = text[2].find(cell).unwrap_or_else(|| panic!("cell `{cell}` missing"));
            assert_eq!(head_col, cell_col, "column {col} aligns");
        }
        let sep_cols: Vec<usize> = text[1].match_indices('─').map(|(i, _)| i).collect();
        let seg_start = *sep_cols.first().unwrap();
        assert_eq!(seg_start, 0, "first segment starts at the left edge");
        let cell_col = text[0].find("Without IF").unwrap();
        assert!(
            sep_cols.contains(&cell_col),
            "a segment starts at every column boundary"
        );
    }

    #[test]
    fn table_shrinks_to_fit_narrow_width() {
        let md = concat!(
            "aaaa | b\n",
            "| --- | --- |\n",
            "cccccc | d"
        );
        let (lines, _i, _l, _, _) = render_md_full(md, 8, &theme());
        for line in plain(&lines) {
            assert!(
                line.width() <= 8,
                "row wider than the table area: {line:?}"
            );
        }
    }

    #[test]
    fn inline_code_span_reports_coords_and_style() {
        let theme = theme();
        let (lines, _, _, codes, _) = render_md_full("use `cargo build` here", 80, &theme);
        assert_eq!(codes.len(), 1);
        let c = &codes[0];
        assert_eq!(c.text, "cargo build");
        assert_eq!((c.line, c.start, c.end), (0, 4, 15));
        assert_eq!(plain(&lines)[0], "use cargo build here");
        let code_span = lines[0]
            .spans
            .iter()
            .find(|s| s.style.bg == Some(theme.panel_alt))
            .expect("inline code has a panel_alt background");
        assert_eq!(code_span.style.fg, Some(theme.comment));
        assert!(
            code_span.content.to_string().contains("cargo build"),
            "code span keeps its text: {code_span:?}"
        );
    }

    #[test]
    fn code_block_gets_padding_and_reports_coords() {
        let theme = theme();
        let md = "intro\n\n```rust\nfn main() {}\n```\n\nafter";
        let (lines, _, _, _, blocks) = render_md_full(md, 40, &theme);
        assert_eq!(blocks.len(), 1);
        let b = &blocks[0];
        assert_eq!(b.height, 3, "top pad + code row + bottom pad");
        assert_eq!(b.text, "fn main() {}", "copy payload is the raw code");
        let text = plain(&lines);
        assert!(text[b.line].chars().all(|c| c == ' '), "top pad row");
        assert!(text[b.line + 1].contains("fn main()"));
        assert!(text[b.line + 2].chars().all(|c| c == ' '), "bottom pad row");
        let style = lines[b.line + 1].spans[0].style;
        assert_eq!(style.bg, Some(theme.panel_alt));
        assert_eq!(style.fg, Some(theme.comment));
    }

    #[test]
    fn image_inside_text_line_stays_inline() {
        let (lines, images) =
            render_md_ex("[text](https://c.e/x) and ![pic](https://c.e/p.png)", 60, &theme());
        assert!(images.is_empty(), "mixed line is not a block");
        assert!(plain(&lines)[0].contains("▣ pic"));
    }

    #[test]
    fn nested_paren_link_target_is_consumed() {
        let md = "[![g](https://c.e/g.png)](https://en.wikipedia.org/wiki/Foo_(bar))";
        let (lines, images) = render_md_ex(md, 60, &theme());
        assert_eq!(images.len(), 1);
        let text = plain(&lines);
        for line in &text {
            assert!(!line.contains(']'), "bracket junk: {line:?}");
        }
    }

    #[test]
    fn empty_alt_falls_back_to_file_name() {
        let (lines, images) = render_md_ex("![](https://c.e/img/banner.png?w=2)", 60, &theme());
        assert_eq!(images.len(), 1);
        assert_eq!(plain(&lines)[images[0].line], "▣ banner");
    }

    #[test]
    fn plain_images_are_left_aligned() {
        let md = "![a](https://c.e/a.png)\n![b](https://c.e/b.png)";
        let (_, images) = render_md_ex(md, 60, &theme());
        assert_eq!(images.len(), 2);
        assert!(images.iter().all(|i| i.align == MdAlign::Left));
    }

    #[test]
    fn center_paragraph_aligns_images_until_close_tag() {
        let md = "<p align=\"center\" style=\"text-align: center;\">\n<img src=\"https://c.e/a.png\" alt=\"a\">\n</p>\n\n![b](https://c.e/b.png)";
        let (_, images) = render_md_ex(md, 60, &theme());
        assert_eq!(images.len(), 2, "wrapper images become blocks");
        assert_eq!(images[0].align, MdAlign::Center, "inside align=center");
        assert_eq!(images[1].align, MdAlign::Left, "after </p> back to left");
    }

    #[test]
    fn center_tag_aligns_following_block() {
        let md = "<center>\n![a](https://c.e/a.png)\n</center>";
        let (_, images) = render_md_ex(md, 60, &theme());
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].align, MdAlign::Center);
    }

    #[test]
    fn text_align_right_in_style_is_honored() {
        let md = "<div style=\"text-align: right\">\n![a](https://c.e/a.png)\n</div>";
        let (_, images) = render_md_ex(md, 60, &theme());
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].align, MdAlign::Right);
    }

    #[test]
    fn linked_badge_row_keeps_wrapper_alignment() {
        let md = "<p align=\"center\">\n[![a](https://c.e/a.png)](https://x/1)\n[![b](https://c.e/b.png)](https://x/2)\n</p>";
        let (_, images) = render_md_ex(md, 60, &theme());
        assert_eq!(images.len(), 2);
        assert!(images.iter().all(|i| i.align == MdAlign::Center));
    }

    #[test]
    fn unclosed_br_inside_center_does_not_reset_alignment() {
        let md = "<center>\n<br>![a](https://c.e/a.png)\n</center>";
        let (_, images) = render_md_ex(md, 60, &theme());
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].align, MdAlign::Center);
    }

    #[test]
    fn entityculling_style_body_yields_expected_alignments() {
        let md = concat!(
            "![Entity Culling Banner](https://tr7zw.github.io/uikit/banner/header_entity_culling.png)\n\n",
            "<p align=\"center\" style=\"text-align: center;\">\n",
            "  <a href=\"https://discord.gg/x\"><img src=\"https://tr7zw.github.io/uikit/social_buttons_icon/Discord-Button-64.png\" alt=\"Discord\" style=\"margin: 5px 10px;\"></a>\n",
            "  <a href=\"https://github.com/x\"><img src=\"https://tr7zw.github.io/uikit/social_buttons_icon/Github-Button-64.png\" alt=\"GitHub\" style=\"margin: 5px 10px;\"></a>\n",
            "</p>\n\n",
            "<br>![Divider](https://tr7zw.github.io/uikit/divider_faded/Divider_01.png)\n\n",
            "<img src=\"https://tr7zw.github.io/uikit/headlines/large/About.png\" alt=\"About\" style=\"margin: 5px 10px;\">"
        );
        let (_, images) = render_md_ex(md, 100, &theme());
        assert_eq!(images.len(), 5, "banner, 2 badges, divider, headline");
        assert_eq!(images[0].align, MdAlign::Left, "banner");
        assert_eq!(images[1].align, MdAlign::Center, "discord badge");
        assert_eq!(images[2].align, MdAlign::Center, "github badge");
        assert_eq!(images[3].align, MdAlign::Left, "divider after </p>");
        assert_eq!(images[4].align, MdAlign::Left, "bare img headline");
        assert_eq!(
            images[1].link.as_deref(),
            Some("https://discord.gg/x"),
            "html anchor target captured"
        );
        assert_eq!(
            images[2].link.as_deref(),
            Some("https://github.com/x"),
            "html anchor target captured"
        );
        assert_eq!(images[0].link, None, "plain banner has no link");
        assert!(
            images[2].line > images[1].line,
            "badge placeholders stay in source order"
        );
    }
}

