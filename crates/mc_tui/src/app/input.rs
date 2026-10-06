use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use super::{
    rect_contains, settings_field_count, split_args, App, ButtonId, Focus, HitAction, Nav,
    OverlayAction, PickerKey, NAV_BUTTON_HEIGHT,
};
use crate::forms::{Form, FormAction, Overlay, TextAction};
use crate::views::list_wheel;

impl App {
    pub(crate) fn handle_terminal_event(&mut self, event: crossterm::event::Event) -> bool {
        match event {
            crossterm::event::Event::Key(key) => {
                if key.kind == crossterm::event::KeyEventKind::Press {
                    self.handle_key(key);
                    true
                } else {
                    false
                }
            }
            crossterm::event::Event::Mouse(mouse) => self.handle_mouse(mouse),
            crossterm::event::Event::Resize(_, _) => true,
            crossterm::event::Event::FocusGained => true,
            crossterm::event::Event::Paste(s) => {
                self.paste_text(&s);
                true
            }
            _ => false,
        }
    }

    /// True while an inline search/filter bar or settings field is capturing character input.
    pub(crate) fn is_typing(&self) -> bool {
        self.settings_edit.is_some()
            || self.settings_dropdown.is_some()
            || self.mods_search_focused
            || self.rp_search_focused
            || self.shaders_search_focused
            || (self.nav == Nav::Browse
                && self.browse.focus == crate::views::browse::BrowseFocus::Search)
    }

    /// The edit field currently capturing keyboard input, if any.
    fn active_edit(&mut self) -> Option<(&mut crate::edit::EditState, &mut String)> {
        if self.settings_dropdown.is_some() {
            return None;
        }
        if let Some((_, buffer)) = self.settings_edit.as_mut() {
            return Some((&mut self.edit, buffer));
        }
        if self.mods_search_focused {
            return Some((&mut self.edit, &mut self.mods_search_query));
        }
        if self.rp_search_focused {
            return Some((&mut self.edit, &mut self.rp_inline_query));
        }
        if self.shaders_search_focused {
            return Some((&mut self.edit, &mut self.shaders_inline_query));
        }
        if self.nav == Nav::Browse
            && self.browse.focus == crate::views::browse::BrowseFocus::Search
        {
            return Some((&mut self.edit, &mut self.browse.search_input));
        }
        None
    }

    /// Feed a key press into the active inline edit field. Returns `true` when
    /// the key was consumed (so global shortcuts do not fire while typing).
    /// Escape/Enter/Tab are left for the regular dispatch (close field, commit).
    pub(crate) fn typing_edit_key(&mut self, key: KeyEvent) -> bool {
        use crate::edit::EditSignal;
        let Some((edit, text)) = self.active_edit() else {
            return false;
        };
        match edit.handle_key(text, key) {
            EditSignal::Unhandled => false,
            EditSignal::Copy(s) | EditSignal::Cut(s) => {
                self.copy_text(&s);
                true
            }
            EditSignal::Handled => true,
        }
    }

    /// Bracketed-paste text into the active edit field (single line).
    pub(crate) fn paste_text(&mut self, text: &str) {
        let Some((edit, buffer)) = self.active_edit() else {
            return;
        };
        edit.insert_str(buffer, text);
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) {
        if self.overlay.is_some() {
            self.handle_overlay_key(key);
            return;
        }

        if self.typing_edit_key(key) {
            return;
        }

        let typing = self.is_typing();
        match key.code {
            KeyCode::Char('q') if !typing && key.modifiers.is_empty() => self.request_quit(),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            KeyCode::Char('?') if !typing => self.show_help(),
            KeyCode::Tab if !typing => self.toggle_focus(),
            KeyCode::BackTab if !typing => self.toggle_focus(),
            KeyCode::F(2) if !typing => self.open_nav(Nav::Accounts),
            KeyCode::F(3) if !typing => self.open_nav(Nav::Launcher),
            KeyCode::F(4) if !typing => self.open_nav(Nav::Modpacks),
            KeyCode::Char(c @ '1'..='9') if !typing => {
                let idx = (c as u8 - b'1') as usize;
                if let Some(nav) = Nav::menu().get(idx) {
                    self.open_nav(*nav);
                }
            }
            KeyCode::Char('n') if !typing && self.nav == Nav::Instances => {
                self.open_create_instance_form()
            }
            _ => self.handle_view_key(key),
        }
    }

    pub(crate) fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Sidebar => Focus::Content,
            _ => Focus::Sidebar,
        };
        if self.focus == Focus::Sidebar {
            self.ensure_nav_visible();
        }
    }

    pub(crate) fn open_nav(&mut self, nav: Nav) {
        if nav == Nav::Browse {
            self.browse_return = self.nav;
        }
        self.nav = nav;
        self.focus = Focus::Content;
        // Close any inline search field from the previous page: it would keep
        // swallowing keystrokes (including ctrl+c) while hidden on the new page.
        self.mods_search_focused = false;
        self.rp_search_focused = false;
        self.shaders_search_focused = false;
        self.settings_edit = None;
        self.settings_dropdown = None;
        self.settings_scroll = 0;
        self.toolbar_scroll = 0;
        self.toolbar_target = 0;
        let max = settings_field_count(nav).saturating_sub(1);
        self.settings_field = self.settings_field.min(max);
        match nav {
            Nav::Instances => self.reload_instances(),
            Nav::Browse => self.open_browse(),
            Nav::Mods => self.reload_mods(),
            Nav::Accounts => self.reload_accounts(),
            Nav::Logs if self.running.is_none() && self.log_buffer.is_empty() => {
                self.load_latest_log();
            }
            Nav::ResourcePacks => self.reload_resource_packs(),
            Nav::Shaders => self.reload_shaders(),
            Nav::Worlds => self.reload_worlds(),
            Nav::Screenshots => self.reload_screenshots(),
            _ => {}
        }
        self.ensure_nav_visible();
    }

    pub(crate) fn handle_view_key(&mut self, key: KeyEvent) {
        if self.focus == Focus::Sidebar {
            self.handle_sidebar_key(key);
            return;
        }
        if key.code == KeyCode::Esc
            && self.nav != Nav::Modpacks
            && self.settings_edit.is_none()
            && self.settings_dropdown.is_none()
            && !self.mods_search_focused
            && !self.rp_search_focused
            && !self.shaders_search_focused
            && !(self.nav == Nav::Browse
                && self.browse.focus == crate::views::browse::BrowseFocus::Search)
        {
            if self.nav == Nav::Browse {
                if self.browse.in_detail() {
                    self.browse_close_detail();
                } else {
                    self.open_nav(self.browse_return);
                }
                return;
            }
            // Content → Sidebar → leave (via open_nav).
            self.open_nav(Nav::Instances);
            return;
        }
        match self.nav {
            Nav::Instances => self.key_instance_grid(key),
            Nav::Browse => self.key_browse(key),
            Nav::Mods => self.key_mods(key),
            Nav::Modpacks => self.key_modpacks(key),
            Nav::Versions => self.key_versions(key),
            Nav::Jvm => self.key_instance_settings(key),
            Nav::Logs => self.key_logs(key),
            Nav::Accounts => self.key_accounts(key),
            Nav::Launcher => self.key_settings(key),
            Nav::ResourcePacks => self.key_resource_packs(key),
            Nav::Shaders => self.key_shaders(key),
            Nav::Worlds => self.key_worlds(key),
            Nav::Screenshots => self.key_screenshots(key),
        }
    }

    pub(crate) fn handle_sidebar_key(&mut self, key: KeyEvent) {
        let all = Nav::menu();
        let idx = all.iter().position(|n| *n == self.nav).unwrap_or(0);
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                let next = (idx + 1).min(all.len() - 1);
                self.open_nav(all[next]);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let prev = idx.saturating_sub(1);
                self.open_nav(all[prev]);
            }
            KeyCode::Enter => self.focus = Focus::Content,
            _ => {}
        }
    }

    pub(crate) fn handle_mouse(&mut self, mouse: MouseEvent) -> bool {
        let new_pos = (mouse.column, mouse.row);
        let old_action = self.mouse_pos.and_then(|pos| self.hit_action_at(pos));
        self.mouse_pos = Some(new_pos);
        let new_action = self.hit_action_at(new_pos);
        let hover_changed = old_action != new_action;

        if self.overlay.is_some() {
            let result = match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    if let Some(HitAction::Overlay(action)) = self.hit_action_at(new_pos) {
                        self.dispatch_overlay_action(action);
                    } else {
                        self.handle_overlay_click();
                    }
                    true
                }
                MouseEventKind::ScrollDown => {
                    self.overlay_scroll(1);
                    true
                }
                MouseEventKind::ScrollUp => {
                    self.overlay_scroll(-1);
                    true
                }
                _ => hover_changed,
            };
            return result;
        }

        match mouse.kind {
            MouseEventKind::ScrollDown => {
                self.hover_scroll(1);
                true
            }
            MouseEventKind::ScrollUp => {
                self.hover_scroll(-1);
                true
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let hit = self.hitbox_at(new_pos);
                let keep = hit.as_ref().map(|h| h.action);
                self.unfocus_search_bars_except(keep);
                if let Some(hit) = hit {
                    self.dispatch_click(hit, new_pos);
                    true
                } else {
                    hover_changed
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some((action, rect)) = self.edit_drag {
                    self.drag_edit(action, rect, new_pos);
                    true
                } else {
                    hover_changed
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if self.edit_drag.take().is_some() {
                    self.edit.end_drag();
                    true
                } else {
                    hover_changed
                }
            }
            _ => hover_changed,
        }
    }

    pub(crate) fn hover_scroll(&mut self, delta: i32) {
        let Some(pos) = self.mouse_pos else {
            self.scroll_active(delta);
            return;
        };
        if self.toolbar_max > 0 && rect_contains(self.toolbar_area, pos) {
            let step = delta * 4;
            let next = (self.toolbar_target as i32 + step).clamp(0, self.toolbar_max as i32) as u16;
            self.toolbar_target = next;
            return;
        }
        if self.nav_max > 0 && rect_contains(self.sidebar_area, pos) {
            let next = (self.nav_target as i32 + delta).clamp(0, self.nav_max as i32) as u16;
            self.nav_target = next;
            return;
        }
        self.scroll_active(delta);
    }

    fn ensure_nav_visible(&mut self) {
        let Some(idx) = Nav::menu().iter().position(|n| *n == self.nav) else {
            return;
        };
        if self.nav_viewport.height == 0 {
            return;
        }
        let top = idx as u16 * NAV_BUTTON_HEIGHT;
        let bottom = top + NAV_BUTTON_HEIGHT;
        if top < self.nav_target {
            self.nav_target = top;
        } else if bottom > self.nav_target + self.nav_viewport.height {
            self.nav_target = bottom - self.nav_viewport.height;
        }
        if self.nav_target > self.nav_max {
            self.nav_target = self.nav_max;
        }
    }

    pub(crate) fn hit_action_at(&self, pos: (u16, u16)) -> Option<HitAction> {
        self.hitboxes
            .iter()
            .find(|h| rect_contains(h.rect, pos))
            .map(|h| h.action)
    }

    pub(crate) fn hitbox_at(&self, pos: (u16, u16)) -> Option<crate::app::Hitbox> {
        self.hitboxes
            .iter()
            .find(|h| rect_contains(h.rect, pos))
            .copied()
    }

    /// Text column inside a filter field under the mouse. The text always
    /// starts one column right of the field edge (accent bar / left inset);
    /// on an empty field every column clamps to 0 anyway.
    fn field_click_col(&self, rect: ratatui::layout::Rect, pos: (u16, u16)) -> usize {
        let rel = pos.0.saturating_sub(rect.x) as usize;
        rel.saturating_sub(1)
    }

    /// A mouse click outside a focused search bar drops its focus, so the
    /// single-letter shortcuts become active again. `keep` is the hitbox the
    /// click landed on — clicking the bar itself must not unfocus it.
    pub(crate) fn unfocus_search_bars_except(&mut self, keep: Option<HitAction>) {
        let mut changed = false;
        if keep != Some(HitAction::ModsSearchBar) && self.mods_search_focused {
            self.mods_search_focused = false;
            changed = true;
        }
        if keep != Some(HitAction::RPSearchBar) && self.rp_search_focused {
            self.rp_search_focused = false;
            changed = true;
        }
        if keep != Some(HitAction::ShadersSearchBar) && self.shaders_search_focused {
            self.shaders_search_focused = false;
            changed = true;
        }
        if keep != Some(HitAction::BrowseSearchBar)
            && self.browse.focus == crate::views::browse::BrowseFocus::Search
        {
            self.browse.focus = crate::views::browse::BrowseFocus::List;
            changed = true;
        }
        if changed {
            self.edit.end_drag();
        }
    }

    /// Handle a mouse click: for text fields place the cursor at the click
    /// position (and start a selection drag); everything else dispatches.
    pub(crate) fn dispatch_click(&mut self, hit: crate::app::Hitbox, pos: (u16, u16)) {
        let action = hit.action;
        match action {
            HitAction::ModsSearchBar => {
                self.mods_search_focused = true;
                let col = self.field_click_col(hit.rect, pos);
                self.edit.start_drag(&self.mods_search_query, col);
                self.edit_drag = Some((action, hit.rect));
            }
            HitAction::RPSearchBar => {
                self.rp_search_focused = true;
                let col = self.field_click_col(hit.rect, pos);
                self.edit.start_drag(&self.rp_inline_query, col);
                self.edit_drag = Some((action, hit.rect));
            }
            HitAction::ShadersSearchBar => {
                self.shaders_search_focused = true;
                let col = self.field_click_col(hit.rect, pos);
                self.edit.start_drag(&self.shaders_inline_query, col);
                self.edit_drag = Some((action, hit.rect));
            }
            HitAction::BrowseSearchBar => {
                if self.browse.focus != crate::views::browse::BrowseFocus::Search {
                    self.browse.search_input = self.browse.query.clone();
                }
                self.browse.focus = crate::views::browse::BrowseFocus::Search;
                let col = self.field_click_col(hit.rect, pos);
                self.edit.start_drag(&self.browse.search_input, col);
                self.edit_drag = Some((action, hit.rect));
            }
            _ => self.dispatch_hit(action),
        }
    }

    pub(crate) fn drag_edit(
        &mut self,
        action: HitAction,
        rect: ratatui::layout::Rect,
        pos: (u16, u16),
    ) {
        let col = self.field_click_col(rect, pos);
        match action {
            HitAction::ModsSearchBar => self.edit.drag_to(&self.mods_search_query, col),
            HitAction::RPSearchBar => self.edit.drag_to(&self.rp_inline_query, col),
            HitAction::ShadersSearchBar => self.edit.drag_to(&self.shaders_inline_query, col),
            HitAction::BrowseSearchBar => self.edit.drag_to(&self.browse.search_input, col),
            _ => {}
        }
    }

    pub(crate) fn dispatch_hit(&mut self, action: HitAction) {
        match action {
            HitAction::NavItem(nav) => self.open_nav(nav),
            HitAction::InstanceTile(idx) => self.select_instance(idx),
            HitAction::WorldRow(idx) => {
                self.worlds_state.select(Some(idx));
                self.focus = Focus::Content;
            }
            HitAction::ScreenshotTile(idx) => {
                self.screenshots_state.select(Some(idx));
                self.focus = Focus::Content;
            }
            HitAction::SearchRow(idx) => {
                self.search_state.select(Some(idx));
                self.focus = Focus::Content;
            }
            HitAction::ProjectVersionRow(idx) => {
                self.project_state.select(Some(idx));
                self.focus = Focus::Content;
            }
            HitAction::ModRow(idx) => {
                self.mods_state.select(Some(idx));
                self.focus = Focus::Content;
            }
            HitAction::ModsSearchBar => {
                self.mods_search_focused = true;
            }
            HitAction::RPSearchBar => {
                self.rp_search_focused = true;
            }
            HitAction::ShadersSearchBar => {
                self.shaders_search_focused = true;
            }
            HitAction::AccountRow(idx) => {
                self.account_state.select(Some(idx));
                self.focus = Focus::Content;
            }
            HitAction::SettingsRow(idx) => {
                self.settings_select_field(idx);
                // Choice rows open their dropdown immediately; text/slider rows
                // start inline edit so mouse users never need Enter.
                if !self.settings_dropdown_options(idx).is_empty() {
                    self.settings_open_dropdown();
                } else {
                    let is_text_or_slider = matches!(
                        (self.nav, idx),
                        (Nav::Launcher, 0..=2) | (Nav::Jvm, 0 | 1 | 3 | 5 | 6)
                    );
                    if is_text_or_slider {
                        self.begin_settings_row_edit();
                    } else if matches!((self.nav, idx), (Nav::Launcher, 4..=6) | (Nav::Jvm, 4)) {
                        self.toggle_selected_bool_row();
                    }
                }
            }
            HitAction::SettingsOption(field, opt) => {
                self.settings_select_field(field);
                self.settings_pick_option(field, opt);
            }
            HitAction::SettingsSlider(field) => {
                self.settings_select_field(field);
                if let Some((_, track, min, max)) = self
                    .settings_sliders
                    .iter()
                    .find(|(f, ..)| *f == field)
                    .cloned()
                {
                    if let Some(pos) = self.mouse_pos {
                        let v = crate::views::settings_ui::slider_value_at(pos.0, track, min, max);
                        self.settings_set_slider(field, v);
                    }
                }
            }
            HitAction::Button(button) => self.dispatch_button(button),
            HitAction::BrowseResult(idx) => self.browse_select_result(idx),
            HitAction::BrowseVersion(idx) => self.browse_select_version(idx),
            HitAction::BrowseDetailTab(idx) => self.browse_select_tab(idx),
            HitAction::BrowseGallery(idx) => self.browse_select_gallery(idx),
            HitAction::BrowseLink(idx) => self.open_project_link(idx),
            HitAction::BrowseBodyLink(idx) => self.open_body_link(idx),
            HitAction::BrowseTextLink(idx) => self.open_text_link(idx),
            HitAction::BrowseChangelogLink(idx) => self.open_changelog_link(idx),
            HitAction::CopyBodyCode(idx) => self.copy_body_code(idx),
            HitAction::CopyBodyCodeBlock(idx) => self.copy_body_code_block(idx),
            HitAction::BrowseInstall => self.browse_install(),
            HitAction::BrowseQuickInstall(idx) => self.browse_quick_install(idx),
            HitAction::BrowseSearchBar => {
                self.browse.focus = crate::views::browse::BrowseFocus::Search;
                self.browse.search_input = self.browse.query.clone();
            }
            HitAction::BrowseFilter(item) => self.browse_filter_click(item),
            HitAction::BrowsePagePrev => self.browse_prev_page(),
            HitAction::BrowsePageNext => self.browse_next_page(),
            HitAction::GroupHeader(idx) => {
                // Header click toggles collapse; also marks the active group.
                let sections = self.instance_sections();
                if let Some(section) = sections.get(idx) {
                    let name = section.name.clone();
                    self.set_selected_group(name.clone());
                    self.toggle_group_collapsed(&name);
                    self.focus = Focus::Content;
                }
            }
            HitAction::Overlay(action) => self.dispatch_overlay_action(action),
            HitAction::Crumb(crumb) => match crumb {
                crate::app::Crumb::Page(nav) => self.open_nav(nav),
                crate::app::Crumb::BrowseList => self.browse_close_detail(),
            },
        }
    }

    pub(crate) fn select_instance(&mut self, idx: usize) {
        if idx >= self.instances.len() {
            return;
        }
        self.instance_state.select(Some(idx));
        self.focus = Focus::Content;
        self.ensure_tile_visible();
        // Refresh per-instance content lists so we never show another
        // instance's mods / packs / worlds / screenshots.
        self.reload_mods();
        self.reload_resource_packs();
        self.reload_shaders();
        self.reload_worlds();
        self.reload_screenshots();
    }

    pub(crate) fn dispatch_button(&mut self, button: ButtonId) {
        match button {
            ButtonId::Launch => self.launch_selected(),
            ButtonId::StopGame => self.stop_selected_game(),
            ButtonId::NewInstance => self.open_create_instance_form(),
            ButtonId::EditInstance => self.open_nav(Nav::Jvm),
            ButtonId::DeleteInstance => self.confirm_delete_instance(),
            ButtonId::InstallInstance => self.install_selected_instance(),
            ButtonId::ChangeVersion => self.open_change_version_picker(),
            ButtonId::RenameInstance => self.open_rename_instance_form(),
            ButtonId::Search => self.open_search_prompt(),
            ButtonId::ImportModpack => self.browse_for_mrpack(),
            ButtonId::InstallProject => self.install_selected_project(),
            ButtonId::ToggleMod => self.toggle_selected_mod(),
            ButtonId::DeleteMod => self.confirm_delete_mod(),
            ButtonId::UpdateMods => self.check_mod_updates(),
            ButtonId::BrowseMods => {
                let kind = crate::views::browse::BrowseKind::Mods;
                self.browse.save_cache();
                self.browse.load_cache(kind);
                self.open_nav(Nav::Browse);
            }
            ButtonId::ShadersBrowse => {
                let kind = crate::views::browse::BrowseKind::Shaders;
                self.browse.save_cache();
                self.browse.load_cache(kind);
                self.open_nav(Nav::Browse);
            }
            ButtonId::RpBrowse => {
                let kind = crate::views::browse::BrowseKind::ResourcePacks;
                self.browse.save_cache();
                self.browse.load_cache(kind);
                self.open_nav(Nav::Browse);
            }
            ButtonId::OfflineLogin => self.open_offline_login(),
            ButtonId::MicrosoftLogin => self.start_microsoft_login(),
            ButtonId::SetActiveAccount => self.set_active_account(),
            ButtonId::DeleteAccount => self.confirm_delete_account(),
            ButtonId::ChangeSkin => self.open_skin_prompt(),
            ButtonId::ClearLogs => self.log_buffer.clear(),
            ButtonId::PauseLogs => self.log_buffer.paused = !self.log_buffer.paused,
            ButtonId::FollowLogs => self.log_follow = !self.log_follow,
            ButtonId::AnalyzeCrash => self.analyze_crash(),
            ButtonId::SaveSettings => self.save_settings(),
            ButtonId::DetectJava => self.spawn_java_discovery(),
            ButtonId::OpenAsciiBgFolder => self.open_ascii_bg_folder(),
            ButtonId::OpenFolder => self.open_current_folder(),
            ButtonId::DeleteSelected => self.confirm_delete_selected(),
            ButtonId::ToggleSelected => self.toggle_selected_entry(),
            ButtonId::NewGroup => self.open_new_group_form(),
            ButtonId::RenameGroup => self.open_rename_group_form(),
            ButtonId::DeleteGroup => self.confirm_delete_group(),
        }
    }

    pub(crate) fn scroll_active(&mut self, delta: i32) {
        if self.overlay.is_some() {
            return;
        }
        match self.nav {
            Nav::Instances => {
                let step = delta * 6;
                let next = (self.tile_target as i64 + step as i64)
                    .clamp(0, self.tile_max as i64) as usize;
                self.tile_target = next;
            }
            Nav::Browse => self.browse_scroll(delta),
            Nav::Versions => {}
            Nav::Modpacks => {
                if self.selected_project.is_some() {
                    list_wheel(
                        &mut self.project_state,
                        self.project_versions.len(),
                        self.project_visible,
                        delta,
                    );
                } else {
                    list_wheel(
                        &mut self.search_state,
                        self.search_results.len(),
                        self.search_visible,
                        delta,
                    );
                }
            }
            Nav::Mods => {
                let len = self.visible_mod_indices().len();
                list_wheel(&mut self.mods_state, len, self.mods_visible, delta);
            }
            Nav::Logs => self.scroll_logs(delta * 3),
            Nav::Jvm => {
                let len = settings_field_count(Nav::Jvm);
                let next = (self.settings_field as i32 + delta).clamp(0, len as i32 - 1) as usize;
                self.settings_field = next;
            }
            Nav::Accounts => list_wheel(
                &mut self.account_state,
                self.accounts.accounts().len(),
                self.accounts_visible,
                delta,
            ),
            Nav::Launcher => {
                let len = settings_field_count(Nav::Launcher);
                let next = (self.settings_field as i32 + delta).clamp(0, len as i32 - 1) as usize;
                self.settings_field = next;
            }
            Nav::ResourcePacks => {
                let len = self.visible_rp_indices().len();
                list_wheel(&mut self.resource_packs_state, len, self.resource_packs_visible, delta);
            }
            Nav::Shaders => {
                let len = self.visible_shader_indices().len();
                list_wheel(&mut self.shaders_state, len, self.shaders_visible, delta);
            }
            Nav::Worlds => {
                // Custom cards: scroll the view one world per notch.
                let total = self.worlds.len();
                if total > 0 {
                    let vis = self.world_visible_rows.max(1);
                    let max = total.saturating_sub(vis);
                    let off =
                        (self.world_scroll as i32 + delta).clamp(0, max as i32) as usize;
                    self.world_scroll = off;
                    if let Some(sel) = self.worlds_state.selected() {
                        let edge = (off + vis).saturating_sub(1).min(total.saturating_sub(1));
                        self.worlds_state.select(Some(sel.clamp(off, edge)));
                    }
                }
            }
            Nav::Screenshots => {
                // Custom grid: scroll the view one row per notch.
                let total = self.screenshots.len();
                if total > 0 {
                    let cols = self.screenshot_cols.max(1);
                    let vis = self.screenshot_visible_rows.max(1);
                    let max = total.div_ceil(cols).saturating_sub(vis);
                    let off =
                        (self.screenshot_scroll as i32 + delta).clamp(0, max as i32) as usize;
                    self.screenshot_scroll = off;
                    if let Some(sel) = self.screenshots_state.selected() {
                        let lo = off * cols;
                        let hi = ((off + vis) * cols)
                            .saturating_sub(1)
                            .min(total.saturating_sub(1));
                        self.screenshots_state.select(Some(sel.clamp(lo, hi)));
                    }
                }
            }
        }
    }

    pub(crate) fn ensure_tile_visible(&mut self) {
        let view_h = self.tile_view_h;
        if view_h == 0 {
            return;
        }
        let sections = self.instance_sections();
        let boxes = crate::views::tiles::layout_panels(&sections, self.tile_body_w);
        let total = crate::views::tiles::content_height(&boxes);
        let max_scroll = total.saturating_sub(view_h);

        if let Some(selected) = self.instance_state.selected() {
            if let Some(y) = crate::views::tiles::instance_content_y(&boxes, &sections, selected) {
                let tile_end = y + crate::views::tiles::TILE_H as usize;
                if y < self.tile_target {
                    self.tile_target = y;
                } else if tile_end > self.tile_target + view_h {
                    self.tile_target = tile_end - view_h;
                }
            }
        }
        self.tile_target = self.tile_target.min(max_scroll);
    }

    pub(crate) fn ensure_screenshot_visible(&mut self) {
        let cols = self.screenshot_cols.max(1);
        let visible = self.screenshot_visible_rows.max(1);
        let Some(selected) = self.screenshots_state.selected() else {
            return;
        };
        let selected_row = selected / cols;
        if selected_row < self.screenshot_scroll {
            self.screenshot_scroll = selected_row;
        } else if selected_row >= self.screenshot_scroll + visible {
            self.screenshot_scroll = selected_row + 1 - visible;
        }
        let rows = self.screenshots.len().div_ceil(cols);
        let max_scroll = rows.saturating_sub(visible);
        self.screenshot_scroll = self.screenshot_scroll.min(max_scroll);
    }

    pub(crate) fn ensure_world_visible(&mut self) {
        let visible = self.world_visible_rows.max(1);
        let Some(selected) = self.worlds_state.selected() else {
            return;
        };
        if selected < self.world_scroll {
            self.world_scroll = selected;
        } else if selected >= self.world_scroll + visible {
            self.world_scroll = selected + 1 - visible;
        }
        let max_scroll = self.worlds.len().saturating_sub(visible);
        self.world_scroll = self.world_scroll.min(max_scroll);
    }

    pub(crate) fn scroll_logs(&mut self, delta: i32) {
        let total = self.log_buffer.visible().count();
        let visible = self.log_visible.max(1);
        let max_scroll = total.saturating_sub(visible);
        let base = if self.log_follow {
            max_scroll
        } else {
            self.log_scroll
        };
        let next = (base as i32 + delta).clamp(0, max_scroll as i32) as usize;
        self.log_scroll = next;
        self.log_follow = next >= max_scroll;
    }

    pub(crate) fn jump_logs(&mut self, to_end: bool) {
        if to_end {
            self.log_follow = true;
        } else {
            self.log_scroll = 0;
            self.log_follow = false;
        }
    }

    // ---------------------------------------------------------------------
    // Overlay input
    // ---------------------------------------------------------------------

    pub(crate) fn handle_overlay_key(&mut self, key: KeyEvent) {
        let Some(overlay) = self.overlay.as_mut() else {
            return;
        };
        match overlay {
            Overlay::Text { value, action, .. } => match key.code {
                KeyCode::Esc => self.overlay = None,
                KeyCode::Enter => {
                    let text = value.clone();
                    let action = action.clone();
                    self.overlay = None;
                    self.submit_text(action, text);
                }
                KeyCode::Backspace => {
                    value.pop();
                }
                KeyCode::Char(c) => value.push(c),
                _ => {}
            },
            Overlay::Form(form) => match key.code {
                KeyCode::Esc => self.overlay = None,
                KeyCode::Enter => {
                    let form = form.clone();
                    self.overlay = None;
                    self.submit_form(form);
                }
                KeyCode::Tab | KeyCode::Down => form.next_field(),
                KeyCode::BackTab | KeyCode::Up => form.prev_field(),
                KeyCode::Left => form.cycle(false),
                KeyCode::Right => form.cycle(true),
                KeyCode::Char(' ') => form.cycle(true),
                KeyCode::Backspace => form.backspace(),
                KeyCode::Char(c) => form.input_char(c),
                _ => {}
            },
            Overlay::Confirm { action, .. } => {
                let action = action.clone();
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                        self.overlay = None;
                        self.confirm(action);
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => self.overlay = None,
                    _ => {}
                }
            }
            Overlay::Message { .. } => {
                if matches!(key.code, KeyCode::Enter | KeyCode::Esc) {
                    self.overlay = None;
                }
            }
            Overlay::DeviceCode { .. } => {
                if key.code == KeyCode::Esc {
                    self.overlay = None;
                }
            }
            Overlay::Picker(_) => self.handle_picker_key(key),
            Overlay::Wizard(_) => self.handle_wizard_key(key),
        }
    }

    pub(crate) fn handle_picker_key(&mut self, key: KeyEvent) {
        let action = {
            let Some(Overlay::Picker(picker)) = self.overlay.as_mut() else {
                return;
            };
            match key.code {
                KeyCode::Esc => PickerKey::Cancel,
                KeyCode::Down | KeyCode::Char('j') => {
                    picker.move_selection(1);
                    PickerKey::None
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    picker.move_selection(-1);
                    PickerKey::None
                }
                KeyCode::Backspace => {
                    picker.query.pop();
                    picker.refilter();
                    PickerKey::None
                }
                KeyCode::Char(c) => {
                    picker.query.push(c);
                    picker.refilter();
                    PickerKey::None
                }
                KeyCode::Enter => match picker.selected_value().map(str::to_string) {
                    Some(value) => PickerKey::Select(picker.target.clone(), value),
                    None => PickerKey::None,
                },
                _ => PickerKey::None,
            }
        };
        match action {
            PickerKey::None => {}
            PickerKey::Cancel => self.cancel_picker(),
            PickerKey::Select(target, value) => {
                self.overlay = None;
                self.apply_picker_value(target, value);
            }
        }
    }

    pub(crate) fn handle_overlay_click(&mut self) {
        match self.overlay.as_ref() {
            // A click outside the Yes button never confirms: close the dialog
            // (same as Esc / "No") so an accidental click cannot delete data.
            Some(Overlay::Confirm { .. }) => self.overlay = None,
            Some(Overlay::Message { .. }) => self.overlay = None,
            _ => {}
        }
    }

    pub(crate) fn dispatch_overlay_action(&mut self, action: OverlayAction) {
        match action {
            OverlayAction::WizardTab(kind) => self.wizard_set_kind(kind),
            OverlayAction::WizardField(idx) => self.wizard_focus_field(idx),
            OverlayAction::WizardPick(idx) => self.wizard_pick_field(idx),
            OverlayAction::WizardSubmit => self.wizard_submit_clicked(),
            OverlayAction::WizardBrowseImport => {
                // For External kind, scan Modrinth App instead of opening file dialog.
                let is_external = matches!(
                    self.overlay,
                    Some(Overlay::Wizard(ref w)) if w.kind == crate::wizard::BuildKind::External
                );
                if is_external {
                    self.scan_modrinth_app_instances();
                } else {
                    self.browse_for_mrpack();
                }
            }
            OverlayAction::WizardResult(idx) => self.wizard_click_result(idx),
            OverlayAction::WizardResultOpen => self.wizard_open_selected_project(),
            OverlayAction::WizardVersion(idx) => self.wizard_select_version(idx),
            OverlayAction::WizardVersionInstall => self.wizard_install_selected_version(),
            OverlayAction::PickerItem(idx) => self.picker_select(idx),
            OverlayAction::FormField(idx) => {
                let Some(Overlay::Form(form)) = self.overlay.as_mut() else {
                    return;
                };
                form.active = idx;
            }
            OverlayAction::FormSubmit => {
                let Some(Overlay::Form(form)) = self.overlay.clone() else {
                    return;
                };
                self.overlay = None;
                self.submit_form(form);
            }
            OverlayAction::TextDone => {}
            OverlayAction::ConfirmYes => {
                let Some(Overlay::Confirm { action, .. }) = self.overlay.clone() else {
                    return;
                };
                self.overlay = None;
                self.confirm(action);
            }
            OverlayAction::ConfirmNo => self.overlay = None,
            OverlayAction::MessageClose => self.overlay = None,
        }
    }

    pub(crate) fn overlay_scroll(&mut self, delta: i32) {
        match self.overlay.as_mut() {
            Some(Overlay::Picker(picker)) => picker.move_selection(delta * 4),
            Some(Overlay::Wizard(wizard)) => wizard.move_selection(delta * 4),
            _ => {}
        }
    }

    pub(crate) fn picker_select(&mut self, idx: usize) {
        let Some(Overlay::Picker(picker)) = self.overlay.as_mut() else {
            return;
        };
        picker.selected = idx;
        let Some(value) = picker.selected_value().map(str::to_string) else {
            return;
        };
        let target = picker.target.clone();
        self.overlay = None;
        self.apply_picker_value(target, value);
    }

    pub(crate) fn submit_text(&mut self, action: TextAction, text: String) {
        match action {
            TextAction::SearchModrinth => self.run_search(text),
            TextAction::BrowseSearch => self.run_browse_search(text),
            TextAction::SearchLogs => {
                self.log_search = text.clone();
                self.log_buffer.filter.search = (!text.is_empty()).then_some(text);
            }
            TextAction::ImportPath => self.import_modpack(PathBuf::from(text)),
            TextAction::JavaPath => {
                let path = PathBuf::from(text.trim());
                if !path.as_os_str().is_empty() {
                    self.settings.java_path = Some(path);
                    self.save_settings_async();
                }
            }
            TextAction::CustomJvmArgs => {
                if let Some(instance) = self.selected_instance().cloned() {
                    self.update_instance_jvm(instance.id(), |jvm| {
                        jvm.custom_jvm_args = split_args(&text);
                    });
                }
            }
            TextAction::CustomGameArgs => {
                if let Some(instance) = self.selected_instance().cloned() {
                    self.update_instance_jvm(instance.id(), |jvm| {
                        jvm.extra_game_args = split_args(&text);
                    });
                }
            }
            TextAction::SkinUrl => self.change_skin(text),
            TextAction::RenameInstance => self.rename_instance(text),
            TextAction::NewGroup => {
                let trimmed = text.trim().to_string();
                if trimmed.is_empty() {
                    return;
                }
                if self.pending_move_on_new_group {
                    self.pending_move_on_new_group = false;
                    self.move_instance_to_group(&trimmed);
                } else {
                    self.create_group(trimmed);
                }
            }
            TextAction::RenameGroup(old) => self.rename_group(old, text),
            TextAction::None => {}
        }
    }

    pub(crate) fn submit_form(&mut self, form: Form) {
        match form.action {
            FormAction::CreateInstance => self.open_create_wizard(),
            FormAction::EditInstanceSettings => self.save_instance_settings_from_form(&form),
            FormAction::ImportModpack => {
                if let Some(path) = form.text_value("Archive") {
                    self.import_modpack(PathBuf::from(path.trim()));
                }
            }
            FormAction::OfflineLogin => {
                if let Some(name) = form.text_value("Username") {
                    self.offline_login(name.trim());
                }
            }
            FormAction::SetJavaPath => {
                if let Some(path) = form.text_value("Java Path") {
                    self.settings.java_path = Some(PathBuf::from(path.trim()));
                    self.save_settings_async();
                }
            }
            FormAction::EditLauncherSettings => self.apply_launcher_settings_form(&form),
        }
    }
}
