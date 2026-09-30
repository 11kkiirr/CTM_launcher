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
use crate::md::{render_md_full, MdAlign};
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

/// One selectable row in the filter sidebar: label, value, active, selected, hit action.
type FilterRow = (String, String, bool, bool, Option<HitAction>);

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

/// Horizontal gap in cells between images packed in one strip row.
const IMG_COL_GAP: u16 = 2;
/// Blank rows between wrapped rows of an image strip.
const IMG_ROW_GAP: u16 = 1;

/// Position of a loaded image block inside the expanded description body.
#[derive(Debug, Clone)]
pub struct BodyImage {
    /// Index of the first line reserved for the image in `body`. Images that
    /// share a horizontal strip share their starting line.
    pub line: usize,
    /// URL of the image (also the key of the protocol cache).
    pub url: String,
    /// Horizontal offset in cells from the left edge of the body column.
    pub x: u16,
    /// Reserved width in cells.
    pub width: u16,
    /// Reserved height in rows.
    pub height: u16,
    /// Link target when the source wraps the image in a link; makes the
    /// rendered block clickable.
    pub link: Option<String>,
}

/// A clickable inline text link inside the expanded description body.
#[derive(Debug, Clone)]
pub struct BodyLink {
    /// Index of the line in `browse.body`.
    pub line: usize,
    /// First column of the link text.
    pub start: usize,
    /// Column just past the last character of the link text.
    pub end: usize,
    /// Link target URL.
    pub url: String,
}

/// An inline code span inside the expanded description body; clickable to
/// copy its text.
#[derive(Debug, Clone)]
pub struct BodyCode {
    /// Index of the line in `browse.body`.
    pub line: usize,
    /// First column of the span.
    pub start: usize,
    /// Column just past the last character of the span.
    pub end: usize,
    /// Full code text (the copy payload).
    pub text: String,
}

/// A code block panel inside the expanded description body; clickable to
/// copy its text.
#[derive(Debug, Clone)]
pub struct BodyCodeBlock {
    /// Index of the first line of the panel in `browse.body`.
    pub line: usize,
    /// Panel height in rows (including padding rows).
    pub height: usize,
    /// Full block text (the copy payload).
    pub text: String,
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
    pub body_images: Vec<BodyImage>,
    pub body_links: Vec<BodyLink>,
    pub body_codes: Vec<BodyCode>,
    pub body_code_blocks: Vec<BodyCodeBlock>,
    pub body_dirty: bool,
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
    /// First visible result card in the list (view scroll, items).
    pub list_scroll: usize,
    /// How many result cards fit in the list viewport.
    pub list_visible: usize,
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
            body_images: Vec::new(),
            body_links: Vec::new(),
            body_codes: Vec::new(),
            body_code_blocks: Vec::new(),
            body_dirty: false,
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
            list_scroll: 0,
            list_visible: 0,
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
    pub body_images: Vec<BodyImage>,
    pub body_links: Vec<BodyLink>,
    pub body_codes: Vec<BodyCode>,
    pub body_code_blocks: Vec<BodyCodeBlock>,
    pub body_dirty: bool,
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
    /// First visible result card in the list (view scroll, items).
    pub list_scroll: usize,
    /// How many result cards fit in the list viewport.
    pub list_visible: usize,
    /// Footer hint while hovering a link/code element (full display text,
    /// e.g. `↗ url` or `⎘ code`); cleared at the start of every render pass.
    pub hover_hint: Option<String>,
    /// Clickable links of the changelog tab; rebuilt on every render of the
    /// changelog (not cached — the changelog is re-rendered each frame).
    pub changelog_links: Vec<BodyLink>,
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
            body_images: Vec::new(),
            body_links: Vec::new(),
            body_codes: Vec::new(),
            body_code_blocks: Vec::new(),
            body_dirty: false,
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
            list_scroll: 0,
            list_visible: 0,
            hover_hint: None,
            changelog_links: Vec::new(),
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
            body_images: self.body_images.clone(),
            body_links: self.body_links.clone(),
            body_codes: self.body_codes.clone(),
            body_code_blocks: self.body_code_blocks.clone(),
            body_dirty: self.body_dirty,
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
            list_scroll: self.list_scroll,
            list_visible: self.list_visible,
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
            self.body_images = cache.body_images;
            self.body_links = cache.body_links;
            self.body_codes = cache.body_codes;
            self.body_code_blocks = cache.body_code_blocks;
            self.body_dirty = cache.body_dirty;
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
            self.list_scroll = cache.list_scroll;
            self.list_visible = cache.list_visible;
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
            self.body_images = Vec::new();
            self.body_links = Vec::new();
            self.body_codes = Vec::new();
            self.body_code_blocks = Vec::new();
            self.body_dirty = false;
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
            self.list_scroll = 0;
            self.list_visible = 0;
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

/// Re-colour the `[start, end)` column range of a body line (used to
/// highlight the link under the mouse pointer).
fn recolor_link_line(
    line: &Line<'static>,
    start: usize,
    end: usize,
    color: Color,
) -> Line<'static> {
    use unicode_width::UnicodeWidthChar;
    let mut spans: Vec<Span> = Vec::new();
    let mut col = 0usize;
    for span in &line.spans {
        let text: String = span.content.to_string();
        let span_start = col;
        let mut width = 0usize;
        for ch in text.chars() {
            width += ch.width().unwrap_or(1);
        }
        let span_end = col + width;
        if span_end <= start || span_start >= end {
            spans.push(span.clone());
        } else {
            let mut before = String::new();
            let mut mid = String::new();
            let mut after = String::new();
            let mut c = col;
            for ch in text.chars() {
                let cw = ch.width().unwrap_or(1);
                if c < start {
                    before.push(ch);
                } else if c < end {
                    mid.push(ch);
                } else {
                    after.push(ch);
                }
                c += cw;
            }
            if !before.is_empty() {
                spans.push(Span::styled(before, span.style));
            }
            if !mid.is_empty() {
                spans.push(Span::styled(mid, span.style.fg(color)));
            }
            if !after.is_empty() {
                spans.push(Span::styled(after, span.style));
            }
        }
        col = span_end;
    }
    Line::from(spans)
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
        let area = Rect {
            x: area.x + 2,
            width: area.width.saturating_sub(2),
            ..area
        };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // search pill: cap / bar / toe
                Constraint::Min(5),   // sidebar + list
            ])
            .split(area);

        // ── Search bar (3-row pill) ──
        let search_focused = self.browse.focus == BrowseFocus::Search;
        let search_bg = if search_focused {
            self.theme.panel_alt
        } else {
            self.theme.panel
        };
        let search_w = chunks[0].width;
        let bar_row = chunks[0].y + 1;
        if search_w > 0 {
            let edge = Style::default().fg(search_bg).bg(self.theme.bg);
            frame.render_widget(
                Paragraph::new(Span::styled("▄".repeat(search_w as usize), edge))
                    .style(Style::default().bg(self.theme.bg)),
                Rect {
                    x: chunks[0].x,
                    y: chunks[0].y,
                    width: search_w,
                    height: 1,
                },
            );
            frame.render_widget(
                Paragraph::new(Span::styled("▀".repeat(search_w as usize), edge))
                    .style(Style::default().bg(self.theme.bg)),
                Rect {
                    x: chunks[0].x,
                    y: chunks[0].y + 2,
                    width: search_w,
                    height: 1,
                },
            );
        }
        let search_rect = Rect {
            x: chunks[0].x,
            y: chunks[0].y,
            width: search_w,
            height: 3,
        };
        // Green accent bar on the left when focused
        if search_focused && search_rect.width > 0 {
            let bar = Rect {
                x: search_rect.x,
                y: bar_row,
                width: 1,
                height: 1,
            };
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
            Rect {
                x: bar_x,
                y: bar_row,
                width: bar_w,
                height: 1,
            },
        );
        self.push_hitbox(search_rect, HitAction::BrowseSearchBar);

        // Split the remaining area: sidebar (22) | 2-char dark gap | results
        const SIDEBAR_W: u16 = 22;
        const GAP_W: u16 = 2;
        let side_w = SIDEBAR_W.min(chunks[1].width);
        let gap_w = (chunks[1].width.saturating_sub(side_w)).min(GAP_W);
        let side = Rect {
            x: chunks[1].x,
            y: chunks[1].y,
            width: side_w,
            height: chunks[1].height,
        };
        let gap = Rect {
            x: side.right(),
            y: chunks[1].y,
            width: gap_w,
            height: chunks[1].height,
        };
        let list = Rect {
            x: gap.right(),
            y: chunks[1].y,
            width: chunks[1].width.saturating_sub(side_w + gap_w),
            height: chunks[1].height,
        };
        if gap.width > 0 {
            frame.render_widget(
                Block::default().style(Style::default().bg(self.theme.bg)),
                gap,
            );
        }
        self.render_browse_filter_sidebar(frame, side);
        self.render_browse_results(frame, list);
    }

    fn render_browse_filter_sidebar(&mut self, frame: &mut Frame, area: Rect) {
        if area.width < 4 || area.height == 0 {
            return;
        }
        frame.render_widget(Block::default().style(self.theme.card()), area);
        let is_focused = self.browse.focus == BrowseFocus::Sidebar;
        let card_bg = Style::default().bg(self.theme.panel_alt);
        let card_x = area.x + 1;
        let card_w = area.width.saturating_sub(2);
        if card_w < 4 {
            return;
        }
        let cx = card_x + 1;
        let cw = card_w.saturating_sub(2);
        let side_name = match self.browse.filter_side {
            SideFilter::All => "all",
            SideFilter::Client => "client",
            SideFilter::Server => "server",
        };
        let sort_name = self.browse.sort.label();
        let compat_name = if self.browse.filter_compat {
            "on"
        } else {
            "off"
        };
        let mut sections: Vec<(String, Vec<FilterRow>)> = Vec::new();
        let page_size = 30u32;
        sections.push((
            self.trs("browse.info"),
            vec![
                (
                    format!("{} ", self.tr("browse.results")),
                    self.browse.total.to_string(),
                    false,
                    false,
                    None,
                ),
                (
                    format!("{} ", self.tr("browse.page")),
                    format!(
                        "{}/{}",
                        self.browse.offset / page_size + 1,
                        self.browse.total.div_ceil(page_size).max(1)
                    ),
                    false,
                    false,
                    None,
                ),
            ],
        ));
        sections.push((
            self.trs("browse.general"),
            vec![
                (
                    format!("{} ", self.tr("browse.sort")),
                    sort_name.to_string(),
                    false,
                    is_focused && matches!(self.browse.sidebar_selected, FilterItem::Sort),
                    Some(HitAction::BrowseFilter(FilterItem::Sort)),
                ),
                (
                    format!("{} ", self.tr("browse.side")),
                    side_name.to_string(),
                    !matches!(self.browse.filter_side, SideFilter::All),
                    is_focused && matches!(self.browse.sidebar_selected, FilterItem::Side),
                    Some(HitAction::BrowseFilter(FilterItem::Side)),
                ),
                (
                    format!("{} ", self.tr("browse.compat")),
                    compat_name.to_string(),
                    self.browse.filter_compat,
                    is_focused && matches!(self.browse.sidebar_selected, FilterItem::Compat),
                    Some(HitAction::BrowseFilter(FilterItem::Compat)),
                ),
            ],
        ));
        let loaders = self.browse.kind.loaders();
        if !loaders.is_empty() {
            let mut rows = Vec::with_capacity(loaders.len());
            for (i, loader) in loaders.iter().enumerate() {
                let active = self.browse.filter_loaders.contains(&loader.to_string());
                rows.push((
                    format!("[{}] ", if active { "x" } else { " " }),
                    (*loader).to_string(),
                    active,
                    is_focused
                        && matches!(self.browse.sidebar_selected, FilterItem::Loader(idx) if idx == i),
                    Some(HitAction::BrowseFilter(FilterItem::Loader(i))),
                ));
            }
            sections.push((self.trs("browse.loaders"), rows));
        }
        let cats = self.browse.kind.categories();
        if !cats.is_empty() {
            let mut rows = Vec::with_capacity(cats.len());
            for (i, cat) in cats.iter().enumerate() {
                let active = self.browse.filter_categories.contains(&cat.to_string());
                rows.push((
                    format!("[{}] ", if active { "x" } else { " " }),
                    (*cat).to_string(),
                    active,
                    is_focused
                        && matches!(self.browse.sidebar_selected, FilterItem::Category(idx) if idx == i),
                    Some(HitAction::BrowseFilter(FilterItem::Category(i))),
                ));
            }
            sections.push((self.trs("browse.categories"), rows));
        }

        let cap_style = Style::default()
            .fg(self.theme.panel_alt)
            .bg(self.theme.panel);
        let mut y = area.y;
        let bottom = area.y + area.height;
        // ── Prev / Next side by side on one row ──
        let has_prev = self.browse.offset > 0;
        let has_next = self.browse.offset + page_size < self.browse.total;
        if card_w >= 12 && y + 2 < bottom {
            let gap_n = 2u16;
            let prev_w = (card_w - gap_n) / 2;
            let next_w = card_w - prev_w - gap_n;
            let nav_buttons = [
                (
                    Rect { x: card_x, y, width: prev_w, height: 3 },
                    self.tr("browse.prev_only").to_string(),
                    has_prev,
                    HitAction::BrowsePagePrev,
                ),
                (
                    Rect {
                        x: card_x + prev_w + gap_n,
                        y,
                        width: next_w,
                        height: 3,
                    },
                    self.tr("browse.next_only").to_string(),
                    has_next,
                    HitAction::BrowsePageNext,
                ),
            ];
            for (rect, text, active, hit) in nav_buttons {
                if rect.width < 4 {
                    continue;
                }
                let hovered = self.is_hovered(rect);
                let bg = if hovered {
                    self.theme.selection_bg
                } else {
                    self.theme.panel_alt
                };
                let edge = Style::default().fg(bg).bg(self.theme.panel);
                frame.render_widget(
                    Paragraph::new(Span::styled("▄".repeat(rect.width as usize), edge))
                        .style(self.theme.card()),
                    Rect { height: 1, ..rect },
                );
                frame.render_widget(
                    Block::default().style(Style::default().bg(bg)),
                    Rect {
                        y: rect.y + 1,
                        height: 1,
                        ..rect
                    },
                );
                let cw_n = rect.width.saturating_sub(2);
                let label = truncate(text.trim(), cw_n as usize);
                let text_fg = if active {
                    self.theme.green
                } else {
                    self.theme.muted
                };
                let pad = (cw_n as usize).saturating_sub(label.chars().count()) / 2;
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(" ".repeat(pad), Style::default().bg(bg)),
                        Span::styled(label, Style::default().fg(text_fg).bg(bg)),
                    ]))
                    .style(Style::default().bg(bg)),
                    Rect {
                        x: rect.x + 1,
                        y: rect.y + 1,
                        width: cw_n,
                        height: 1,
                    },
                );
                frame.render_widget(
                    Paragraph::new(Span::styled("▀".repeat(rect.width as usize), edge))
                        .style(self.theme.card()),
                    Rect {
                        y: rect.y + 2,
                        height: 1,
                        ..rect
                    },
                );
                self.push_hitbox(rect, hit);
            }
            y += 3;
        }
        for (title, rows) in sections {
            let want = 1 + rows.len() as u16;
            if y >= bottom {
                return;
            }
            frame.render_widget(
                Paragraph::new(Span::styled("▄".repeat(card_w as usize), cap_style))
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
            frame.render_widget(
                Block::default().style(card_bg),
                Rect {
                    x: card_x,
                    y,
                    width: card_w,
                    height: h,
                },
            );
            frame.render_widget(
                Paragraph::new(Span::styled(
                    truncate(&title, cw as usize),
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
            for (i, row) in rows.into_iter().enumerate().take(h as usize - 1) {
                self.browse_filter_row(frame, cx, cw, y + 1 + i as u16, &row);
            }
            if y + h < bottom {
                frame.render_widget(
                    Paragraph::new(Span::styled("▀".repeat(card_w as usize), cap_style))
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
        }
    }

    fn browse_filter_row(
        &mut self,
        frame: &mut Frame,
        x: u16,
        w: u16,
        row_y: u16,
        row: &FilterRow,
    ) {
        let (label, value, active, sel, hit) = row;
        let bg = if *sel {
            self.theme.selection_bg
        } else {
            self.theme.panel_alt
        };
        let rect = Rect {
            x,
            y: row_y,
            width: w,
            height: 1,
        };
        frame.render_widget(Block::default().style(Style::default().bg(bg)), rect);
        let label_w = label.chars().count().min(w as usize);
        let val_w = (w as usize).saturating_sub(label_w);
        let val_fg = if *sel {
            self.theme.green_bright
        } else if *active {
            self.theme.green
        } else {
            self.theme.fg
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    truncate(label, label_w),
                    Style::default().fg(self.theme.muted).bg(bg),
                ),
                Span::styled(
                    truncate(value, val_w),
                    Style::default().fg(val_fg).bg(bg),
                ),
            ]))
            .style(Style::default().bg(bg)),
            rect,
        );
        if let Some(hit) = hit {
            self.push_hitbox(rect, *hit);
        }
    }

    fn render_browse_results(&mut self, frame: &mut Frame, area: Rect) {
        const CARD_H: u16 = 5;
        const ICON_W: u16 = 8;
        const PILL_W: u16 = 14;

        // Darkest background anywhere there is no content.
        let page = Style::default().bg(self.theme.bg);
        frame.render_widget(Block::default().style(page), area);
        if area.width == 0 || area.height == 0 {
            return;
        }
        let hits = self.browse.results.clone();
        let visible_items = (area.height / CARD_H) as usize;
        self.browse.list_visible = visible_items;
        let max_first = hits.len().saturating_sub(visible_items);
        let first_visible_item = self.browse.list_scroll.min(max_first);
        self.browse.list_scroll = first_visible_item;

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
            let card_y = area.y + (item as u16) * CARD_H;
            let card_rect = Rect {
                x: area.x,
                y: card_y,
                width: area.width,
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
                self.theme.panel
            };
            let surface = Style::default().bg(bg);
            let edge = Style::default().fg(bg).bg(self.theme.bg);
            frame.render_widget(
                Paragraph::new(Span::styled("▄".repeat(area.width as usize), edge))
                    .style(page),
                Rect {
                    x: area.x,
                    y: card_y,
                    width: area.width,
                    height: 1,
                },
            );
            frame.render_widget(
                Paragraph::new(Span::styled("▀".repeat(area.width as usize), edge))
                    .style(page),
                Rect {
                    x: area.x,
                    y: card_y + CARD_H - 1,
                    width: area.width,
                    height: 1,
                },
            );
            frame.render_widget(
                Block::default().style(surface),
                Rect {
                    x: area.x,
                    y: card_y + 1,
                    width: area.width,
                    height: CARD_H - 2,
                },
            );

            // Icon column confined to the content rows so it never
            // overlaps the card caps, with a 1 cell left margin.
            // Drawn after the caps.
            let left_pad: u16 = if area.width > 1 { 1 } else { 0 };
            let icon_x = area.x + left_pad;
            let icon_w = ICON_W.min(area.width.saturating_sub(left_pad));
            let icon_rect = Rect {
                x: icon_x,
                y: card_y + 1,
                width: icon_w,
                height: CARD_H - 2,
            };
            self.render_browse_card_icon(frame, icon_rect, &hit, bg);

            // Text content (right of icon, pill zone reserved
            // with a 2 cell margin from the card edge)
            let text_x = icon_x + icon_w;
            let pill_x = area.x + area.width.saturating_sub(PILL_W + 2);
            let text_w = pill_x.saturating_sub(text_x + 1);

            let title_style = if is_selected {
                Style::default()
                    .fg(self.theme.green)
                    .bg(bg)
                    .add_modifier(ratatui::style::Modifier::BOLD)
            } else if is_hovered {
                Style::default().fg(self.theme.green).bg(bg)
            } else {
                Style::default().fg(self.theme.fg).bg(bg)
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
                    y: card_y + 1,
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
                    y: card_y + 2,
                    width: text_w,
                    height: 1,
                },
            );

            // Row 3: version + stats + side
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
            if let Some(cat) = hit.display_categories.first() {
                line3.push(Span::styled(
                    format!("  {cat}"),
                    Style::default().fg(self.theme.comment).bg(bg),
                ));
            }
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
                line3.push(Span::styled(
                    format!("  \u{00B7} {side_label}"),
                    Style::default().fg(self.theme.comment).bg(bg),
                ));
            }
            frame.render_widget(
                Paragraph::new(Line::from(line3)).style(surface),
                Rect {
                    x: text_x,
                    y: card_y + 3,
                    width: text_w,
                    height: 1,
                },
            );

            // Install pill on the right, spanning all 3 content rows.
            let installed = self.is_hit_installed(&hit);
            let install_label = truncate(
                &if installed {
                    self.tr("browse.installed").to_string()
                } else {
                    self.tr("browse.install").to_string()
                },
                PILL_W as usize,
            );
            let pill_rect = Rect {
                x: pill_x,
                y: card_y + 1,
                width: PILL_W.min(area.width.saturating_sub(pill_x - area.x)),
                height: CARD_H - 2,
            };
            let pill_hover = self.is_hovered(pill_rect);
            let pill_bg = if !installed && pill_hover {
                self.theme.selection_bg
            } else {
                self.theme.panel_alt
            };
            let pill_fg = if installed {
                self.theme.muted
            } else if pill_hover {
                self.theme.green_bright
            } else {
                self.theme.green
            };
            let pill_edge = Style::default().fg(pill_bg).bg(bg);
            let label_w = install_label.chars().count();
            let pad_left = (PILL_W as usize).saturating_sub(label_w) / 2;
            let pad_right = (PILL_W as usize).saturating_sub(label_w + pad_left);
            let pill_style = Style::default().fg(pill_fg).bg(pill_bg);
            frame.render_widget(
                Paragraph::new(Span::styled("▄".repeat(pill_rect.width as usize), pill_edge))
                    .style(Style::default().bg(bg)),
                Rect { height: 1, ..pill_rect },
            );
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(" ".repeat(pad_left), pill_style),
                    Span::styled(install_label, pill_style),
                    Span::styled(" ".repeat(pad_right), pill_style),
                ]))
                .style(pill_style),
                Rect {
                    y: pill_rect.y + 1,
                    height: 1,
                    ..pill_rect
                },
            );
            frame.render_widget(
                Paragraph::new(Span::styled("▀".repeat(pill_rect.width as usize), pill_edge))
                    .style(Style::default().bg(bg)),
                Rect {
                    y: pill_rect.y + 2,
                    height: 1,
                    ..pill_rect
                },
            );
            self.push_hitbox(pill_rect, HitAction::BrowseQuickInstall(idx));

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
                Paragraph::new(Span::styled(
                    hint,
                    Style::default().fg(self.theme.muted).bg(self.theme.bg),
                ))
                .style(page),
                area,
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
            self.browse.body_for = project.id.clone();
            self.browse.body_width = body_w;
            self.browse.body_dirty = true;
        }
        if self.browse.body_dirty {
            self.expand_body(&project.body, body_w);
            self.browse.body_dirty = false;
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
            .min(self.browse.body.len().saturating_sub(visible));
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
        let codes = self.browse.body_codes.clone();
        let mut code_hint: Option<String> = None;
        for (idx, code) in codes.iter().enumerate() {
            if code.line < start || code.line >= end || code.end <= code.start {
                continue;
            }
            let rect = Rect {
                x: inner.x + code.start as u16,
                y: inner.y + (code.line - start) as u16,
                width: (code.end - code.start) as u16,
                height: 1,
            };
            self.push_hitbox(rect, HitAction::CopyBodyCode(idx));
            if code_hint.is_none() && self.is_hovered(rect) {
                code_hint = Some(format!("⎘ {}", code.text));
            }
        }
        let blocks = self.browse.body_code_blocks.clone();
        let mut block_hint: Option<String> = None;
        for (idx, block) in blocks.iter().enumerate() {
            let y0 = block.line.max(start);
            let y1 = (block.line + block.height).min(end);
            if y0 >= y1 {
                continue;
            }
            let rect = Rect {
                x: inner.x,
                y: inner.y + (y0 - start) as u16,
                width: body_w as u16,
                height: (y1 - y0) as u16,
            };
            self.push_hitbox(rect, HitAction::CopyBodyCodeBlock(idx));
            if block_hint.is_none() && self.is_hovered(rect) {
                block_hint = Some(format!("⎘ {}", block.text.replace('\n', " · ")));
            }
        }
        let links = self.browse.body_links.clone();
        let mut hot: Option<BodyLink> = None;
        if code_hint.is_none() && block_hint.is_none() {
            for link in &links {
                if link.line < start || link.line >= end || link.end <= link.start {
                    continue;
                }
                let rect = Rect {
                    x: inner.x + link.start as u16,
                    y: inner.y + (link.line - start) as u16,
                    width: (link.end - link.start) as u16,
                    height: 1,
                };
                if self.is_hovered(rect) {
                    hot = Some(link.clone());
                    break;
                }
            }
        }
        if let Some(hint) = code_hint.or(block_hint) {
            self.browse.hover_hint = Some(hint);
        } else if let Some(link) = &hot {
            self.browse.hover_hint = Some(format!("↗ {}", link.url));
        }
        let hot_pos = hot.map(|l| (l.line, l.start, l.end));
        let accent = self.theme.green_bright;
        let lines: Vec<Line> = self
            .browse
            .body
            .iter()
            .skip(start)
            .take(end - start)
            .enumerate()
            .map(|(i, line)| match hot_pos {
                Some((hl, hs, he)) if hl == start + i => recolor_link_line(line, hs, he, accent),
                _ => line.clone(),
            })
            .collect();
        frame.render_widget(Paragraph::new(lines).style(self.theme.card()), inner);
        for (idx, link) in links.iter().enumerate() {
            if link.line < start || link.line >= end || link.end <= link.start {
                continue;
            }
            let rect = Rect {
                x: inner.x + link.start as u16,
                y: inner.y + (link.line - start) as u16,
                width: (link.end - link.start) as u16,
                height: 1,
            };
            self.push_hitbox(rect, HitAction::BrowseTextLink(idx));
        }
        self.render_body_images(frame, inner, start, end, body_w as u16);
    }

    /// Overlay loaded description images onto their reserved rows.
    fn render_body_images(
        &mut self,
        frame: &mut Frame,
        inner: Rect,
        start: usize,
        end: usize,
        body_w: u16,
    ) {
        let images = self.browse.body_images.clone();
        for (idx, img) in images.iter().enumerate() {
            let bottom = img.line + img.height as usize;
            if img.line < start || bottom > end {
                continue;
            }
            let y = inner.y + (img.line - start) as u16;
            let rect = Rect {
                x: inner.x + img.x,
                y,
                width: img.width.min(body_w),
                height: img.height,
            };
            if rect.x + rect.width > inner.x + inner.width {
                continue;
            }
            if self.ensure_image_protocol(&img.url) {
                if let Some(proto) = self.browse_protocols.get_mut(&img.url) {
                    frame.render_stateful_widget(
                        StatefulImage::default().resize(Resize::Fit(None)),
                        rect,
                        proto,
                    );
                }
            }
            if let Some(link) = &img.link {
                if self.is_hovered(rect) {
                    self.browse.hover_hint = Some(format!("↗ {link}"));
                    let marker = Rect {
                        x: rect.x + rect.width.saturating_sub(1),
                        y: rect.y + rect.height.saturating_sub(1),
                        width: 1,
                        height: 1,
                    };
                    frame.render_widget(
                        Paragraph::new(Span::styled(
                            "↗",
                            Style::default().fg(self.theme.green_bright),
                        )),
                        marker,
                    );
                }
                self.push_hitbox(rect, HitAction::BrowseBodyLink(idx));
            }
        }
    }

    /// Rebuild `body` from markdown, reserving space for loaded images and
    /// kicking off fetches for the ones not seen yet.
    ///
    /// Images that are adjacent in the source (separated only by blank lines)
    /// and share one alignment are packed into horizontal strips like Modrinth
    /// does: greedy width-based rows, positioned left/centred/right per the
    /// surrounding HTML wrapper, with one blank row between wrapped rows. The
    /// markdown placeholder/blank lines a strip covers are consumed by the
    /// reservation.
    fn expand_body(&mut self, project_body: &str, width: usize) {
        let (md, images, md_links, md_codes, md_blocks) =
            render_md_full(project_body, width, &self.theme);
        let mut body: Vec<Line> = Vec::with_capacity(md.len());
        let mut body_images: Vec<BodyImage> = Vec::new();
        let mut body_links: Vec<BodyLink> = Vec::new();
        let mut body_codes: Vec<BodyCode> = Vec::new();
        let mut body_code_blocks: Vec<BodyCodeBlock> = Vec::new();
        let mut link_i = 0usize;
        let mut code_i = 0usize;
        let mut block_i = 0usize;
        let mut next = 0usize;
        let mut idx = 0usize;
        while idx < md.len() {
            if next >= images.len() || images[next].line != idx {
                while link_i < md_links.len() && md_links[link_i].line <= idx {
                    if md_links[link_i].line == idx {
                        body_links.push(BodyLink {
                            line: body.len(),
                            start: md_links[link_i].start,
                            end: md_links[link_i].end,
                            url: md_links[link_i].url.clone(),
                        });
                    }
                    link_i += 1;
                }
                while code_i < md_codes.len() && md_codes[code_i].line <= idx {
                    if md_codes[code_i].line == idx {
                        body_codes.push(BodyCode {
                            line: body.len(),
                            start: md_codes[code_i].start,
                            end: md_codes[code_i].end,
                            text: md_codes[code_i].text.clone(),
                        });
                    }
                    code_i += 1;
                }
                while block_i < md_blocks.len() && md_blocks[block_i].line < idx {
                    block_i += 1;
                }
                if block_i < md_blocks.len() && md_blocks[block_i].line == idx {
                    body_code_blocks.push(BodyCodeBlock {
                        line: body.len(),
                        height: md_blocks[block_i].height,
                        text: md_blocks[block_i].text.clone(),
                    });
                    block_i += 1;
                }
                body.push(md[idx].clone());
                idx += 1;
                continue;
            }
            // Collect the run of consecutive images whose gaps are blank only
            // and whose alignment matches.
            let base_align = images[next].align;
            let mut last = images[next].line;
            let mut run_end = next + 1;
            while run_end < images.len() {
                if images[run_end].align != base_align {
                    break;
                }
                let gap_start = last + 1;
                let gap_end = images[run_end].line;
                if gap_start < gap_end
                    && !md[gap_start..gap_end]
                        .iter()
                        .all(|line| line.spans.is_empty())
                {
                    break;
                }
                last = images[run_end].line;
                run_end += 1;
            }
            let run = &images[next..run_end];
            for im in run {
                self.browse_fetch_image(&im.url);
            }
            let sizes: Option<Vec<(u16, u16)>> = run
                .iter()
                .map(|im| self.body_image_block(&im.url, width))
                .collect();
            if let Some(sizes) = sizes {
                let (strip_h, placements) =
                    Self::layout_image_strip(&sizes, width as u16, base_align);
                let strip_line = body.len();
                for _ in 0..strip_h {
                    body.push(Line::default());
                }
                for (k, im) in run.iter().enumerate() {
                    let (x, y) = placements[k];
                    body_images.push(BodyImage {
                        line: strip_line + y as usize,
                        url: im.url.clone(),
                        x,
                        width: sizes[k].0,
                        height: sizes[k].1,
                        link: im.link.clone(),
                    });
                }
                idx = last + 1;
                next = run_end;
                continue;
            }
            // Partially loaded: reserve the first image alone and retry the
            // rest once the remaining fetches complete (`body_dirty`).
            if let Some((cols, rows)) = self.body_image_block(&run[0].url, width) {
                let x = Self::strip_x(base_align, cols, width as u16);
                let strip_line = body.len();
                for _ in 0..rows {
                    body.push(Line::default());
                }
                body_images.push(BodyImage {
                    line: strip_line,
                    url: run[0].url.clone(),
                    x,
                    width: cols,
                    height: rows,
                    link: run[0].link.clone(),
                });
            } else {
                body.push(md[idx].clone());
            }
            idx = images[next].line + 1;
            next += 1;
        }
        self.browse.body = body;
        self.browse.body_images = body_images;
        self.browse.body_links = body_links;
        self.browse.body_codes = body_codes;
        self.browse.body_code_blocks = body_code_blocks;
    }

    /// Pack image block sizes into horizontal rows aligned per `align`.
    ///
    /// Returns the total strip height and, per image, its `(x, y)` offset
    /// inside the strip. Rows fill greedily up to `width` cells, images in a
    /// row are separated by [`IMG_COL_GAP`] cells and wrapped rows by
    /// [`IMG_ROW_GAP`] blank rows.
    fn layout_image_strip(
        sizes: &[(u16, u16)],
        width: u16,
        align: MdAlign,
    ) -> (u16, Vec<(u16, u16)>) {
        let mut placements = vec![(0u16, 0u16); sizes.len()];
        if sizes.is_empty() {
            return (0, placements);
        }
        let width = width.max(1);
        let mut y = 0u16;
        let mut i = 0usize;
        while i < sizes.len() {
            let mut row: Vec<usize> = Vec::new();
            let mut row_w = 0u16;
            while i < sizes.len() {
                let cols = sizes[i].0;
                let need = if row.is_empty() { cols } else { IMG_COL_GAP + cols };
                if !row.is_empty() && row_w + need > width {
                    break;
                }
                row.push(i);
                row_w += need;
                i += 1;
                if row_w >= width {
                    break;
                }
            }
            let row_h = row
                .iter()
                .map(|&k| sizes[k].1)
                .max()
                .unwrap_or(1)
                .max(1);
            let mut x = Self::strip_x(align, row_w, width);
            for &k in &row {
                placements[k] = (x, y);
                x += sizes[k].0 + IMG_COL_GAP;
            }
            y = y.saturating_add(row_h).saturating_add(IMG_ROW_GAP);
        }
        let total = y.saturating_sub(IMG_ROW_GAP).max(1);
        (total, placements)
    }

    /// Horizontal start of a row of `row_w` cells inside `width` cells.
    fn strip_x(align: MdAlign, row_w: u16, width: u16) -> u16 {
        match align {
            MdAlign::Left => 0,
            MdAlign::Center => width.saturating_sub(row_w) / 2,
            MdAlign::Right => width.saturating_sub(row_w),
        }
    }

    /// Cell size reserved for a loaded image scaled to fit `width` columns.
    ///
    /// Images at least half as wide as the body are stretched to the full
    /// width (Modrinth renders large description images edge to edge), which
    /// also keeps them on a row of their own; smaller ones keep their natural
    /// size so badge-style icons can pack side by side.
    /// Returns `None` while the image is still loading.
    fn body_image_block(&self, url: &str, width: usize) -> Option<(u16, u16)> {
        let img = self.browse_images.get(url)?;
        let (font_w, font_h) = self.picker.font_size();
        let font_w = font_w.max(1) as u64;
        let font_h = font_h.max(1) as u64;
        let img_w = img.width.max(1) as u64;
        let img_h = img.height.max(1) as u64;
        let natural_cols = img_w.div_ceil(font_w);
        let (cols, rows) = if natural_cols >= (width as u64).div_ceil(2) {
            let cols = width as u64;
            let scaled_px_h = img_h * cols * font_w / img_w;
            (cols, scaled_px_h.div_ceil(font_h))
        } else {
            (natural_cols, img_h.div_ceil(font_h))
        };
        let cols = cols.clamp(1, width as u64) as u16;
        let rows = rows.clamp(1, 16) as u16;
        Some((cols, rows))
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
        let (lines, links) = self.browse_changelog_render(w);
        let visible = list.height as usize;
        self.browse.changelog_visible = visible;
        self.browse.changelog_scroll = self
            .browse
            .changelog_scroll
            .min(lines.len().saturating_sub(visible));
        let start = self.browse.changelog_scroll;
        frame.render_widget(Clear, list);
        self.browse.changelog_links = links;
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
        let mut hot: Option<BodyLink> = None;
        for link in &self.browse.changelog_links {
            if link.line < start || link.line >= start + visible || link.end <= link.start {
                continue;
            }
            let rect = Rect {
                x: list.x + link.start as u16,
                y: list.y + (link.line - start) as u16,
                width: (link.end - link.start) as u16,
                height: 1,
            };
            if self.is_hovered(rect) {
                hot = Some(link.clone());
                break;
            }
        }
        if let Some(link) = &hot {
            self.browse.hover_hint = Some(format!("↗ {}", link.url));
        }
        let hot_pos = hot.map(|l| (l.line, l.start, l.end));
        let accent = self.theme.green_bright;
        let shown: Vec<Line> = lines
            .into_iter()
            .skip(start)
            .take(visible)
            .enumerate()
            .map(|(i, line)| match hot_pos {
                Some((hl, hs, he)) if hl == start + i => {
                    recolor_link_line(&line, hs, he, accent)
                }
                _ => line,
            })
            .collect();
        frame.render_widget(Paragraph::new(shown).style(self.theme.card()), list);
        for (idx, link) in self.browse.changelog_links.clone().iter().enumerate() {
            if link.line < start || link.line >= start + visible || link.end <= link.start {
                continue;
            }
            let rect = Rect {
                x: list.x + link.start as u16,
                y: list.y + (link.line - start) as u16,
                width: (link.end - link.start) as u16,
                height: 1,
            };
            self.push_hitbox(rect, HitAction::BrowseChangelogLink(idx));
        }
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
                self.browse_ensure_selected_visible();
            }
            KeyCode::Char('G') => {
                self.browse.selected = self.browse.results.len().saturating_sub(1);
                self.browse_ensure_selected_visible();
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
            self.browse_wheel(delta);
        }
    }

    /// Mouse wheel over the results list scrolls the view (one card per
    /// notch); the selection stays put. Paging at the ends preserves the
    /// infinite-scroll behaviour.
    pub(crate) fn browse_wheel(&mut self, delta: i32) {
        let len = self.browse.results.len();
        if len == 0 {
            return;
        }
        let max = len.saturating_sub(self.browse.list_visible.max(1)) as i32;
        let next = self.browse.list_scroll as i32 + delta;
        if next < 0 {
            if self.browse.offset > 0 {
                self.browse_prev_page();
            }
            return;
        }
        if next > max {
            if self.browse.offset + 30 < self.browse.total {
                self.browse_next_page();
            }
            return;
        }
        self.browse.list_scroll = next as usize;
    }

    pub(crate) fn browse_move(&mut self, delta: i32) {
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
        self.browse_ensure_selected_visible();
    }

    /// Pull the list view so the keyboard selection stays visible.
    pub(crate) fn browse_ensure_selected_visible(&mut self) {
        let vis = self.browse.list_visible.max(1);
        if self.browse.selected < self.browse.list_scroll {
            self.browse.list_scroll = self.browse.selected;
        } else if self.browse.selected >= self.browse.list_scroll + vis {
            self.browse.list_scroll = self.browse.selected + 1 - vis;
        }
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

    fn browse_changelog_render(&self, width: usize) -> (Vec<Line<'static>>, Vec<BodyLink>) {
        let Some(version) = self.browse.versions.get(self.browse.version_selected) else {
            return (Vec::new(), Vec::new());
        };
        let Some(body) = version
            .changelog
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        else {
            return (Vec::new(), Vec::new());
        };
        let (lines, _images, links, _codes, _blocks) = render_md_full(body, width.max(10), &self.theme);
        let links = links
            .into_iter()
            .map(|l| BodyLink {
                line: l.line,
                start: l.start,
                end: l.end,
                url: l.url,
            })
            .collect();
        (lines, links)
    }

    fn browse_changelog_len(&self) -> usize {
        self.browse_changelog_render(80).0.len()
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
