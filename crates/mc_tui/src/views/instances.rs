//! Instance Overview and instance-scoped Settings pages.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, ButtonId};
use crate::views::buttons_row;

impl App {
    /// The instance "Overview" page: identity, actions and details.
    pub(crate) fn render_overview(&mut self, frame: &mut Frame, area: Rect) {
        let Some(instance) = self.selected_instance().cloned() else {
            return;
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(1),
                Constraint::Min(5),
            ])
            .split(area);

        let title_block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.block_border_focused())
            .title(Line::from(" Instance ").style(self.theme.header()));
        let title_inner = title_block.inner(chunks[0]);
        frame.render_widget(title_block, chunks[0]);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(instance.name().to_string(), self.theme.header()),
                Span::styled(
                    format!("   {}", instance.metadata.descriptor()),
                    self.theme.dim(),
                ),
            ]))
            .style(self.theme.base()),
            title_inner,
        );

        buttons_row(
            self,
            frame,
            chunks[1].x + 1,
            chunks[1].y,
            area.x + area.width,
            &[
                ("Launch", ButtonId::Launch),
                ("Install / Repair", ButtonId::InstallInstance),
                ("New", ButtonId::NewInstance),
                ("Edit", ButtonId::EditInstance),
                ("Delete", ButtonId::DeleteInstance),
            ],
        );

        self.render_overview_details(frame, chunks[2], &instance);
    }

    fn render_overview_details(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        instance: &mc_core::instance::Instance,
    ) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.block_border())
            .title(Line::from(" Details ").style(self.theme.header()));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let jvm = &instance.metadata.jvm;
        let modpack = instance
            .metadata
            .modpack
            .as_ref()
            .map(|m| format!("{} {}", m.name, m.version.clone().unwrap_or_default()))
            .unwrap_or_else(|| "—".to_string());
        let java = jvm
            .java_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "auto-detect".to_string());
        let last_played = instance
            .metadata
            .last_played
            .map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
            .unwrap_or_else(|| "never".to_string());

        let lines = vec![
            Line::from(vec![
                Span::styled("  Game version   ", self.theme.dim()),
                Span::styled(instance.metadata.game_version.clone(), self.theme.base()),
            ]),
            Line::from(vec![
                Span::styled("  Modloader      ", self.theme.dim()),
                Span::styled(
                    format!(
                        "{} {}",
                        instance.metadata.loader,
                        instance.metadata.loader_version.clone().unwrap_or_default()
                    ),
                    self.theme.accent(),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Memory         ", self.theme.dim()),
                Span::styled(
                    format!("{}–{} MB", jvm.min_memory_mb, jvm.max_memory_mb),
                    self.theme.base(),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Garbage coll.  ", self.theme.dim()),
                Span::styled(jvm.gc.label().to_string(), self.theme.base()),
            ]),
            Line::from(vec![
                Span::styled("  Java           ", self.theme.dim()),
                Span::styled(java, self.theme.base()),
            ]),
            Line::from(vec![
                Span::styled("  Modpack        ", self.theme.dim()),
                Span::styled(modpack, self.theme.info_style()),
            ]),
            Line::from(vec![
                Span::styled("  Installed mods ", self.theme.dim()),
                Span::styled(self.installed_mods.len().to_string(), self.theme.base()),
            ]),
            Line::from(vec![
                Span::styled("  Last played    ", self.theme.dim()),
                Span::styled(last_played, self.theme.base()),
            ]),
            Line::from(vec![
                Span::styled("  Directory      ", self.theme.dim()),
                Span::styled(instance.root.display().to_string(), self.theme.dim()),
            ]),
        ];
        frame.render_widget(
            Paragraph::new(lines)
                .style(self.theme.base())
                .wrap(Wrap { trim: false }),
            inner,
        );
    }

    pub(crate) fn key_overview(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter | KeyCode::Char('l') => self.launch_selected(),
            KeyCode::Char('i') => self.install_selected_instance(),
            KeyCode::Char('e') => self.open_edit_instance_form(),
            KeyCode::Char('d') => self.confirm_delete_instance(),
            KeyCode::Char('n') => self.open_create_instance_form(),
            _ => {}
        }
    }

    /// The instance-scoped "Settings" page (JVM, memory, Java).
    pub(crate) fn render_instance_settings(&mut self, frame: &mut Frame, area: Rect) {
        let Some(instance) = self.selected_instance().cloned() else {
            return;
        };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(10),
                Constraint::Min(3),
            ])
            .split(area);

        buttons_row(
            self,
            frame,
            chunks[0].x + 1,
            chunks[0].y,
            area.x + area.width,
            &[("Edit Settings", ButtonId::EditInstance)],
        );

        let jvm = &instance.metadata.jvm;
        let values = [
            ("Min RAM (MB)", jvm.min_memory_mb.to_string()),
            ("Max RAM (MB)", jvm.max_memory_mb.to_string()),
            ("Garbage Collector", jvm.gc.label().to_string()),
            (
                "Java Path",
                jvm.java_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "auto-detect".to_string()),
            ),
            (
                "Fullscreen",
                if jvm.fullscreen { "yes" } else { "no" }.to_string(),
            ),
            ("Custom JVM Args", jvm.custom_jvm_args.join(" ")),
            ("Extra Game Args", jvm.extra_game_args.join(" ")),
        ];

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.block_border())
            .title(Line::from(" Instance Settings ").style(self.theme.header()));
        let inner = block.inner(chunks[1]);
        frame.render_widget(block, chunks[1]);
        for (idx, (label, value)) in values.iter().enumerate() {
            if idx as u16 >= inner.height {
                break;
            }
            let row = Rect {
                x: inner.x,
                y: inner.y + idx as u16,
                width: inner.width,
                height: 1,
            };
            let line = Line::from(vec![
                Span::styled(format!("  {label:<20}"), self.theme.dim()),
                Span::styled(value.clone(), self.theme.base()),
            ]);
            frame.render_widget(Paragraph::new(line).style(self.theme.base()), row);
        }

        frame.render_widget(
            Paragraph::new(Span::styled(
                "  Press Enter or 'e' to edit. Changes apply the next time you launch.",
                self.theme.dim(),
            ))
            .style(self.theme.base()),
            chunks[2],
        );
    }

    pub(crate) fn key_instance_settings(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter | KeyCode::Char('e') => self.open_edit_instance_form(),
            _ => {}
        }
    }
}
