//! Instance picker as separate collapsible group panels on the dark page bg.

use crossterm::event::{KeyCode, KeyEvent};
use mc_core::instance::Instance;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;

use crate::app::{rect_contains, App, ButtonId, HitAction, Nav};
use crate::settings::AsciiBgAnchor;
use crate::views::{pill_row, truncate};

/// Cards are wider than tall, but close to square once cell aspect is taken
/// into account.
pub(crate) const TILE_W: u16 = 24;
pub(crate) const TILE_H: u16 = 10;
pub(crate) const GAP_X: u16 = 2;
pub(crate) const GAP_Y: u16 = 1;
/// Height of the vertically centered inner content block.
const CONTENT_H: u16 = 6;
/// One-line section header strip at the top of each group panel.
const HEADER_H: usize = 1;
/// Dark page-bg gutter on the left/right of every group panel.
const SIDE_PAD: u16 = 2;
/// Dark page-bg rows between group panels (same look as SIDE_PAD).
const PANEL_GAP: usize = 1;
/// Empty page-bg row under the last tile row inside a group panel.
const BOTTOM_PAD: usize = 1;

/// A vertical slice of the Builds area: optional group name + instance range.
#[derive(Debug, Clone)]
pub(crate) struct Section {
    /// Empty string = ungrouped.
    pub name: String,
    /// Start index into `App::instances`.
    pub start: usize,
    pub count: usize,
    /// Collapsed panels only show the header strip.
    pub collapsed: bool,
}

impl App {
    /// Current section layout for the Builds area (collapse-aware).
    pub(crate) fn instance_sections(&self) -> Vec<Section> {
        let mut sections = sections_from(&self.instances, &self.groups);
        for s in &mut sections {
            s.collapsed = self.collapsed_groups.contains(&s.name);
        }
        sections
    }

    /// Render the build action toolbar and the stacked group panels.
    pub(crate) fn render_instance_grid(&mut self, frame: &mut Frame, area: Rect) {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(3),
            ])
            .split(area);

        let launch = self.tr("btn.launch");
        let new = self.tr("btn.new");
        let edit = self.tr("btn.edit");
        let install = self.tr("btn.install_repair");
        let import = self.tr("btn.import");
        let versions = self.tr("btn.versions");
        let rename = self.tr("btn.rename");
        let delete = self.tr("btn.delete");
        let new_grp = self.tr("btn.new_group");
        let ren_grp = self.tr("btn.rename_group");
        let del_grp = self.tr("btn.delete_group");
        pill_row(
            self,
            frame,
            rows[0].x + SIDE_PAD,
            rows[0].y,
            area.x + area.width,
            &[
                (launch, "Enter", ButtonId::Launch),
                (new, "n", ButtonId::NewInstance),
                (edit, "e", ButtonId::EditInstance),
                (install, "i", ButtonId::InstallInstance),
                (import, "p", ButtonId::ImportModpack),
                (versions, "v", ButtonId::ChangeVersion),
                (rename, "r", ButtonId::RenameInstance),
                (delete, "d", ButtonId::DeleteInstance),
                (new_grp, "y", ButtonId::NewGroup),
                (ren_grp, "Y", ButtonId::RenameGroup),
                (del_grp, "D", ButtonId::DeleteGroup),
            ],
        );

        self.render_group_panels(frame, rows[2]);
    }

    /// Dark page background + group panels in a horizontal flow (wrap rows).
    fn render_group_panels(&mut self, frame: &mut Frame, area: Rect) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        // Darkest page colour shows through SIDE_PAD and PANEL_GAP.
        fill_rect(
            frame,
            area,
            Style::default().bg(self.theme.bg),
        );

        let body = area;
        if body.height == 0 {
            return;
        }

        let max_panel_w = body.width.saturating_sub(SIDE_PAD);
        if max_panel_w < 4 {
            return;
        }

        let full_cols = ((max_panel_w + GAP_X) / (TILE_W + GAP_X)).max(1) as usize;
        self.tile_columns = full_cols;
        self.tile_body_w = body.width;
        let view_h = body.height as usize;
        self.ensure_tile_visible(view_h);

        // ASCII on the page bg (panel-coloured); panels redraw it dark on top.
        self.render_ascii_bg(frame, body, None, false, &[]);

        let sections = self.instance_sections();
        let boxes = layout_panels(&sections, body.width);
        let scroll = self.tile_scroll;
        let selected = self.instance_state.selected();

        for b in &boxes {
            let section = &sections[b.section];

            // Clip panel to the viewport.
            if b.y >= scroll + view_h {
                continue;
            }
            if b.y + b.height <= scroll {
                continue;
            }

            let vis_start = b.y.max(scroll);
            let vis_end = (b.y + b.height).min(scroll + view_h);
            if vis_start >= vis_end {
                continue;
            }

            let screen_y = body.y + (vis_start - scroll) as u16;
            let screen_h = (vis_end - vis_start) as u16;
            let panel_rect = Rect {
                x: body.x + b.x,
                y: screen_y,
                width: b.width,
                height: screen_h,
            };

            // Panel surface (clear symbols so page-bg art cannot bleed through).
            fill_rect(frame, panel_rect, Style::default().bg(self.theme.panel));
            let cards: Vec<Rect> = self
                .visible_tiles(section, b.cols, panel_rect, b.y, scroll, body)
                .into_iter()
                .map(|(r, _)| r)
                .collect();
            self.render_ascii_bg(frame, body, Some(panel_rect), true, &cards);

            self.render_group_panel(
                frame,
                panel_rect,
                section,
                b.section,
                b.cols,
                b.y,
                scroll,
                body,
                selected,
            );
        }
    }

    /// One group panel: header strip + optional tile grid, flat `panel` surface.
    #[allow(clippy::too_many_arguments)]
    fn render_group_panel(
        &mut self,
        frame: &mut Frame,
        panel_rect: Rect,
        section: &Section,
        section_idx: usize,
        cols: usize,
        panel_top: usize,
        scroll: usize,
        body: Rect,
        selected: Option<usize>,
    ) {
        // Header strip (always at panel_top) — panel surface already filled.
        if panel_top >= scroll && panel_top < scroll + body.height as usize {
            let hy = body.y + (panel_top - scroll) as u16;
            if hy >= panel_rect.y && hy < panel_rect.y + panel_rect.height {
                let header = Rect {
                    x: panel_rect.x,
                    y: hy,
                    width: panel_rect.width,
                    height: 1,
                };
                self.render_section_header(frame, header, section);
                self.push_hitbox(header, HitAction::GroupHeader(section_idx));
            }
        }

        if section.collapsed || section.count == 0 {
            return;
        }

        for (rect, idx) in self.visible_tiles(section, cols, panel_rect, panel_top, scroll, body) {
            if idx >= self.instances.len() {
                continue;
            }
            let hovered = self.is_hovered(rect);
            let is_selected = selected == Some(idx);
            let instance = self.instances[idx].clone();
            self.render_tile(frame, rect, &instance, is_selected, hovered);
            self.push_hitbox(rect, HitAction::InstanceTile(idx));
        }
    }

    /// Screen rects + instance indices of cards visible in `panel_rect`.
    fn visible_tiles(
        &self,
        section: &Section,
        cols: usize,
        panel_rect: Rect,
        panel_top: usize,
        scroll: usize,
        body: Rect,
    ) -> Vec<(Rect, usize)> {
        let mut out = Vec::new();
        if section.collapsed || section.count == 0 {
            return out;
        }
        let tile_base = panel_top + HEADER_H;
        let cols = cols.max(1);
        let tile_rows = section.count.div_ceil(cols);
        let inner_x = panel_rect.x + SIDE_PAD;
        let inner_w = panel_rect.width.saturating_sub(SIDE_PAD);
        let max_cols = ((inner_w + GAP_X) / (TILE_W + GAP_X)).max(1) as usize;
        let cols = cols.min(max_cols);
        for r in 0..tile_rows {
            let row_y = tile_base + r * (TILE_H as usize + GAP_Y as usize);
            let row_end = row_y + TILE_H as usize;
            if row_end <= scroll || row_y >= scroll + body.height as usize {
                continue;
            }
            if row_y < scroll {
                continue;
            }
            let screen_y = body.y + (row_y - scroll) as u16;
            if screen_y + TILE_H > body.y + body.height {
                continue;
            }
            for c in 0..cols {
                let local = r * cols + c;
                if local >= section.count {
                    break;
                }
                let idx = section.start + local;
                if idx >= self.instances.len() {
                    break;
                }
                let rect = Rect {
                    x: inner_x + c as u16 * (TILE_W + GAP_X),
                    y: screen_y,
                    width: TILE_W,
                    height: TILE_H,
                };
                if rect.x + rect.width > panel_rect.x + panel_rect.width {
                    break;
                }
                if rect.y + rect.height > panel_rect.y + panel_rect.height {
                    continue;
                }
                out.push((rect, idx));
            }
        }
        out
    }

    /// Draw ASCII art. `clip = None` → page bg (fg = panel colour);
    /// `clip = Some(rect)` → only inside that panel (fg = darkest page bg).
    /// Cells under `skip` (instance cards) are never drawn.
    fn render_ascii_bg(
        &self,
        frame: &mut Frame,
        grid: Rect,
        clip: Option<Rect>,
        on_panel: bool,
        skip: &[Rect],
    ) {
        let Ok(art) = std::fs::read_to_string(self.paths.ascii_bg_file()) else {
            return;
        };
        let art = art.trim_end_matches('\n').to_string();
        if art.is_empty() {
            return;
        }
        let lines: Vec<&str> = art.split('\n').collect();
        let art_h = lines.len() as u16;
        let art_w = lines.iter().map(|l| l.len() as u16).max().unwrap_or(0);
        let (x_off, y_off) = match self.settings.ascii_bg_anchor {
            AsciiBgAnchor::TopLeft => (0, 0),
            AsciiBgAnchor::TopRight => (grid.width.saturating_sub(art_w), 0),
            AsciiBgAnchor::BottomLeft => (0, grid.height.saturating_sub(art_h)),
            AsciiBgAnchor::BottomRight => (
                grid.width.saturating_sub(art_w),
                grid.height.saturating_sub(art_h),
            ),
        };
        let (fg, bg) = if on_panel {
            (self.theme.bg, self.theme.panel)
        } else {
            (self.theme.panel, self.theme.bg)
        };
        let style = Style::default().fg(fg).bg(bg);
        for (i, line) in lines.iter().enumerate() {
            let y = grid.y + y_off + i as u16;
            if y >= grid.y + grid.height {
                break;
            }
            let line_x = grid.x + SIDE_PAD + x_off;
            let line_w = line.chars().count() as u16;
            if line_w == 0 {
                continue;
            }
            let line_rect = Rect {
                x: line_x,
                y,
                width: line_w,
                height: 1,
            };
            let draw = match clip {
                Some(c) => intersect_rect(line_rect, c),
                None => intersect_rect(line_rect, grid),
            };
            if draw.width == 0 || draw.height == 0 {
                continue;
            }
            let start = draw.x.saturating_sub(line_x) as usize;
            let chars: Vec<char> = line.chars().collect();
            // Render only runs of cells that are not under an instance card.
            let mut seg_start = 0u16;
            while seg_start < draw.width {
                let x = draw.x + seg_start;
                let y = draw.y;
                if skip.iter().any(|s| rect_contains(*s, (x, y))) {
                    seg_start += 1;
                    continue;
                }
                let mut seg_len = 1u16;
                while seg_start + seg_len < draw.width {
                    let sx = draw.x + seg_start + seg_len;
                    if skip.iter().any(|s| rect_contains(*s, (sx, y))) {
                        break;
                    }
                    seg_len += 1;
                }
                let char_start = start + seg_start as usize;
                let slice: String = chars
                    .iter()
                    .skip(char_start)
                    .take(seg_len as usize)
                    .collect();
                if !slice.is_empty() {
                    let seg = Rect {
                        x: draw.x + seg_start,
                        y: draw.y,
                        width: seg_len,
                        height: 1,
                    };
                    frame.render_widget(
                        Paragraph::new(Span::styled(slice, style))
                            .style(Style::default().bg(bg)),
                        seg,
                    );
                }
                seg_start += seg_len;
            }
        }
    }

    /// Header strip: `▼ Name  3` / `▶ Name  3` — plain panel bg, no highlight.
    fn render_section_header(&self, frame: &mut Frame, rect: Rect, section: &Section) {
        let bg = self.theme.panel;
        let surface = Style::default().bg(bg);
        fill_rect(frame, rect, surface);

        let label = if section.name.is_empty() {
            self.tr("builds.ungrouped").to_string()
        } else {
            section.name.clone()
        };
        let chevron = if section.collapsed { "▶" } else { "▼" };
        let count_str = format!("{}", section.count);
        let w = rect.width as usize;
        if w == 0 {
            return;
        }

        let name = truncate(&label, w.saturating_sub(chevron.chars().count() + count_str.len() + 5));
        let line = Line::from(vec![
            Span::styled(
                format!(" {chevron} "),
                Style::default().fg(self.theme.muted).bg(bg),
            ),
            Span::styled(name, self.theme.header()),
            Span::styled(
                format!("  {count_str}"),
                Style::default().fg(self.theme.muted).bg(bg),
            ),
        ]);
        frame.render_widget(Paragraph::new(line).style(surface), rect);
    }

    fn render_tile(
        &self,
        frame: &mut Frame,
        rect: Rect,
        instance: &Instance,
        selected: bool,
        hovered: bool,
    ) {
        let bg = if selected {
            self.theme.selection_bg
        } else if hovered {
            self.theme.hover_bg
        } else {
            self.theme.panel_alt
        };
        let surface = Style::default().bg(bg);
        fill_rect(frame, rect, surface);
        crate::views::accent_bar(frame, rect, &self.theme);
        let pad_left: u16 = 3;
        if rect.width < pad_left + 3 || rect.height < CONTENT_H {
            return;
        }

        let top = rect.y + rect.height.saturating_sub(CONTENT_H) / 2;
        let cw = rect.width - pad_left;

        let short = short_name(instance.name());
        let box_w = (short.chars().count() as u16 + 4).max(6).min(cw);
        let box_x = rect.x + rect.width.saturating_sub(box_w) / 2;
        let box_rect = Rect {
            x: box_x,
            y: top,
            width: box_w,
            height: 3,
        };
        let box_border = if selected {
            self.theme.green_bright
        } else {
            self.theme.border
        };
        let logo_box = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(box_border))
            .style(surface);
        let logo_inner = logo_box.inner(box_rect);
        frame.render_widget(logo_box, box_rect);
        let short_style = if selected {
            self.theme.accent_bright()
        } else {
            Style::default().fg(self.theme.fg).bg(bg)
        };
        frame.render_widget(
            Paragraph::new(Span::styled(short, short_style))
                .alignment(Alignment::Center)
                .style(surface),
            logo_inner,
        );

        let title_style = if selected {
            self.theme.accent_bright()
        } else {
            Style::default()
                .fg(self.theme.fg)
                .bg(bg)
                .add_modifier(Modifier::BOLD)
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                truncate(instance.name(), cw as usize),
                title_style,
            ))
            .alignment(Alignment::Center)
            .style(surface),
            Rect {
                x: rect.x,
                y: top + 3,
                width: rect.width,
                height: 1,
            },
        );

        let meta = format!(
            "{} / {}",
            instance.metadata.game_version,
            instance.metadata.loader.as_str()
        );
        frame.render_widget(
            Paragraph::new(Span::styled(
                truncate(&meta, cw as usize),
                Style::default().fg(self.theme.muted).bg(bg),
            ))
            .alignment(Alignment::Center)
            .style(surface),
            Rect {
                x: rect.x,
                y: top + 4,
                width: rect.width,
                height: 1,
            },
        );

        let status = if instance.is_linked() {
            match &instance.metadata.modpack {
                Some(pack) => format!("🔗 {} (linked)", pack.name),
                None => format!("🔗 {}", self.tr("builds.linked")),
            }
        } else {
            match &instance.metadata.modpack {
                Some(pack) => format!("⛁ {}", pack.name),
                None => self.tr("builds.ready").to_string(),
            }
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                truncate(&status, cw as usize),
                self.theme.accent(),
            ))
            .alignment(Alignment::Center)
            .style(surface),
            Rect {
                x: rect.x,
                y: top + 5,
                width: rect.width,
                height: 1,
            },
        );
    }

    /// Keyboard navigation for the instance grid (section-aware, collapse-aware).
    pub(crate) fn key_instance_grid(&mut self, key: KeyEvent) {
        let len = self.instances.len();
        if len == 0 {
            match key.code {
                KeyCode::Enter | KeyCode::Char('n') => self.open_create_instance_form(),
                KeyCode::Char('y') => self.open_new_group_form(),
                KeyCode::Char('c') => self.toggle_selected_group_collapsed(),
                _ => {}
            }
            return;
        }
        let current = self.instance_state.selected().unwrap_or(0);
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => self.move_grid_vert(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_grid_vert(-1),
            KeyCode::Right | KeyCode::Char('l') => self.move_grid_horiz(1),
            KeyCode::Left | KeyCode::Char('h') => self.move_grid_horiz(-1),
            KeyCode::PageDown => {
                let steps = self.tile_visible_rows.max(1);
                for _ in 0..steps {
                    self.move_grid_vert(1);
                }
            }
            KeyCode::PageUp => {
                let steps = self.tile_visible_rows.max(1);
                for _ in 0..steps {
                    self.move_grid_vert(-1);
                }
            }
            KeyCode::Char('g') => {
                self.select_instance(current);
                self.open_group_picker();
            }
            KeyCode::Char('G') => {
                self.instance_state.select(Some(len - 1));
            }
            KeyCode::Char('c') => self.toggle_selected_group_collapsed(),
            KeyCode::Char('y') => self.open_new_group_form(),
            KeyCode::Char('Y') => self.open_rename_group_form(),
            KeyCode::Char('D') => self.confirm_delete_group(),
            KeyCode::Enter => {
                self.select_instance(current);
                self.launch_selected();
            }
            KeyCode::Char('n') => self.open_create_instance_form(),
            KeyCode::Char('e') => {
                self.select_instance(current);
                self.open_nav(Nav::Jvm);
            }
            KeyCode::Char('i') => {
                self.select_instance(current);
                self.install_selected_instance();
            }
            KeyCode::Char('p') => self.open_import_prompt(),
            KeyCode::Char('v') => {
                self.select_instance(current);
                self.open_change_version_picker();
            }
            KeyCode::Char('r') => {
                self.select_instance(current);
                self.open_rename_instance_form();
            }
            KeyCode::Char('d') => {
                self.select_instance(current);
                self.confirm_delete_instance();
            }
            _ => {}
        }
    }

    /// Move the selection by whole visual rows across section boundaries.
    /// Collapsed panels are skipped (tiles are not visible).
    pub(crate) fn move_grid_vert(&mut self, delta_rows: i32) {
        let steps = delta_rows.unsigned_abs();
        if delta_rows == 0 {
            return;
        }
        for _ in 0..steps {
            if delta_rows > 0 {
                self.grid_step_down();
            } else {
                self.grid_step_up();
            }
        }
    }

    /// Move left/right inside the current section's row; at the edge step to
    /// the adjacent section on the same visual row (flow layout).
    pub(crate) fn move_grid_horiz(&mut self, delta: i32) {
        let Some(current) = self.instance_state.selected() else {
            return;
        };
        let sections = self.instance_sections();
        let boxes = layout_panels(&sections, self.tile_body_w);
        let Some((si, local)) = locate_in_sections(&sections, current) else {
            return;
        };
        let section = &sections[si];
        if section.count == 0 || section.collapsed {
            return;
        }
        let Some(b) = boxes.iter().find(|b| b.section == si) else {
            return;
        };
        let cols = b.cols.max(1);
        let col = local % cols;
        if delta > 0 {
            if col + 1 < cols && local + 1 < section.count {
                self.instance_state
                    .select(Some(section.start + local + 1));
                self.focus = crate::app::Focus::Content;
                return;
            }
            // Next expanded section on the same row, to the right.
            let next = boxes
                .iter()
                .filter(|o| {
                    o.section != si
                        && o.y == b.y
                        && o.x > b.x
                        && !sections[o.section].collapsed
                        && sections[o.section].count > 0
                })
                .min_by_key(|o| o.x);
            if let Some(n) = next {
                let ns = &sections[n.section];
                let target = col.min(ns.count - 1);
                self.instance_state.select(Some(ns.start + target));
                self.focus = crate::app::Focus::Content;
            }
        } else if col > 0 && local > 0 {
            self.instance_state.select(Some(section.start + local - 1));
            self.focus = crate::app::Focus::Content;
        } else {
            // Previous expanded section on the same row, to the left.
            let prev = boxes
                .iter()
                .filter(|o| {
                    o.section != si
                        && o.y == b.y
                        && o.x < b.x
                        && !sections[o.section].collapsed
                        && sections[o.section].count > 0
                })
                .max_by_key(|o| o.x);
            if let Some(p) = prev {
                let ps = &sections[p.section];
                let p_cols = p.cols.max(1);
                let col_p = col.min(p_cols - 1);
                let rows = ps.count.div_ceil(p_cols);
                let last_row_start = (rows - 1) * p_cols;
                let target = if last_row_start + col_p < ps.count {
                    last_row_start + col_p
                } else {
                    ps.count - 1
                };
                self.instance_state.select(Some(ps.start + target));
                self.focus = crate::app::Focus::Content;
            }
        }
    }

    fn grid_step_down(&mut self) {
        let current = match self.instance_state.selected() {
            Some(i) => i,
            None => {
                // Land on the first tile of the first expanded section.
                let sections = self.instance_sections();
                if let Some(s) = sections.iter().find(|s| !s.collapsed && s.count > 0) {
                    self.instance_state.select(Some(s.start));
                }
                return;
            }
        };
        let sections = self.instance_sections();
        let boxes = layout_panels(&sections, self.tile_body_w);
        let Some((si, local)) = locate_in_sections(&sections, current) else {
            return;
        };
        let section = &sections[si];
        if section.collapsed || section.count == 0 {
            return;
        }
        let Some(b) = boxes.iter().find(|b| b.section == si) else {
            return;
        };
        let cols = b.cols.max(1);
        if local + cols < section.count {
            self.instance_state
                .select(Some(section.start + local + cols));
            return;
        }
        // Section visually below: prefer same column, then nearest x.
        let col = local % cols;
        let below = boxes
            .iter()
            .filter(|o| {
                o.section != si
                    && o.y > b.y
                    && !sections[o.section].collapsed
                    && sections[o.section].count > 0
            })
            .min_by_key(|o| {
                let dy = o.y - b.y;
                let dx = o.x.abs_diff(b.x);
                (dy, dx)
            });
        if let Some(n) = below {
            let ns = &sections[n.section];
            let target = col.min(ns.count - 1);
            // Prefer first row at same column (step down ≈ next row).
            self.instance_state.select(Some(ns.start + target));
        }
    }

    fn grid_step_up(&mut self) {
        let current = match self.instance_state.selected() {
            Some(i) => i,
            None => return,
        };
        let sections = self.instance_sections();
        let boxes = layout_panels(&sections, self.tile_body_w);
        let Some((si, local)) = locate_in_sections(&sections, current) else {
            return;
        };
        let section = &sections[si];
        if section.collapsed || section.count == 0 {
            return;
        }
        let Some(b) = boxes.iter().find(|b| b.section == si) else {
            return;
        };
        let cols = b.cols.max(1);
        if local >= cols {
            self.instance_state
                .select(Some(section.start + local - cols));
            return;
        }
        // Section visually above: prefer same column, then nearest x.
        let col = local % cols;
        let above = boxes
            .iter()
            .filter(|o| {
                o.section != si
                    && o.y < b.y
                    && !sections[o.section].collapsed
                    && sections[o.section].count > 0
            })
            .max_by_key(|o| {
                let dy = b.y - o.y;
                let dx = o.x.abs_diff(b.x);
                // Closer row wins; among equal distance prefer larger y (already max).
                (std::cmp::Reverse(dy), std::cmp::Reverse(dx))
            });
        if let Some(p) = above {
            let ps = &sections[p.section];
            let p_cols = p.cols.max(1);
            let rows = ps.count.div_ceil(p_cols);
            let last_row_start = (rows - 1) * p_cols;
            let target = col.min(p_cols - 1);
            let local_t = if last_row_start + target < ps.count {
                last_row_start + target
            } else {
                ps.count - 1
            };
            self.instance_state.select(Some(ps.start + local_t));
        }
    }

    /// True when the vertical wheel should move the grid selection.
    pub(crate) fn grid_has_selection(&self) -> bool {
        self.instance_state.selected().is_some() || !self.instances.is_empty()
    }
}

/// Sections in display order: ungrouped first (from sorted instances), then
/// named groups; empty registry groups are appended so they stay visible.
pub(crate) fn sections_from(instances: &[Instance], groups: &[String]) -> Vec<Section> {
    let mut sections = Vec::new();
    let mut i = 0;
    while i < instances.len() {
        let g = instances[i].metadata.group.clone();
        let start = i;
        while i < instances.len() && instances[i].metadata.group == g {
            i += 1;
        }
        sections.push(Section {
            name: g,
            start,
            count: i - start,
            collapsed: false,
        });
    }
    for g in groups {
        if !sections.iter().any(|s| s.name == *g) {
            sections.push(Section {
                name: g.clone(),
                start: instances.len(),
                count: 0,
                collapsed: false,
            });
        }
    }
    sections
}

fn locate_in_sections(sections: &[Section], idx: usize) -> Option<(usize, usize)> {
    sections
        .iter()
        .enumerate()
        .find(|(_, s)| s.count > 0 && idx >= s.start && idx < s.start + s.count)
        .map(|(si, s)| (si, idx - s.start))
}

/// A placed group panel in content coordinates (before viewport scroll).
#[derive(Debug, Clone)]
pub(crate) struct PanelBox {
    pub section: usize,
    pub x: u16,
    pub y: usize,
    pub width: u16,
    pub height: usize,
    pub cols: usize,
}

/// Dark page-bg gap between horizontally adjacent group panels.
const H_GAP: u16 = SIDE_PAD;

/// Flow-layout group panels: left→right, wrap to the next row when full.
pub(crate) fn layout_panels(sections: &[Section], body_w: u16) -> Vec<PanelBox> {
    if body_w == 0 || sections.is_empty() {
        return Vec::new();
    }
    let max_w = body_w.saturating_sub(SIDE_PAD);
    if max_w < 4 {
        return Vec::new();
    }
    let full_cols = (((max_w + GAP_X) / (TILE_W + GAP_X)) as usize).max(1);
    let mut out = Vec::with_capacity(sections.len());
    let mut x = SIDE_PAD;
    let mut y = 0usize;
    let mut row_h = 0usize;
    for (i, s) in sections.iter().enumerate() {
        let w = panel_width(s, full_cols, max_w);
        let cols = cols_for_width(w);
        let h = panel_height(s, cols);
        if i > 0 && x.saturating_add(w) > body_w {
            y = y.saturating_add(row_h + PANEL_GAP);
            x = SIDE_PAD;
            row_h = 0;
        }
        out.push(PanelBox {
            section: i,
            x,
            y,
            width: w,
            height: h,
            cols,
        });
        x = x.saturating_add(w + H_GAP);
        row_h = row_h.max(h);
    }
    out
}

/// Tile columns that fit inside a panel of width `w`.
fn cols_for_width(w: u16) -> usize {
    let inner = w.saturating_sub(SIDE_PAD * 2);
    (((inner + GAP_X) / (TILE_W + GAP_X)) as usize).max(1)
}

/// Height of one group panel: header (+ tile rows + bottom pad when expanded).
pub(crate) fn panel_height(section: &Section, cols: usize) -> usize {
    if section.collapsed || section.count == 0 {
        return HEADER_H;
    }
    let rows = section.count.div_ceil(cols.max(1));
    HEADER_H
        + rows * TILE_H as usize
        + rows.saturating_sub(1) * GAP_Y as usize
        + BOTTOM_PAD
}

/// Panel width: 2-col gutters + widest row of tiles (with gaps), clamped to `max_w`.
/// Collapsed/empty groups keep the width their tiles would need when expanded.
pub(crate) fn panel_width(section: &Section, cols: usize, max_w: u16) -> u16 {
    let n = if section.count == 0 {
        1
    } else {
        section.count.min(cols.max(1))
    } as u16;
    let w = SIDE_PAD * 2 + n * TILE_W + n.saturating_sub(1) * GAP_X;
    w.min(max_w).max((SIDE_PAD * 2 + TILE_W).min(max_w))
}

/// Content-line height of the scrollable Builds body (flow rows + gaps).
pub(crate) fn content_height(boxes: &[PanelBox]) -> usize {
    boxes.iter().map(|b| b.y + b.height).max().unwrap_or(0)
}

/// Absolute content Y of an instance's tile top (header when its panel is collapsed).
pub(crate) fn instance_content_y(
    boxes: &[PanelBox],
    sections: &[Section],
    idx: usize,
) -> Option<usize> {
    let (si, _local) = locate_in_sections(sections, idx)?;
    let b = boxes.iter().find(|b| b.section == si)?;
    let s = &sections[si];
    if s.collapsed || s.count == 0 {
        return Some(b.y);
    }
    let local = idx - s.start;
    let cols = b.cols.max(1);
    let row = local / cols;
    Some(b.y + HEADER_H + row * (TILE_H as usize + GAP_Y as usize))
}

/// Snap a scroll offset back to a panel-row top or tile-row start.
pub(crate) fn snap_scroll(
    boxes: &[PanelBox],
    sections: &[Section],
    scroll: usize,
) -> usize {
    let mut ys: Vec<usize> = vec![0];
    for b in boxes {
        ys.push(b.y);
        let s = &sections[b.section];
        if s.collapsed || s.count == 0 {
            continue;
        }
        ys.push(b.y + HEADER_H);
        let rows = s.count.div_ceil(b.cols.max(1));
        for r in 0..rows {
            ys.push(b.y + HEADER_H + r * (TILE_H as usize + GAP_Y as usize));
        }
    }
    ys.sort_unstable();
    ys.dedup();
    let mut best = 0usize;
    for y in ys {
        if y > scroll {
            return best;
        }
        best = y;
    }
    best
}

fn intersect_rect(a: Rect, b: Rect) -> Rect {
    let x1 = a.x.max(b.x);
    let y1 = a.y.max(b.y);
    let x2 = a.x.saturating_add(a.width).min(b.x.saturating_add(b.width));
    let y2 = a.y.saturating_add(a.height).min(b.y.saturating_add(b.height));
    if x2 <= x1 || y2 <= y1 {
        return Rect::default();
    }
    Rect {
        x: x1,
        y: y1,
        width: x2 - x1,
        height: y2 - y1,
    }
}

/// Fill `rect` with spaces + `style` so underlying symbols cannot bleed through.
fn fill_rect(frame: &mut Frame, rect: Rect, style: Style) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let buf = frame.buffer_mut();
    for y in rect.y..rect.y.saturating_add(rect.height) {
        for x in rect.x..rect.x.saturating_add(rect.width) {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ");
                cell.set_style(style);
            }
        }
    }
}

/// A two-character uppercase short name, e.g. `test1` -> `TE`.
fn short_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .take(2)
        .collect::<String>()
        .to_ascii_uppercase()
}
