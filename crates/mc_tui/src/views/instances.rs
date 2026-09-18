//! Instances screen: list, launch actions and instance details.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, ButtonId, HitAction};
use crate::views::{buttons_row, jump, move_sel, register_rows};

impl App {
    pub(crate) fn render_instances(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(5),
                Constraint::Length(9),
            ])
            .split(area);

        buttons_row(
            self,
            frame,
            chunks[0].x + 1,
            chunks[0].y,
            area.x + area.width,
            &[
                ("Launch", ButtonId::Launch),
                ("New", ButtonId::NewInstance),
                ("Install", ButtonId::InstallInstance),
                ("Edit", ButtonId::EditInstance),
                ("Delete", ButtonId::DeleteInstance),
            ],
        );

        let items: Vec<ListItem> = self
            .instances
            .iter()
            .map(|instance| {
                let badge = instance.metadata.loader.label();
                let pack = instance
                    .metadata
                    .modpack
                    .as_ref()
                    .map(|m| format!("  ⛁ {}", m.name))
                    .unwrap_or_default();
                ListItem::new(Line::from(vec![
                    Span::styled(instance.name().to_string(), self.theme.base()),
                    Span::styled(
                        format!("  {}", instance.metadata.descriptor()),
                        self.theme.dim(),
                    ),
                    Span::styled(format!("  [{badge}]"), self.theme.accent()),
                    Span::styled(pack, self.theme.info_style()),
                ]))
            })
            .collect();

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.block_border())
            .title(
                Line::from(format!(" Instances ({}) ", self.instances.len()))
                    .style(self.theme.header()),
            );
        let inner = block.inner(chunks[1]);
        let list = List::new(items)
            .block(block)
            .highlight_style(self.theme.selection())
            .highlight_symbol("▸ ");
        frame.render_stateful_widget(list, chunks[1], &mut self.instance_state);
        register_rows(
            &mut self.hitboxes,
            &self.instance_state,
            inner,
            self.instances.len(),
            HitAction::InstanceRow,
        );

        self.render_instance_details(frame, chunks[2]);
    }

    fn render_instance_details(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.block_border())
            .title(Line::from(" Details ").style(self.theme.header()));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let Some(instance) = self.selected_instance().cloned() else {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "No instance selected. Press 'n' to create one.",
                    self.theme.dim(),
                ))
                .style(self.theme.base()),
                inner,
            );
            return;
        };

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
        let mod_count = self.installed_mods.len();

        let lines = vec![
            Line::from(vec![
                Span::styled("  Game:    ", self.theme.dim()),
                Span::styled(instance.metadata.game_version.clone(), self.theme.base()),
                Span::styled("   Loader: ", self.theme.dim()),
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
                Span::styled("  RAM:     ", self.theme.dim()),
                Span::styled(
                    format!("{}–{} MB", jvm.min_memory_mb, jvm.max_memory_mb),
                    self.theme.base(),
                ),
                Span::styled("   GC: ", self.theme.dim()),
                Span::styled(jvm.gc.label().to_string(), self.theme.base()),
            ]),
            Line::from(vec![
                Span::styled("  Java:    ", self.theme.dim()),
                Span::styled(java, self.theme.base()),
            ]),
            Line::from(vec![
                Span::styled("  Modpack: ", self.theme.dim()),
                Span::styled(modpack, self.theme.info_style()),
                Span::styled(format!("   Mods: {mod_count}"), self.theme.dim()),
            ]),
            Line::from(vec![
                Span::styled("  Played:  ", self.theme.dim()),
                Span::styled(last_played, self.theme.base()),
            ]),
            Line::from(vec![
                Span::styled("  Path:    ", self.theme.dim()),
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

    pub(crate) fn key_instances(&mut self, key: KeyEvent) {
        let len = self.instances.len();
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => move_sel(&mut self.instance_state, len, 1),
            KeyCode::Up | KeyCode::Char('k') => move_sel(&mut self.instance_state, len, -1),
            KeyCode::Char('g') => jump(&mut self.instance_state, len, false),
            KeyCode::Char('G') => jump(&mut self.instance_state, len, true),
            KeyCode::Enter | KeyCode::Char('l') => self.launch_selected(),
            KeyCode::Char('n') => self.open_create_instance_form(),
            KeyCode::Char('i') => self.install_selected_instance(),
            KeyCode::Char('e') => self.open_edit_instance_form(),
            KeyCode::Char('d') => self.confirm_delete_instance(),
            _ => {}
        }
    }
}
