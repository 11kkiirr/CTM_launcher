//! Modrinth Browser — a store-like page for mods, modpacks, resource packs and
//! shaders.
//!
//! Behaves like a minimal web browser: tabs for the four content types, a
//! search bar, a paged list of the most popular projects, and a project detail
//! page with a rendered Markdown description plus a preview image (truecolor
//! half-blocks) when the terminal supports it.

use std::collections::HashSet;

use crossterm::event::{KeyCode, KeyEvent};
use mc_core::modrinth::{Project, SearchHit, Version};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;
use ratatui_image::{Resize, StatefulImage};

use crate::app::{App, HitAction};
use crate::md::render_md;
use crate::views::truncate;

/// The four content categories the browser covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowseKind {
    Mods,
    Modpacks,
    Resourcepacks,
    Shaders,
}

impl BrowseKind {
    pub fn all() -> [BrowseKind; 4] {
        [
            BrowseKind::Mods,
            BrowseKind::Modpacks,
            BrowseKind::Resourcepacks,
            BrowseKind::Shaders,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            BrowseKind::Mods => "Mods",
            BrowseKind::Modpacks => "Modpacks",
            BrowseKind::Resourcepacks => "Resourcepacks",
            BrowseKind::Shaders => "Shaders",
        }
    }

    /// The Modrinth `project_type` facet value.
    pub fn project_type(&self) -> &'static str {
        match self {
            BrowseKind::Mods => "mod",
            BrowseKind::Modpacks => "modpack",
            BrowseKind::Resourcepacks => "resourcepack",
            BrowseKind::Shaders => "shader",
        }
    }

    /// Sort order used for the default "popular" listing.
    pub fn sort(&self) -> &'static str {
        "downloads"
    }
}

/// Which region of the browser owns keyboard input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowseFocus {
    List,
    Body,
    Versions,
}

/// The complete browser state.
#[derive(Debug, Clone)]
pub struct Browse {
    pub kind: BrowseKind,
    pub query: String,
    pub results: Vec<SearchHit>,
    pub selected: usize,
    pub offset: u32,
    pub total: u32,
    pub loading: bool,
    /// Detailed view state.
    pub detail: Option<Project>,
    pub versions: Vec<Version>,
    pub version_selected: usize,
    pub focus: BrowseFocus,
    pub body: Vec<Line<'static>>,
    pub body_for: String,
    pub body_width: usize,
    pub body_scroll: usize,
    pub body_visible: usize,
    pub image_requested: HashSet<String>,
}

impl Default for Browse {
    fn default() -> Self {
        Self {
            kind: BrowseKind::Mods,
            query: String::new(),
            results: Vec::new(),
            selected: 0,
            offset: 0,
            total: 0,
            loading: false,
            detail: None,
            versions: Vec::new(),
            version_selected: 0,
            focus: BrowseFocus::List,
            body: Vec::new(),
            body_for: String::new(),
            body_width: 0,
            body_scroll: 0,
            body_visible: 0,
            image_requested: HashSet::new(),
        }
    }
}

impl Browse {
    pub fn in_detail(&self) -> bool {
        self.detail.is_some()
    }
}

impl App {
    // ---------------------------------------------------------------------
    // Rendering
    // ---------------------------------------------------------------------

    pub(crate) fn render_browse(&mut self, frame: &mut Frame, area: Rect) {
        if self.browse.in_detail() {
            self.render_browse_detail(frame, area);
        } else {
            self.render_browse_list(frame, area);
        }
    }

    fn render_browse_list(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(5),
            ])
            .split(area);

        self.render_browse_tabs(frame, chunks[0]);

        // Search / status line.
        let status = if self.browse.query.is_empty() {
            format!(
                "popular {}  ·  {} results  ·  's' search, Enter open",
                self.browse.kind.label(),
                self.browse.total
            )
        } else {
            format!(
                "search '{}'  ·  {} results",
                self.browse.query,
                self.browse.total
            )
        };
        frame.render_widget(
            Paragraph::new(Span::styled(status, self.theme.card_dim())).style(self.theme.card()),
            chunks[1],
        );

        self.render_browse_results(frame, chunks[2]);
    }

    fn render_browse_tabs(&mut self, frame: &mut Frame, area: Rect) {
        let mut x = area.x;
        for kind in BrowseKind::all() {
            let label = format!(" {} ", kind.label());
            let width = label.chars().count() as u16;
            let rect = Rect {
                x,
                y: area.y,
                width,
                height: 1,
            };
            let style = if kind == self.browse.kind {
                self.theme.accent_bright()
            } else if self.is_hovered(rect) {
                self.theme.hover()
            } else {
                self.theme.dim()
            };
            frame.render_widget(Paragraph::new(Span::styled(label, style)), rect);
            self.push_hitbox(rect, HitAction::BrowseKindTab(kind));
            x += width + 1;
        }
        let hint = "  1-4 type · s search · n next page";
        frame.render_widget(
            Paragraph::new(Span::styled(hint, self.theme.card_comment())),
            Rect { x, y: area.y, width: area.width.saturating_sub(x.saturating_sub(area.x)), height: 1 },
        );
    }

    fn render_browse_results(&mut self, frame: &mut Frame, area: Rect) {
        let inner = crate::views::card(self, frame, area, self.browse.focus == BrowseFocus::List);
        if inner.height == 0 {
            return;
        }
        let hits = self.browse.results.clone();
        let visible = inner.height as usize;
        let start = self.browse.selected.saturating_sub(visible.saturating_sub(1));
        let end = (start + visible).min(hits.len().max(1));
        let visible_hits: Vec<SearchHit> = hits
            .iter()
            .skip(start)
            .take(end - start)
            .map(SearchHit::clone)
            .collect();
        self.browse_prefetch_icons(&visible_hits);
        for row in 0..visible {
            let idx = start + row;
            let Some(hit) = hits.get(idx).cloned() else {
                break;
            };
            let rect = Rect {
                x: inner.x,
                y: inner.y + row as u16,
                width: inner.width,
                height: 1,
            };
            let style = if idx == self.browse.selected {
                self.theme.row_selected()
            } else if self.is_hovered(rect) {
                self.theme.row_hover()
            } else {
                self.theme.row()
            };
            let categories = if hit.display_categories.is_empty() {
                String::new()
            } else {
                hit.display_categories.first().cloned().unwrap_or_default()
            };
            let mut spans: Vec<Span> = Vec::new();
            spans.extend(self.browse_row_icon(&hit));
            spans.push(Span::styled(truncate(&hit.title, 26), style));
            spans.push(Span::styled(format!("  by {}", truncate(&hit.author, 12)), self.theme.card_dim()));
            spans.push(Span::styled(format!("  ⤓ {}", hit.downloads), self.theme.accent()));
            spans.push(Span::styled(format!("  ♡ {}", hit.follows), self.theme.card_dim()));
            spans.push(Span::styled(format!("  {categories}"), self.theme.card_comment()));
            frame.render_widget(Paragraph::new(Line::from(spans)).style(self.theme.card()), rect);
            self.push_hitbox(rect, HitAction::BrowseResult(idx));
        }
        if hits.is_empty() {
            let hint = if self.browse.loading {
                "Loading…".to_string()
            } else if self.browse.query.is_empty() {
                "No projects found.".to_string()
            } else {
                format!("No results for '{}'. Press 's' to change the query.", self.browse.query)
            };
            frame.render_widget(
                Paragraph::new(Span::styled(hint, self.theme.card_dim())).style(self.theme.card()),
                inner,
            );
        }
    }

    /// Request icon fetches for the currently visible search results. Icons are
    /// fetched lazily (and deduplicated) so only what is on screen downloads.
    fn browse_prefetch_icons(&mut self, hits: &[SearchHit]) {
        for hit in hits {
            if let Some(url) = hit.icon_url.as_ref() {
                self.browse_fetch_image(url);
            }
        }
    }

/// A small icon thumbnail (or a coloured placeholder) for a search result,
/// plus a trailing space. When a full-resolution image is queued on top, this
/// renders the cheap colour swatch as a fallback underneath.
fn browse_row_icon(&self, hit: &SearchHit) -> Vec<Span<'_>> {
    const ICON_W: u32 = 3;
    let mut spans: Vec<Span> = Vec::new();
    let color = hit
        .color
        .map(|c| ratatui::style::Color::Rgb((c >> 16) as u8, (c >> 8) as u8, c as u8))
        .unwrap_or(self.theme.green_dim);
    let fallback = Span::styled("   ", Style::default().bg(color));
    let Some(url) = hit.icon_url.as_ref() else {
        spans.push(fallback);
        spans.push(Span::styled(" ", self.theme.card()));
        return spans;
    };
    let Some(img) = self.browse_images.get(url) else {
        spans.push(fallback);
        spans.push(Span::styled(" ", self.theme.card()));
        return spans;
    };
    if !crate::images::terminal_supports_truecolor() {
        spans.push(fallback);
    } else {
        let lines = crate::images::image_lines(img, ICON_W, 1, self.theme.panel);
        if let Some(line) = lines.first() {
            for s in line.spans.iter() {
                spans.push(Span::styled(s.content.to_string(), s.style));
            }
        }
    }
    spans.push(Span::styled(" ", self.theme.card()));
    spans
}

    fn render_browse_detail(&mut self, frame: &mut Frame, area: Rect) {
        let Some(project) = self.browse.detail.clone() else {
            return;
        };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Min(5)])
            .split(area);

        self.render_browse_tabs(frame, chunks[0]);
        self.render_browse_detail_header(frame, chunks[1], &project);

        let left_w = (area.width as usize * 2 / 5).min(38).max(24) as u16;
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(left_w), Constraint::Length(1), Constraint::Min(1)])
            .split(chunks[2]);
        self.render_browse_detail_left(frame, columns[0]);
        self.render_browse_detail_body(frame, columns[2]);
    }

    fn render_browse_detail_header(&mut self, frame: &mut Frame, area: Rect, project: &Project) {
        let line = Line::from(vec![
            Span::styled("◀ back  ", self.theme.accent()),
            Span::styled(truncate(&project.title, 28), self.theme.header()),
            Span::styled(format!("  {}  ", project.project_type), self.theme.card_comment()),
            Span::styled(format!("⤓ {}", project.downloads), self.theme.accent()),
            Span::styled(format!("  ♡ {}", project.followers), self.theme.card_dim()),
            Span::styled(
                format!("  {} version(s)", self.browse.versions.len()),
                self.theme.card_comment(),
            ),
        ]);
        frame.render_widget(Paragraph::new(line).style(self.theme.card()), area);
    }

    fn render_browse_detail_left(&mut self, frame: &mut Frame, area: Rect) {
        let inner = crate::views::card(
            self,
            frame,
            area,
            self.browse.focus == BrowseFocus::Versions,
        );
        if inner.height == 0 {
            return;
        }
        let Some(project) = self.browse.detail.clone() else {
            return;
        };

        let icon_h = 9u16;
        let meta_h = 4u16;
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(icon_h),
                Constraint::Length(meta_h),
                Constraint::Length(1),
                Constraint::Min(2),
                Constraint::Length(1),
            ])
            .split(inner);

        // Icon preview (truecolor half-blocks) or a colour swatch.
        self.render_browse_icon(frame, rows[0], &project);

        // Meta block.
        let author = project
            .additional_categories
            .first()
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());
        let updated = project.updated.clone();
        let updated = if updated.is_empty() {
            String::new()
        } else {
            updated.chars().take(10).collect::<String>()
        };
        let meta: Vec<Line> = vec![
            Line::from(vec![
                Span::styled("Author  ", self.theme.card_dim()),
                Span::styled(author, self.theme.card()),
            ]),
            Line::from(vec![
                Span::styled("License ", self.theme.card_dim()),
                Span::styled(
                    project.license.as_ref().map(|l| l.name.clone()).unwrap_or_default(),
                    self.theme.card(),
                ),
            ]),
            Line::from(vec![
                Span::styled("Updated ", self.theme.card_dim()),
                Span::styled(updated, self.theme.card()),
            ]),
            Line::from(vec![
                Span::styled("Install ", self.theme.card_dim()),
                Span::styled(self.browse.kind.label().to_string(), self.theme.accent()),
            ]),
        ];
        frame.render_widget(Paragraph::new(meta).style(self.theme.card()), rows[1]);

        // Versions header + list.
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Versions", self.theme.header()),
                Span::styled("   v focus · Enter install", self.theme.card_dim()),
            ]))
            .style(self.theme.card()),
            rows[2],
        );

        let versions_area = rows[3];
        let visible = versions_area.height as usize;
        let start = self
            .browse
            .version_selected
            .saturating_sub(visible.saturating_sub(1));
        for row in 0..visible {
            let idx = start + row;
            let Some(version) = self.browse.versions.get(idx).cloned() else {
                break;
            };
            let rect = Rect {
                x: versions_area.x,
                y: versions_area.y + row as u16,
                width: versions_area.width,
                height: 1,
            };
            let style = if idx == self.browse.version_selected {
                self.theme.row_selected()
            } else if self.is_hovered(rect) {
                self.theme.row_hover()
            } else {
                self.theme.row()
            };
            let game = version
                .game_versions
                .first()
                .cloned()
                .unwrap_or_default();
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(truncate(&version.version_number, 12), style),
                    Span::styled(format!("  {game}", ), self.theme.card_dim()),
                    Span::styled(format!("  {}", version.loaders.join("+")), self.theme.card_comment()),
                ]))
                .style(self.theme.card()),
                rect,
            );
            self.push_hitbox(rect, HitAction::BrowseVersion(idx));
        }
        if self.browse.versions.is_empty() {
            frame.render_widget(
                Paragraph::new(Span::styled("No versions", self.theme.card_dim()))
                    .style(self.theme.card()),
                versions_area,
            );
        }

        // Install button.
        let install_rect = rows[4];
        let install_style = if self.is_hovered(install_rect) {
            self.theme.hover()
        } else {
            self.theme.accent()
        };
        frame.render_widget(
            Paragraph::new(Span::styled("  [ Install selected version ]", install_style))
                .style(self.theme.card()),
            install_rect,
        );
        self.push_hitbox(install_rect, HitAction::BrowseInstall);
    }

    fn render_browse_icon(&mut self, frame: &mut Frame, area: Rect, project: &Project) {
        let color = project
            .color
            .map(|c| ratatui::style::Color::Rgb((c >> 16) as u8, (c >> 8) as u8, c as u8))
            .unwrap_or(self.theme.green_dim);
        let Some(url) = project.icon_url.as_ref() else {
            self.render_browse_icon_fallback(frame, area, project, color);
            return;
        };
        let Some(img) = self.browse_images.get(url) else {
            self.render_browse_icon_fallback(frame, area, project, color);
            return;
        };

        // Build the render state once per image: it encodes the pixels for the
        // negotiated protocol (kitty/sixel/iterm2/halfblocks) and re-uses the
        // encoding while the area stays the same.
        if self.browse_protocols.get(url).is_none() {
            let Some(dynamic) = crate::images::to_dynamic_image(&img) else {
                self.render_browse_icon_fallback(frame, area, project, color);
                return;
            };
            let protocol = self.picker.new_resize_protocol(dynamic);
            self.browse_protocols.insert(url.to_string(), protocol);
        }
        if let Some(proto) = self.browse_protocols.get_mut(url) {
            frame.render_stateful_widget(
                StatefulImage::default().resize(Resize::Fit(None)),
                area,
                proto,
            );
            return;
        }
        self.render_browse_icon_fallback(frame, area, project, color);
    }

    /// Placeholder when no icon image is available or the terminal lacks
    /// truecolor: a project-coloured swatch with the title.
    fn render_browse_icon_fallback(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        project: &Project,
        color: ratatui::style::Color,
    ) {
        let suffix = if crate::images::terminal_supports_truecolor() {
            String::new()
        } else {
            " (no image support)".to_string()
        };
        let max_w = (area.width as usize).saturating_sub(4 + suffix.chars().count());
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!("  {}{suffix}  ", truncate(&project.title, max_w)),
                Style::default().fg(color).bg(self.theme.panel),
            ))
            .style(self.theme.card()),
            area,
        );
    }

    fn render_browse_detail_body(&mut self, frame: &mut Frame, area: Rect) {
        let inner = crate::views::card(self, frame, area, self.browse.focus == BrowseFocus::Body);
        if inner.height == 0 {
            return;
        }
        let Some(project) = self.browse.detail.clone() else {
            return;
        };

        let header = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Description", self.theme.header()),
                Span::styled("   j/k scroll · v versions", self.theme.card_dim()),
            ]))
            .style(self.theme.card()),
            header,
        );

        let body_area = Rect {
            x: inner.x,
            y: inner.y + 1,
            width: inner.width,
            height: inner.height.saturating_sub(1),
        };
        let body_w = (inner.width as usize).saturating_sub(2).max(10);

        // Re-render the cached body when the project or the width changed.
        if self.browse.body_for != project.id || self.browse.body_width != body_w {
            self.browse.body = render_md(&project.body, body_w, &self.theme);
            self.browse.body_for = project.id.clone();
            self.browse.body_width = body_w;
            self.browse.body_scroll = self.browse.body_scroll.min(self.browse.body.len().saturating_sub(1));
        }
        let visible = body_area.height as usize;
        self.browse.body_visible = visible;
        self.browse.body_scroll = self.browse.body_scroll.min(
            self.browse.body.len().saturating_sub(visible).max(0),
        );
        let start = self.browse.body_scroll;
        let end = (start + visible).min(self.browse.body.len().max(1));

        frame.render_widget(Clear, body_area);
        if self.browse.body.is_empty() {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "No description provided.",
                    self.theme.card_dim(),
                ))
                .style(self.theme.card()),
                body_area,
            );
            return;
        }
        let lines: Vec<Line> = self.browse.body.iter().skip(start).take(end - start).map(Line::clone).collect();
        frame.render_widget(Paragraph::new(lines).style(self.theme.card()), body_area);
    }

    // ---------------------------------------------------------------------
    // Keyboard input
    // ---------------------------------------------------------------------

    pub(crate) fn key_browse(&mut self, key: KeyEvent) {
        if self.browse.in_detail() {
            self.key_browse_detail(key);
        } else {
            self.key_browse_list(key);
        }
    }

    fn key_browse_list(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => self.browse_move(1),
            KeyCode::Up | KeyCode::Char('k') => self.browse_move(-1),
            KeyCode::Char('g') => {
                self.browse.selected = 0;
            }
            KeyCode::Char('G') => {
                self.browse.selected = self.browse.results.len().saturating_sub(1);
            }
            KeyCode::Char('n') | KeyCode::Char('N') => self.browse_load_more(),
            KeyCode::Char('s') | KeyCode::Char('/') => self.open_browse_search(),
            KeyCode::Char('1') => self.browse_switch_kind(BrowseKind::Mods),
            KeyCode::Char('2') => self.browse_switch_kind(BrowseKind::Modpacks),
            KeyCode::Char('3') => self.browse_switch_kind(BrowseKind::Resourcepacks),
            KeyCode::Char('4') => self.browse_switch_kind(BrowseKind::Shaders),
            KeyCode::Char('t') => self.browse_cycle_kind(),
            KeyCode::Enter => self.browse_open_selected(),
            _ => {}
        }
    }

    fn key_browse_detail(&mut self, key: KeyEvent) {
        match self.browse.focus {
            BrowseFocus::Body => match key.code {
                KeyCode::Down | KeyCode::Char('j') => self.browse_scroll_body(1),
                KeyCode::Up | KeyCode::Char('k') => self.browse_scroll_body(-1),
                KeyCode::PageDown => self.browse_scroll_body(10),
                KeyCode::PageUp => self.browse_scroll_body(-10),
                KeyCode::Tab | KeyCode::Char('v') => self.browse.focus = BrowseFocus::Versions,
                _ => {}
            },
            BrowseFocus::Versions => match key.code {
                KeyCode::Down | KeyCode::Char('j') => self.browse_version_move(1),
                KeyCode::Up | KeyCode::Char('k') => self.browse_version_move(-1),
                KeyCode::Tab | KeyCode::Char('v') => self.browse.focus = BrowseFocus::Body,
                KeyCode::Enter | KeyCode::Char('i') => self.browse_install(),
                _ => {}
            },
            _ => {}
        }
    }

    /// Mouse wheel / page scroll routing.
    pub(crate) fn browse_scroll(&mut self, delta: i32) {
        if self.browse.in_detail() {
            if self.browse.focus == BrowseFocus::Versions {
                self.browse_version_move(delta * 3);
            } else {
                self.browse_scroll_body(delta * 3);
            }
        } else {
            self.browse_move(delta * 4);
        }
    }

    fn browse_move(&mut self, delta: i32) {
        let len = self.browse.results.len();
        if len == 0 {
            return;
        }
        let next = (self.browse.selected as i32 + delta).clamp(0, len as i32 - 1) as usize;
        self.browse.selected = next;
    }

    pub(crate) fn browse_version_move(&mut self, delta: i32) {
        let len = self.browse.versions.len();
        if len == 0 {
            return;
        }
        let next = (self.browse.version_selected as i32 + delta).clamp(0, len as i32 - 1) as usize;
        self.browse.version_selected = next;
    }

    fn browse_scroll_body(&mut self, delta: i32) {
        let max = self.browse.body.len().saturating_sub(self.browse.body_visible).max(0);
        let next = (self.browse.body_scroll as i32 + delta).clamp(0, max as i32) as usize;
        self.browse.body_scroll = next;
    }

    // ---------------------------------------------------------------------
    // Mouse actions
    // ---------------------------------------------------------------------

    pub(crate) fn browse_select_result(&mut self, idx: usize) {
        if idx < self.browse.results.len() {
            self.browse.selected = idx;
            self.browse_open_selected();
        }
    }

    pub(crate) fn browse_select_version(&mut self, idx: usize) {
        if idx < self.browse.versions.len() {
            self.browse.version_selected = idx;
            self.browse.focus = BrowseFocus::Versions;
        }
    }

    pub(crate) fn browse_switch_kind(&mut self, kind: BrowseKind) {
        if self.browse.kind == kind && self.browse.detail.is_none() && !self.browse.results.is_empty() {
            return;
        }
        self.browse.kind = kind;
        self.browse_close_detail();
        self.browse_query_reset();
        self.browse_load_first_page();
    }

    pub(crate) fn browse_cycle_kind(&mut self) {
        let kinds = BrowseKind::all();
        let idx = kinds.iter().position(|k| *k == self.browse.kind).unwrap_or(0);
        self.browse_switch_kind(kinds[(idx + 1) % kinds.len()]);
    }
}