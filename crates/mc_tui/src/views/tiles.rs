//! Instance picker as separate collapsible group panels on the dark page bg.

use crossterm::event::{KeyCode, KeyEvent};
use mc_core::instance::Instance;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;

use crate::app::{App, ButtonId, HitAction};
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
/// Dark page-bg gutter on the left of every group panel.
const SIDE_PAD: u16 = 2;
/// Dark page-bg rows between group panels (same look as SIDE_PAD).
const PANEL_GAP: usize = 1;

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

    /// Dark page background + one flat panel per group, separated by bg gutters.
    fn render_group_panels(&mut self, frame: &mut Frame, area: Rect) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        // Darkest page colour shows through SIDE_PAD and PANEL_GAP.
        frame.render_widget(
            Block::default().style(Style::default().bg(self.theme.bg)),
            area,
        );

        let title_area = Rect {
            x: area.x + SIDE_PAD,
            y: area.y,
            width: area.width.saturating_sub(SIDE_PAD),
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(crate::views::section_title(
                self.tr("builds.title"),
                &format!("{}", self.instances.len()),
                &self.theme,
            ))
            .style(Style::default().bg(self.theme.bg)),
            title_area,
        );

        let body = Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: area.height.saturating_sub(1),
        };
        if body.height == 0 {
            return;
        }

        // Panel column: 2-char left gutter, flush to the right edge.
        let panel_x = body.x + SIDE_PAD;
        let panel_w = body.width.saturating_sub(SIDE_PAD);
        if panel_w < 4 {
            return;
        }

        let cols = ((panel_w + GAP_X) / (TILE_W + GAP_X)).max(1) as usize;
        self.tile_columns = cols;
        let view_h = body.height as usize;
        self.ensure_tile_visible(view_h);

        // ASCII art sits on the page bg behind the panels.
        self.render_ascii_bg(frame, body);

        let sections = self.instance_sections();
        let scroll = self.tile_scroll;
        let selected = self.instance_state.selected();
        let mut y_content = 0usize;

        for (si, section) in sections.iter().enumerate() {
            if si > 0 {
                y_content += PANEL_GAP;
            }
            let panel_h = panel_height(section, cols);
            let panel_top = y_content;
            y_content += panel_h;

            // Clip panel to the viewport.
            if panel_top >= scroll + view_h {
                break;
            }
            if panel_top + panel_h <= scroll {
                continue;
            }

            let vis_start = panel_top.max(scroll);
            let vis_end = (panel_top + panel_h).min(scroll + view_h);
            if vis_start >= vis_end {
                continue;
            }

            let screen_y = body.y + (vis_start - scroll) as u16;
            let screen_h = (vis_end - vis_start) as u16;
            let panel_rect = Rect {
                x: panel_x,
                y: screen_y,
                width: panel_w,
                height: screen_h,
            };

            let active = section.name == self.selected_group
                || (self.selected_group.is_empty()
                    && section.name.is_empty()
                    && selected.is_some_and(|i| {
                        i >= section.start && i < section.start + section.count
                    }));
            self.render_group_panel(
                frame,
                panel_rect,
                section,
                si,
                cols,
                panel_top,
                scroll,
                body,
                active,
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
        active: bool,
        selected: Option<usize>,
    ) {
        // Panel surface (group bg colour — unchanged from the old cards).
        frame.render_widget(
            Block::default().style(Style::default().bg(self.theme.panel)),
            panel_rect,
        );

        // Header strip (always at panel_top).
        if panel_top >= scroll && panel_top < scroll + body.height as usize {
            let hy = body.y + (panel_top - scroll) as u16;
            if hy >= panel_rect.y && hy < panel_rect.y + panel_rect.height {
                let header = Rect {
                    x: panel_rect.x,
                    y: hy,
                    width: panel_rect.width,
                    height: 1,
                };
                let hovered = self.is_hovered(header);
                self.render_section_header(frame, header, section, active, hovered);
                self.push_hitbox(header, HitAction::GroupHeader(section_idx));
            }
        }

        if section.collapsed || section.count == 0 {
            return;
        }

        // Tile rows below the header.
        let tile_base = panel_top + HEADER_H;
        let tile_rows = section.count.div_ceil(cols);
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
            // Keep tiles inside the panel's horizontal bounds.
            let inner_x = panel_rect.x + 1;
            let inner_w = panel_rect.width.saturating_sub(1);
            let max_cols = ((inner_w + GAP_X) / (TILE_W + GAP_X)).max(1) as usize;
            let cols = cols.min(max_cols);
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
                let hovered = self.is_hovered(rect);
                let is_selected = selected == Some(idx);
                let instance = self.instances[idx].clone();
                self.render_tile(frame, rect, &instance, is_selected, hovered);
                self.push_hitbox(rect, HitAction::InstanceTile(idx));
            }
        }
    }

    fn render_ascii_bg(&self, frame: &mut Frame, grid: Rect) {
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
        let dim = Style::default().fg(self.theme.muted).bg(self.theme.bg);
        for (i, line) in lines.iter().enumerate() {
            let y = grid.y + y_off + i as u16;
            if y >= grid.y + grid.height {
                break;
            }
            let x = grid.x + SIDE_PAD + x_off;
            let max_w = (grid.x + grid.width).saturating_sub(x);
            if max_w == 0 {
                break;
            }
            frame.render_widget(
                Paragraph::new(Span::styled(truncate(line, max_w as usize), dim))
                    .style(Style::default().bg(self.theme.bg)),
                Rect {
                    x,
                    y,
                    width: line.len().min(max_w.into()) as u16,
                    height: 1,
                },
            );
        }
    }

    /// Header strip: `▼ Name  3` / `▶ Name  3`.
    fn render_section_header(
        &self,
        frame: &mut Frame,
        rect: Rect,
        section: &Section,
        active: bool,
        hovered: bool,
    ) {
        let bg = if active {
            self.theme.selection_bg
        } else if hovered {
            self.theme.hover_bg
        } else {
            self.theme.panel
        };
        let surface = Style::default().bg(bg);
        frame.render_widget(Block::default().style(surface), rect);

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

        let name_style = if active {
            self.theme.accent_bright()
        } else {
            self.theme.header()
        };
        let count_style = if active {
            self.theme.accent()
        } else {
            Style::default().fg(self.theme.muted).bg(bg)
        };
        let chev_style = if active {
            self.theme.accent()
        } else {
            Style::default().fg(self.theme.muted).bg(bg)
        };

        // Flat: `▼ Name  3` — no dash filler.
        let name = truncate(&label, w.saturating_sub(chevron.chars().count() + count_str.len() + 5));
        let line = Line::from(vec![
            Span::styled(format!(" {chevron} "), chev_style),
            Span::styled(name, name_style),
            Span::styled(format!("  {count_str}"), count_style),
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
        frame.render_widget(Block::default().style(surface), rect);
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
                self.open_edit_instance_form();
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

    /// Move left/right inside the current section's row (does not cross sections).
    pub(crate) fn move_grid_horiz(&mut self, delta: i32) {
        let cols = self.tile_columns.max(1);
        let Some(current) = self.instance_state.selected() else {
            return;
        };
        let sections = self.instance_sections();
        let Some((si, local)) = locate_in_sections(&sections, current) else {
            return;
        };
        let section = &sections[si];
        if section.count == 0 || section.collapsed {
            return;
        }
        let col = local % cols;
        if delta > 0 {
            if col + 1 < cols && local + 1 < section.count {
                self.instance_state
                    .select(Some(section.start + local + 1));
                self.focus = crate::app::Focus::Content;
            }
        } else if col > 0 && local > 0 {
            self.instance_state.select(Some(section.start + local - 1));
            self.focus = crate::app::Focus::Content;
        }
    }

    fn grid_step_down(&mut self) {
        let cols = self.tile_columns.max(1);
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
        let Some((si, local)) = locate_in_sections(&sections, current) else {
            return;
        };
        let section = &sections[si];
        if !section.collapsed && local + cols < section.count {
            self.instance_state
                .select(Some(section.start + local + cols));
            return;
        }
        // Next expanded section with tiles: same column if possible.
        let col = local % cols;
        for next in sections.iter().skip(si + 1) {
            if next.count == 0 || next.collapsed {
                continue;
            }
            let target = if next.count > col { col } else { next.count - 1 };
            self.instance_state.select(Some(next.start + target));
            return;
        }
    }

    fn grid_step_up(&mut self) {
        let cols = self.tile_columns.max(1);
        let current = match self.instance_state.selected() {
            Some(i) => i,
            None => return,
        };
        let sections = self.instance_sections();
        let Some((si, local)) = locate_in_sections(&sections, current) else {
            return;
        };
        let section = &sections[si];
        if !section.collapsed && local >= cols {
            self.instance_state
                .select(Some(section.start + local - cols));
            return;
        }
        // Previous expanded section: same column on its last row.
        let col = local % cols;
        for prev in sections.iter().take(si).rev() {
            if prev.count == 0 || prev.collapsed {
                continue;
            }
            let rows = prev.count.div_ceil(cols);
            let last_row_start = (rows - 1) * cols;
            let target = if prev.count > last_row_start + col {
                last_row_start + col
            } else {
                prev.count - 1
            };
            self.instance_state.select(Some(prev.start + target));
            return;
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

/// Height of one group panel: header (+ tile rows when expanded).
pub(crate) fn panel_height(section: &Section, cols: usize) -> usize {
    if section.collapsed || section.count == 0 {
        return HEADER_H;
    }
    let rows = section.count.div_ceil(cols.max(1));
    HEADER_H + rows * TILE_H as usize + rows.saturating_sub(1) * GAP_Y as usize
}

/// Content-line height of the scrollable Builds body (panels + gaps).
pub(crate) fn content_height(sections: &[Section], cols: usize) -> usize {
    let cols = cols.max(1);
    let mut h = 0usize;
    for (i, s) in sections.iter().enumerate() {
        if i > 0 {
            h += PANEL_GAP;
        }
        h += panel_height(s, cols);
    }
    h
}

/// Absolute content Y of an instance's tile top (header when its panel is collapsed).
pub(crate) fn instance_content_y(sections: &[Section], cols: usize, idx: usize) -> Option<usize> {
    let cols = cols.max(1);
    let (si, local) = locate_in_sections(sections, idx)?;
    let mut y = 0usize;
    for (i, s) in sections.iter().enumerate() {
        if i > 0 {
            y += PANEL_GAP;
        }
        if i == si {
            if s.collapsed || s.count == 0 {
                return Some(y);
            }
            y += HEADER_H;
            let row = local / cols;
            y += row * (TILE_H as usize + GAP_Y as usize);
            return Some(y);
        }
        y += panel_height(s, cols);
    }
    None
}

/// Snap a scroll offset back to a panel top or tile-row start.
pub(crate) fn snap_scroll(sections: &[Section], cols: usize, scroll: usize) -> usize {
    let cols = cols.max(1);
    let mut best = 0usize;
    let mut y = 0usize;
    for (i, s) in sections.iter().enumerate() {
        if i > 0 {
            y += PANEL_GAP;
            if y > scroll {
                return best;
            }
            // Gap start is not a snap target; panel top is.
            best = y;
            if y > scroll {
                return best;
            }
        }
        // Panel top / header.
        if y > scroll {
            return best;
        }
        best = y;
        if !s.collapsed && s.count > 0 {
            y += HEADER_H;
            let rows = s.count.div_ceil(cols);
            for r in 0..rows {
                if y > scroll {
                    return best;
                }
                best = y;
                if r + 1 < rows {
                    y += TILE_H as usize + GAP_Y as usize;
                } else {
                    y += TILE_H as usize;
                }
            }
        } else {
            y += panel_height(s, cols);
        }
    }
    if y <= scroll {
        best = y;
    }
    best
}

/// A two-character uppercase short name, e.g. `test1` -> `TE`.
fn short_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .take(2)
        .collect::<String>()
        .to_ascii_uppercase()
}
