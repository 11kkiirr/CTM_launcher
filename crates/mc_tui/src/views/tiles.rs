//! Instance picker rendered as compact, squarish cards in the main content area.

use crossterm::event::{KeyCode, KeyEvent};
use mc_core::instance::Instance;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;

use crate::app::{App, ButtonId, Focus, HitAction};
use crate::settings::AsciiBgAnchor;
use crate::views::{pill_row, truncate};

/// Cards are wider than tall, but close to square once cell aspect is taken
/// into account.
const TILE_W: u16 = 24;
const TILE_H: u16 = 10;
const GAP_X: u16 = 2;
const GAP_Y: u16 = 1;
/// Height of the vertically centered inner content block.
const CONTENT_H: u16 = 6;

impl App {
    /// Render the build action toolbar plus the grid of instance cards.
    pub(crate) fn render_instance_grid(&mut self, frame: &mut Frame, area: Rect) {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(3),
            ])
            .split(area);

        pill_row(
            self,
            frame,
            rows[0].x + 2,
            rows[0].y,
            area.x + area.width,
            &[
                ("Launch", "Enter", ButtonId::Launch),
                ("+ New", "n", ButtonId::NewInstance),
                ("Edit", "e", ButtonId::EditInstance),
                ("Install / Repair", "i", ButtonId::InstallInstance),
                ("Import", "p", ButtonId::ImportModpack),
                ("Versions", "v", ButtonId::ChangeVersion),
                ("Rename", "r", ButtonId::RenameInstance),
                ("Delete", "d", ButtonId::DeleteInstance),
            ],
        );

        let focused = self.focus == Focus::Content;
        let inner = crate::views::card(self, frame, rows[2], focused);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let title_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(crate::views::section_title(
                "Builds",
                &format!("{}", self.instances.len()),
                &self.theme,
            ))
            .style(self.theme.card()),
            title_area,
        );

        let grid = Rect {
            x: inner.x + 1,
            y: inner.y + 2,
            width: inner.width.saturating_sub(1),
            height: inner.height.saturating_sub(2),
        };
        if grid.width < TILE_W || grid.height < TILE_H {
            frame.render_widget(
                Paragraph::new(Span::styled("…", self.theme.card_dim())).style(self.theme.card()),
                grid,
            );
            return;
        }

        let cols = ((grid.width + GAP_X) / (TILE_W + GAP_X)).max(1) as usize;
        let visible_rows = ((grid.height + GAP_Y) / (TILE_H + GAP_Y)).max(1) as usize;
        self.tile_columns = cols;
        self.ensure_tile_visible(visible_rows);

        // Draw the ASCII-art background behind the tiles.
        if let Ok(art) = std::fs::read_to_string(self.paths.ascii_bg_file()) {
            let art = art.trim_end_matches('\n').to_string();
            if !art.is_empty() {
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
                let dim = Style::default().fg(self.theme.muted);
                for (i, line) in lines.iter().enumerate() {
                    let y = grid.y + y_off + i as u16;
                    if y >= grid.y + grid.height {
                        break;
                    }
                    let x = grid.x + x_off;
                    let max_w = grid.x + grid.width - x;
                    frame.render_widget(
                        Paragraph::new(Span::styled(
                            truncate(line, max_w as usize),
                            dim,
                        ))
                        .style(self.theme.card()),
                        Rect {
                            x,
                            y,
                            width: line.len().min(max_w.into()) as u16,
                            height: 1,
                        },
                    );
                }
            }
        }

        let total = self.instances.len();
        let selected = self.instance_state.selected();

        for row in 0..visible_rows {
            for col in 0..cols {
                let idx = (self.tile_scroll + row) * cols + col;
                if idx >= total {
                    break;
                }
                let rect = tile_rect(grid, row, col);
                let hovered = self.is_hovered(rect);
                let is_selected = selected == Some(idx);
                let instance = self.instances[idx].clone();
                self.render_tile(frame, rect, &instance, is_selected, hovered);
                self.push_hitbox(rect, HitAction::InstanceTile(idx));
            }
        }

        // "New Build" tile right after the last instance.
        let add_row = total / cols;
        let add_col = total % cols;
        if add_row >= self.tile_scroll && add_row < self.tile_scroll + visible_rows {
            let rect = tile_rect(grid, add_row - self.tile_scroll, add_col);
            let hovered = self.is_hovered(rect);
            self.render_add_tile(frame, rect, hovered);
            self.push_hitbox(rect, HitAction::AddInstance);
        }
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
        // Left accent bar on every tile.
        crate::views::accent_bar(frame, rect, &self.theme);
        // 2-char left indent after the bar.
        let pad_left: u16 = 3;
        if rect.width < pad_left + 3 || rect.height < CONTENT_H {
            return;
        }

        let top = rect.y + rect.height.saturating_sub(CONTENT_H) / 2;
        let cw = rect.width - pad_left;

        // 1) Centered logo / short-name box. The outline stays.
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

        // 2) Instance title (bold).
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

        // 3) Loader & version on one muted line (blends with the card).
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

        // 4) Game status.
        let status = match &instance.metadata.modpack {
            Some(pack) => format!("⛁ {}", pack.name),
            None => "Ready to play".to_string(),
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

    fn render_add_tile(&self, frame: &mut Frame, rect: Rect, hovered: bool) {
        let bg = if hovered {
            self.theme.selection_bg
        } else {
            self.theme.panel_alt
        };
        let surface = Style::default().bg(bg);
        frame.render_widget(Block::default().style(surface), rect);
        crate::views::accent_bar(frame, rect, &self.theme);

        let style = if hovered {
            self.theme.accent_bright()
        } else {
            Style::default().fg(self.theme.muted)
        };
        let top = rect.height.saturating_sub(2) / 2;
        let mut lines: Vec<Line> = (0..top).map(|_| Line::from("")).collect();
        lines.push(Line::from(Span::styled("＋", style)));
        lines.push(Line::from(Span::styled("New Build", style)));
        frame.render_widget(
            Paragraph::new(lines)
                .alignment(Alignment::Center),
            rect,
        );
    }

    /// Keyboard navigation for the instance picker.
    pub(crate) fn key_instance_grid(&mut self, key: KeyEvent) {
        let cols = self.tile_columns.max(1);
        let len = self.instances.len();
        if len == 0 {
            if matches!(key.code, KeyCode::Enter | KeyCode::Char('n')) {
                self.open_create_instance_form();
            }
            return;
        }
        let current = self.instance_state.selected().unwrap_or(0);
        let select = |app: &mut Self, idx: usize| {
            app.instance_state.select(Some(idx.min(len - 1)));
        };
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                if current + cols < len {
                    select(self, current + cols);
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if current >= cols {
                    select(self, current - cols);
                }
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if current % cols != cols - 1 && current + 1 < len {
                    select(self, current + 1);
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if current % cols != 0 {
                    select(self, current - 1);
                }
            }
            KeyCode::Char('g') => select(self, 0),
            KeyCode::Char('G') => select(self, len - 1),
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
}

fn tile_rect(inner: Rect, row: usize, col: usize) -> Rect {
    Rect {
        x: inner.x + col as u16 * (TILE_W + GAP_X),
        y: inner.y + row as u16 * (TILE_H + GAP_Y),
        width: TILE_W,
        height: TILE_H,
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
