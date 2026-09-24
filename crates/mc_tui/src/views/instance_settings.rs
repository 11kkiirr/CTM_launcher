//! Instance-scoped Settings page (JVM, memory, Java) — sectioned panels.

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
use crate::views::{buttons_row, section_title};

const FIELD_COUNT: usize = 7;

type SectionFields = Vec<(usize, FieldKind, &'static str)>;

/// Instance JVM settings: Java+Memory together, then Display, then Arguments.
#[allow(clippy::type_complexity)]
fn instance_sections() -> Vec<(&'static str, SectionFields)> {
    vec![
        (
            "instance_settings.section.java",
            vec![
                (0, FieldKind::Slider, "instance_settings.min_ram"),
                (1, FieldKind::Slider, "instance_settings.max_ram"),
                (2, FieldKind::Choice, "instance_settings.gc"),
                (3, FieldKind::Text, "instance_settings.java_path"),
            ],
        ),
        (
            "instance_settings.section.display",
            vec![(4, FieldKind::Bool, "instance_settings.fullscreen")],
        ),
        (
            "instance_settings.section.args",
            vec![
                (5, FieldKind::Text, "instance_settings.jvm_args"),
                (6, FieldKind::Text, "instance_settings.game_args"),
            ],
        ),
    ]
}

impl App {
    pub(crate) fn render_instance_settings(&mut self, frame: &mut Frame, area: Rect) {
        if self.selected_instance().is_none() {
            return;
        }
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(6),
                Constraint::Length(1),
            ])
            .split(area);

        let save = self.tr("btn.save");
        let detect = self.tr("btn.detect_java");
        buttons_row(
            self,
            frame,
            chunks[0].x + SIDE_PAD,
            chunks[0].y,
            area.x + area.width,
            &[
                (save, "s", ButtonId::SaveSettings),
                (detect, "J", ButtonId::DetectJava),
            ],
        );

        self.settings_sliders.clear();
        paint_page_bg(self, frame, chunks[2]);
        self.render_instance_settings_body(frame, chunks[2]);

        frame.render_widget(
            Paragraph::new(Span::styled(
                self.tr("instance_settings.hint"),
                Style::default().fg(self.theme.muted).bg(self.theme.bg),
            ))
            .style(Style::default().bg(self.theme.bg)),
            chunks[3],
        );
    }

    fn instance_field_h(&self, idx: usize, kind: FieldKind) -> u16 {
        let mut h = 1u16;
        if kind == FieldKind::Slider && idx == self.settings_field {
            h += 1; // scale labels under selected slider
        }
        if kind == FieldKind::Choice && self.settings_dropdown.map(|(f, _)| f) == Some(idx) {
            h += self.settings_dropdown_options(idx).len() as u16;
        }
        h
    }

    fn render_instance_settings_body(&mut self, frame: &mut Frame, area: Rect) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let instance = match self.selected_instance().cloned() {
            Some(i) => i,
            None => return,
        };
        let jvm = instance.metadata.jvm.clone();

        let title = self.tr("instance_settings.title").to_string();
        let title_area = Rect {
            x: area.x + SIDE_PAD,
            y: area.y,
            width: area.width.saturating_sub(SIDE_PAD),
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(section_title(&title, instance.name(), &self.theme))
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

        let panel_x = body.x + SIDE_PAD;
        let panel_w = body.width.saturating_sub(SIDE_PAD);
        if panel_w < 8 {
            return;
        }

        let lang = self.lang();
        let yes = crate::i18n::tr_string(lang, "common.yes");
        let no = crate::i18n::tr_string(lang, "common.no");
        let auto = crate::i18n::tr_string(lang, "settings.auto_detect");
        // Values only — labels come from section field keys.
        let values: Vec<String> = vec![
            jvm.min_memory_mb.to_string(),
            jvm.max_memory_mb.to_string(),
            jvm.gc.label().to_string(),
            jvm.java_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or(auto),
            if jvm.fullscreen { yes } else { no },
            jvm.custom_jvm_args.join(" "),
            jvm.extra_game_args.join(" "),
        ];

        if self.settings_field >= FIELD_COUNT {
            self.settings_field = FIELD_COUNT - 1;
        }

        let sections = instance_sections();
        let mut total_h: u16 = 0;
        for (si, (_, fields)) in sections.iter().enumerate() {
            if si > 0 {
                total_h += PANEL_GAP;
            }
            total_h += 1;
            for (idx, kind, _) in fields {
                total_h += self.instance_field_h(*idx, *kind);
            }
        }

        let max_scroll = total_h.saturating_sub(body.height);
        if self.settings_scroll > max_scroll {
            self.settings_scroll = max_scroll;
        }
        let scroll = self.settings_scroll;

        let mut y_content = 0u16;
        for (si, (title_key, fields)) in sections.iter().enumerate() {
            if si > 0 {
                y_content += PANEL_GAP;
            }

            let mut section_h = 1u16;
            for (idx, kind, _) in fields {
                section_h += self.instance_field_h(*idx, *kind);
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
            let screen_h = vis_end - vis_start;
            let panel_rect = Rect {
                x: panel_x,
                y: screen_y,
                width: panel_w,
                height: screen_h,
            };

            // Solid panel body so gutters/row seams never show page-bg holes.
            fill_rect(frame, panel_rect, Style::default().bg(self.theme.panel));

            let section_title_text = self.tr(title_key).to_string();
            let header_rect = Rect {
                x: panel_rect.x,
                y: panel_rect.y,
                width: panel_rect.width,
                height: 1,
            };
            section_header(self, frame, header_rect, &section_title_text);

            let inner = Rect {
                x: panel_rect.x + 1,
                y: panel_rect.y + 1,
                width: panel_rect.width.saturating_sub(2),
                height: panel_rect.height.saturating_sub(1),
            };

            // Rows stack tightly — only label/value column gap, no blank rows.
            let mut row_y = panel_top + 1;
            for (idx, kind, label_key) in fields {
                let field_h = self.instance_field_h(*idx, *kind);

                if row_y + field_h > scroll && row_y < scroll + body.height {
                    if row_y >= scroll && row_y < scroll + body.height {
                        let screen_row_y = body.y + (row_y - scroll);
                        let row = Rect {
                            x: inner.x,
                            y: screen_row_y,
                            width: inner.width,
                            height: 1,
                        };
                        if screen_row_y >= inner.y && screen_row_y < inner.y + inner.height.max(1)
                        {
                            let label = self.tr(label_key).to_string();
                            let value = values.get(*idx).cloned().unwrap_or_default();
                            self.render_instance_row(frame, row, *idx, *kind, &label, &value, &jvm);
                        }
                    }

                    // Scale labels under the selected RAM slider — same
                    // horizontal span as the track so they never spill into
                    // the value/input columns.
                    if *kind == FieldKind::Slider
                        && *idx == self.settings_field
                        && field_h >= 2
                    {
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
                            // Scale continues the selected row highlight —
                            // never paint a dark strip under the green row.
                            let bg = self.theme.selection_bg;
                            draw_slider_scale(
                                self,
                                frame,
                                track,
                                RAM_MIN,
                                effective_ram_max(),
                                bg,
                            );
                        }
                    }

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
                            let screen_oy = body.y + (oy - scroll);
                            if screen_oy < inner.y || screen_oy >= inner.y + inner.height.max(1) {
                                continue;
                            }
                            let opt_row = Rect {
                                x: inner.x,
                                y: screen_oy,
                                width: inner.width,
                                height: 1,
                            };
                            let is_sel = oi == sel;
                            let style = if is_sel {
                                self.theme.row_selected()
                            } else {
                                self.theme.row()
                            };
                            let marker = if is_sel { "▸ " } else { "  " };
                            frame.render_widget(
                                Paragraph::new(Line::from(vec![
                                    Span::styled(format!("   {}", marker), style),
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

                row_y += field_h;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_instance_row(
        &mut self,
        frame: &mut Frame,
        row: Rect,
        idx: usize,
        kind: FieldKind,
        label: &str,
        value: &str,
        jvm: &mc_core::instance::JvmConfig,
    ) {
        let selected = idx == self.settings_field;
        let editing = selected && self.settings_edit.as_ref().map(|(f, _)| *f) == Some(idx);
        let hovered = self.is_hovered(row);
        let style = row_style(self, selected, hovered);
        let lab = label_cell(label);
        let label_style = if selected {
            Style::default().fg(self.theme.green).bg(self.theme.selection_bg)
        } else {
            self.theme.card_dim()
        };
        // Full-row backdrop first — sliders paint cell-by-cell on top.
        fill_rect(frame, row, style);

        match kind {
            FieldKind::Slider => {
                let track_max = effective_ram_max();
                let track_min = RAM_MIN;
                let (smin, smax) = match idx {
                    0 => {
                        let (lo, hi) = min_ram_range(jvm.max_memory_mb);
                        (lo, hi.max(lo))
                    }
                    _ => {
                        let (lo, hi) = max_ram_range(jvm.min_memory_mb);
                        (lo, hi)
                    }
                };
                let cur = match idx {
                    0 => jvm.min_memory_mb.min(track_max),
                    1 => jvm.max_memory_mb.min(track_max),
                    _ => 0,
                };
                let over = is_ram_over_half(cur);

                let val_num = if editing {
                    self.settings_edit.as_ref().unwrap().1.clone()
                } else {
                    cur.to_string()
                };
                let mb_label = crate::i18n::tr_string(self.lang(), "settings.mb");
                let value_text = format!("{} {}", cur, mb_label);

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
                let chevron = if selected && self.settings_dropdown.map(|(f, _)| f) == Some(idx) {
                    "▴"
                } else {
                    "▾"
                };
                let display = if editing {
                    format!("{}█", self.settings_edit.as_ref().unwrap().1)
                } else {
                    format!("{}  {}", value, chevron)
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

    pub(crate) fn key_instance_settings(&mut self, key: KeyEvent) {
        if self.selected_instance().is_none() {
            return;
        }

        if let Some((field, buffer)) = self.settings_edit.as_mut() {
            let field = *field;
            match key.code {
                KeyCode::Esc => {
                    self.settings_edit = None;
                }
                KeyCode::Enter => {
                    let buf = buffer.clone();
                    self.settings_edit = None;
                    self.commit_instance_setting(field, &buf);
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
                0 | 1 | 3 | 5 | 6 => self.begin_instance_settings_edit(),
                2 => self.settings_open_dropdown(),
                _ => {}
            },
            KeyCode::Char(' ') => self.toggle_instance_setting(self.settings_field),
            KeyCode::Left => self.instance_settings_adjust(false),
            KeyCode::Right => self.instance_settings_adjust(true),
            KeyCode::Char('s') => self.save_instance_settings_now(),
            KeyCode::Char('J') => self.spawn_java_discovery(),
            _ => {}
        }
    }
}
