//! Instance picker rendered as friendly square tiles in the main content area.

use crossterm::event::{KeyCode, KeyEvent};
use mc_core::instance::Instance;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, HitAction};

const TILE_W: u16 = 24;
const TILE_H: u16 = 7;
const GAP: u16 = 1;

impl App {
    /// Render the grid of instance tiles plus a "New Build" tile.
    pub(crate) fn render_instance_grid(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(self.theme.block_border())
            .title(
                Line::from(format!(" Builds ({}) ", self.instances.len()))
                    .style(self.theme.header()),
            );
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.width < TILE_W || inner.height < TILE_H {
            frame.render_widget(
                Paragraph::new(Span::styled("…", self.theme.dim())).style(self.theme.base()),
                inner,
            );
            return;
        }

        let cols = ((inner.width + GAP) / (TILE_W + GAP)).max(1) as usize;
        let visible_rows = ((inner.height + GAP) / (TILE_H + GAP)).max(1) as usize;
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
                let rect = tile_rect(inner, row, col);
                let hovered = self.is_hovered(rect);
                let is_selected = selected == Some(idx);
                self.render_tile(frame, rect, &self.instances[idx], is_selected, hovered);
                self.push_hitbox(rect, HitAction::InstanceTile(idx));
            }
        }

        // "New Build" tile right after the last instance.
        let add_row = total / cols;
        let add_col = total % cols;
        if add_row >= self.tile_scroll && add_row < self.tile_scroll + visible_rows {
            let rect = tile_rect(inner, add_row - self.tile_scroll, add_col);
            let hovered = self.is_hovered(rect);
            let border = if hovered {
                self.theme.block_border_focused()
            } else {
                self.theme.block_border()
            };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(ratatui::widgets::BorderType::Rounded)
                .border_style(border)
                .style(if hovered {
                    self.theme.hover()
                } else {
                    self.theme.base()
                });
            let inner_tile = block.inner(rect);
            frame.render_widget(block, rect);
            let style = if hovered {
                self.theme.accent()
            } else {
                self.theme.dim()
            };
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(""),
                    Line::from(""),
                    Line::from(Span::styled("＋  New Build", style)),
                ])
                .alignment(Alignment::Center)
                .style(self.theme.base()),
                inner_tile,
            );
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
        let border = if selected || hovered {
            self.theme.border_focused
        } else {
            self.theme.border
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(ratatui::style::Style::default().fg(border))
            .style(if selected {
                self.theme.selection()
            } else if hovered {
                self.theme.hover()
            } else {
                self.theme.base()
            });
        let inner = block.inner(rect);
        frame.render_widget(block, rect);

        let width = inner.width as usize;
        let loader = instance.metadata.loader.label();
        let subtitle = match &instance.metadata.modpack {
            Some(pack) => format!("⛁ {}", pack.name),
            None => format!("{} · {loader}", instance.metadata.game_version),
        };
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                truncate(instance.name(), width),
                self.theme.header(),
            )),
            Line::from(Span::styled(
                truncate(
                    &format!("{} · {loader}", instance.metadata.game_version),
                    width,
                ),
                self.theme.dim(),
            )),
            Line::from(Span::styled(
                truncate(&subtitle, width),
                self.theme.info_style(),
            )),
            Line::from(""),
        ];
        frame.render_widget(
            Paragraph::new(lines)
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true })
                .style(self.theme.base()),
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
            KeyCode::Enter => self.select_instance(current),
            KeyCode::Char('n') => self.open_create_instance_form(),
            _ => {}
        }
    }
}

fn tile_rect(inner: Rect, row: usize, col: usize) -> Rect {
    Rect {
        x: inner.x + col as u16 * (TILE_W + GAP),
        y: inner.y + row as u16 * (TILE_H + GAP),
        width: TILE_W,
        height: TILE_H,
    }
}

/// Truncate a string to `max` display columns, appending `…` when clipped.
fn truncate(input: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = input.chars().collect();
    if chars.len() <= max {
        return input.to_string();
    }
    let mut out: String = chars.into_iter().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}
