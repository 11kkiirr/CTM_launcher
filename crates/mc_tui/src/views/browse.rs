//! Modrinth Browser — a store-like page for mods, modpacks, resource packs and
//! shaders.
//!
//! Behaves like a minimal web browser: tabs for the four content types, a
//! search bar, a paged list of the most popular projects, and a project detail
//! page with a rendered Markdown description plus a preview image (truecolor
//! half-blocks) when the terminal supports it.

use std::collections::{HashMap, HashSet};

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

/// The content category the browser covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BrowseKind {
    Mods,
    ResourcePacks,
    Shaders,
}

impl BrowseKind {
    pub fn label(&self) -> &'static str {
        match self {
            BrowseKind::Mods => crate::i18n::en_static("browse.kind_mods"),
            BrowseKind::ResourcePacks => crate::i18n::en_static("browse.kind_rp"),
            BrowseKind::Shaders => crate::i18n::en_static("browse.kind_shaders"),
        }
    }

    pub fn label_lang(&self, lang: crate::i18n::Lang) -> String {
        let key = match self {
            BrowseKind::Mods => "browse.kind_mods",
            BrowseKind::ResourcePacks => "browse.kind_rp",
            BrowseKind::Shaders => "browse.kind_shaders",
        };
        crate::i18n::tr_string(lang, key)
    }

    /// The Modrinth `project_type` facet value.
    pub fn project_type(&self) -> &'static str {
        match self {
            BrowseKind::Mods => "mod",
            BrowseKind::ResourcePacks => "resourcepack",
            BrowseKind::Shaders => "shader",
        }
    }

    /// Loader names shown in the sidebar.
    pub fn loaders(&self) -> &'static [&'static str] {
        match self {
            BrowseKind::Mods => &["fabric", "forge", "neoforge", "quilt"],
            BrowseKind::ResourcePacks => &[],
            BrowseKind::Shaders => &["iris", "optifine"],
        }
    }

    /// Category facet values shown in the sidebar.
    pub fn categories(&self) -> &'static [&'static str] {
        match self {
            BrowseKind::Mods => &[
                "adventure", "decoration", "magic", "mobs", "optimization",
                "library", "technology", "worldgen", "games", "social",
                "storage", "transport", "utilitarian", "crafting",
            ],
            BrowseKind::ResourcePacks => &[
                "aesthetic", "texture-packs", "sound-packs", "hud", "fonts",
            ],
            BrowseKind::Shaders => &[
                "performance", "realistic", "fantasy", "vanilla", "toon",
            ],
        }
    }
}

/// Which region of the browser owns keyboard input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowseFocus {
    Search,
    Sidebar,
    List,
    Body,
    Versions,
}

/// Items inside the filter sidebar that can be focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterItem {
    Sort,
    Side,
    Compat,
    Loader(usize),
    Category(usize),
}

/// Client / server side filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideFilter {
    All,
    Client,
    Server,
}

impl SideFilter {
    pub fn cycle(self) -> Self {
        match self {
            SideFilter::All => SideFilter::Client,
            SideFilter::Client => SideFilter::Server,
            SideFilter::Server => SideFilter::All,
        }
    }
}

/// Sort order for browse results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Downloads,
    Follows,
    Newest,
    Updated,
    Relevance,
}

impl SortOrder {
    pub fn as_str(self) -> &'static str {
        match self {
            SortOrder::Downloads => "downloads",
            SortOrder::Follows => "follows",
            SortOrder::Newest => "newest",
            SortOrder::Updated => "updated",
            SortOrder::Relevance => "relevance",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SortOrder::Downloads => "popular",
            SortOrder::Follows => "follows",
            SortOrder::Newest => "newest",
            SortOrder::Updated => "updated",
            SortOrder::Relevance => "relevance",
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            SortOrder::Downloads => SortOrder::Follows,
            SortOrder::Follows => SortOrder::Newest,
            SortOrder::Newest => SortOrder::Updated,
            SortOrder::Updated => SortOrder::Relevance,
            SortOrder::Relevance => SortOrder::Downloads,
        }
    }
}

/// Per-kind cached browser state.
#[derive(Debug, Clone)]
pub struct BrowseCache {
    pub query: String,
    pub search_input: String,
    pub results: Vec<SearchHit>,
    pub selected: usize,
    pub offset: u32,
    pub total: u32,
    pub loading: bool,
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
    pub filter_compat: bool,
    pub filter_side: SideFilter,
    pub filter_categories: Vec<String>,
    pub filter_loaders: Vec<String>,
    pub sort: SortOrder,
    pub sidebar_selected: FilterItem,
}

impl Default for BrowseCache {
    fn default() -> Self {
        Self {
            query: String::new(),
            search_input: String::new(),
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
            filter_compat: true,
            filter_side: SideFilter::All,
            filter_categories: Vec::new(),
            filter_loaders: Vec::new(),
            sort: SortOrder::Downloads,
            sidebar_selected: FilterItem::Sort,
        }
    }
}

/// The complete browser state.
#[derive(Debug, Clone)]
pub struct Browse {
    pub kind: BrowseKind,
    /// Per-kind caches so each content type has its own results/scroll/etc.
    pub caches: HashMap<BrowseKind, BrowseCache>,
    // The fields below are shared convenience accessors — kept in sync
    // with the active kind's cache.
    pub query: String,
    pub search_input: String,
    pub results: Vec<SearchHit>,
    pub selected: usize,
    pub offset: u32,
    pub total: u32,
    pub loading: bool,
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
    pub filter_compat: bool,
    pub filter_side: SideFilter,
    pub filter_categories: Vec<String>,
    pub filter_loaders: Vec<String>,
    pub sort: SortOrder,
    pub sidebar_selected: FilterItem,
}

impl Default for Browse {
    fn default() -> Self {
        Self {
            kind: BrowseKind::Mods,
            caches: HashMap::new(),
            query: String::new(),
            search_input: String::new(),
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
            filter_compat: true,
            filter_side: SideFilter::All,
            filter_categories: Vec::new(),
            filter_loaders: Vec::new(),
            sort: SortOrder::Downloads,
            sidebar_selected: FilterItem::Sort,
        }
    }
}

impl Browse {
    /// Save the current active fields into the cache for the current kind.
    pub fn save_cache(&mut self) {
        let cache = BrowseCache {
            query: self.query.clone(),
            search_input: self.search_input.clone(),
            results: self.results.clone(),
            selected: self.selected,
            offset: self.offset,
            total: self.total,
            loading: self.loading,
            detail: self.detail.clone(),
            versions: self.versions.clone(),
            version_selected: self.version_selected,
            focus: self.focus,
            body: self.body.clone(),
            body_for: self.body_for.clone(),
            body_width: self.body_width,
            body_scroll: self.body_scroll,
            body_visible: self.body_visible,
            image_requested: self.image_requested.clone(),
            filter_compat: self.filter_compat,
            filter_side: self.filter_side,
            filter_categories: self.filter_categories.clone(),
            filter_loaders: self.filter_loaders.clone(),
            sort: self.sort,
            sidebar_selected: self.sidebar_selected,
        };
        self.caches.insert(self.kind, cache);
    }

    /// Load cached state for the given kind into the active fields.
    /// If no cache exists, reset to defaults for that kind.
    pub fn load_cache(&mut self, kind: BrowseKind) {
        if let Some(cache) = self.caches.remove(&kind) {
            self.query = cache.query;
            self.search_input = cache.search_input;
            self.results = cache.results;
            self.selected = cache.selected;
            self.offset = cache.offset;
            self.total = cache.total;
            self.loading = cache.loading;
            self.detail = cache.detail;
            self.versions = cache.versions;
            self.version_selected = cache.version_selected;
            self.focus = cache.focus;
            self.body = cache.body;
            self.body_for = cache.body_for;
            self.body_width = cache.body_width;
            self.body_scroll = cache.body_scroll;
            self.body_visible = cache.body_visible;
            self.image_requested = cache.image_requested;
            self.filter_compat = cache.filter_compat;
            self.filter_side = cache.filter_side;
            self.filter_categories = cache.filter_categories;
            self.filter_loaders = cache.filter_loaders;
            self.sort = cache.sort;
            self.sidebar_selected = cache.sidebar_selected;
        } else {
            // No cache — reset to defaults for this kind.
            self.query = String::new();
            self.search_input = String::new();
            self.results = Vec::new();
            self.selected = 0;
            self.offset = 0;
            self.total = 0;
            self.loading = false;
            self.detail = None;
            self.versions = Vec::new();
            self.version_selected = 0;
            self.focus = BrowseFocus::List;
            self.body = Vec::new();
            self.body_for = String::new();
            self.body_width = 0;
            self.body_scroll = 0;
            self.body_visible = 0;
            self.image_requested = HashSet::new();
            self.filter_compat = matches!(kind, BrowseKind::Mods);
            self.filter_side = SideFilter::All;
            self.filter_categories = Vec::new();
            self.filter_loaders = Vec::new();
            self.sort = SortOrder::Downloads;
            self.sidebar_selected = FilterItem::Sort;
        }
        self.kind = kind;
    }
}

impl Browse {
    pub fn in_detail(&self) -> bool {
        self.detail.is_some()
    }
}

impl App {
    /// Whether the selected instance already has this Modrinth project
    /// installed (mods by jar id/slug, RP/shaders by folder/file name).
    pub(crate) fn is_hit_installed(&self, hit: &SearchHit) -> bool {
        self.is_modrinth_installed(&hit.project_id, &hit.slug, &hit.title)
    }

    /// Shared installed check for a Modrinth project (id / slug / title).
    pub(crate) fn is_modrinth_installed(
        &self,
        project_id: &str,
        slug: &str,
        title: &str,
    ) -> bool {
        let slug = slug.to_lowercase();
        let title = title.to_lowercase();
        let project_id = project_id.to_lowercase();
        match self.browse.kind {
            BrowseKind::Mods => self.installed_mods.iter().any(|m| {
                let mid = m.mod_id.to_lowercase();
                if !mid.is_empty() && (mid == slug || mid == project_id) {
                    return true;
                }
                let stem = mc_core::modrinth::logical_name(&m.file_name)
                    .trim_end_matches(".jar")
                    .to_lowercase();
                if stem == slug
                    || stem.starts_with(&format!("{slug}-"))
                    || stem.starts_with(&format!("{slug}."))
                {
                    return true;
                }
                let name = m.mod_name.to_lowercase();
                !name.is_empty() && (name == title || name == slug)
            }),
            BrowseKind::ResourcePacks => self
                .resource_packs
                .iter()
                .any(|p| Self::name_matches_project(p, &slug, &title)),
            BrowseKind::Shaders => self
                .shaders
                .iter()
                .any(|s| Self::name_matches_project(s, &slug, &title)),
        }
    }

    /// Loose match of an installed pack/shader file name against a Modrinth
    /// slug/title (ignores case, separators and `.zip` / `.disabled`).
    fn name_matches_project(name: &str, slug: &str, title: &str) -> bool {
        let norm = |s: &str| -> String {
            s.to_lowercase()
                .trim_end_matches(".disabled")
                .trim_end_matches(".zip")
                .chars()
                .filter(|c| c.is_alphanumeric())
                .collect()
        };
        let n = norm(name);
        let s = norm(slug);
        let t = norm(title);
        !n.is_empty() && ((s.len() > 2 && n.contains(&s)) || (!t.is_empty() && n.contains(&t)))
    }

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
                Constraint::Length(1), // search bar
                Constraint::Length(1), // status
                Constraint::Min(5),   // sidebar + list
            ])
            .split(area);

        // ── Search bar (row 0) ──
        let search_focused = self.browse.focus == BrowseFocus::Search;
        let search_bg = if search_focused {
            self.theme.panel_alt
        } else {
            self.theme.panel
        };
        let search_rect = Rect {
            x: chunks[0].x,
            y: chunks[0].y,
            width: chunks[0].width,
            height: 1,
        };
        // Green accent bar on the left when focused
        if search_focused && search_rect.width > 0 {
            let bar = Rect { x: search_rect.x, y: search_rect.y, width: 1, height: 1 };
            frame.render_widget(
                Paragraph::new(Span::styled(" ", self.theme.accent()))
                    .style(Style::default().bg(search_bg)),
                bar,
            );
        }
        let search_label = self.tr("dialog.filter");
        let query_part = if self.browse.search_input.is_empty() && self.browse.query.is_empty() {
            Span::styled(
                self.tr("browse.search_placeholder"),
                Style::default().fg(self.theme.muted).bg(search_bg),
            )
        } else if search_focused {
            Span::styled(
                format!("{}█", self.browse.search_input),
                Style::default().fg(self.theme.green).bg(search_bg),
            )
        } else if !self.browse.query.is_empty() {
            Span::styled(
                &self.browse.query,
                Style::default().fg(self.theme.fg).bg(search_bg),
            )
        } else {
            Span::styled(
                &self.browse.search_input,
                Style::default().fg(self.theme.fg).bg(search_bg),
            )
        };
        let search_line = Line::from(vec![
            Span::styled(
                search_label,
                Style::default().fg(if search_focused { self.theme.green } else { self.theme.muted }).bg(search_bg),
            ),
            query_part,
        ]);
        let bar_x = if search_focused { search_rect.x + 1 } else { search_rect.x };
        let bar_w = if search_focused { search_rect.width.saturating_sub(1) } else { search_rect.width };
        frame.render_widget(
            Paragraph::new(search_line).style(Style::default().bg(search_bg)),
            Rect { x: bar_x, y: search_rect.y, width: bar_w, height: 1 },
        );
        self.push_hitbox(search_rect, HitAction::BrowseSearchBar);

        // ── Status / filter line (row 1) ──
        let page_size = 30u32;
        let current_page = self.browse.offset / page_size + 1;
        let total_pages = if self.browse.total == 0 {
            1
        } else {
            (self.browse.total + page_size - 1) / page_size
        };
        let status = if self.browse.query.is_empty() {
            format!(
                "{}  ·  page {}/{}  ·  {} results",
                self.browse.sort.label(),
                current_page,
                total_pages,
                self.browse.total
            )
        } else {
            format!(
                "'{}'  ·  page {}/{}  ·  {} results",
                self.browse.query,
                current_page,
                total_pages,
                self.browse.total
            )
        };
        let mut filter_parts: Vec<String> = Vec::new();
        if self.browse.filter_compat {
            if let Some(inst) = self.selected_instance() {
                filter_parts.push(format!(
                    "{} {}",
                    inst.metadata.game_version,
                    inst.metadata.loader
                ));
            }
        }
        match self.browse.filter_side {
            SideFilter::Client => filter_parts.push("client".to_string()),
            SideFilter::Server => filter_parts.push("server".to_string()),
            SideFilter::All => {}
        }
        if !self.browse.filter_categories.is_empty() {
            filter_parts.push(self.browse.filter_categories.join("+"));
        }
        let filter_str = if filter_parts.is_empty() {
            String::new()
        } else {
            format!("  [{}]", filter_parts.join(" · "))
        };
        let status_line = Line::from(vec![
            Span::styled(status, self.theme.card()),
            Span::styled(filter_str, self.theme.accent()),
        ]);
        frame.render_widget(
            Paragraph::new(status_line).style(self.theme.card()),
            chunks[1],
        );

        // Page navigation buttons on the right side of the status line.
        let nav_w = 16u16; // " « prev  next » " width
        if chunks[1].width > nav_w + 2 {
            let nav_x = chunks[1].x + chunks[1].width - nav_w;
            let nav_rect = Rect { x: nav_x, y: chunks[1].y, width: nav_w, height: 1 };
            let has_prev = self.browse.offset > 0;
            let has_next = self.browse.offset + 30 < self.browse.total;
            let prev_style = if has_prev { self.theme.accent() } else { self.theme.dim() };
            let next_style = if has_next { self.theme.accent() } else { self.theme.dim() };
            let nav_line = Line::from(vec![
                Span::styled(self.tr("browse.prev_only"), prev_style),
                Span::styled("│", self.theme.card_dim()),
                Span::styled(self.tr("browse.next_only"), next_style),
            ]);
            frame.render_widget(Paragraph::new(nav_line).style(self.theme.card()), nav_rect);
            // Hit areas for the two halves.
            let mid = nav_x + nav_w / 2;
            let prev_rect = Rect { x: nav_x, y: chunks[1].y, width: nav_w / 2, height: 1 };
            let next_rect = Rect { x: mid, y: chunks[1].y, width: nav_w - nav_w / 2, height: 1 };
            self.push_hitbox(prev_rect, HitAction::BrowsePagePrev);
            self.push_hitbox(next_rect, HitAction::BrowsePageNext);
        }

        // Split the remaining area: sidebar (22 chars) | results
        const SIDEBAR_W: u16 = 22;
        let body_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(SIDEBAR_W),
                Constraint::Min(10),
            ])
            .split(chunks[2]);
        self.render_browse_filter_sidebar(frame, body_chunks[0]);
        self.render_browse_results(frame, body_chunks[1]);
    }

    fn render_browse_filter_sidebar(&mut self, frame: &mut Frame, area: Rect) {
        let inner = crate::views::card(self, frame, area, self.browse.focus == BrowseFocus::Sidebar);
        if inner.height == 0 {
            return;
        }
        let is_focused = self.browse.focus == BrowseFocus::Sidebar;
        let max_w = inner.width as usize;
        let mut y = inner.y;

        // ── General section ──
        let general_header = Rect { x: inner.x, y, width: inner.width, height: 1 };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" \u{258e}", self.theme.accent()),
                Span::styled(self.tr("browse.general"), self.theme.header()),
            ])).style(self.theme.card()),
            general_header,
        );
        y += 1;

        // Sort row
        {
            let style = if is_focused && matches!(self.browse.sidebar_selected, FilterItem::Sort) {
                self.theme.row_selected()
            } else {
                self.theme.card()
            };
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(self.tr("browse.sort"), self.theme.card_dim()),
                    Span::styled(self.browse.sort.label().to_string(), style),
                ]))
                .style(self.theme.card()),
                Rect { x: inner.x, y, width: inner.width, height: 1 },
            );
            self.push_hitbox(Rect { x: inner.x, y, width: inner.width, height: 1 }, HitAction::BrowseFilter(FilterItem::Sort));
            y += 1;
        }

        // Side row
        {
            let side_label = match self.browse.filter_side {
                SideFilter::All => "all",
                SideFilter::Client => "client",
                SideFilter::Server => "server",
            };
            let style = if is_focused && matches!(self.browse.sidebar_selected, FilterItem::Side) {
                self.theme.row_selected()
            } else {
                self.theme.card()
            };
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(self.tr("browse.side"), self.theme.card_dim()),
                    Span::styled(side_label.to_string(), style),
                ]))
                .style(self.theme.card()),
                Rect { x: inner.x, y, width: inner.width, height: 1 },
            );
            self.push_hitbox(Rect { x: inner.x, y, width: inner.width, height: 1 }, HitAction::BrowseFilter(FilterItem::Side));
            y += 1;
        }

        // Compat row
        {
            let compat_text = if self.browse.filter_compat { "on" } else { "off" };
            let style = if is_focused && matches!(self.browse.sidebar_selected, FilterItem::Compat) {
                self.theme.row_selected()
            } else {
                self.theme.card()
            };
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(self.tr("browse.compat"), self.theme.card_dim()),
                    Span::styled(format!("  {compat_text}"), style),
                ]))
                .style(self.theme.card()),
                Rect { x: inner.x, y, width: inner.width, height: 1 },
            );
            self.push_hitbox(Rect { x: inner.x, y, width: inner.width, height: 1 }, HitAction::BrowseFilter(FilterItem::Compat));
            y += 1;
        }

        // ── Loaders section ──
        let loaders = self.browse.kind.loaders();
        if !loaders.is_empty() {
            y += 1;
            let loader_header = Rect { x: inner.x, y, width: inner.width, height: 1 };
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                Span::styled(" \u{258e}", self.theme.accent()),
                Span::styled(self.tr("browse.loaders"), self.theme.header()),
                ])).style(self.theme.card()),
                loader_header,
            );
            y += 1;
            for (i, loader) in loaders.iter().enumerate() {
                let is_active = self.browse.filter_loaders.contains(&loader.to_string());
                let is_selected = matches!(self.browse.sidebar_selected, FilterItem::Loader(idx) if idx == i);
                let marker = if is_active { "x" } else { " " };
                let style = if is_focused && is_selected {
                    self.theme.row_selected()
                } else if is_active {
                    self.theme.accent()
                } else {
                    self.theme.card()
                };
                let label = format!("   [{marker}] {loader}");
                let truncated = truncate(&label, max_w);
                frame.render_widget(
                    Paragraph::new(Span::styled(truncated, style)).style(self.theme.card()),
                    Rect { x: inner.x, y, width: inner.width, height: 1 },
                );
                self.push_hitbox(Rect { x: inner.x, y, width: inner.width, height: 1 }, HitAction::BrowseFilter(FilterItem::Loader(i)));
                y += 1;
            }
        }

        // ── Categories section ──
        let cats = self.browse.kind.categories();
        if !cats.is_empty() {
            y += 1;
            let cat_header = Rect { x: inner.x, y, width: inner.width, height: 1 };
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                Span::styled(" \u{258e}", self.theme.accent()),
                Span::styled(self.tr("browse.categories"), self.theme.header()),
                ])).style(self.theme.card()),
                cat_header,
            );
            y += 1;
            for (i, cat) in cats.iter().enumerate() {
                let is_active = self.browse.filter_categories.contains(&cat.to_string());
                let is_selected = matches!(self.browse.sidebar_selected, FilterItem::Category(idx) if idx == i);
                let marker = if is_active { "x" } else { " " };
                let style = if is_focused && is_selected {
                    self.theme.row_selected()
                } else if is_active {
                    self.theme.accent()
                } else {
                    self.theme.card()
                };
                let label = format!("   [{marker}] {cat}");
                let truncated = truncate(&label, max_w);
                frame.render_widget(
                    Paragraph::new(Span::styled(truncated, style)).style(self.theme.card()),
                    Rect { x: inner.x, y, width: inner.width, height: 1 },
                );
                self.push_hitbox(Rect { x: inner.x, y, width: inner.width, height: 1 }, HitAction::BrowseFilter(FilterItem::Category(i)));
                y += 1;
            }
        }
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
            let install_w: u16 = 12; // width reserved for the Install/Installed label
            let text_x = inner.x + ICON_W + bar_w;
            let text_w = card_rect.width.saturating_sub(ICON_W + bar_w + install_w);

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
                self.tr("empty.no_description").to_string()
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
                ("required", "required") => self.tr("browse.both"),
                ("required", _) => self.tr("browse.client_only"),
                (_, "required") => self.tr("browse.server_only"),
                ("optional", "optional") => self.tr("browse.both_opt"),
                ("optional", _) => self.tr("browse.client_opt"),
                (_, "optional") => self.tr("browse.server_opt"),
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

            // Minimal install affordance on the right of the middle row.
            let installed = self.is_hit_installed(&hit);
            let install_rect = Rect {
                x: inner.x + ICON_W + bar_w + text_w,
                y: card_y + 2,
                width: install_w,
                height: 1,
            };
            let label = if installed {
                self.tr("browse.installed")
            } else {
                self.tr("browse.install")
            };
            let install_hover = self.is_hovered(install_rect);
            let label_style = if installed {
                Style::default().fg(self.theme.muted).bg(bg)
            } else if install_hover {
                Style::default()
                    .fg(self.theme.green_bright)
                    .bg(bg)
                    .add_modifier(ratatui::style::Modifier::BOLD)
            } else {
                Style::default().fg(self.theme.green).bg(bg)
            };
            frame.render_widget(
                Paragraph::new(Span::styled(label, label_style))
                    .style(surface)
                    .alignment(ratatui::layout::Alignment::Right),
                install_rect,
            );
            self.push_hitbox(install_rect, HitAction::BrowseQuickInstall(idx));

            self.push_hitbox(card_rect, HitAction::BrowseResult(idx));
        }

        if hits.is_empty() {
            let hint = if self.browse.loading {
                self.tr("browse.loading").to_string()
            } else if self.browse.query.is_empty() {
                self.tr("empty.no_projects").to_string()
            } else {
                crate::i18n::tr_string(self.lang(), "browse.no_results")
                    .replace("{}", &self.browse.query)
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
            .constraints([Constraint::Length(1), Constraint::Min(5)])
            .split(area);

        self.render_browse_detail_header(frame, chunks[0], &project);

        let left_w = (area.width as usize * 2 / 5).clamp(24, 38) as u16;
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(left_w), Constraint::Length(1), Constraint::Min(1)])
            .split(chunks[1]);
        self.render_browse_detail_left(frame, columns[0]);
        self.render_browse_detail_body(frame, columns[2]);
    }

    fn render_browse_detail_header(&mut self, frame: &mut Frame, area: Rect, project: &Project) {
        let line = Line::from(vec![
            Span::styled(self.tr("browse.back"), self.theme.accent()),
            Span::styled(truncate(&project.title, 28), self.theme.header()),
            Span::styled(format!("  {}  ", project.project_type), self.theme.card_comment()),
            Span::styled(format!("\u{2913} {}", project.downloads), self.theme.accent()),
            Span::styled(format!("  \u{2661} {}", project.followers), self.theme.card_dim()),
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

        // Full-width icon at the top
        let icon_h = (inner.height / 3).clamp(6, 12);
        let icon_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: icon_h,
        };
        self.render_browse_icon(frame, icon_area, &project);

        // Version list below icon
        let remaining_h = inner.height.saturating_sub(icon_h);
        if remaining_h == 0 {
            return;
        }

        // Separate compatible vs all versions
        let instance_gv = self
            .selected_instance()
            .map(|i| i.metadata.game_version.clone());
        let instance_loader = self
            .selected_instance()
            .map(|i| i.metadata.loader.as_str().to_string());

        let mut compatible: Vec<usize> = Vec::new();
        let mut incompatible: Vec<usize> = Vec::new();
        for (idx, ver) in self.browse.versions.iter().enumerate() {
            let gv_match = instance_gv
                .as_ref()
                .map(|gv| ver.game_versions.iter().any(|v| v == gv))
                .unwrap_or(true);
            let ld_match = instance_loader
                .as_ref()
                .map(|ld| ver.loaders.iter().any(|v| v == ld))
                .unwrap_or(true);
            if gv_match && ld_match {
                compatible.push(idx);
            } else {
                incompatible.push(idx);
            }
        }

        // Version header
        let header_y = inner.y + icon_h;
        let header_rect = Rect {
            x: inner.x,
            y: header_y,
            width: inner.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(self.tr("browse.versions"), self.theme.header()),
                Span::styled(
                    crate::i18n::tr_string(self.lang(), "browse.compatible")
                        .replace("{}", &compatible.len().to_string())
                        .replace("{}", &self.browse.versions.len().to_string()),
                    self.theme.card_dim(),
                ),
            ]))
            .style(self.theme.card()),
            header_rect,
        );

        // Version list (compatible first, then incompatible)
        let list_y = header_y + 1;
        let list_h = remaining_h.saturating_sub(1);
        let visible = list_h as usize;

        // Build ordered list: compatible first, then incompatible
        let mut ordered: Vec<usize> = compatible.clone();
        ordered.extend(incompatible.iter());

        let start = self
            .browse
            .version_selected
            .saturating_sub(visible.saturating_sub(1));
        for row in 0..visible {
            let list_idx = start + row;
            let Some(&idx) = ordered.get(list_idx) else {
                break;
            };
            let Some(version) = self.browse.versions.get(idx).cloned() else {
                break;
            };
            let rect = Rect {
                x: inner.x,
                y: list_y + row as u16,
                width: inner.width,
                height: 1,
            };
            let is_compatible = compatible.contains(&idx);
            let style = if idx == self.browse.version_selected {
                self.theme.row_selected()
            } else if self.is_hovered(rect) {
                self.theme.row_hover()
            } else if is_compatible {
                self.theme.row()
            } else {
                self.theme.card_dim()
            };
            let game = version
                .game_versions
                .first()
                .cloned()
                .unwrap_or_default();
            let compat_marker = " ";
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(compat_marker, style),
                    Span::styled(
                        truncate(&version.version_number, 12),
                        style,
                    ),
                    Span::styled(
                        format!("  {game}"),
                        self.theme.card_dim(),
                    ),
                    Span::styled(
                        format!("  {}", version.loaders.join("+")),
                        self.theme.card_comment(),
                    ),
                ]))
                .style(self.theme.card()),
                rect,
            );
            self.push_hitbox(rect, HitAction::BrowseVersion(idx));
        }
        if self.browse.versions.is_empty() {
            frame.render_widget(
                Paragraph::new(Span::styled(self.tr("empty.no_versions"), self.theme.card_dim()))
                    .style(self.theme.card()),
                Rect {
                    x: inner.x,
                    y: list_y,
                    width: inner.width,
                    height: list_h,
                },
            );
        }

        // Install button — shows Installed when the project is already present.
        let installed = self.is_modrinth_installed(&project.id, &project.slug, &project.title);
        let install_y = inner.y + inner.height - 1;
        let install_rect = Rect {
            x: inner.x,
            y: install_y,
            width: inner.width,
            height: 1,
        };
        let install_style = if installed {
            self.theme.dim()
        } else if self.is_hovered(install_rect) {
            self.theme.hover()
        } else {
            self.theme.accent()
        };
        let install_label = if installed {
            "  [ Installed — Install anyway ]"
        } else {
            "  [ Install selected version ]"
        };
        frame.render_widget(
            Paragraph::new(Span::styled(install_label, install_style))
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
                Span::styled(self.tr("browse.description"), self.theme.header()),
                Span::styled(self.tr("browse.scroll_hint"), self.theme.card_dim()),
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
                    self.tr("empty.no_description_long"),
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
        // Tab toggles between sidebar, search, and list focus.
        if key.code == KeyCode::Tab {
            self.browse.focus = match self.browse.focus {
                BrowseFocus::Search => BrowseFocus::Sidebar,
                BrowseFocus::Sidebar => BrowseFocus::List,
                BrowseFocus::List | _ => BrowseFocus::Search,
            };
            return;
        }

        // When search bar is focused, character input goes to the search field.
        if self.browse.focus == BrowseFocus::Search {
            match key.code {
                KeyCode::Esc => {
                    self.browse.focus = BrowseFocus::List;
                }
                KeyCode::Enter => {
                    self.browse.query = self.browse.search_input.trim().to_string();
                    self.browse_close_detail();
                    self.browse_load_first_page();
                    self.browse.focus = BrowseFocus::List;
                }
                KeyCode::Backspace => {
                    self.browse.search_input.pop();
                }
                KeyCode::Char(c) => {
                    self.browse.search_input.push(c);
                }
                _ => {}
            }
            return;
        }

        // When sidebar is focused, j/k navigate filter items, Enter/Space activates.
        if self.browse.focus == BrowseFocus::Sidebar {
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => self.browse_sidebar_move(1),
                KeyCode::Up | KeyCode::Char('k') => self.browse_sidebar_move(-1),
                KeyCode::Enter | KeyCode::Char(' ') => {
                    let item = self.browse.sidebar_selected;
                    self.browse_filter_click(item);
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Down | KeyCode::Char('j') => self.browse_move(1),
            KeyCode::Up | KeyCode::Char('k') => self.browse_move(-1),
            KeyCode::Char('g') => {
                self.browse.selected = 0;
            }
            KeyCode::Char('G') => {
                self.browse.selected = self.browse.results.len().saturating_sub(1);
            }
            KeyCode::Char('n') | KeyCode::Char('N') => self.browse_next_page(),
            KeyCode::Char('p') | KeyCode::Char('P') => self.browse_prev_page(),
            KeyCode::Char('f') => {
                self.browse.filter_compat = !self.browse.filter_compat;
                self.browse_load_first_page();
            }
            KeyCode::Char('c') => {
                self.browse.filter_side = self.browse.filter_side.cycle();
                self.browse_load_first_page();
            }
            KeyCode::Char('o') => {
                self.browse.sort = self.browse.sort.cycle();
                self.browse_load_first_page();
            }
            KeyCode::Char('i') => {
                let idx = self.browse.selected;
                self.browse_quick_install(idx);
            }
            KeyCode::Char('[') => self.browse_prev_page(),
            KeyCode::Char(']') => self.browse_next_page(),
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
        let next = self.browse.selected as i32 + delta;
        // Auto-load next page when scrolling past the end.
        if next >= len as i32 && self.browse.offset + 30 < self.browse.total {
            self.browse_next_page();
            return;
        }
        // Auto-load prev page when scrolling before the start.
        if next < 0 && self.browse.offset > 0 {
            self.browse_prev_page();
            return;
        }
        self.browse.selected = next.clamp(0, len as i32 - 1) as usize;
    }

    /// Navigate through sidebar filter items.
    /// Layout: Sort(0), Side(1), Compat(2), Loader(0..), Category(0..)
    fn browse_sidebar_move(&mut self, delta: i32) {
        use crate::views::browse::FilterItem;
        let n_loaders = self.browse.kind.loaders().len();
        let n_cats = self.browse.kind.categories().len();
        // Fixed items (3) + loaders + categories
        let total = 3 + n_loaders + n_cats;
        if total == 0 {
            return;
        }
        let cur = match self.browse.sidebar_selected {
            FilterItem::Sort => 0usize,
            FilterItem::Side => 1,
            FilterItem::Compat => 2,
            FilterItem::Loader(i) => 3 + i,
            FilterItem::Category(i) => 3 + n_loaders + i,
        };
        let next = (cur as i32 + delta).clamp(0, total as i32 - 1) as usize;
        self.browse.sidebar_selected = match next {
            0 => FilterItem::Sort,
            1 => FilterItem::Side,
            2 => FilterItem::Compat,
            i if i < 3 + n_loaders => FilterItem::Loader(i - 3),
            i => FilterItem::Category(i - 3 - n_loaders),
        };
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

}