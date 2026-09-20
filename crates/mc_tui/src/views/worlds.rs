//! World manager — rich list with thumbnail icons and multi-line info.

use crossterm::event::KeyEvent;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;
use ratatui_image::{Resize, StatefulImage};

use crate::app::{App, ButtonId, Focus, HitAction};
use crate::views::{buttons_row, card, section_title, truncate};

/// Each world row is 5 lines of text + 1 line gap = 6 total.
const ROW_H: u16 = 6;
const ICON_W: u16 = 8;
const ICON_H: u16 = 5;

impl App {
    pub(crate) fn render_worlds(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
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
            chunks[0].x + 1,
            chunks[0].y,
            area.x + area.width,
            &[
                ("Open Folder", "Enter", ButtonId::BrowseMods),
                ("Delete", "d", ButtonId::DeleteMod),
            ],
        );

        frame.render_widget(
            Paragraph::new(section_title(
                "Worlds",
                &format!("{}", self.worlds.len()),
                &self.theme,
            ))
            .style(self.theme.card()),
            chunks[1],
        );

        let focused = self.focus == Focus::Content;
        let inner = card(self, frame, chunks[2], focused);
        if inner.height == 0 || inner.width == 0 {
            return;
        }

        if self.worlds.is_empty() {
            let lines = vec![
                Line::from(""),
                Line::from(Span::styled("No worlds found.", self.theme.header())),
                Line::from(Span::styled(
                    "Worlds are stored in the saves folder.",
                    self.theme.card_dim(),
                )),
            ];
            frame.render_widget(Paragraph::new(lines).style(self.theme.card()), inner);
            return;
        }

        let total = self.worlds.len();
        let visible = (inner.height / ROW_H) as usize;
        let selected = self.worlds_state.selected();
        let instance = self.selected_instance().cloned();
        let game_dir = instance.map(|i| i.game_dir());

        for row_idx in 0..visible {
            if row_idx >= total {
                break;
            }
            let y = inner.y + row_idx as u16 * ROW_H;
            let rect = Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: ROW_H,
            };
            let name = self.worlds[row_idx].clone();
            let is_selected = selected == Some(row_idx);
            let hovered = self.is_hovered(rect);
            self.render_world_row(frame, rect, &name, game_dir.as_deref(), is_selected, hovered);
            self.push_hitbox(rect, HitAction::InstanceTile(row_idx));
        }
    }

    fn render_world_row(
        &mut self,
        frame: &mut Frame,
        rect: Rect,
        name: &str,
        game_dir: Option<&std::path::Path>,
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
        let text_x = rect.x + pad_left + ICON_W + 1;
        let text_w = rect.width.saturating_sub(pad_left + ICON_W + 2);

        // Icon area on the left.
        let icon_rect = Rect {
            x: rect.x + pad_left,
            y: rect.y + 1,
            width: ICON_W.min(rect.width.saturating_sub(pad_left)),
            height: ICON_H.min(rect.height.saturating_sub(1)),
        };

        if let Some(dir) = game_dir {
            let icon_path = dir.join("saves").join(name).join("icon.png");
            let path_str = icon_path.to_string_lossy().to_string();
            self.load_local_image(&path_str);

            if let Some(img) = self.local_images.get(&path_str) {
                if crate::images::terminal_supports_truecolor() {
                    if self.local_protocols.get(&path_str).is_none() {
                        if let Some(dynamic) = crate::images::to_dynamic_image(img) {
                            let protocol = self.picker.new_resize_protocol(dynamic);
                            self.local_protocols.insert(path_str.clone(), protocol);
                        }
                    }
                    if let Some(proto) = self.local_protocols.get_mut(&path_str) {
                        frame.render_stateful_widget(
                            StatefulImage::default().resize(Resize::Fit(None)),
                            icon_rect,
                            proto,
                        );
                    }
                } else {
                    let lines = crate::images::image_lines(
                        img,
                        icon_rect.width as u32,
                        icon_rect.height as u32,
                        bg,
                    );
                    for (i, line) in lines.iter().enumerate() {
                        let row = Rect {
                            x: icon_rect.x,
                            y: icon_rect.y + i as u16,
                            width: icon_rect.width,
                            height: 1,
                        };
                        if row.y >= icon_rect.y + icon_rect.height {
                            break;
                        }
                        frame.render_widget(
                            Paragraph::new(Line::from(line.spans.clone())).style(surface),
                            row,
                        );
                    }
                }
            } else {
                let loading_style = if selected {
                    self.theme.accent()
                } else {
                    self.theme.card_dim()
                };
                frame.render_widget(
                    Paragraph::new(Span::styled("...", loading_style)).style(surface),
                    icon_rect,
                );
            }
        }

        // Text lines to the right of the icon.
        let name_style = if selected {
            self.theme.accent_bright()
        } else {
            Style::default().fg(self.theme.fg).bg(bg).add_modifier(ratatui::style::Modifier::BOLD)
        };
        let dim_style = if selected {
            self.theme.accent()
        } else {
            self.theme.card_dim()
        };

        let lines = vec![
            Line::from(Span::styled(truncate(name, text_w as usize), name_style)),
            Line::from(""),
            Line::from(Span::styled(
                format!("Seed:    {}", "unknown"),
                dim_style,
            )),
            Line::from(Span::styled(
                format!("Game:    {}", "Java"),
                dim_style,
            )),
            Line::from(Span::styled(
                format!("Version: {}", "latest"),
                dim_style,
            )),
        ];

        for (i, line) in lines.iter().enumerate() {
            let row_y = rect.y + i as u16;
            if row_y >= rect.y + rect.height {
                break;
            }
            let row_rect = Rect {
                x: text_x,
                y: row_y,
                width: text_w,
                height: 1,
            };
            frame.render_widget(
                Paragraph::new(line.clone()).style(surface),
                row_rect,
            );
        }
    }

    pub(crate) fn key_worlds(&mut self, key: KeyEvent) {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                move_sel_worlds(&mut self.worlds_state, self.worlds.len(), 1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                move_sel_worlds(&mut self.worlds_state, self.worlds.len(), -1);
            }
            KeyCode::Char('g') => self.worlds_state.select(Some(0)),
            KeyCode::Char('G') => {
                let last = self.worlds.len().saturating_sub(1);
                self.worlds_state.select(Some(last));
            }
            _ => {}
        }
    }
}

fn move_sel_worlds(state: &mut ratatui::widgets::ListState, len: usize, delta: i32) {
    if len == 0 {
        state.select(None);
        return;
    }
    let current = state.selected().unwrap_or(0) as i32;
    let next = (current + delta).clamp(0, len as i32 - 1) as usize;
    state.select(Some(next));
}
