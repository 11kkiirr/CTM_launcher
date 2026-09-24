//! Settings screen: Java paths, default RAM/GC and UI toggles.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::{App, ButtonId, Focus, HitAction};
use crate::views::{buttons_row, card, section_title};

const FIELD_COUNT: usize = 9;

impl App {
    pub(crate) fn render_settings(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(FIELD_COUNT as u16 + 3),
                Constraint::Length(1),
                Constraint::Min(5),
            ])
            .split(area);

        let edit = self.tr("btn.edit");
        let save = self.tr("btn.save");
        let detect = self.tr("btn.detect_java");
        let ascii = self.tr("btn.ascii_folder");
        buttons_row(
            self,
            frame,
            chunks[0].x + 2,
            chunks[0].y,
            area.x + area.width,
            &[
                (edit, "e", ButtonId::EditSettings),
                (save, "s", ButtonId::SaveSettings),
                (detect, "J", ButtonId::DetectJava),
                (ascii, "a", ButtonId::OpenAsciiBgFolder),
            ],
        );

        self.render_settings_fields(frame, chunks[2]);
        self.render_java_list(frame, chunks[4]);
    }

    fn render_settings_fields(&mut self, frame: &mut Frame, area: Rect) {
        let focused = self.focus == Focus::Content;
        let inner = card(self, frame, area, focused);
        if inner.height == 0 {
            return;
        }

        let title = self.tr("settings.title").to_string();
        frame.render_widget(
            Paragraph::new(section_title(&title, "", &self.theme))
                .style(self.theme.card()),
            Rect {
                x: inner.x,
                y: inner.y,
                width: inner.width,
                height: 1,
            },
        );

        let lang = self.settings.language;
        let s = &self.settings;
        let java = s
            .java_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| crate::i18n::tr(lang, "settings.auto_detect").to_string());
        let yes = crate::i18n::tr(lang, "common.yes").to_string();
        let no = crate::i18n::tr(lang, "common.no").to_string();
        let mb = crate::i18n::tr(lang, "settings.mb");
        let values = [
            (
                crate::i18n::tr(lang, "settings.java_path").to_string(),
                java,
            ),
            (
                crate::i18n::tr(lang, "settings.min_ram").to_string(),
                format!("{} {}", s.default_min_memory_mb, mb),
            ),
            (
                crate::i18n::tr(lang, "settings.max_ram").to_string(),
                format!("{} {}", s.default_max_memory_mb, mb),
            ),
            (
                crate::i18n::tr(lang, "settings.gc").to_string(),
                s.default_gc.label().to_string(),
            ),
            (
                crate::i18n::tr(lang, "settings.show_progress").to_string(),
                if s.show_progress { yes.clone() } else { no.clone() },
            ),
            (
                crate::i18n::tr(lang, "settings.confirm_quit").to_string(),
                if s.confirm_quit { yes.clone() } else { no.clone() },
            ),
            (
                crate::i18n::tr(lang, "settings.auto_scroll").to_string(),
                if s.log_auto_scroll { yes.clone() } else { no.clone() },
            ),
            (
                crate::i18n::tr(lang, "settings.ascii_anchor").to_string(),
                s.ascii_bg_anchor.label_lang(lang),
            ),
            (
                crate::i18n::tr(lang, "settings.language").to_string(),
                s.language.native_label().to_string(),
            ),
        ];

        for (idx, (label, value)) in values.iter().enumerate() {
            let y = inner.y + 2 + idx as u16;
            if y >= inner.y + inner.height {
                break;
            }
            let row = Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: 1,
            };
            let selected = idx == self.settings_field;
            let style = if selected {
                self.theme.row_selected()
            } else {
                self.theme.row()
            };
            let marker = if selected { "▸ " } else { "  " };
            let line = Line::from(vec![
                Span::styled(marker.to_string(), style),
                Span::styled(format!("{label:<20}"), self.theme.card_dim()),
                Span::styled(value.clone(), style),
            ]);
            frame.render_widget(Paragraph::new(line).style(style), row);
            self.hitboxes.push(crate::app::Hitbox {
                rect: row,
                action: HitAction::SettingsRow(idx),
            });
        }
    }

    fn render_java_list(&mut self, frame: &mut Frame, area: Rect) {
        let inner = card(self, frame, area, false);
        if inner.height == 0 {
            return;
        }

        let title = self.tr("settings.java_runtimes").to_string();
        let rescan = self.tr("settings.rescan_hint").to_string();
        let suffix = format!("{}  ·  {}", self.java_installations.len(), rescan);
        frame.render_widget(
            Paragraph::new(section_title(&title, &suffix, &self.theme))
                .style(self.theme.card()),
            Rect {
                x: inner.x,
                y: inner.y,
                width: inner.width,
                height: 1,
            },
        );

        let list_area = Rect {
            x: inner.x,
            y: inner.y + 1,
            width: inner.width,
            height: inner.height.saturating_sub(1),
        };
        let items: Vec<ListItem> = self
            .java_installations
            .iter()
            .map(|java| {
                ListItem::new(Line::from(vec![
                    Span::styled(format!("Java {:<3} ", java.major), self.theme.accent()),
                    Span::styled(java.version.clone(), self.theme.card()),
                    Span::styled(format!("  {}", java.path.display()), self.theme.card_dim()),
                ]))
                .style(self.theme.row())
            })
            .collect();

        frame.render_widget(List::new(items).style(self.theme.card()), list_area);

        if self.java_installations.is_empty() {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    self.tr("settings.scanning_java"),
                    self.theme.card_dim(),
                ))
                .style(self.theme.card()),
                list_area,
            );
        }
    }

    pub(crate) fn key_settings(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                self.settings_field = (self.settings_field + 1).min(FIELD_COUNT - 1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.settings_field = self.settings_field.saturating_sub(1);
            }
            KeyCode::Enter => self.open_settings_form(),
            KeyCode::Char(' ') => self.toggle_setting(self.settings_field),
            KeyCode::Left => match self.settings_field {
                3 => self.cycle_setting_gc(false),
                7 => self.cycle_ascii_bg_anchor(false),
                8 => self.cycle_setting_language(false),
                _ => {}
            },
            KeyCode::Right => match self.settings_field {
                3 => self.cycle_setting_gc(true),
                7 => self.cycle_ascii_bg_anchor(true),
                8 => self.cycle_setting_language(true),
                _ => {}
            },
            KeyCode::Char('s') => self.save_settings(),
            KeyCode::Char('J') => self.spawn_java_discovery(),
            KeyCode::Char('a') => self.open_ascii_bg_folder(),
            _ => {}
        }
    }
}
