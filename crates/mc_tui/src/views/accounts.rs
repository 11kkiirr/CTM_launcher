//! Accounts & Skins screen: account switcher, device-code auth and skin tools.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, ButtonId, HitAction};
use crate::views::{buttons_row, hovered_index, jump, move_sel, register_rows, row_style};

impl App {
    pub(crate) fn render_accounts(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(5),
                Constraint::Length(10),
            ])
            .split(area);

        buttons_row(
            self,
            frame,
            chunks[0].x + 1,
            chunks[0].y,
            area.x + area.width,
            &[
                ("Offline", ButtonId::OfflineLogin),
                ("Microsoft", ButtonId::MicrosoftLogin),
                ("Set Active", ButtonId::SetActiveAccount),
                ("Skin", ButtonId::ChangeSkin),
                ("Remove", ButtonId::DeleteAccount),
            ],
        );

        let inner = Block::default().borders(Borders::ALL).inner(chunks[1]);
        let active_id = self.accounts.active_id().map(str::to_string);
        let account_count = self.accounts.accounts().len();
        let selected = self.account_state.selected();
        let hovered = hovered_index(self, inner, self.account_state.offset(), account_count);
        let items: Vec<ListItem> = self
            .accounts
            .accounts()
            .iter()
            .enumerate()
            .map(|(idx, account)| {
                let active = active_id.as_deref() == Some(account.id.as_str());
                let marker = if active { "● " } else { "  " };
                let kind = match account.kind {
                    mc_core::auth::AccountKind::Microsoft => "Microsoft",
                    mc_core::auth::AccountKind::Offline => "Offline",
                };
                ListItem::new(Line::from(vec![
                    Span::styled(marker, self.theme.accent()),
                    Span::styled(account.username.clone(), self.theme.base()),
                    Span::styled(format!("  [{kind}]"), self.theme.dim()),
                ]))
                .style(row_style(self, idx, selected, hovered))
            })
            .collect();

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.block_border())
            .title(
                Line::from(format!(" Accounts ({}) ", self.accounts.accounts().len()))
                    .style(self.theme.header()),
            );
        let list = List::new(items).block(block).highlight_symbol("▸ ");
        frame.render_stateful_widget(list, chunks[1], &mut self.account_state);
        register_rows(
            &mut self.hitboxes,
            &self.account_state,
            inner,
            self.accounts.accounts().len(),
            HitAction::AccountRow,
        );

        self.render_account_details(frame, chunks[2]);
    }

    fn render_account_details(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.block_border())
            .title(Line::from(" Account / Skin ").style(self.theme.header()));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let account = self
            .account_state
            .selected()
            .and_then(|idx| self.accounts.accounts().get(idx))
            .cloned();

        let Some(account) = account else {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "Press 'n' for an offline account or 'm' to sign in with Microsoft.",
                    self.theme.dim(),
                ))
                .style(self.theme.base()),
                inner,
            );
            return;
        };

        let kind = match account.kind {
            mc_core::auth::AccountKind::Microsoft => "Microsoft",
            mc_core::auth::AccountKind::Offline => "Offline",
        };
        let token_state = match account.kind {
            mc_core::auth::AccountKind::Microsoft => {
                if account.token_valid(60) {
                    "valid".to_string()
                } else {
                    "expired (will refresh on launch)".to_string()
                }
            }
            mc_core::auth::AccountKind::Offline => "n/a".to_string(),
        };
        let head = mc_core::skins::head_render_url(&account.id, 128);
        let body = mc_core::skins::body_render_url(&account.id);
        let active = self.accounts.active_id() == Some(account.id.as_str());

        let lines = vec![
            Line::from(vec![
                Span::styled("  User:  ", self.theme.dim()),
                Span::styled(account.username.clone(), self.theme.header()),
                Span::styled(if active { "  (active)" } else { "" }, self.theme.accent()),
            ]),
            Line::from(vec![
                Span::styled("  Type:  ", self.theme.dim()),
                Span::styled(kind.to_string(), self.theme.base()),
                Span::styled("   Token: ", self.theme.dim()),
                Span::styled(token_state, self.theme.base()),
            ]),
            Line::from(vec![
                Span::styled("  UUID:  ", self.theme.dim()),
                Span::styled(account.id.clone(), self.theme.base()),
            ]),
            Line::from(vec![
                Span::styled("  Head:  ", self.theme.dim()),
                Span::styled(head, self.theme.info_style()),
            ]),
            Line::from(vec![
                Span::styled("  Body:  ", self.theme.dim()),
                Span::styled(body, self.theme.info_style()),
            ]),
            Line::from(Span::styled(
                "  Press 'c' to set a skin from a URL or local .png (Microsoft accounts only).",
                self.theme.dim(),
            )),
        ];
        frame.render_widget(
            Paragraph::new(lines)
                .style(self.theme.base())
                .wrap(Wrap { trim: false }),
            inner,
        );
    }

    pub(crate) fn key_accounts(&mut self, key: KeyEvent) {
        let len = self.accounts.accounts().len();
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => move_sel(&mut self.account_state, len, 1),
            KeyCode::Up | KeyCode::Char('k') => move_sel(&mut self.account_state, len, -1),
            KeyCode::Char('g') => jump(&mut self.account_state, len, false),
            KeyCode::Char('G') => jump(&mut self.account_state, len, true),
            KeyCode::Char('n') => self.open_offline_login(),
            KeyCode::Char('m') => self.start_microsoft_login(),
            KeyCode::Enter => self.set_active_account(),
            KeyCode::Char('d') => self.confirm_delete_account(),
            KeyCode::Char('c') => self.open_skin_prompt(),
            _ => {}
        }
    }
}
