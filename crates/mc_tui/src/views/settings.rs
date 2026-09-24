//! Settings screen: sectioned panels, RAM sliders, choice dropdowns.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, ButtonId, HitAction};
use crate::views::settings_ui::{
    draw_slider, draw_slider_scale, effective_ram_max, fill_rect, is_ram_over_half, label_cell,
    max_ram_range, min_ram_range, paint_page_bg, row_style, section_header,
    slider_input_rect, slider_track_rect, slider_value_rect, FieldKind, INPUT_W, LABEL_GAP,
    LABEL_W, PANEL_GAP, RAM_MIN, SIDE_PAD,
};
use crate::views::{buttons_row, section_title, truncate};

const FIELD_COUNT: usize = 9;

type SectionFields = Vec<(usize, FieldKind, &'static str)>;

/// Launcher settings grouped into two panels.
/// Field indices: 0 path · 1 min · 2 max · 3 gc · 4–6 bools · 7 anchor · 8 lang.
#[allow(clippy::type_complexity)]
fn launcher_sections() -> Vec<(&'static str, SectionFields)> {
    vec![
        (
            "settings.section.java",
            vec![
                (0, FieldKind::Text, "settings.java_path"),
                (1, FieldKind::Slider, "settings.min_ram"),
                (2, FieldKind::Slider, "settings.max_ram"),
                (3, FieldKind::Choice, "settings.gc"),
            ],
        ),
        (
            "settings.section.interface",
            vec![
                (4, FieldKind::Bool, "settings.show_progress"),
                (5, FieldKind::Bool, "settings.confirm_quit"),
                (6, FieldKind::Bool, "settings.auto_scroll"),
                (7, FieldKind::Choice, "settings.ascii_anchor"),
                (8, FieldKind::Choice, "settings.language"),
            ],
        ),
    ]
}

impl App {
    pub(crate) fn render_settings(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(6),
            ])
            .split(area);

        let save = self.tr("btn.save");
        let detect = self.tr("btn.detect_java");
        let ascii = self.tr("btn.ascii_folder");
        buttons_row(
            self,
            frame,
            chunks[0].x + SIDE_PAD,
            chunks[0].y,
            area.x + area.width,
            &[
                (save, "s", ButtonId::SaveSettings),
                (detect, "J", ButtonId::DetectJava),
                (ascii, "a", ButtonId::OpenAsciiBgFolder),
            ],
        );

        self.settings_sliders.clear();
        paint_page_bg(self, frame, chunks[2]);
        self.render_settings_body(frame, chunks[2]);
    }

    fn field_h(&self, idx: usize, kind: FieldKind) -> u16 {
        let mut h = 1u16;
        if kind == FieldKind::Slider && idx == self.settings_field {
            h += 1; // scale labels row under the selected slider
        }
        if kind == FieldKind::Choice && self.settings_dropdown.map(|(f, _)| f) == Some(idx) {
            h += self.settings_dropdown_options(idx).len() as u16;
        }
        h
    }

    fn runtimes_h(&self) -> u16 {
        // header + items (or 1 empty/scanning line)
        let n = self.java_installations.len().max(1) as u16;
        1 + n
    }

    fn render_settings_body(&mut self, frame: &mut Frame, area: Rect) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let title = self.tr("settings.title").to_string();
        frame.render_widget(
            Paragraph::new(section_title(&title, "", &self.theme))
                .style(Style::default().bg(self.theme.bg)),
            Rect {
                x: area.x + SIDE_PAD,
                y: area.y,
                width: area.width.saturating_sub(SIDE_PAD),
                height: 1,
            },
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

        let panel_x = body.x + SIDE_PAD;
        let panel_w = body.width.saturating_sub(SIDE_PAD);
        if panel_w < 16 {
            return;
        }

        if self.settings_field >= FIELD_COUNT {
            self.settings_field = FIELD_COUNT - 1;
        }

        let lang = self.lang();
        let s = &self.settings;
        let values: Vec<String> = vec![
            s.java_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| crate::i18n::tr(lang, "settings.auto_detect").to_string()),
            format!("{}", s.default_min_memory_mb),
            format!("{}", s.default_max_memory_mb),
            s.default_gc.label().to_string(),
            if s.show_progress {
                crate::i18n::tr_string(lang, "common.yes")
            } else {
                crate::i18n::tr_string(lang, "common.no")
            },
            if s.confirm_quit {
                crate::i18n::tr_string(lang, "common.yes")
            } else {
                crate::i18n::tr_string(lang, "common.no")
            },
            if s.log_auto_scroll {
                crate::i18n::tr_string(lang, "common.yes")
            } else {
                crate::i18n::tr_string(lang, "common.no")
            },
            s.ascii_bg_anchor.label_lang(lang),
            s.language.native_label().to_string(),
        ];

        // ── Measure ──
        let sections = launcher_sections();
        let mut total_h: u16 = 0;
        for (si, (_, fields)) in sections.iter().enumerate() {
            if si > 0 {
                total_h += PANEL_GAP;
            }
            total_h += 1; // header
            for (idx, kind, _) in fields {
                total_h += self.field_h(*idx, *kind);
            }
            // Java section also hosts the runtimes list.
            if si == 0 {
                total_h += self.runtimes_h();
            }
        }

        let max_scroll = total_h.saturating_sub(body.height);
        if self.settings_scroll > max_scroll {
            self.settings_scroll = max_scroll;
        }
        let scroll = self.settings_scroll;

        // ── Render ──
        let mut y_content = 0u16;
        for (si, (title_key, fields)) in sections.iter().enumerate() {
            if si > 0 {
                y_content += PANEL_GAP;
            }

            let mut section_h = 1u16;
            for (idx, kind, _) in fields {
                section_h += self.field_h(*idx, *kind);
            }
            if si == 0 {
                section_h += self.runtimes_h();
            }

            let panel_top = y_content;
            y_content += section_h;

            if panel_top >= scroll + body.height {
                break;
            }
            if panel_top + section_h <= scroll {
                continue;
            }

            let vis_start = panel_top.max(scroll);
            let vis_end = (panel_top + section_h).min(scroll + body.height);
            if vis_start >= vis_end {
                continue;
            }

            let screen_y = body.y + (vis_start - scroll);
            let panel_rect = Rect {
                x: panel_x,
                y: screen_y,
                width: panel_w,
                height: vis_end - vis_start,
            };

            // Solid panel body so gutters/row seams never show page-bg holes.
            fill_rect(frame, panel_rect, Style::default().bg(self.theme.panel));

            let st = self.tr(title_key).to_string();
            let header_rect = Rect {
                x: panel_rect.x,
                y: panel_rect.y,
                width: panel_rect.width,
                height: 1,
            };
            section_header(self, frame, header_rect, &st);

            let inner = Rect {
                x: panel_rect.x + 1,
                y: panel_rect.y + 1,
                width: panel_rect.width.saturating_sub(2),
                height: panel_rect.height.saturating_sub(1),
            };

            // Rows stack with no blank lines — only the label/value column gap.
            let mut row_y = panel_top + 1;
            for (idx, kind, label_key) in fields {
                let h = self.field_h(*idx, *kind);

                if row_y + h > scroll && row_y < scroll + body.height {
                    let screen_row_y = body.y + row_y.saturating_sub(scroll);
                    if screen_row_y >= inner.y && screen_row_y < inner.y + inner.height.max(1) {
                        let label = self.tr(label_key).to_string();
                        let value = values.get(*idx).cloned().unwrap_or_default();
                        let row = Rect {
                            x: inner.x,
                            y: screen_row_y,
                            width: inner.width,
                            height: 1,
                        };
                        self.render_settings_row(frame, row, *idx, *kind, &label, &value);
                    }

                    // Scale labels under the selected RAM slider — same
                    // horizontal span as the track so they never spill into
                    // the value/input columns.
                    if *kind == FieldKind::Slider && *idx == self.settings_field && h >= 2 {
                        let scale_y = body.y + (row_y + 1).saturating_sub(scroll);
                        if scale_y >= inner.y && scale_y < inner.y + inner.height.max(1) {
                            let row = Rect {
                                x: inner.x,
                                y: body.y + row_y.saturating_sub(scroll),
                                width: inner.width,
                                height: 1,
                            };
                            let mut track = slider_track_rect(row);
                            track.y = scale_y;
                            let track_min = RAM_MIN;
                            let track_max = effective_ram_max();
                            // Scale continues the selected row highlight —
                            // never paint a dark strip under the green row.
                            let bg = self.theme.selection_bg;
                            draw_slider_scale(self, frame, track, track_min, track_max, bg);
                        }
                    }

                    // Dropdown options.
                    if *kind == FieldKind::Choice
                        && self.settings_dropdown.map(|(f, _)| f) == Some(*idx)
                    {
                        let opts = self.settings_dropdown_options(*idx);
                        let sel = self.settings_dropdown.map(|(_, s)| s).unwrap_or(0);
                        for (oi, opt) in opts.iter().enumerate() {
                            let oy = row_y + 1 + oi as u16;
                            if oy < scroll || oy >= scroll + body.height {
                                continue;
                            }
                            let sy = body.y + (oy - scroll);
                            if sy < inner.y || sy >= inner.y + inner.height.max(1) {
                                continue;
                            }
                            let opt_row = Rect {
                                x: inner.x,
                                y: sy,
                                width: inner.width,
                                height: 1,
                            };
                            let is_sel = oi == sel;
                            let style = if is_sel {
                                self.theme.row_selected()
                            } else {
                                self.theme.row()
                            };
                            frame.render_widget(
                                Paragraph::new(Line::from(vec![
                                    Span::styled(
                                        format!("   {} ", if is_sel { "▸" } else { " " }),
                                        style,
                                    ),
                                    Span::styled(opt.clone(), style),
                                ]))
                                .style(style),
                                opt_row,
                            );
                            self.hitboxes.push(crate::app::Hitbox {
                                rect: opt_row,
                                action: HitAction::SettingsOption(*idx, oi),
                            });
                        }
                    }
                }

                row_y += h;
            }

            // Java runtimes block at the bottom of the Java section.
            if si == 0 {
                let rt_h = self.runtimes_h();
                if row_y + rt_h > scroll && row_y < scroll + body.height {
                    let sy_start = body.y + row_y.saturating_sub(scroll);
                    let visible = (row_y + rt_h).min(scroll + body.height) - row_y.max(scroll);
                    let rt_area = Rect {
                        x: inner.x,
                        y: sy_start,
                        width: inner.width,
                        height: visible,
                    };
                    self.render_java_runtimes(frame, rt_area);
                }
            }
        }
    }

    fn render_settings_row(
        &mut self,
        frame: &mut Frame,
        row: Rect,
        idx: usize,
        kind: FieldKind,
        label: &str,
        value: &str,
    ) {
        let selected = idx == self.settings_field;
        let editing = selected && self.settings_edit.as_ref().map(|(f, _)| *f) == Some(idx);
        let hovered = self.is_hovered(row);
        let style = row_style(self, selected, hovered);
        let lab = label_cell(label);
        // Full-row backdrop first — sliders paint cell-by-cell on top.
        fill_rect(frame, row, style);

        match kind {
            FieldKind::Slider => {
                // Layout: [label][gap][track][gap][value][input]
                let track_max = effective_ram_max();
                let track_min = RAM_MIN;
                let (smin, smax, cur) = match idx {
                    1 => {
                        let (lo, hi) = min_ram_range(self.settings.default_max_memory_mb);
                        (lo, track_max.min(hi.max(RAM_MIN)), self.settings.default_min_memory_mb)
                    }
                    2 => {
                        let (lo, hi) = max_ram_range(self.settings.default_min_memory_mb);
                        (RAM_MIN.max(lo), track_max.min(hi), self.settings.default_max_memory_mb)
                    }
                    _ => (RAM_MIN, track_max, 0),
                };
                let over = is_ram_over_half(cur);

                let val_num = if editing {
                    self.settings_edit.as_ref().unwrap().1.clone()
                } else {
                    cur.to_string()
                };
                let mb = crate::i18n::tr_string(self.lang(), "settings.mb");
                let value_text = format!("{} {}", cur, mb);

                let track = slider_track_rect(row);
                let value_rect = slider_value_rect(row, track);
                let input_rect = slider_input_rect(row, value_rect);
                let row_bg = if selected {
                    self.theme.selection_bg
                } else if hovered {
                    self.theme.hover_bg
                } else {
                    self.theme.panel
                };
                draw_slider(
                    self,
                    frame,
                    track,
                    cur.clamp(smin, smax.min(track_max)),
                    track_min,
                    track_max,
                    selected,
                    row_bg,
                );
                self.settings_sliders.push((idx, track, track_min, track_max));

                // Label — already LABEL_W + gap; do not re-truncate.
                let label_style = if selected {
                    if over {
                        Style::default().fg(self.theme.error).bg(self.theme.selection_bg)
                    } else {
                        Style::default().fg(self.theme.green).bg(self.theme.selection_bg)
                    }
                } else {
                    self.theme.card_dim()
                };
                frame.render_widget(
                    Paragraph::new(Span::styled(lab.clone(), label_style)).style(style),
                    Rect {
                        x: row.x,
                        y: row.y,
                        width: LABEL_W + LABEL_GAP,
                        height: 1,
                    },
                );

                let value_style = if over {
                    Style::default().fg(self.theme.error)
                } else {
                    style
                };
                frame.render_widget(
                    Paragraph::new(Span::styled(value_text, value_style)).style(style),
                    value_rect,
                );

                let input_style = if editing {
                    self.theme.row_selected()
                } else if selected {
                    // Blend into the selected row — no dark box on green.
                    Style::default()
                        .bg(self.theme.selection_bg)
                        .fg(if over {
                            self.theme.error
                        } else {
                            self.theme.green
                        })
                } else if over {
                    Style::default()
                        .bg(row_bg)
                        .fg(self.theme.error)
                } else {
                    Style::default().bg(row_bg).fg(self.theme.fg)
                };
                let display = if editing {
                    format!("[{}█]", val_num)
                } else {
                    format!("[{}]", val_num)
                };
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        crate::views::truncate(&display, INPUT_W as usize),
                        input_style,
                    ))
                    .style(input_style),
                    input_rect,
                );

                // Hitboxes: track first (more specific), then row fallback.
                self.hitboxes.push(crate::app::Hitbox {
                    rect: track,
                    action: HitAction::SettingsSlider(idx),
                });
                self.hitboxes.push(crate::app::Hitbox {
                    rect: row,
                    action: HitAction::SettingsRow(idx),
                });
            }
            FieldKind::Choice => {
                let open = self.settings_dropdown.map(|(f, _)| f) == Some(idx);
                let chevron = if open { "▴" } else { "▾" };
                let display = if editing {
                    format!("{}█", self.settings_edit.as_ref().unwrap().1)
                } else {
                    format!("{}  {}", value, chevron)
                };
                let label_style = if selected {
                    Style::default().fg(self.theme.green).bg(self.theme.selection_bg)
                } else {
                    self.theme.card_dim()
                };
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(lab.clone(), label_style),
                        Span::styled(display, style),
                    ]))
                    .style(style),
                    row,
                );
                self.hitboxes.push(crate::app::Hitbox {
                    rect: row,
                    action: HitAction::SettingsRow(idx),
                });
            }
            _ => {
                let display = if editing {
                    format!("{}█", self.settings_edit.as_ref().unwrap().1)
                } else {
                    value.to_string()
                };
                let label_style = if selected {
                    Style::default().fg(self.theme.green).bg(self.theme.selection_bg)
                } else {
                    self.theme.card_dim()
                };
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(lab.clone(), label_style),
                        Span::styled(display, style),
                    ]))
                    .style(style),
                    row,
                );
                self.hitboxes.push(crate::app::Hitbox {
                    rect: row,
                    action: HitAction::SettingsRow(idx),
                });
            }
        }
    }

    /// Java runtimes list embedded in the Java section (no outer card chrome).
    fn render_java_runtimes(&mut self, frame: &mut Frame, area: Rect) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        let title = self.tr("settings.java_runtimes").to_string();
        let rescan = self.tr("settings.rescan_hint").to_string();
        let count = self.java_installations.len();
        let header = format!(" {}  {} · {}", title, count, rescan);
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                truncate(&header, area.width as usize),
                self.theme.card_comment(),
            )]))
            .style(Style::default().bg(self.theme.panel)),
            Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: 1,
            },
        );

        let list_y = area.y + 1;
        let list_h = area.height.saturating_sub(1);
        if list_h == 0 {
            return;
        }

        if self.java_installations.is_empty() {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    self.tr("settings.scanning_java"),
                    self.theme.card_dim(),
                ))
                .style(Style::default().bg(self.theme.panel)),
                Rect {
                    x: area.x,
                    y: list_y,
                    width: area.width,
                    height: 1,
                },
            );
            return;
        }

        for (i, java) in self.java_installations.iter().enumerate().take(list_h as usize) {
            let line = Line::from(vec![
                Span::styled(format!("  Java {:<3} ", java.major), self.theme.accent()),
                Span::styled(java.version.clone(), self.theme.card()),
                Span::styled(format!("  {}", java.path.display()), self.theme.card_dim()),
            ]);
            frame.render_widget(
                Paragraph::new(line).style(Style::default().bg(self.theme.panel)),
                Rect {
                    x: area.x,
                    y: list_y + i as u16,
                    width: area.width,
                    height: 1,
                },
            );
        }
    }

    pub(crate) fn key_settings(&mut self, key: KeyEvent) {
        if let Some((field, buffer)) = self.settings_edit.as_mut() {
            let field = *field;
            match key.code {
                KeyCode::Esc => {
                    self.settings_edit = None;
                }
                KeyCode::Enter => {
                    let buf = buffer.clone();
                    self.settings_edit = None;
                    self.commit_launcher_setting(field, &buf);
                }
                KeyCode::Backspace => {
                    buffer.pop();
                }
                KeyCode::Char(c) => buffer.push(c),
                _ => {}
            }
            return;
        }

        if self.settings_dropdown.is_some() {
            match key.code {
                KeyCode::Esc => self.settings_close_dropdown(),
                KeyCode::Enter | KeyCode::Char(' ') => {
                    if let Some((field, opt)) = self.settings_dropdown {
                        self.settings_pick_option(field, opt);
                    }
                }
                KeyCode::Down | KeyCode::Char('j') | KeyCode::Right => {
                    self.settings_dropdown_move(1)
                }
                KeyCode::Up | KeyCode::Char('k') | KeyCode::Left => {
                    self.settings_dropdown_move(-1)
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                self.settings_field = (self.settings_field + 1).min(FIELD_COUNT - 1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.settings_field = self.settings_field.saturating_sub(1);
            }
            KeyCode::Enter => match self.settings_field {
                0..=2 => self.begin_settings_edit(),
                3 | 7 | 8 => self.settings_open_dropdown(),
                4..=6 => self.toggle_setting(self.settings_field),
                _ => {}
            },
            KeyCode::Char(' ') => self.toggle_setting(self.settings_field),
            KeyCode::Left => self.settings_adjust(false),
            KeyCode::Right => self.settings_adjust(true),
            KeyCode::Char('s') => self.save_settings(),
            KeyCode::Char('J') => self.spawn_java_discovery(),
            KeyCode::Char('a') => self.open_ascii_bg_folder(),
            _ => {}
        }
    }
}

