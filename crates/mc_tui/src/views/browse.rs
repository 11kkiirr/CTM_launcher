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
use ratatui::widgets::{Block, Clear, Paragraph};
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
        const CARD_H: u16 = 5;
        const ICON_W: u16 = 12;
        const GAP: u16 = 1;

        let inner = crate::views::card(self, frame, area, self.browse.focus == BrowseFocus::List);
        if inner.height == 0 {
            return;
        }
        let hits = self.browse.results.clone();
        let step = CARD_H + GAP;
        let visible_items = (inner.height / step) as usize;
        let first_visible_item =
            self.browse.selected.saturating_sub(visible_items.saturating_sub(1));

        let visible_hits: Vec<SearchHit> = hits
            .iter()
            .skip(first_visible_item)
            .take(visible_items)
            .map(SearchHit::clone)
            .collect();
        self.browse_prefetch_icons(&visible_hits);

        for item in 0..visible_items {
            let idx = first_visible_item + item;
            let Some(hit) = hits.get(idx).cloned() else {
                break;
            };
            let card_y = inner.y + (item as u16) * step;
            let card_rect = Rect {
                x: inner.x,
                y: card_y,
                width: inner.width,
                height: CARD_H,
            };
            let is_selected = idx == self.browse.selected;
            let is_hovered = self.is_hovered(card_rect);

            // Card background
            let bg = if is_selected {
                self.theme.selection_bg
            } else if is_hovered {
                self.theme.hover_bg
            } else {
                self.theme.panel_alt
            };
            let surface = Style::default().bg(bg);
            frame.render_widget(Block::default().style(surface), card_rect);

            // Icon area (full card height) on the left
            let icon_rect = Rect {
                x: inner.x,
                y: card_y,
                width: ICON_W,
                height: CARD_H,
            };
            self.render_browse_card_icon(frame, icon_rect, &hit, bg);

            // Green accent bar on the left, just right of the icon
            if is_selected && card_rect.width > ICON_W {
                let bar = Rect {
                    x: inner.x + ICON_W,
                    y: card_rect.y,
                    width: 1,
                    height: CARD_H,
                };
                let bar_lines: Vec<Line> = (0..CARD_H)
                    .map(|_| {
                        Line::from(Span::styled(
                            "\u{258e}",
                            Style::default().fg(self.theme.green),
                        ))
                    })
                    .collect();
                frame.render_widget(Paragraph::new(bar_lines).style(surface), bar);
            }

            // Text content (right of icon + bar)
            let bar_w: u16 = if is_selected && card_rect.width > ICON_W { 1 } else { 0 };
            let text_x = inner.x + ICON_W + bar_w;
            let text_w = card_rect.width.saturating_sub(ICON_W + bar_w);

            let title_style = if is_selected {
                Style::default()
                    .fg(self.theme.green)
                    .bg(bg)
                    .add_modifier(ratatui::style::Modifier::BOLD)
            } else if is_hovered {
                Style::default().fg(self.theme.green).bg(bg)
            } else {
                self.theme.row()
            };
            let dim_style = if is_selected {
                Style::default().fg(self.theme.green).bg(bg)
            } else {
                Style::default().fg(self.theme.muted).bg(bg)
            };

            // Row 1: title
            frame.render_widget(
                Paragraph::new(Span::styled(
                    truncate(&hit.title, text_w as usize),
                    title_style,
                ))
                .style(surface),
                Rect {
                    x: text_x,
                    y: card_y,
                    width: text_w,
                    height: 1,
                },
            );

            // Row 2: description
            let desc = if hit.description.is_empty() {
                "No description".to_string()
            } else {
                truncate(&hit.description, text_w as usize)
            };
            frame.render_widget(
                Paragraph::new(Span::styled(
                    desc,
                    Style::default().fg(self.theme.comment).bg(bg),
                ))
                .style(surface),
                Rect {
                    x: text_x,
                    y: card_y + 1,
                    width: text_w,
                    height: 1,
                },
            );

            // Row 3: version + downloads + follows
            let version = hit.latest_version.as_deref().unwrap_or("--");
            let mut line3: Vec<Span> = Vec::new();
            line3.push(Span::styled(format!("v{version}"), dim_style));
            line3.push(Span::styled(
                format!("  \u{2913} {}", hit.downloads),
                Style::default().fg(self.theme.green).bg(bg),
            ));
            line3.push(Span::styled(
                format!("  \u{2661} {}", hit.follows),
                dim_style,
            ));
            let categories = if hit.display_categories.is_empty() {
                String::new()
            } else {
                format!(
                    "  {}",
                    hit.display_categories.first().cloned().unwrap_or_default()
                )
            };
            if !categories.is_empty() {
                line3.push(Span::styled(
                    categories,
                    Style::default().fg(self.theme.comment).bg(bg),
                ));
            }
            frame.render_widget(
                Paragraph::new(Line::from(line3)).style(surface),
                Rect {
                    x: text_x,
                    y: card_y + 2,
                    width: text_w,
                    height: 1,
                },
            );

            // Row 4: client/server side
            let side_label = match (hit.client_side.as_str(), hit.server_side.as_str()) {
                ("required", "required") => "Client + Server",
                ("required", _) => "Client",
                (_, "required") => "Server",
                ("optional", "optional") => "Client + Server (optional)",
                ("optional", _) => "Client (optional)",
                (_, "optional") => "Server (optional)",
                _ => "",
            };
            if !side_label.is_empty() {
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        side_label,
                        Style::default().fg(self.theme.comment).bg(bg),
                    ))
                    .style(surface),
                    Rect {
                        x: text_x,
                        y: card_y + 3,
                        width: text_w,
                        height: 1,
                    },
                );
            }

            self.push_hitbox(card_rect, HitAction::BrowseResult(idx));
        }

        if hits.is_empty() {
            let hint = if self.browse.loading {
                "Loading...".to_string()
            } else if self.browse.query.is_empty() {
                "No projects found.".to_string()
            } else {
                format!(
                    "No results for '{}'. Press 's' to change the query.",
                    self.browse.query
                )
            };
            frame.render_widget(
                Paragraph::new(Span::styled(hint, self.theme.card_dim())).style(self.theme.card()),
                inner,
            );
        }
    }

    /// Render a full-height icon on the left side of a browse card.
    /// Uses the same ratatui-image protocol rendering as the detail view
    /// when available, falling back to truecolor half-blocks or a colour swatch.
    fn render_browse_card_icon(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        hit: &SearchHit,
        bg: ratatui::style::Color,
    ) {
        let color = hit
            .color
            .map(|c| ratatui::style::Color::Rgb((c >> 16) as u8, (c >> 8) as u8, c as u8))
            .unwrap_or(self.theme.green_dim);
        let surface = Style::default().bg(bg);

        let Some(url) = hit.icon_url.as_ref() else {
            self.render_card_icon_fallback(frame, area, color);
            return;
        };

        // Try the full-protocol image rendering (kitty/sixel/iterm2/halfblocks).
        if let Some(img) = self.browse_images.get(url) {
            if crate::images::terminal_supports_truecolor() {
                if self.browse_protocols.get(url).is_none() {
                    if let Some(dynamic) = crate::images::to_dynamic_image(img) {
                        let protocol = self.picker.new_resize_protocol(dynamic);
                        self.browse_protocols.insert(url.to_string(), protocol);
                    }
                }
                if let Some(proto) = self.browse_protocols.get_mut(url) {
                    frame.render_stateful_widget(
                        StatefulImage::default().resize(Resize::Fit(None)),
                        area,
                        proto,
                    );
                    return;
                }
            }
            // Half-block fallback when protocol unavailable.
            let icon_lines =
                crate::images::image_lines(img, area.width as u32, area.height as u32, bg);
            for (i, line) in icon_lines.iter().enumerate() {
                let row = Rect { x: area.x, y: area.y + i as u16, width: area.width, height: 1 };
                if row.y >= area.y + area.height { break; }
                frame.render_widget(
                    Paragraph::new(Line::from(line.spans.clone())).style(surface), row,
                );
            }
            for i in icon_lines.len()..area.height as usize {
                let row = Rect { x: area.x, y: area.y + i as u16, width: area.width, height: 1 };
                frame.render_widget(Paragraph::new("").style(surface), row);
            }
            return;
        }

        // Colour swatch when no image loaded yet.
        self.render_card_icon_fallback(frame, area, color);
    }

    fn render_card_icon_fallback(
        &self,
        frame: &mut Frame,
        area: Rect,
        color: ratatui::style::Color,
    ) {
        for row_idx in 0..area.height {
            let row = Rect { x: area.x, y: area.y + row_idx, width: area.width, height: 1 };
            frame.render_widget(
                Paragraph::new(Span::styled(
                    " ".repeat(area.width as usize),
                    Style::default().bg(color),
                )),
                row,
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