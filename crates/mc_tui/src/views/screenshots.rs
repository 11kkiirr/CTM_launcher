//! Screenshot viewer — image cards in a grid, like instance tiles.

use crossterm::event::KeyEvent;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;
use ratatui_image::{Resize, StatefulImage};

use crate::app::{App, ButtonId, Focus, HitAction};
use crate::views::{buttons_row, card, section_title, truncate};

const CARD_W: u16 = 32;
const CARD_H: u16 = 10;
const GAP_X: u16 = 2;
const GAP_Y: u16 = 1;

impl App {
    pub(crate) fn render_screenshots(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
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
                "Screenshots",
                &format!("{}", self.screenshots.len()),
                &self.theme,
            ))
            .style(self.theme.card()),
            chunks[2],
        );

        let focused = self.focus == Focus::Content;
        let inner = card(self, frame, chunks[3], focused);
        if inner.height == 0 || inner.width == 0 {
            return;
        }

        if self.screenshots.is_empty() {
            let lines = vec![
                Line::from(""),
                Line::from(Span::styled("No screenshots found.", self.theme.header())),
                Line::from(Span::styled(
                    "Screenshots are stored in the screenshots folder.",
                    self.theme.card_dim(),
                )),
            ];
            frame.render_widget(Paragraph::new(lines).style(self.theme.card()), inner);
            return;
        }

        let total = self.screenshots.len();
        let cols = ((inner.width + GAP_X) / (CARD_W + GAP_X)).max(1) as usize;
        let visible_rows = ((inner.height + GAP_Y) / (CARD_H + GAP_Y)).max(1) as usize;
        let grid_w = cols as u16 * CARD_W + (cols as u16).saturating_sub(1) * GAP_X;
        let offset_x = inner.width.saturating_sub(grid_w) / 2;

        let instance = self.selected_instance().cloned();
        let game_dir = instance.map(|i| i.game_dir());

        for row in 0..visible_rows {
            for col in 0..cols {
                let idx = row * cols + col;
                if idx >= total {
                    break;
                }
                let rect = Rect {
                    x: inner.x + offset_x + col as u16 * (CARD_W + GAP_X),
                    y: inner.y + row as u16 * (CARD_H + GAP_Y),
                    width: CARD_W,
                    height: CARD_H,
                };
                let name = self.screenshots[idx].clone();
                let is_selected = self.screenshots_state.selected() == Some(idx);
                let hovered = self.is_hovered(rect);
                self.render_screenshot_card(frame, rect, &name, game_dir.as_deref(), is_selected, hovered);
                self.push_hitbox(rect, HitAction::InstanceTile(idx));
            }
        }
    }

    fn render_screenshot_card(
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

        if rect.width < 6 || rect.height < 4 {
            return;
        }

        let pad_left: u16 = 2;
        // Image area: top portion of the card.
        let img_h = rect.height.saturating_sub(3);
        let img_area = Rect {
            x: rect.x + pad_left,
            y: rect.y,
            width: rect.width.saturating_sub(pad_left),
            height: img_h,
        };

        // Load the image file.
        if let Some(dir) = game_dir {
            let path = dir.join("screenshots").join(name);
            let path_str = path.to_string_lossy().to_string();
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
                            img_area,
                            proto,
                        );
                    }
                } else {
                    let lines = crate::images::image_lines(
                        img,
                        img_area.width as u32,
                        img_area.height as u32,
                        bg,
                    );
                    for (i, line) in lines.iter().enumerate() {
                        let row = Rect {
                            x: img_area.x,
                            y: img_area.y + i as u16,
                            width: img_area.width,
                            height: 1,
                        };
                        if row.y >= img_area.y + img_area.height {
                            break;
                        }
                        frame.render_widget(
                            Paragraph::new(Line::from(line.spans.clone())).style(surface),
                            row,
                        );
                    }
                }
            } else {
                // Placeholder while loading.
                let loading_style = if selected {
                    self.theme.accent()
                } else {
                    self.theme.card_dim()
                };
                frame.render_widget(
                    Paragraph::new(Span::styled("...", loading_style)).style(surface),
                    img_area,
                );
            }
        }

        // Caption below the image.
        let caption = truncate(name, rect.width.saturating_sub(pad_left) as usize);
        let caption_style = if selected {
            self.theme.accent_bright()
        } else {
            Style::default().fg(self.theme.fg).bg(bg)
        };
        frame.render_widget(
            Paragraph::new(Span::styled(caption, caption_style))
                .alignment(Alignment::Center)
                .style(surface),
            Rect {
                x: rect.x,
                y: rect.y + rect.height.saturating_sub(2),
                width: rect.width,
                height: 1,
            },
        );
    }

    pub(crate) fn key_screenshots(&mut self, key: KeyEvent) {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                self.screenshots_state
                    .select(self.screenshots_state.selected().map(|i| (i + 1).min(self.screenshots.len().saturating_sub(1))));
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.screenshots_state
                    .select(self.screenshots_state.selected().map(|i| i.saturating_sub(1)));
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if let Some(i) = self.screenshots_state.selected() {
                    let cols = ((self.sidebar_area.width + GAP_X) / (CARD_W + GAP_X)).max(1) as usize;
                    self.screenshots_state.select(Some(i.saturating_sub(cols)));
                }
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if let Some(i) = self.screenshots_state.selected() {
                    let cols = ((self.sidebar_area.width + GAP_X) / (CARD_W + GAP_X)).max(1) as usize;
                    let next = (i + cols).min(self.screenshots.len().saturating_sub(1));
                    self.screenshots_state.select(Some(next));
                }
            }
            KeyCode::Char('g') => self.screenshots_state.select(Some(0)),
            KeyCode::Char('G') => {
                let last = self.screenshots.len().saturating_sub(1);
                self.screenshots_state.select(Some(last));
            }
            _ => {}
        }
    }
}
