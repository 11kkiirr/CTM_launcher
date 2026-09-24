//! Shared sectioned-settings chrome: panels, RAM sliders, rows.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;
use std::sync::OnceLock;

use crate::app::App;

/// Dark page-bg gutter on the left of every settings panel.
pub(crate) const SIDE_PAD: u16 = 2;
/// Dark page-bg rows between section panels.
pub(crate) const PANEL_GAP: u16 = 1;
/// Fixed label column width (cells) so values always start on the same column.
pub(crate) const LABEL_W: u16 = 22;
/// Gap between the label cell and the value/track column.
pub(crate) const LABEL_GAP: u16 = 2;
/// Width of the "NNNN MB" readout to the right of a slider track.
pub(crate) const VALUE_W: u16 = 10;
/// Width of the `[nnnn]` inline input cell.
pub(crate) const INPUT_W: u16 = 8;
/// Minimum / maximum RAM (MB). Values snap to multiples of `RAM_STEP`.
pub(crate) const RAM_MIN: u32 = 512;
/// Hard ceiling when system RAM cannot be detected.
pub(crate) const RAM_MAX: u32 = 32768;
/// Snap/adjust step (MB).
pub(crate) const RAM_STEP: u32 = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FieldKind {
    Text,
    Slider,
    Choice,
    Bool,
}

/// Detect total system RAM in MB (Linux `/proc/meminfo`, else fallback).
pub(crate) fn system_ram_mb() -> u32 {
    static CACHED: OnceLock<u32> = OnceLock::new();
        *CACHED.get_or_init(|| {
        let mut mb = 0u32;
        if let Ok(text) = std::fs::read_to_string("/proc/meminfo") {
            for line in text.lines() {
                if let Some(rest) = line.strip_prefix("MemTotal:") {
                    if let Some(kb) = rest.split_whitespace().next() {
                        if let Ok(v) = kb.parse::<u64>() {
                            mb = (v / 1024) as u32;
                        }
                    }
                    break;
                }
            }
        }
        if mb < 1024 {
            mb = 8192;
        }
        // Leave a little headroom for OS + launcher itself.
        let usable = mb.saturating_sub(512);
        // Snap down to 1024 grid.
        ((usable / RAM_STEP) * RAM_STEP).max(RAM_MIN + RAM_STEP)
    })
}

/// Effective slider ceiling: never above real RAM (with a small reserve).
pub(crate) fn effective_ram_max() -> u32 {
    let lo = RAM_MIN + RAM_STEP;
    let hi = RAM_MAX.max(lo);
    system_ram_mb().clamp(lo, hi)
}

/// ~60% of system RAM — above this the slider turns red and a warning fires.
/// Half is the common working point, so it must stay green.
pub(crate) fn ram_warn_threshold() -> u32 {
    let t = (system_ram_mb() as u64 * 60 / 100) as u32;
    // Snap down onto the grid so the threshold sits on a tick.
    (t / RAM_STEP * RAM_STEP).max(RAM_MIN)
}

/// True when `mb` exceeds ~60% of physical RAM.
pub(crate) fn is_ram_over_half(mb: u32) -> bool {
    mb > ram_warn_threshold()
}

/// Map value → cell index. Single integer formula shared by the handle,
/// tick marks and scale labels so they always land on the same column.
pub(crate) fn value_to_pos(value: u32, min: u32, max: u32, w: usize) -> usize {
    if w < 3 {
        return 1;
    }
    let usable = (w - 3) as u64;
    let span = max.saturating_sub(min).max(1) as u64;
    let v = value.saturating_sub(min) as u64;
    // Rounded integer division: (v * usable + span/2) / span
    let pos = 1 + ((v * usable + span / 2) / span) as usize;
    pos.clamp(1, w - 2)
}

/// Nice scale stops on the RAM grid from `min` to `max` (inclusive).
/// Yields whole-GB labels after the 512 floor: `512, 1, 2, … 15`.
pub(crate) fn scale_values(min: u32, max: u32) -> Vec<u32> {
    let min = min.min(max);
    let mut out = vec![min];
    let mut v = RAM_STEP;
    while v < max {
        if v > min {
            out.push(v);
        }
        v = v.saturating_add(RAM_STEP);
    }
    if max > min {
        out.push(max);
    }
    out
}

/// `(position, value)` for every scale stop on a track of width `w`.
pub(crate) fn scale_stops(min: u32, max: u32, w: usize) -> Vec<(usize, u32)> {
    scale_values(min, max)
        .into_iter()
        .map(|v| (value_to_pos(v, min, max, w), v))
        .collect()
}

/// Snap a RAM value onto the `RAM_STEP` grid within `[lo, hi]`.
pub(crate) fn snap_ram(v: u32, lo: u32, hi: u32) -> u32 {
    let lo = lo.max(RAM_MIN);
    let hi = hi.min(effective_ram_max()).max(lo);
    if v <= lo {
        return lo;
    }
    if v >= hi {
        return hi;
    }
    let base = (v / RAM_STEP) * RAM_STEP;
    let candidates = [base, base + RAM_STEP];
    let mut best = lo;
    let mut best_d = v.abs_diff(lo);
    for c in candidates {
        if c < lo || c > hi {
            continue;
        }
        let d = v.abs_diff(c);
        if d < best_d {
            best_d = d;
            best = c;
        }
    }
    if v.abs_diff(hi) < best_d {
        best = hi;
    }
    best
}

/// Allowed range for the min-RAM slider given the current max.
pub(crate) fn min_ram_range(max_mb: u32) -> (u32, u32) {
    let hi_cap = effective_ram_max();
    let max_mb = max_mb.min(hi_cap);
    (RAM_MIN, max_mb.saturating_sub(RAM_STEP).max(RAM_MIN))
}

/// Allowed range for the max-RAM slider given the current min.
pub(crate) fn max_ram_range(min_mb: u32) -> (u32, u32) {
    let cap = effective_ram_max();
    (
        min_mb.saturating_add(RAM_STEP).min(cap),
        cap,
    )
}

/// Paint the darkest page background so gutters read as background.
pub(crate) fn paint_page_bg(app: &App, frame: &mut Frame, area: Rect) {
    frame.render_widget(Block::default().style(Style::default().bg(app.theme.bg)), area);
}

/// Fill a rectangle with a solid style (section panel body, row backdrop).
pub(crate) fn fill_rect(frame: &mut Frame, rect: Rect, style: Style) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    frame.render_widget(Block::default().style(style), rect);
}

/// Section header strip: plain title on panel bg — no accordion affordances.
pub(crate) fn section_header(app: &mut App, frame: &mut Frame, rect: Rect, title: &str) {
    let line = Line::from(Span::styled(title.to_string(), app.theme.header()));
    frame.render_widget(Paragraph::new(line).style(Style::default().bg(app.theme.panel)), rect);
}

/// Draw a RAM slider: `•──|──|──●──|──|──•`
///
/// `bg` must match the surrounding row so the track never punches a dark
/// hole through a selected/hovered highlight. Ticks, handle and scale labels
/// all share [`value_to_pos`], so `●` always sits on a `|` for grid values.
///
/// Filled/handle turn **red** when `value` exceeds ~60% of system RAM.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_slider(
    app: &App,
    frame: &mut Frame,
    rect: Rect,
    value: u32,
    min: u32,
    max: u32,
    selected: bool,
    bg: Color,
) {
    if rect.width < 3 || rect.height == 0 {
        return;
    }
    let w = rect.width as usize;
    let handle = value_to_pos(value, min, max, w);
    let warn = is_ram_over_half(value);

    let (filled_fg, handle_fg) = if warn {
        (app.theme.error, app.theme.error)
    } else if selected {
        (app.theme.green_bright, app.theme.green_bright)
    } else {
        (app.theme.green, app.theme.green)
    };
    let empty_fg = app.theme.muted;
    let tick_fg = app.theme.comment;
    let minor_fg = if warn {
        app.theme.error
    } else {
        app.theme.muted
    };

    let mut cells: Vec<Span> = Vec::with_capacity(w);
    for i in 0..w {
        let (ch, style) = if i == 0 || i == w - 1 {
            ("•", Style::default().fg(tick_fg))
        } else if i == handle {
            ("●", Style::default().fg(handle_fg))
        } else if i < handle {
            ("━", Style::default().fg(filled_fg))
        } else {
            ("─", Style::default().fg(empty_fg))
        };
        cells.push(Span::styled(ch, style));
    }

    // Tick marks at the same positions as scale labels / the handle.
    for pos in scale_stops(min, max, w).into_iter().map(|(p, _)| p) {
        if pos == handle || pos == 0 || pos + 1 >= w {
            continue;
        }
        let fg = if warn && pos < handle {
            app.theme.error
        } else if pos < handle {
            app.theme.green
        } else {
            minor_fg
        };
        cells[pos] = Span::styled("|", Style::default().fg(fg));
    }

    frame.render_widget(
        Paragraph::new(Line::from(cells)).style(Style::default().bg(bg)),
        rect,
    );
}

/// Scale row under a selected slider: whole-GB labels at scale stops.
///
/// `rect` must be the **track** rect (same x/width as the slider) so labels
/// never spill into the value/input columns. `bg` matches the row highlight.
pub(crate) fn draw_slider_scale(
    app: &App,
    frame: &mut Frame,
    rect: Rect,
    min: u32,
    max: u32,
    bg: Color,
) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let w = rect.width as usize;
    let mut cells: Vec<Span> = vec![Span::styled(" ", Style::default().bg(bg)); w];
    let style = Style::default().fg(app.theme.muted).bg(bg);

    let mut last_end: isize = -2;
    let stops = scale_stops(min, max, w);
    for (i, (pos, v)) in stops.iter().enumerate() {
        let text = format_scale_mb(*v);
        let chars: Vec<char> = text.chars().collect();
        let start = *pos as isize - (chars.len() as isize / 2);
        let end = start + chars.len() as isize;
        if start <= last_end + 1 {
            if i + 1 == stops.len() {
                // Keep the max label pinned to the far right if it collided.
                let start = (w as isize - chars.len() as isize).max(last_end + 2);
                if start >= 0 && start + chars.len() as isize <= w as isize {
                    for (k, ch) in chars.iter().enumerate() {
                        let x = start + k as isize;
                        if x >= 0 && (x as usize) < w {
                            cells[x as usize] = Span::styled(ch.to_string(), style);
                        }
                    }
                    last_end = start + chars.len() as isize - 1;
                }
            }
            continue;
        }
        if start < 0 || end > w as isize {
            continue;
        }
        for (k, ch) in chars.iter().enumerate() {
            cells[start as usize + k] = Span::styled(ch.to_string(), style);
        }
        last_end = end - 1;
    }

    frame.render_widget(Paragraph::new(Line::from(cells)).style(style), rect);
}

/// Compact RAM label for the scale row: `512`, `1`, `15`.
fn format_scale_mb(v: u32) -> String {
    if v < 1024 {
        format!("{v}")
    } else {
        format!("{}", v / 1024)
    }
}

/// Slider row geometry shared by track, scale, value and input cells.
///
/// Layout: `[label LABEL_W][gap][track …][gap][value][input]`
pub(crate) fn slider_track_rect(row: Rect) -> Rect {
    let track_w = row
        .width
        .saturating_sub(LABEL_W + LABEL_GAP * 2 + VALUE_W + INPUT_W)
        .max(8);
    Rect {
        x: row.x + LABEL_W + LABEL_GAP,
        y: row.y,
        width: track_w,
        height: 1,
    }
}

pub(crate) fn slider_value_rect(row: Rect, track: Rect) -> Rect {
    Rect {
        x: track.x + track.width + LABEL_GAP,
        y: row.y,
        width: VALUE_W,
        height: 1,
    }
}

pub(crate) fn slider_input_rect(row: Rect, value: Rect) -> Rect {
    Rect {
        x: value.x + value.width,
        y: row.y,
        width: INPUT_W,
        height: 1,
    }
}

/// Click position → value on the `[min,max]` scale, snapped to `RAM_STEP`.
pub(crate) fn slider_value_at(x: u16, track: Rect, min: u32, max: u32) -> u32 {
    if track.width < 3 {
        return min;
    }
    let w = track.width as usize;
    let rel = x.saturating_sub(track.x) as usize;
    let pos = rel.clamp(1, w - 2);
    let usable = (w - 3) as f64;
    let ratio = (pos - 1) as f64 / usable.max(1.0);
    let raw = min as f64 + ratio * (max.saturating_sub(min)) as f64;
    snap_ram(raw.round() as u32, min, max)
}

/// Style a settings row: selected → row_selected, hovered → row_hover, else row.
pub(crate) fn row_style(app: &App, selected: bool, hovered: bool) -> Style {
    if selected {
        app.theme.row_selected()
    } else if hovered {
        app.theme.row_hover()
    } else {
        app.theme.row()
    }
}

/// Fixed-width label cell: truncated to `LABEL_W` (with `…`) then padded
/// with `LABEL_GAP` trailing spaces so the value/track column always starts
/// at `LABEL_W + LABEL_GAP`.
///
/// Callers must **not** re-truncate this string — that was double-truncating
/// and injecting a spurious `…` after the pad spaces.
pub(crate) fn label_cell(label: &str) -> String {
    let max = LABEL_W as usize;
    let chars: Vec<char> = label.chars().collect();
    let mut s: String = if chars.len() > max {
        chars[..max.saturating_sub(1)].iter().collect::<String>() + "…"
    } else {
        chars.iter().collect()
    };
    let pad = max.saturating_sub(s.chars().count()) + LABEL_GAP as usize;
    s.push_str(&" ".repeat(pad));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_lands_on_1024_grid() {
        let hi = effective_ram_max();
        assert_eq!(snap_ram(4096, RAM_MIN, hi), 4096);
        assert_eq!(snap_ram(4500, RAM_MIN, hi), 4096);
        assert_eq!(snap_ram(4700, RAM_MIN, hi), 5 * 1024);
        assert_eq!(snap_ram(1023, RAM_MIN, hi), 1024);
        assert_eq!(snap_ram(512, RAM_MIN, hi), 512);
        assert_eq!(snap_ram(0, RAM_MIN, hi), RAM_MIN);
        assert_eq!(snap_ram(999_999, RAM_MIN, hi), hi);
    }

    #[test]
    fn snap_respects_subrange() {
        assert_eq!(snap_ram(3000, 4096, 8192), 4096);
        assert_eq!(snap_ram(5000, 4096, 8192), 5 * 1024);
    }

    #[test]
    fn scale_stops_are_whole_gb_and_aligned_with_handle() {
        let w = 80usize;
        let min = RAM_MIN;
        let max = effective_ram_max();
        let stops = scale_stops(min, max, w);
        assert!(stops.len() >= 4);
        // Labels must be whole numbers (or the 512 floor).
        for (_, v) in &stops {
            assert!(*v == RAM_MIN || *v % RAM_STEP == 0, "non-grid stop {v}");
            let text = format_scale_mb(*v);
            assert!(
                !text.contains('.'),
                "scale label must be whole: {text} for {v}"
            );
        }
        // Handle for a grid value sits exactly on its tick position.
        for (_, v) in &stops {
            let tick = value_to_pos(*v, min, max, w);
            let handle = value_to_pos(*v, min, max, w);
            assert_eq!(tick, handle);
        }
        // Positions are non-decreasing and inside the open track.
        let mut prev = 0;
        for (p, _) in &stops {
            assert!(*p >= prev);
            assert!(*p >= 1 && *p < w - 1);
            prev = *p;
        }
    }

    #[test]
    fn effective_max_never_exceeds_system_ram() {
        let sys = system_ram_mb();
        assert!(effective_ram_max() <= sys.max(RAM_MIN + RAM_STEP));
        assert!(effective_ram_max() >= RAM_MIN + RAM_STEP);
    }

    #[test]
    fn warn_threshold_is_sixty_percent_of_system() {
        let sys = system_ram_mb();
        let expected = {
            let t = (sys as u64 * 60 / 100) as u32;
            (t / RAM_STEP * RAM_STEP).max(RAM_MIN)
        };
        assert_eq!(ram_warn_threshold(), expected);
        assert!(!is_ram_over_half(ram_warn_threshold()));
        assert!(is_ram_over_half(ram_warn_threshold() + RAM_STEP));
        // Half of RAM stays green — the common working point.
        assert!(!is_ram_over_half(sys / 2));
    }

    #[test]
    fn min_max_ranges_never_conflict() {
        let cap = effective_ram_max();
        let (_, hi) = min_ram_range(RAM_MIN + RAM_STEP);
        assert!(hi <= RAM_MIN + RAM_STEP);
        let (lo, hi2) = max_ram_range(cap.saturating_sub(RAM_STEP));
        assert!(lo <= hi2);
        assert!(hi2 <= cap);
    }

    #[test]
    fn label_cell_is_exact_width_without_spurious_ellipsis() {
        let short = label_cell("Путь к Java");
        assert_eq!(
            short.chars().count(),
            LABEL_W as usize + LABEL_GAP as usize
        );
        assert!(
            !short.contains('…'),
            "short label must not gain an ellipsis: {short:?}"
        );

        let long = label_cell("Очень длинная подпись, которая точно не влезет");
        assert_eq!(
            long.chars().count(),
            LABEL_W as usize + LABEL_GAP as usize
        );
        assert!(long.contains('…'));
        // Gap spaces must remain at the end so the value column starts clean.
        assert!(long.ends_with("  "));
    }

    #[test]
    fn slider_track_leaves_room_for_value_and_input() {
        let row = Rect {
            x: 3,
            y: 5,
            width: 100,
            height: 1,
        };
        let track = slider_track_rect(row);
        let value = slider_value_rect(row, track);
        let input = slider_input_rect(row, value);
        assert_eq!(track.x, row.x + LABEL_W + LABEL_GAP);
        assert_eq!(input.x + input.width, row.x + row.width);
        assert!(track.width >= 8);
        // Scale under the track must share the track's horizontal span.
        assert_eq!(track.x + track.width + LABEL_GAP, value.x);
    }

    #[test]
    fn format_scale_is_whole() {
        assert_eq!(format_scale_mb(512), "512");
        assert_eq!(format_scale_mb(1024), "1");
        assert_eq!(format_scale_mb(15360), "15");
    }
}
