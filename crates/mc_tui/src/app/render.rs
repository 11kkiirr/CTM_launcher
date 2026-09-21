use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use super::{
    footer_hints, info_line, truncate_str, App, Focus, HitAction, Nav, OverlayAction,
    NAV_BUTTON_HEIGHT,
};
use crate::forms::Overlay;

    // ---------------------------------------------------------------------

impl App {
// Rendering
// ---------------------------------------------------------------------

pub(crate) fn render(&mut self, frame: &mut Frame) {
    self.hitboxes.clear();
    let area = frame.area();
    frame.render_widget(Block::default().style(self.theme.base()), area);

    let progress_active = self.settings.show_progress && self.progress.is_some();
    let mut rows = vec![
        Constraint::Length(1), // header bar
        Constraint::Min(3),    // body
        Constraint::Length(1), // status bar
    ];
    if progress_active {
        rows.push(Constraint::Length(1));
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(rows)
        .split(area);

    self.render_header(frame, chunks[0]);

    // Split the body horizontally first: content area | gap | sidebar.
    // The sidebar spans the full body height (including the 1-row gap above content).
    let body_columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(20),
            Constraint::Length(1),
            Constraint::Length(30),
        ])
        .split(chunks[1]);

    // Content area gets the 1-row top gap like before.
    let content_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(3)])
        .split(body_columns[0]);
    let content = content_rows[1];

    self.sidebar_area = body_columns[2];
    self.render_nav_panel(frame, body_columns[2]);

    match self.nav {
        Nav::Instances => self.render_instance_grid(frame, content),
        Nav::Accounts => self.render_accounts(frame, content),
        Nav::Launcher => self.render_settings(frame, content),
        Nav::Browse => self.render_browse(frame, content),
        _ if self.selected_instance().is_none() => self.render_empty_state(frame, content),
        Nav::Mods => self.render_mods(frame, content),
        Nav::Modpacks => self.render_modpacks(frame, content),
        Nav::Versions => self.render_versions(frame, content),
        Nav::Jvm => self.render_instance_settings(frame, content),
        Nav::Logs => self.render_logs(frame, content),
        Nav::ResourcePacks => self.render_resource_packs(frame, content),
        Nav::Shaders => self.render_shaders(frame, content),
        Nav::Worlds => self.render_worlds(frame, content),
        Nav::Screenshots => self.render_screenshots(frame, content),
    }

    self.render_footer(frame, chunks[2]);
    if progress_active {
        self.render_progress(frame, chunks[3]);
    }
    self.render_overlay(frame, area);
}

/// The right-hand navigation + build info sidebar, a single flat card.
pub(crate) fn render_nav_panel(&mut self, frame: &mut Frame, area: Rect) {
    let focused = self.focus == Focus::Sidebar;
    let inner = crate::views::card(self, frame, area, focused);

    // 1-char left indent for all sidebar content.
    let content = Rect {
        x: inner.x + 1,
        y: inner.y,
        width: inner.width.saturating_sub(1),
        height: inner.height,
    };

    let menu = Nav::menu();
    let nav_height = menu.len() as u16 * NAV_BUTTON_HEIGHT;
    let has_build = self.selected_instance().is_some();
    let build_detail_height = if has_build { 5 } else { 0 };

    // Layout: build name (1) → "Navigation" (1) → nav buttons → spacer → build details
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(if has_build { 1 } else { 0 }), // build name
            Constraint::Length(1),                             // "Navigation" header
            Constraint::Length(nav_height),                   // nav buttons
            Constraint::Min(1),                               // spacer
            Constraint::Length(1),                            // "Build Info" header
            Constraint::Length(build_detail_height),          // build details
        ])
        .split(content);

    // Build name pinned above navigation.
    if has_build {
        if let Some(instance) = self.selected_instance() {
            let name = truncate_str(instance.name(), chunks[0].width as usize);
            frame.render_widget(
                Paragraph::new(Span::styled(name, self.theme.header()))
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(self.theme.card()),
                chunks[0],
            );
        }
    }

    // Empty space where "Navigation" was.

    for (idx, nav) in menu.iter().enumerate() {
        let y = chunks[2].y + idx as u16 * NAV_BUTTON_HEIGHT;
        if y + NAV_BUTTON_HEIGHT > chunks[2].y + chunks[2].height {
            break;
        }
        let rect = Rect {
            x: chunks[2].x,
            y,
            width: chunks[2].width,
            height: NAV_BUTTON_HEIGHT,
        };
        self.render_nav_button(frame, rect, *nav, idx + 1);
    }

    frame.render_widget(
        Paragraph::new(Span::styled("Build Info", self.theme.card_comment()))
            .style(self.theme.card()),
        chunks[4],
    );
    self.render_build_info(frame, chunks[5]);
}

/// A chunky, padded navigation block button with an index number. The
/// active entry gets a solid dark-green fill. No borders.
fn render_nav_button(&mut self, frame: &mut Frame, rect: Rect, nav: Nav, number: usize) {
    let selected = nav == self.nav;
    let hovered = self.is_hovered(rect);
    let bg = if selected || hovered {
        self.theme.selection_bg
    } else {
        self.theme.panel_alt
    };
    frame.render_widget(Block::default().style(Style::default().bg(bg)), rect);

    // Left accent bar: green for active, muted gray for inactive.
    let bar_style = if selected {
        Style::default().fg(self.theme.green).bg(bg)
    } else {
        Style::default().fg(self.theme.muted).bg(bg)
    };
    if rect.height > 0 {
        let bar_rect = Rect {
            x: rect.x,
            y: rect.y,
            width: 1,
            height: rect.height,
        };
        let bar_lines: Vec<Line> = (0..rect.height)
            .map(|_| Line::from(Span::styled("▎", bar_style)))
            .collect();
        frame.render_widget(Paragraph::new(bar_lines), bar_rect);
    }

    let surface = Style::default().bg(bg);
    let num_style = if selected {
        self.theme.accent_bright()
    } else if hovered {
        self.theme.accent()
    } else {
        self.theme.comment_style()
    };
    let label_style = if selected {
        self.theme.accent_bright()
    } else if hovered {
        self.theme.accent()
    } else {
        Style::default().fg(self.theme.fg).bg(bg)
    };
    let row = Rect {
        x: rect.x + 1,
        y: rect.y + rect.height / 2,
        width: rect.width.saturating_sub(1),
        height: 1,
    };
    if rect.height > 0 {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!(" {number}   "), num_style),
                Span::styled(nav.menu_label().to_string(), label_style),
            ]))
            .style(surface),
            row,
        );
    }
    self.push_hitbox(rect, HitAction::NavItem(nav));
}

fn render_build_info(&mut self, frame: &mut Frame, area: Rect) {
    let Some(instance) = self.selected_instance().cloned() else {
        frame.render_widget(
            Paragraph::new(Span::styled("No build selected.", self.theme.card_dim()))
                .style(self.theme.card()),
            area,
        );
        return;
    };

    let jvm = &instance.metadata.jvm;
    let lines = vec![
        info_line("Version", &instance.metadata.game_version, &self.theme),
        info_line("Loader", instance.metadata.loader.label(), &self.theme),
        info_line("Memory", &format!("{} MB", jvm.max_memory_mb), &self.theme),
        info_line("GC", jvm.gc.label(), &self.theme),
        info_line("Mods", &self.installed_mods.len().to_string(), &self.theme),
    ];
    frame.render_widget(Paragraph::new(lines).style(self.theme.card()), area);
}

pub(crate) fn render_empty_state(&mut self, frame: &mut Frame, area: Rect) {
    let inner = crate::views::card(self, frame, area, true);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled("No build selected", self.theme.header())),
        Line::from(Span::styled(
            "Pick a build from the Instances page, or create a new one.",
            self.theme.card_dim(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Press 'n' or click [+ New Build].",
            self.theme.accent(),
        )),
    ];
    frame.render_widget(Paragraph::new(lines).style(self.theme.card()), inner);
}

pub(crate) fn render_header(&mut self, frame: &mut Frame, area: Rect) {
    frame.render_widget(Block::default().style(self.theme.bar()), area);

    let active = self
        .accounts
        .active()
        .map(|a| a.username.clone())
        .unwrap_or_else(|| "no account".to_string());

    let subtitle = match self.nav {
        Nav::Instances => "Instances".to_string(),
        Nav::Browse => format!("Browse {}", self.browse.kind.label()),
        Nav::Accounts => "Accounts".to_string(),
        Nav::Launcher => "Launcher Settings".to_string(),
        _ => self
            .selected_instance()
            .map(|i| i.name().to_string())
            .unwrap_or_else(|| "no build".to_string()),
    };
    let left = Line::from(vec![
        Span::styled(" CTMLauncher", self.theme.accent_bright()),
        Span::styled("  ›  ", self.theme.comment_style()),
        Span::styled(subtitle, self.theme.dim()),
    ]);
    frame.render_widget(Paragraph::new(left).style(self.theme.bar()), area);

    // Top-right: the account badge and the launcher settings button side by
    // side. Both remain reachable now that they are out of the nav menu.
    let account = format!("@ {active}");
    let account_w = account.chars().count() as u16;
    let settings = "⚙ Settings";
    let settings_w = settings.chars().count() as u16;
    let total = account_w + 2 + settings_w;
    if total + 2 < area.width {
        let start = area.x + area.width - total - 1;

        let acc_rect = Rect {
            x: start,
            y: area.y,
            width: account_w,
            height: 1,
        };
        let acc_style = if self.nav == Nav::Accounts {
            self.theme.row_selected()
        } else if self.is_hovered(acc_rect) {
            self.theme.hover()
        } else {
            self.theme.accent()
        };
        frame.render_widget(
            Paragraph::new(Span::styled(account, acc_style)).style(self.theme.bar()),
            acc_rect,
        );
        self.push_hitbox(acc_rect, HitAction::NavItem(Nav::Accounts));

        let set_rect = Rect {
            x: start + account_w + 2,
            y: area.y,
            width: settings_w,
            height: 1,
        };
        let set_style = if self.nav == Nav::Launcher {
            self.theme.row_selected()
        } else if self.is_hovered(set_rect) {
            self.theme.hover()
        } else {
            self.theme.accent()
        };
        frame.render_widget(
            Paragraph::new(Span::styled(settings, set_style)).style(self.theme.bar()),
            set_rect,
        );
        self.push_hitbox(set_rect, HitAction::NavItem(Nav::Launcher));
    }
}

/// A one-line status bar: status message on the left, green-hotkey hints on
/// the right.
pub(crate) fn render_footer(&mut self, frame: &mut Frame, area: Rect) {
    frame.render_widget(Block::default().style(self.theme.bar()), area);

    let hints = footer_hints(self.nav);
    let mut spans: Vec<Span> = Vec::new();
    for (idx, (key, label)) in hints.iter().enumerate() {
        if idx > 0 {
            spans.push(Span::styled("  ", self.theme.comment_style()));
        }
        spans.push(Span::styled(key.to_string(), self.theme.accent()));
        spans.push(Span::styled(format!(" {label}"), self.theme.dim()));
    }
    let hint_width: u16 = spans.iter().map(|s| s.width() as u16).sum();
    let hint_rect = Rect {
        x: area.x + area.width.saturating_sub(hint_width),
        y: area.y,
        width: hint_width.min(area.width),
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(self.theme.bar()),
        hint_rect,
    );

    let status_width = area.width.saturating_sub(hint_width).saturating_sub(2);
    if status_width > 4 {
        let (status_style, status_text) = if let Some(toast) = &self.toast {
            let style = if toast.error {
                self.theme.error_style()
            } else {
                self.theme.accent()
            };
            (style, toast.message.clone())
        } else {
            (self.theme.dim(), self.status.clone())
        };
        let busy = self.progress.is_some();
        let marker = if busy {
            let frames = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
            format!("{} ", frames[(self.tick as usize) % frames.len()])
        } else {
            "● ".to_string()
        };
        let rect = Rect {
            x: area.x,
            y: area.y,
            width: status_width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!(" {marker}"), status_style),
                Span::styled(
                    truncate_str(&status_text, status_width.saturating_sub(3) as usize),
                    status_style,
                ),
            ]))
            .style(self.theme.bar()),
            rect,
        );
    }
}

fn render_progress(&mut self, frame: &mut Frame, area: Rect) {
    if let Some((ratio, label)) = &self.progress {
        let gauge = crate::widgets::ProgressBar::new(ratio.unwrap_or(0.0), label, &self.theme);
        frame.render_widget(gauge, area);
    }
}

pub(crate) fn render_overlay(&mut self, frame: &mut Frame, area: Rect) {
    let Some(overlay) = self.overlay.clone() else {
        return;
    };
    // A modal scrim: blank the screen behind the dialog so no fragments of
    // the underlying view show through around the popup. Overlay hitboxes
    // replace the content hitboxes while a dialog is open.
    self.hitboxes.clear();
    frame.render_widget(ratatui::widgets::Clear, area);
    frame.render_widget(Block::default().style(self.theme.base()), area);
    let surface = Style::default().bg(self.theme.panel_alt);
    match overlay {
        Overlay::Text {
            title,
            prompt,
            value,
            ..
        } => {
            let popup = crate::widgets::centered_rect(60, 20, area);
            let lines = vec![
                Line::from(Span::styled(prompt, self.theme.dim())),
                Line::from(Span::styled(format!("{value}█"), self.theme.accent())),
                Line::from(""),
                Line::from(Span::styled("Enter confirm · Esc cancel", self.theme.dim())),
            ];
            crate::widgets::render_popup(frame, popup, &title, lines, &self.theme);
            self.push_hitbox(popup, HitAction::Overlay(OverlayAction::TextDone));
        }
        Overlay::Form(form) => {
            let height = (form.fields.len() as u16 * 2 + 4).min(area.height.saturating_sub(2));
            let popup = crate::widgets::centered_rect(
                64,
                (height * 100 / area.height.max(1)).max(20),
                area,
            );
            frame.render_widget(ratatui::widgets::Clear, popup);
            frame.render_widget(Block::default().style(surface), popup);
            crate::views::accent_bar(frame, popup, &self.theme);

            let content = Rect {
                x: popup.x + 2,
                y: popup.y + 1,
                width: popup.width.saturating_sub(3),
                height: popup.height.saturating_sub(2),
            };
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Min(1),
                ])
                .split(content);
            frame.render_widget(
                Paragraph::new(Span::styled(form.title.clone(), self.theme.header()))
                    .style(surface),
                rows[0],
            );

            let field_area = rows[2];
            let mut y = field_area.y;
            for (idx, field) in form.fields.iter().enumerate() {
                if y + 2 > field_area.y + field_area.height {
                    break;
                }
                let active = idx == form.active;
                let label_style = if active {
                    self.theme.accent()
                } else {
                    self.theme.dim()
                };
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        format!("{} {}", if active { "▸" } else { " " }, field.label),
                        label_style,
                    )))
                    .style(surface),
                    Rect {
                        x: field_area.x,
                        y,
                        width: field_area.width,
                        height: 1,
                    },
                );
                let value_style = if active {
                    self.theme.row_selected()
                } else {
                    self.theme.row()
                };
                let display = match &field.kind {
                    crate::forms::FieldKind::Bool(_) => {
                        format!("< {} >", field.value)
                    }
                    crate::forms::FieldKind::Choice { .. } => format!("< {} >", field.value),
                    _ if active => format!("{}█", field.value),
                    _ => field.value.clone(),
                };
                let hint = if field.hint.is_empty() {
                    String::new()
                } else {
                    format!("   ({})", field.hint)
                };
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(format!("  {display}"), value_style),
                        Span::styled(hint, self.theme.dim()),
                    ]))
                    .style(surface),
                    Rect {
                        x: field_area.x,
                        y: y + 1,
                        width: field_area.width,
                        height: 1,
                    },
                );
                self.push_hitbox(
                    Rect {
                        x: field_area.x,
                        y,
                        width: field_area.width,
                        height: 2,
                    },
                    HitAction::Overlay(OverlayAction::FormField(idx)),
                );
                y += 2;
            }
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "Tab/↑↓ field · ←→ change · Space toggle · Enter submit · Esc cancel",
                    self.theme.dim(),
                ))
                .style(surface),
                Rect {
                    x: field_area.x,
                    y: y + 1,
                    width: field_area.width,
                    height: 1,
                },
            );
            self.push_hitbox(
                Rect {
                    x: field_area.x,
                    y: y + 1,
                    width: field_area.width,
                    height: 1,
                },
                HitAction::Overlay(OverlayAction::FormSubmit),
            );
        }
        Overlay::Confirm { title, message, .. } => {
            let popup = crate::widgets::centered_rect(52, 22, area);
            let lines = vec![
                Line::from(Span::styled(message, self.theme.warning_style())),
                Line::from(""),
                Line::from(Span::styled("[Y]es   [N]o", self.theme.accent())),
            ];
            crate::widgets::render_popup(frame, popup, &title, lines, &self.theme);
            let mid = popup.width / 2;
            self.push_hitbox(
                Rect {
                    x: popup.x,
                    y: popup.y,
                    width: mid,
                    height: popup.height,
                },
                HitAction::Overlay(OverlayAction::ConfirmYes),
            );
            self.push_hitbox(
                Rect {
                    x: popup.x + mid,
                    y: popup.y,
                    width: popup.width - mid,
                    height: popup.height,
                },
                HitAction::Overlay(OverlayAction::ConfirmNo),
            );
        }
        Overlay::Message { title, lines } => {
            let popup = crate::widgets::centered_rect(74, 84, area);
            let mut rendered: Vec<Line> = lines.into_iter().map(Line::from).collect();
            rendered.push(Line::from(""));
            rendered.push(Line::from(Span::styled(
                "Enter/Esc to close",
                self.theme.dim(),
            )));
            crate::widgets::render_popup(frame, popup, &title, rendered, &self.theme);
            self.push_hitbox(popup, HitAction::Overlay(OverlayAction::MessageClose));
        }
        Overlay::DeviceCode(prompt) => {
            let popup = crate::widgets::centered_rect(64, 40, area);
            let lines = vec![
                Line::from(Span::styled(
                    "1. Open the URL below in a browser:",
                    self.theme.dim(),
                )),
                Line::from(Span::styled(
                    prompt.verification_uri.clone(),
                    self.theme.info_style(),
                )),
                Line::from(""),
                Line::from(Span::styled("2. Enter this code:", self.theme.dim())),
                Line::from(Span::styled(prompt.user_code.clone(), self.theme.header())),
                Line::from(""),
                Line::from(Span::styled(prompt.message.clone(), self.theme.dim())),
                Line::from(""),
                Line::from(Span::styled(
                    "Waiting for sign-in... Esc to dismiss",
                    self.theme.accent(),
                )),
            ];
            crate::widgets::render_popup(frame, popup, "Microsoft Sign-in", lines, &self.theme);
        }
        Overlay::Wizard(_) => self.render_wizard(frame, area),
        Overlay::Picker(picker) => {
            let popup = crate::widgets::centered_rect(60, 72, area);
            frame.render_widget(ratatui::widgets::Clear, popup);
            frame.render_widget(Block::default().style(surface), popup);
            crate::views::accent_bar(frame, popup, &self.theme);

            let content = Rect {
                x: popup.x + 2,
                y: popup.y + 1,
                width: popup.width.saturating_sub(3),
                height: popup.height.saturating_sub(2),
            };
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Min(1),
                ])
                .split(content);
            frame.render_widget(
                Paragraph::new(Span::styled(picker.title.clone(), self.theme.header()))
                    .style(surface),
                rows[0],
            );
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled("Filter  ", self.theme.comment_style()),
                    Span::styled(format!("{}█", picker.query), self.theme.accent()),
                ]))
                .style(surface),
                rows[2],
            );

            let list_area = rows[3];
            let visible = list_area.height as usize;
            let start = picker.selected.saturating_sub(visible.saturating_sub(1));
            for row in 0..visible {
                let idx = start + row;
                if idx >= picker.filtered.len() {
                    break;
                }
                let real = picker.filtered[idx];
                let Some(label) = picker.items.get(real) else {
                    continue;
                };
                let rect = Rect {
                    x: list_area.x,
                    y: list_area.y + row as u16,
                    width: list_area.width,
                    height: 1,
                };
                let style = if idx == picker.selected {
                    self.theme.row_selected()
                } else {
                    self.theme.row()
                };
                frame.render_widget(
                    Paragraph::new(Span::styled(label.clone(), style)).style(surface),
                    rect,
                );
                self.push_hitbox(rect, HitAction::Overlay(OverlayAction::PickerItem(idx)));
            }
            if picker.filtered.is_empty() {
                frame.render_widget(
                    Paragraph::new(Span::styled("No matches.", self.theme.dim()))
                        .style(surface),
                    list_area,
                );
            }
        }
    }
}
}
