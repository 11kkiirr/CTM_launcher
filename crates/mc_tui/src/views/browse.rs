//! Modrinth Browser — a store-like page for mods, modpacks, resource packs and
//! shaders.
//!
//! Behaves like a minimal web browser: tabs for the four content types, a
//! search bar, a paged list of the most popular projects, and a project detail
//! page with a rendered Markdown description plus a preview image (truecolor
//! half-blocks) when the terminal supports it.

use std::collections::{HashMap, HashSet};

use crossterm::event::{KeyCode, KeyEvent};
use mc_core::modrinth::{Member, Project, SearchHit, Version};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;
use ratatui_image::{Resize, StatefulImage};

use crate::app::{App, HitAction};
use crate::md::render_md;
use crate::views::{
    action_cell, begin_action_row, pill_cell, row_widths, truncate,
};

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

/// Tabs of the project detail page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DetailTab {
    #[default]
    Description,
    Gallery,
    Changelog,
    Versions,
}

impl DetailTab {
    pub fn all() -> [DetailTab; 4] {
        [
            DetailTab::Description,
            DetailTab::Gallery,
            DetailTab::Changelog,
            DetailTab::Versions,
        ]
    }

    pub fn label_key(self) -> &'static str {
        match self {
            DetailTab::Description => "browse.tab_description",
            DetailTab::Gallery => "browse.tab_gallery",
            DetailTab::Changelog => "browse.tab_changelog",
            DetailTab::Versions => "browse.versions",
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
    pub members: Vec<Member>,
    pub versions: Vec<Version>,
    pub version_selected: usize,
    pub focus: BrowseFocus,
    pub body: Vec<Line<'static>>,
    pub body_for: String,
    pub body_width: usize,
    pub body_scroll: usize,
    pub body_visible: usize,
    pub image_requested: HashSet<String>,
    pub detail_tab: DetailTab,
    pub gallery_selected: usize,
    pub gallery_scroll: usize,
    pub gallery_cols: usize,
    pub changelog_scroll: usize,
    pub changelog_visible: usize,
    pub link_hover: Option<usize>,
    pub link_marquee: u64,
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
            members: Vec::new(),
            versions: Vec::new(),
            version_selected: 0,
            focus: BrowseFocus::List,
            body: Vec::new(),
            body_for: String::new(),
            body_width: 0,
            body_scroll: 0,
            body_visible: 0,
            image_requested: HashSet::new(),
            detail_tab: DetailTab::Description,
            gallery_selected: 0,
            gallery_scroll: 0,
            gallery_cols: 1,
            changelog_scroll: 0,
            changelog_visible: 0,
            link_hover: None,
            link_marquee: 0,
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
    pub members: Vec<Member>,
    pub versions: Vec<Version>,
    pub version_selected: usize,
    pub focus: BrowseFocus,
    pub body: Vec<Line<'static>>,
    pub body_for: String,
    pub body_width: usize,
    pub body_scroll: usize,
    pub body_visible: usize,
    pub image_requested: HashSet<String>,
    pub detail_tab: DetailTab,
    pub gallery_selected: usize,
    pub gallery_scroll: usize,
    pub gallery_cols: usize,
    pub changelog_scroll: usize,
    pub changelog_visible: usize,
    pub link_hover: Option<usize>,
    pub link_marquee: u64,
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
            members: Vec::new(),
            versions: Vec::new(),
            version_selected: 0,
            focus: BrowseFocus::List,
            body: Vec::new(),
            body_for: String::new(),
            body_width: 0,
            body_scroll: 0,
            body_visible: 0,
            image_requested: HashSet::new(),
            detail_tab: DetailTab::Description,
            gallery_selected: 0,
            gallery_scroll: 0,
            gallery_cols: 1,
            changelog_scroll: 0,
            changelog_visible: 0,
            link_hover: None,
            link_marquee: 0,
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
            members: self.members.clone(),
            versions: self.versions.clone(),
            version_selected: self.version_selected,
            focus: self.focus,
            body: self.body.clone(),
            body_for: self.body_for.clone(),
            body_width: self.body_width,
            body_scroll: self.body_scroll,
            body_visible: self.body_visible,
            image_requested: self.image_requested.clone(),
            detail_tab: self.detail_tab,
            gallery_selected: self.gallery_selected,
            gallery_scroll: self.gallery_scroll,
            gallery_cols: self.gallery_cols,
            changelog_scroll: self.changelog_scroll,
            changelog_visible: self.changelog_visible,
            link_hover: self.link_hover,
            link_marquee: self.link_marquee,
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
            self.members = cache.members;
            self.versions = cache.versions;
            self.version_selected = cache.version_selected;
            self.focus = cache.focus;
            self.body = cache.body;
            self.body_for = cache.body_for;
            self.body_width = cache.body_width;
            self.body_scroll = cache.body_scroll;
            self.body_visible = cache.body_visible;
            self.image_requested = cache.image_requested;
            self.detail_tab = cache.detail_tab;
            self.gallery_selected = cache.gallery_selected;
            self.gallery_scroll = cache.gallery_scroll;
            self.gallery_cols = cache.gallery_cols;
            self.changelog_scroll = cache.changelog_scroll;
            self.changelog_visible = cache.changelog_visible;
            self.link_hover = cache.link_hover;
            self.link_marquee = cache.link_marquee;
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
            self.members = Vec::new();
            self.versions = Vec::new();
            self.version_selected = 0;
            self.focus = BrowseFocus::List;
            self.body = Vec::new();
            self.body_for = String::new();
            self.body_width = 0;
            self.body_scroll = 0;
            self.body_visible = 0;
            self.image_requested = HashSet::new();
            self.detail_tab = DetailTab::Description;
            self.gallery_selected = 0;
            self.gallery_scroll = 0;
            self.gallery_cols = 1;
            self.changelog_scroll = 0;
            self.changelog_visible = 0;
            self.link_hover = None;
            self.link_marquee = 0;
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

fn format_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Strip the URL scheme for compact display (`https://x` → `x`).
fn short_url(url: &str) -> &str {
    url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url)
}

/// Ping-pong marquee offset: pause at both ends, `step` advances the phase.
fn marquee_offset(step: u64, max: usize) -> usize {
    if max == 0 {
        return 0;
    }
    const PAUSE: u64 = 12;
    let max = max as u64;
    let period = 2 * PAUSE + 2 * max;
    match step % period {
        t if t < PAUSE => 0,
        t if t < PAUSE + max => (t - PAUSE) as usize,
        t if t < 2 * PAUSE + max => max as usize,
        t => (2 * PAUSE + 2 * max - t) as usize,
    }
}

/// Wrap `items` into chip rows (` item ` on a raised background).
fn chip_lines(
    items: &[String],
    width: usize,
    bg: Color,
    fg: impl Fn(&str) -> Color,
) -> Vec<Line<'static>> {
    if items.is_empty() || width == 0 {
        return vec![Line::from(Span::styled(
            "—",
            Style::default().fg(fg("")),
        ))];
    }
    let mut rows: Vec<Line> = Vec::new();
    let mut current: Vec<Span> = Vec::new();
    let mut used = 0usize;
    for item in items {
        let chip = format!(" {item} ");
        let w = chip.chars().count() + 1;
        if !current.is_empty() && used + w > width {
            rows.push(Line::from(std::mem::take(&mut current)));
            used = 0;
        }
        current.push(Span::styled(chip, Style::default().fg(fg(item)).bg(bg)));
        current.push(Span::styled(" ", Style::default()));
        used += w;
    }
    if !current.is_empty() {
        rows.push(Line::from(current));
    }
    rows
}

/// Localized environment support value, or `None` when unknown.
fn env_label(raw: &str, lang: crate::i18n::Lang) -> Option<String> {
    let key = match raw {
        "required" => "browse.env_required",
        "optional" => "browse.env_optional",
        "unsupported" => "browse.env_unsupported",
        _ => return None,
    };
    Some(crate::i18n::tr_string(lang, key))
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
        if area.width == 0 || area.height == 0 {
            return;
        }
        let area = Rect {
            x: area.x + 2,
            width: area.width.saturating_sub(2),
            ..area
        };
        if area.width == 0 {
            return;
        }
        if area.height > 0 {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "▀".repeat(area.width as usize),
                    Style::default().fg(self.theme.bg).bg(self.theme.panel),
                ))
                .style(Style::default().bg(self.theme.panel)),
                Rect { height: 1, ..area },
            );
        }
        let area = Rect {
            y: area.y + 1,
            height: area.height.saturating_sub(1),
            ..area
        };
        if area.height == 0 {
            return;
        }
        let text_w = (area.width as usize).saturating_sub(9 + 1);
        let summary_rows = if project.description.trim().is_empty() {
            0
        } else if project.description.chars().count() > text_w {
            2
        } else {
            1
        };
        let header_h = (2 + summary_rows as u16).clamp(3, 4).min(area.height);
        let rest = area.height.saturating_sub(header_h);
        let tabs_h = 3u16.min(rest);
        let header = Rect {
            height: header_h,
            ..area
        };
        let tabs = Rect {
            y: header.y + header_h,
            height: tabs_h,
            ..area
        };
        let body = Rect {
            y: tabs.y + tabs_h,
            height: rest.saturating_sub(tabs_h),
            ..area
        };
        self.render_detail_header(frame, header, &project);
        self.render_detail_tabs(frame, tabs, &project);
        if body.height == 0 || body.width == 0 {
            return;
        }
        const SIDE_W: u16 = 30;
        if body.width >= 64 {
            let side = Rect {
                width: SIDE_W.min(body.width),
                ..body
            };
            let main = Rect {
                x: side.right() + 2,
                width: body.width.saturating_sub(side.width + 2),
                ..body
            };
            self.render_detail_sidebar(frame, side, &project);
            if main.width > 0 {
                self.render_detail_main(frame, main, &project);
            }
        } else {
            self.render_detail_main(frame, body, &project);
        }
    }

    fn render_detail_header(&mut self, frame: &mut Frame, area: Rect, project: &Project) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        frame.render_widget(Block::default().style(self.theme.card()), area);
        // Square icon in pixels so the fitted image has no padding offset:
        // width_cells = height_rows * cell_h / cell_w.
        let (font_w, font_h) = self.picker.font_size();
        let square_w = if font_w == 0 {
            area.height.saturating_mul(2)
        } else {
            ((area.height as u32 * font_h.max(1) as u32).div_ceil(font_w as u32)) as u16
        };
        let icon = Rect {
            width: square_w.clamp(2, area.width),
            height: area.height,
            ..area
        };
        self.render_browse_icon(frame, icon, project);

        let text = Rect {
            x: icon.right() + 1,
            width: area.width.saturating_sub(icon.width + 1),
            ..area
        };
        if text.width == 0 {
            return;
        }
        let mut ty = text.y;
        let bottom = text.y + text.height;
        frame.render_widget(
            Paragraph::new(Span::styled(
                truncate(&project.title, text.width as usize),
                self.theme.header(),
            ))
            .style(self.theme.card()),
            Rect { y: ty, height: 1, ..text },
        );
        ty += 1;
        if !project.description.trim().is_empty() {
            let w = text.width as usize;
            let first: String = project.description.chars().take(w).collect();
            let rest: String = project.description.chars().skip(w).take(w).collect();
            for line in [first, rest] {
                if line.trim().is_empty() || ty >= bottom {
                    continue;
                }
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        truncate(&line, w),
                        self.theme.card_dim(),
                    ))
                    .style(self.theme.card()),
                    Rect { y: ty, height: 1, ..text },
                );
                ty += 1;
            }
        }
        if ty < bottom {
            let stats = Line::from(vec![
                Span::styled(
                    format!("\u{2b73} {}", format_count(project.downloads)),
                    self.theme.accent(),
                ),
                Span::styled(
                    format!("  \u{2661} {}  ", format_count(project.followers)),
                    self.theme.card_dim(),
                ),
                Span::styled(project.project_type.clone(), self.theme.card_comment()),
            ]);
            frame.render_widget(
                Paragraph::new(stats).style(self.theme.card()),
                Rect { y: ty, height: 1, ..text },
            );
        }
    }

    fn render_download_pill(&mut self, frame: &mut Frame, pill: Rect, project: &Project) {
        if pill.width == 0 || pill.height != 3 {
            return;
        }
        let installed = self.is_modrinth_installed(&project.id, &project.slug, &project.title);
        let hovered = self.is_hovered(pill);
        let bg = if installed && !hovered {
            self.theme.panel_alt
        } else if installed {
            self.theme.selection_bg
        } else {
            self.theme.green
        };
        let fg = if installed || hovered {
            self.theme.fg
        } else {
            self.theme.panel
        };
        let label = if installed {
            self.tr("browse.installed").to_string()
        } else {
            self.tr("browse.download").to_string()
        };
        let pad = pill.width.saturating_sub(label.chars().count() as u16) as usize;
        let line = Line::from(vec![
            Span::styled(
                " ".repeat(pad / 2),
                Style::default().fg(fg).bg(bg),
            ),
            Span::styled(label, Style::default().fg(fg).bg(bg)),
            Span::styled(
                " ".repeat(pad - pad / 2),
                Style::default().fg(fg).bg(bg),
            ),
        ]);
        pill_cell(self, frame, pill, 0, line, bg, HitAction::BrowseInstall);
    }

    fn render_detail_tabs(&mut self, frame: &mut Frame, area: Rect, project: &Project) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        const PILL_W: u16 = 14;
        let pill_w = if area.height >= 3 {
            PILL_W.min(area.width)
        } else {
            0
        };
        if pill_w > 0 {
            let pill = Rect {
                x: area.x + area.width - pill_w,
                width: pill_w,
                height: 3,
                ..area
            };
            self.render_download_pill(frame, pill, project);
        }
        let items: Vec<(String, String, usize)> = DetailTab::all()
            .iter()
            .enumerate()
            .map(|(i, t)| {
                (
                    self.tr(t.label_key()).to_string(),
                    (i + 1).to_string(),
                    i,
                )
            })
            .collect();
        let widths = row_widths(items.iter().map(|(l, k, _)| (l.as_str(), k.as_str())));
        let max_x = area.x + area.width.saturating_sub(pill_w + (pill_w > 0) as u16);
        let (scroll, viewport) = begin_action_row(self, area.x, area.y, max_x, &widths);
        self.toolbar_area.height = 3;
        let mut vx = 0u16;
        for (i, (label, key, idx)) in items.iter().enumerate() {
            if let Some((cell, skip)) = action_cell(viewport, vx, widths[i], scroll) {
                let rect = Rect {
                    y: viewport.y,
                    height: 3,
                    ..cell
                };
                let active = *idx == self.browse.detail_tab as usize;
                let hovered = self.is_hovered(rect);
                let bg = if hovered {
                    self.theme.selection_bg
                } else {
                    self.theme.panel_alt
                };
                let label_style = if active {
                    self.theme.accent_bright()
                } else if hovered {
                    self.theme.accent()
                } else {
                    Style::default().fg(self.theme.muted)
                };
                let line = Line::from(vec![
                    Span::styled(format!(" {label}  "), label_style),
                    Span::styled(key.clone(), self.theme.accent()),
                    Span::styled(" ", self.theme.comment_style()),
                ]);
                pill_cell(
                    self,
                    frame,
                    rect,
                    skip,
                    line,
                    bg,
                    HitAction::BrowseDetailTab(*idx),
                );
            }
            vx = vx.saturating_add(widths[i] + 1);
        }
    }

    fn render_detail_sidebar(&mut self, frame: &mut Frame, area: Rect, project: &Project) {
        if area.width < 4 || area.height == 0 {
            return;
        }
        frame.render_widget(Block::default().style(self.theme.card()), area);
        self.browse.link_hover = None;
        let card_bg = Style::default().bg(self.theme.panel_alt);
        let card_x = area.x + 1;
        let card_w = area.width.saturating_sub(2);
        let cx = card_x + 1;
        let cw = card_w.saturating_sub(1);
        let mut y = area.y;
        let bottom = area.y + area.height;
        macro_rules! panel {
            ($h:expr, $body:block) => {{
                let want: u16 = $h;
                if y >= bottom || want == 0 {
                    return;
                }
                let cap_style = Style::default()
                    .fg(self.theme.panel_alt)
                    .bg(self.theme.panel);
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        "▄".repeat(card_w as usize),
                        cap_style,
                    ))
                    .style(self.theme.card()),
                    Rect {
                        x: card_x,
                        y,
                        width: card_w,
                        height: 1,
                    },
                );
                y += 1;
                let h = want.min(bottom.saturating_sub(y));
                if h == 0 {
                    return;
                }
                let rect = Rect {
                    x: card_x,
                    y,
                    width: card_w,
                    height: h,
                };
                frame.render_widget(Block::default().style(card_bg), rect);
                $body
                if y + h < bottom {
                    frame.render_widget(
                        Paragraph::new(Span::styled(
                            "▀".repeat(card_w as usize),
                            cap_style,
                        ))
                        .style(self.theme.card()),
                        Rect {
                            x: card_x,
                            y: y + h,
                            width: card_w,
                            height: 1,
                        },
                    );
                }
                y += h + 1;
            }};
        }

        let chip_bg = self.theme.panel_alt;
        let chip_fg = self.theme.fg;
        let compact = mc_core::modrinth::compact_game_versions(&project.game_versions);
        let gv_lines = chip_lines(&compact, cw as usize, chip_bg, |_| chip_fg);
        let ld_lines = chip_lines(&project.loaders, cw as usize, chip_bg, |l| {
            self.theme.loader_color(l)
        });
        let client = env_label(&project.client_side, self.lang());
        let server = env_label(&project.server_side, self.lang());
        let mut compat: Vec<Line> = vec![
            Line::from(Span::styled(
                self.tr("browse.side_compat"),
                self.theme.header(),
            )),
            Line::from(Span::styled(
                self.tr("browse.mc_java"),
                self.theme.dim(),
            )),
        ];
        compat.extend(gv_lines);
        compat.push(Line::from(Span::styled(
            self.tr("browse.platforms"),
            self.theme.dim(),
        )));
        compat.extend(ld_lines);
        if client.is_some() || server.is_some() {
            compat.push(Line::from(Span::styled(
                self.tr("browse.environments"),
                self.theme.dim(),
            )));
            if let Some(c) = client {
                compat.push(Line::from(vec![
                    Span::styled("Client: ", self.theme.dim()),
                    Span::styled(c, self.theme.accent()),
                ]));
            }
            if let Some(s) = server {
                compat.push(Line::from(vec![
                    Span::styled("Server: ", self.theme.dim()),
                    Span::styled(s, self.theme.accent()),
                ]));
            }
        }
        panel!(compat.len() as u16, {
            for (i, line) in compat.into_iter().enumerate() {
                frame.render_widget(
                    Paragraph::new(line).style(card_bg),
                    Rect {
                        x: cx,
                        y: y + i as u16,
                        width: cw,
                        height: 1,
                    },
                );
            }
        });

        let links = self.project_links(project);
        if !links.is_empty() {
            let h = 1 + links.len() as u16;
            panel!(h, {
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        self.tr("browse.links"),
                        self.theme.header(),
                    ))
                    .style(card_bg),
                    Rect {
                        x: cx,
                        y,
                        width: cw,
                        height: 1,
                    },
                );
                for (idx, (label, url)) in links.iter().enumerate() {
                    let row_y = y + 1 + idx as u16;
                    let chip = format!(" {label} ");
                    let chip_w = (chip.chars().count() as u16).min(cw);
                    let chip_rect = Rect {
                        x: cx,
                        y: row_y,
                        width: chip_w,
                        height: 1,
                    };
                    let chip_hovered = self.is_hovered(chip_rect);
                    let bg = if chip_hovered {
                        self.theme.selection_bg
                    } else {
                        self.theme.panel_alt
                    };
                    let fg = if chip_hovered {
                        self.theme.green_bright
                    } else {
                        self.theme.green
                    };
                    frame.render_widget(
                        Paragraph::new(Span::styled(chip, Style::default().fg(fg).bg(bg)))
                            .style(card_bg),
                        chip_rect,
                    );
                    let url_w = cw.saturating_sub(chip_w + 2);
                    if url_w > 4 {
                        let url_rect = Rect {
                            x: chip_rect.right() + 1,
                            y: row_y,
                            width: url_w,
                            height: 1,
                        };
                        let url_hovered = self.is_hovered(url_rect);
                        if url_hovered {
                            self.browse.link_hover = Some(idx);
                        }
                        let shown = short_url(url);
                        let text = if shown.chars().count() > url_w as usize {
                            if url_hovered {
                                let max =
                                    shown.chars().count().saturating_sub(url_w as usize);
                                let off = marquee_offset(self.tick / 6, max);
                                shown.chars().skip(off).take(url_w as usize).collect()
                            } else {
                                truncate(shown, url_w as usize)
                            }
                        } else {
                            shown.to_string()
                        };
                        let url_style = if url_hovered {
                            Style::default()
                                .fg(self.theme.muted)
                                .bg(Color::Rgb(0x28, 0x28, 0x28))
                        } else {
                            self.theme.dim()
                        };
                        frame.render_widget(
                            Paragraph::new(Span::styled(text, url_style)).style(card_bg),
                            url_rect,
                        );
                        self.push_hitbox(url_rect, HitAction::BrowseLink(idx));
                    }
                    self.push_hitbox(chip_rect, HitAction::BrowseLink(idx));
                }
            });
        }

        let mut tags = project.categories.clone();
        tags.extend(project.additional_categories.clone());
        if !tags.is_empty() {
            let tag_lines = chip_lines(&tags, cw as usize, chip_bg, |_| chip_fg);
            let h = 1 + tag_lines.len() as u16;
            panel!(h, {
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        self.tr("browse.tags"),
                        self.theme.header(),
                    ))
                    .style(card_bg),
                    Rect {
                        x: cx,
                        y,
                        width: cw,
                        height: 1,
                    },
                );
                for (i, line) in tag_lines.into_iter().enumerate() {
                    frame.render_widget(
                        Paragraph::new(line).style(card_bg),
                        Rect {
                            x: cx,
                            y: y + 1 + i as u16,
                            width: cw,
                            height: 1,
                        },
                    );
                }
            });
        }
        if let Some(license) = project.license.as_ref() {
            let name = truncate(&license.name, cw as usize);
            panel!(2, {
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        self.tr("browse.license"),
                        self.theme.header(),
                    ))
                    .style(card_bg),
                    Rect {
                        x: cx,
                        y,
                        width: cw,
                        height: 1,
                    },
                );
                frame.render_widget(
                    Paragraph::new(Span::styled(name, self.theme.dim())).style(card_bg),
                    Rect {
                        x: cx,
                        y: y + 1,
                        width: cw,
                        height: 1,
                    },
                );
            });
        }
        if !self.browse.members.is_empty() {
            let members = self.browse.members.clone();
            let h = 1 + 2 * members.len() as u16;
            panel!(h, {
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        self.tr("browse.creators"),
                        self.theme.header(),
                    ))
                    .style(card_bg),
                    Rect {
                        x: cx,
                        y,
                        width: cw,
                        height: 1,
                    },
                );
                for (i, member) in members.iter().enumerate() {
                    let my = y + 1 + 2 * i as u16;
                    let avatar = Rect {
                        x: cx,
                        y: my,
                        width: 5.min(cw),
                        height: 2,
                    };
                    let name_rect = Rect {
                        x: avatar.right() + 1,
                        y: my,
                        width: cw.saturating_sub(avatar.width + 1),
                        height: 1,
                    };
                    let role_rect = Rect { y: my + 1, ..name_rect };
                    let initial = member
                        .user
                        .username
                        .chars()
                        .next()
                        .map(|c| c.to_string())
                        .unwrap_or_default();
                    match member.user.avatar_url.clone() {
                        Some(url) => self.render_gallery_image(frame, avatar, &url, &initial),
                        None => {
                            frame.render_widget(
                            Paragraph::new(Span::styled(
                                format!("  {initial}  "),
                                Style::default().fg(self.theme.muted).bg(self.theme.panel_alt),
                            ))
                                .style(card_bg),
                                avatar,
                            );
                        }
                    }
                    frame.render_widget(
                        Paragraph::new(Span::styled(
                            truncate(&member.user.username, name_rect.width as usize),
                            Style::default().fg(self.theme.fg),
                        ))
                        .style(card_bg),
                        name_rect,
                    );
                    frame.render_widget(
                        Paragraph::new(Span::styled(
                            truncate(&member.role, role_rect.width as usize),
                            self.theme.dim(),
                        ))
                        .style(card_bg),
                        role_rect,
                    );
                }
            });
        }
        let _ = y;
    }

    pub(crate) fn project_links(&self, project: &Project) -> Vec<(String, String)> {
        [
            ("browse.link_issues", project.issues_url.clone()),
            ("browse.link_source", project.source_url.clone()),
            ("browse.link_wiki", project.wiki_url.clone()),
            ("browse.link_discord", project.discord_url.clone()),
        ]
        .into_iter()
        .filter_map(|(key, url)| url.map(|u| (self.tr(key).to_string(), u)))
        .collect()
    }

    fn render_detail_main(&mut self, frame: &mut Frame, area: Rect, project: &Project) {
        match self.browse.detail_tab {
            DetailTab::Description => self.render_detail_description(frame, area, project),
            DetailTab::Gallery => self.render_detail_gallery(frame, area, project),
            DetailTab::Changelog => self.render_detail_changelog(frame, area),
            DetailTab::Versions => self.render_detail_versions(frame, area, project),
        }
    }

    fn render_detail_description(&mut self, frame: &mut Frame, area: Rect, project: &Project) {
        let inner = crate::views::card(self, frame, area, self.browse.focus == BrowseFocus::Body);
        if inner.height == 0 {
            return;
        }
        let body_w = (inner.width as usize).saturating_sub(2).max(10);
        if self.browse.body_for != project.id || self.browse.body_width != body_w {
            self.browse.body = render_md(&project.body, body_w, &self.theme);
            self.browse.body_for = project.id.clone();
            self.browse.body_width = body_w;
            self.browse.body_scroll = self
                .browse
                .body_scroll
                .min(self.browse.body.len().saturating_sub(1));
        }
        let visible = inner.height as usize;
        self.browse.body_visible = visible;
        self.browse.body_scroll = self
            .browse
            .body_scroll
            .min(self.browse.body.len().saturating_sub(visible).max(0));
        let start = self.browse.body_scroll;
        let end = (start + visible).min(self.browse.body.len().max(1));

        frame.render_widget(Clear, inner);
        if self.browse.body.is_empty() {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    self.tr("empty.no_description_long"),
                    self.theme.card_dim(),
                ))
                .style(self.theme.card()),
                inner,
            );
            return;
        }
        let lines: Vec<Line> = self
            .browse
            .body
            .iter()
            .skip(start)
            .take(end - start)
            .map(Line::clone)
            .collect();
        frame.render_widget(Paragraph::new(lines).style(self.theme.card()), inner);
    }

    fn render_detail_changelog(&mut self, frame: &mut Frame, area: Rect) {
        let inner = crate::views::card(self, frame, area, self.browse.focus == BrowseFocus::Body);
        if inner.height == 0 {
            return;
        }
        let version = self.browse.versions.get(self.browse.version_selected).cloned();
        let w = (inner.width as usize).saturating_sub(2).max(10);
        let head = match &version {
            Some(v) => format!(
                "{}  {}",
                truncate(&v.version_number, 24),
                truncate(&v.date_published, 10)
            ),
            None => self.tr("browse.versions").to_string(),
        };
        let head_rect = Rect {
            height: 1,
            ..inner
        };
        frame.render_widget(
            Paragraph::new(Span::styled(head, self.theme.header())).style(self.theme.card()),
            head_rect,
        );
        let list = Rect {
            y: inner.y + 1,
            height: inner.height.saturating_sub(1),
            ..inner
        };
        if list.height == 0 {
            return;
        }
        let lines = self.browse_changelog_lines(w);
        let visible = list.height as usize;
        self.browse.changelog_visible = visible;
        self.browse.changelog_scroll = self
            .browse
            .changelog_scroll
            .min(lines.len().saturating_sub(visible));
        let start = self.browse.changelog_scroll;
        frame.render_widget(Clear, list);
        if lines.is_empty() {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    self.tr("browse.no_changelog"),
                    self.theme.card_dim(),
                ))
                .style(self.theme.card()),
                list,
            );
            return;
        }
        let shown: Vec<Line> = lines.into_iter().skip(start).take(visible).collect();
        frame.render_widget(Paragraph::new(shown).style(self.theme.card()), list);
    }

    fn render_detail_gallery(&mut self, frame: &mut Frame, area: Rect, project: &Project) {
        let inner = crate::views::card(self, frame, area, false);
        if inner.height == 0 || inner.width == 0 {
            return;
        }
        if project.gallery.is_empty() {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    self.tr("browse.no_gallery"),
                    self.theme.card_dim(),
                ))
                .style(self.theme.card()),
                inner,
            );
            return;
        }
        self.browse.gallery_selected = self
            .browse
            .gallery_selected
            .min(project.gallery.len() - 1);
        let preview_h = (inner.height / 2).clamp(6, 12).min(inner.height);
        let preview = Rect {
            height: preview_h,
            ..inner
        };
        let grid = Rect {
            y: preview.y + preview_h,
            height: inner.height.saturating_sub(preview_h),
            ..inner
        };
        let sel = self.browse.gallery_selected;
        if let Some(image) = project.gallery.get(sel) {
            let caption = image
                .description
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| format!("#{sel}"));
            self.render_gallery_image(frame, preview, &image.url, &caption);
        }
        const THUMB_W: u16 = 18;
        const THUMB_H: u16 = 7;
        let cols = (grid.width / THUMB_W).max(1) as usize;
        self.browse.gallery_cols = cols;
        let rows = (grid.height / THUMB_H) as usize;
        if rows == 0 {
            return;
        }
        let row_count = project.gallery.len().div_ceil(cols);
        self.browse.gallery_scroll = self
            .browse
            .gallery_scroll
            .min(row_count.saturating_sub(rows));
        for row in 0..rows {
            let gi_row = self.browse.gallery_scroll + row;
            for col in 0..cols {
                let gi = gi_row * cols + col;
                let Some(image) = project.gallery.get(gi) else {
                    break;
                };
                let cell = Rect {
                    x: grid.x + col as u16 * THUMB_W,
                    y: grid.y + row as u16 * THUMB_H,
                    width: THUMB_W.min(grid.width.saturating_sub(col as u16 * THUMB_W)),
                    height: THUMB_H,
                };
                if cell.width == 0 {
                    continue;
                }
                let selected = gi == sel;
                if selected {
                    frame.render_widget(
                        Block::default().style(Style::default().bg(self.theme.selection_bg)),
                        cell,
                    );
                }
                let img_area = if selected {
                    Rect {
                        x: cell.x + 1,
                        y: cell.y,
                        width: cell.width.saturating_sub(2),
                        ..cell
                    }
                } else {
                    cell
                };
                if img_area.width > 0 {
                    self.render_gallery_image(frame, img_area, &image.url, &format!("#{gi}"));
                }
                self.push_hitbox(cell, HitAction::BrowseGallery(gi));
            }
        }
    }

    fn render_gallery_image(&mut self, frame: &mut Frame, area: Rect, url: &str, fallback: &str) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        if self.ensure_image_protocol(url) {
            if let Some(proto) = self.browse_protocols.get_mut(url) {
                frame.render_stateful_widget(
                    StatefulImage::default().resize(Resize::Fit(None)),
                    area,
                    proto,
                );
                return;
            }
        }
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!("  {}  ", truncate(fallback, area.width as usize)),
                Style::default().fg(self.theme.muted).bg(self.theme.panel),
            ))
            .style(self.theme.card()),
            area,
        );
    }

    fn ensure_image_protocol(&mut self, url: &str) -> bool {
        if self.browse_protocols.contains_key(url) {
            return true;
        }
        let Some(img) = self.browse_images.get(url) else {
            return false;
        };
        let Some(dynamic) = crate::images::to_dynamic_image(img) else {
            return false;
        };
        let protocol = self.picker.new_resize_protocol(dynamic);
        self.browse_protocols.insert(url.to_string(), protocol);
        true
    }

    fn render_detail_versions(&mut self, frame: &mut Frame, area: Rect, project: &Project) {
        let _ = project;
        let inner = crate::views::card(
            self,
            frame,
            area,
            self.browse.focus == BrowseFocus::Versions,
        );
        if inner.height == 0 {
            return;
        }

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

        let header_rect = Rect {
            x: inner.x,
            y: inner.y,
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

        let list_y = inner.y + 1;
        let list_h = inner.height.saturating_sub(1);
        let visible = list_h as usize;

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
            let mut spans = vec![
                Span::styled(" ", style),
                Span::styled(truncate(&version.version_number, 16), style),
                Span::styled(format!("  {game}  "), self.theme.card_dim()),
            ];
            for (i, loader) in version.loaders.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::styled("+", self.theme.card_dim()));
                }
                spans.push(Span::styled(
                    loader.clone(),
                    Style::default().fg(self.theme.loader_color(loader)),
                ));
            }
            spans.push(Span::styled(
                format!("  {}", truncate(&version.date_published, 10)),
                self.theme.card_dim(),
            ));
            frame.render_widget(
                Paragraph::new(Line::from(spans)).style(self.theme.card()),
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
        if self.ensure_image_protocol(url) {
            if let Some(proto) = self.browse_protocols.get_mut(url) {
                frame.render_stateful_widget(
                    StatefulImage::default().resize(Resize::Fit(None)),
                    area,
                    proto,
                );
                return;
            }
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
        match key.code {
            KeyCode::Char('1') => self.browse_select_tab(0),
            KeyCode::Char('2') => self.browse_select_tab(1),
            KeyCode::Char('3') => self.browse_select_tab(2),
            KeyCode::Char('4') => self.browse_select_tab(3),
            KeyCode::Tab => {
                let next = (self.browse.detail_tab as usize + 1) % DetailTab::all().len();
                self.browse_select_tab(next);
            }
            KeyCode::Char('v') => self.browse_select_tab(3),
            KeyCode::Enter | KeyCode::Char('i') => self.browse_install(),
            KeyCode::Down | KeyCode::Char('j') => self.browse_detail_move(1),
            KeyCode::Up | KeyCode::Char('k') => self.browse_detail_move(-1),
            KeyCode::Left | KeyCode::Char('h') => self.browse_gallery_move(-1),
            KeyCode::Right | KeyCode::Char('l') => self.browse_gallery_move(1),
            KeyCode::PageDown => self.browse_detail_move(10),
            KeyCode::PageUp => self.browse_detail_move(-10),
            _ => {}
        }
    }

    fn browse_detail_move(&mut self, delta: i32) {
        match self.browse.detail_tab {
            DetailTab::Versions => self.browse_version_move(delta),
            DetailTab::Description => self.browse_scroll_body(delta),
            DetailTab::Changelog => self.browse_scroll_changelog(delta),
            DetailTab::Gallery => {
                let cols = self.browse.gallery_cols.max(1) as i32;
                self.browse_gallery_move(delta * cols);
            }
        }
    }

    /// Mouse wheel / page scroll routing.
    pub(crate) fn browse_scroll(&mut self, delta: i32) {
        if self.browse.in_detail() {
            match self.browse.detail_tab {
                DetailTab::Versions => self.browse_version_move(delta * 3),
                DetailTab::Description => self.browse_scroll_body(delta * 3),
                DetailTab::Changelog => self.browse_scroll_changelog(delta * 3),
                DetailTab::Gallery => {
                    let next = (self.browse.gallery_scroll as i32 + delta).max(0) as usize;
                    self.browse.gallery_scroll = next;
                }
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

    pub(crate) fn browse_select_tab(&mut self, idx: usize) {
        let tabs = DetailTab::all();
        let Some(tab) = tabs.get(idx).copied() else {
            return;
        };
        self.browse.detail_tab = tab;
        self.browse.focus = match tab {
            DetailTab::Versions => BrowseFocus::Versions,
            _ => BrowseFocus::Body,
        };
        if tab == DetailTab::Gallery {
            self.browse_fetch_gallery();
        }
    }

    pub(crate) fn browse_select_gallery(&mut self, idx: usize) {
        let Some(project) = self.browse.detail.as_ref() else {
            return;
        };
        if idx < project.gallery.len() {
            self.browse.gallery_selected = idx;
            self.browse_gallery_ensure_visible();
        }
    }

    /// Advance the link marquee while a truncated link is hovered.
    pub(crate) fn browse_link_tick(&mut self) -> bool {
        if self.browse.link_hover.is_none() {
            return false;
        }
        self.browse.link_marquee = self.browse.link_marquee.wrapping_add(1);
        true
    }

    fn browse_gallery_move(&mut self, delta: i32) {
        if self.browse.detail_tab != DetailTab::Gallery {
            return;
        }
        let Some(project) = self.browse.detail.as_ref() else {
            return;
        };
        let len = project.gallery.len();
        if len == 0 {
            return;
        }
        let next = (self.browse.gallery_selected as i32 + delta).clamp(0, len as i32 - 1) as usize;
        self.browse.gallery_selected = next;
        self.browse_gallery_ensure_visible();
    }

    fn browse_gallery_ensure_visible(&mut self) {
        let Some(project) = self.browse.detail.as_ref() else {
            return;
        };
        let len = project.gallery.len();
        if len == 0 {
            return;
        }
        let cols = self.browse.gallery_cols.max(1);
        let sel_row = self.browse.gallery_selected / cols;
        if sel_row < self.browse.gallery_scroll {
            self.browse.gallery_scroll = sel_row;
        }
    }

    fn browse_changelog_lines(&self, width: usize) -> Vec<Line<'static>> {
        let Some(version) = self.browse.versions.get(self.browse.version_selected) else {
            return Vec::new();
        };
        let Some(body) = version
            .changelog
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        else {
            return Vec::new();
        };
        render_md(body, width.max(10), &self.theme)
    }

    fn browse_changelog_len(&self) -> usize {
        self.browse_changelog_lines(80).len()
    }

    fn browse_scroll_changelog(&mut self, delta: i32) {
        let max = self
            .browse_changelog_len()
            .saturating_sub(self.browse.changelog_visible);
        let next = (self.browse.changelog_scroll as i32 + delta).clamp(0, max as i32) as usize;
        self.browse.changelog_scroll = next;
    }

    pub(crate) fn browse_fetch_gallery(&mut self) {
        let Some(project) = self.browse.detail.clone() else {
            return;
        };
        for image in &project.gallery {
            self.browse_fetch_image(image.url.as_str());
        }
    }

}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_url_strips_scheme() {
        assert_eq!(short_url("https://github.com/x"), "github.com/x");
        assert_eq!(short_url("http://a/b"), "a/b");
        assert_eq!(short_url("github.com/x"), "github.com/x");
    }

    #[test]
    fn marquee_pauses_at_both_ends() {
        assert_eq!(marquee_offset(0, 0), 0);
        assert_eq!(marquee_offset(0, 5), 0);
        assert_eq!(marquee_offset(11, 5), 0);
        assert_eq!(marquee_offset(12, 5), 0);
        assert_eq!(marquee_offset(13, 5), 1);
        assert_eq!(marquee_offset(16, 5), 4);
        assert_eq!(marquee_offset(17, 5), 5);
        assert_eq!(marquee_offset(28, 5), 5);
        assert_eq!(marquee_offset(29, 5), 5);
        assert_eq!(marquee_offset(30, 5), 4);
        assert_eq!(marquee_offset(33, 5), 1);
        assert_eq!(marquee_offset(34, 5), 0);
    }
}
