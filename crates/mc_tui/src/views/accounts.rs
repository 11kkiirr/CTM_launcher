//! Accounts & Skins screen: account switcher, device-code auth and skin tools.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, ButtonId, Focus, HitAction};
use crate::views::{
    buttons_row, card, hovered_index, jump, move_sel, register_rows, row_style, section_title,
};

impl App {
    pub(crate) fn render_accounts(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(5),
                Constraint::Length(1),
                Constraint::Length(10),
            ])
            .split(area);

        let offline = self.tr("btn.offline");
        let microsoft = self.tr("btn.microsoft");
        let set_active = self.tr("btn.set_active");
        let skin = self.tr("btn.skin");
        let remove = self.tr("btn.remove");
        buttons_row(
            self,
            frame,
            chunks[0].x + 2,
            chunks[0].y,
            area.x + area.width,
            &[
                (offline, "n", ButtonId::OfflineLogin),
                (microsoft, "m", ButtonId::MicrosoftLogin),
                (set_active, "Enter", ButtonId::SetActiveAccount),
                (skin, "c", ButtonId::ChangeSkin),
                (remove, "d", ButtonId::DeleteAccount),
            ],
        );

        self.render_account_list(frame, chunks[2]);
        self.render_account_details(frame, chunks[4]);
    }

    fn render_account_list(&mut self, frame: &mut Frame, area: Rect) {
        let focused = self.focus == Focus::Content;
        let inner = card(self, frame, area, focused);
        if inner.height == 0 {
            return;
        }

        let account_count = self.accounts.accounts().len();
        let title = self.tr("accounts.title").to_string();
        frame.render_widget(
            Paragraph::new(section_title(&title, &account_count.to_string(), &self.theme))
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
        let active_id = self.accounts.active_id().map(str::to_string);
        let selected = self.account_state.selected();
        let hovered = hovered_index(self, list_area, self.account_state.offset(), account_count);
        let items: Vec<ListItem> = self
            .accounts
            .accounts()
            .iter()
            .enumerate()
            .map(|(idx, account)| {
                let active = active_id.as_deref() == Some(account.id.as_str());
                let marker = if active { "● " } else { "○ " };
                let kind = match account.kind {
                    mc_core::auth::AccountKind::Microsoft => "Microsoft",
                    mc_core::auth::AccountKind::Offline => "Offline",
                };
                ListItem::new(Line::from(vec![
                    Span::styled(marker, self.theme.accent()),
                    Span::styled(account.username.clone(), self.theme.card()),
                    Span::styled(format!("  [{kind}]"), self.theme.card_dim()),
                ]))
                .style(row_style(self, idx, selected, hovered))
            })
            .collect();

        let list = List::new(items)
            .highlight_symbol("▸ ")
            .highlight_style(self.theme.row_selected());
        frame.render_stateful_widget(list, list_area, &mut self.account_state);
        register_rows(
            &mut self.hitboxes,
            &self.account_state,
            list_area,
            account_count,
            HitAction::AccountRow,
        );

        if account_count == 0 {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    self.tr("empty.no_accounts"),
                    self.theme.card_dim(),
                ))
                .style(self.theme.card()),
                list_area,
            );
        }
    }

    fn render_account_details(&mut self, frame: &mut Frame, area: Rect) {
        let inner = card(self, frame, area, false);
        if inner.height == 0 {
            return;
        }

        let account = self
            .account_state
            .selected()
            .and_then(|idx| self.accounts.accounts().get(idx))
            .cloned();

        let Some(account) = account else {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    self.tr("empty.accounts_hint"),
                    self.theme.card_dim(),
                ))
                .style(self.theme.card()),
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
                    self.tr("accounts.valid").to_string()
                } else {
                    self.tr("accounts.expired").to_string()
                }
            }
            mc_core::auth::AccountKind::Offline => self.tr("accounts.na").to_string(),
        };
        let head = mc_core::skins::head_render_url(&account.id, 128);
        let body = mc_core::skins::body_render_url(&account.id);
        let active = self.accounts.active_id() == Some(account.id.as_str());

        let lines = vec![
            Line::from(vec![
                Span::styled(account.username.clone(), self.theme.header()),
                Span::styled(
                    if active {
                        self.tr("accounts.active").to_string()
                    } else {
                        String::new()
                    },
                    self.theme.accent(),
                ),
            ]),
            Line::from(vec![
                Span::styled(self.tr("accounts.type"), self.theme.card_dim()),
                Span::styled(kind.to_string(), self.theme.card()),
                Span::styled(self.tr("accounts.token"), self.theme.card_dim()),
                Span::styled(token_state, self.theme.card()),
            ]),
            Line::from(vec![
                Span::styled(self.tr("accounts.uuid"), self.theme.card_dim()),
                Span::styled(account.id.clone(), self.theme.card()),
            ]),
            Line::from(vec![
                Span::styled(self.tr("accounts.head"), self.theme.card_dim()),
                Span::styled(head, self.theme.info_style()),
            ]),
            Line::from(vec![
                Span::styled(self.tr("accounts.body"), self.theme.card_dim()),
                Span::styled(body, self.theme.info_style()),
            ]),
            Line::from(Span::styled(
                self.tr("empty.skin_hint"),
                self.theme.card_dim(),
            )),
        ];
        frame.render_widget(
            Paragraph::new(lines)
                .style(self.theme.card())
                .wrap(Wrap { trim: false }),
            inner,
        );
    }

    pub(crate) fn key_accounts(&mut self, key: KeyEvent) {
        let len = self.accounts.accounts().len();
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => move_sel(&mut self.account_state, len, 1),
            KeyCode::Up | KeyCode::Char('k') => move_sel(&mut self.account_state, len, -1),
            KeyCode::PageDown => move_sel(&mut self.account_state, len, 10),
            KeyCode::PageUp => move_sel(&mut self.account_state, len, -10),
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
