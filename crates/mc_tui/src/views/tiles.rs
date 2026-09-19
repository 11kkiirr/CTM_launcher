//! Instance picker rendered as flat, borderless cards in the main content area.

use crossterm::event::{KeyCode, KeyEvent};
use mc_core::instance::{Instance, LoaderType};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use crate::app::{App, ButtonId, Focus, HitAction};
use crate::views::{buttons_row, truncate};

const TILE_W: u16 = 30;
const TILE_H: u16 = 7;
const GAP_X: u16 = 2;
const GAP_Y: u16 = 1;

/// A small glyph for a mod loader.
pub(crate) fn loader_icon(loader: LoaderType) -> &'static str {
    match loader {
        LoaderType::NeoForge | LoaderType::Forge => "⚙",
        LoaderType::Fabric | LoaderType::Quilt => "⚡",
        LoaderType::Paper => "◇",
        LoaderType::Vanilla => "▣",
    }
}

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

        buttons_row(
            self,
            frame,
            rows[0].x,
            rows[0].y,
            area.x + area.width,
            &[
                ("Launch", ButtonId::Launch),
                ("Install / Repair", ButtonId::InstallInstance),
                ("New", ButtonId::NewInstance),
                ("Edit", ButtonId::EditInstance),
                ("Change Version", ButtonId::ChangeVersion),
                ("Delete", ButtonId::DeleteInstance),
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
            x: inner.x,
            y: inner.y + 2,
            width: inner.width,
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
        } else {
            self.theme.panel_alt
        };
        let surface = Style::default().bg(bg);
        frame.render_widget(Block::default().style(surface), rect);
        if selected {
            crate::views::accent_bar(frame, rect, &self.theme);
        }

        let inner = Rect {
            x: rect.x + 2,
            y: rect.y + 1,
            width: rect.width.saturating_sub(4),
            height: rect.height.saturating_sub(2),
        };
        let width = inner.width as usize;
        let loader = instance.metadata.loader;
        let title_style = if selected || hovered {
            self.theme.accent()
        } else {
            Style::default().fg(self.theme.fg)
        };
        let meta = format!(
            "{} {}  {}",
            loader_icon(loader),
            loader.label(),
            instance.metadata.game_version
        );
        let sub = match &instance.metadata.modpack {
            Some(pack) => format!("⛁ {}", pack.name),
            None => "● Ready".to_string(),
        };
        let sub_style = if instance.metadata.modpack.is_some() {
            self.theme.info_style()
        } else {
            self.theme.accent()
        };
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(truncate(instance.name(), width), title_style)),
            Line::from(Span::styled(truncate(&meta, width), self.theme.dim())),
            Line::from(Span::styled(truncate(&sub, width), sub_style)),
        ];
        frame.render_widget(Paragraph::new(lines).style(surface), inner);
    }

    fn render_add_tile(&self, frame: &mut Frame, rect: Rect, hovered: bool) {
        let bg = if hovered {
            self.theme.selection_bg
        } else {
            self.theme.panel_alt
        };
        let surface = Style::default().bg(bg);
        frame.render_widget(Block::default().style(surface), rect);
        let style = if hovered {
            self.theme.accent()
        } else {
            self.theme.card_dim()
        };
        let inner = Rect {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
        };
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(""),
                Line::from(""),
                Line::from(Span::styled("＋  New Build", style)),
            ])
            .alignment(Alignment::Center)
            .style(surface),
            inner,
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
            KeyCode::Char('i') => {
                self.select_instance(current);
                self.install_selected_instance();
            }
            KeyCode::Char('e') => {
                self.select_instance(current);
                self.open_edit_instance_form();
            }
            KeyCode::Char('d') => {
                self.select_instance(current);
                self.confirm_delete_instance();
            }
            KeyCode::Char('n') => self.open_create_instance_form(),
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
