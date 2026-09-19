//! Square instance tiles shown in the Prism-style sidebar.

use mc_core::instance::Instance;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::{App, HitAction};

const TILE_W: u16 = 14;
const TILE_H: u16 = 5;
const GAP: u16 = 1;

impl App {
    /// Render the grid of square instance tiles plus an "Add Instance" tile.
    pub(crate) fn render_instance_tiles(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.block_border())
            .title(
                Line::from(format!(" Instances ({}) ", self.instances.len()))
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

        // "Add Instance" tile placed right after the last instance.
        let add_row = total / cols;
        let add_col = total % cols;
        if add_row >= self.tile_scroll && add_row < self.tile_scroll + visible_rows {
            let display_row = add_row - self.tile_scroll;
            let rect = tile_rect(inner, display_row, add_col);
            let hovered = self.is_hovered(rect);
            let border = if hovered {
                self.theme.block_border_focused()
            } else {
                self.theme.block_border()
            };
            let block = Block::default()
                .borders(Borders::ALL)
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
                    Line::from(Span::styled("+ Add Instance", style)),
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
            self.theme.block_border_focused()
        } else {
            self.theme.block_border()
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border)
            .style(if selected {
                self.theme.selection()
            } else if hovered {
                self.theme.hover()
            } else {
                self.theme.base()
            });
        let inner = block.inner(rect);
        frame.render_widget(block, rect);

        let name = truncate(instance.name(), inner.width as usize);
        let loader = instance.metadata.loader.label();
        let third = match &instance.metadata.modpack {
            Some(pack) => truncate(&format!("⛁ {}", pack.name), inner.width as usize),
            None => instance.metadata.game_version.clone(),
        };
        let lines = vec![
            Line::from(Span::styled(name, self.theme.header())),
            Line::from(Span::styled(
                truncate(
                    &format!("{} · {loader}", instance.metadata.game_version),
                    inner.width as usize,
                ),
                self.theme.dim(),
            )),
            Line::from(Span::styled(third, self.theme.info_style())),
        ];
        frame.render_widget(Paragraph::new(lines).style(self.theme.base()), inner);
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
