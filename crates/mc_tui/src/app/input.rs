use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use super::{
    rect_contains, move_selection, settings_field_count, split_args, App, ButtonId, Focus,
    HitAction, Nav, OverlayAction, PickerKey,
};
use crate::forms::{Form, FormAction, Overlay, TextAction};

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

    pub(crate) fn handle_key(&mut self, key: KeyEvent) {
        if self.overlay.is_some() {
            self.handle_overlay_key(key);
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
    }

    pub(crate) fn open_nav(&mut self, nav: Nav) {
        if nav == Nav::Browse {
            self.browse_return = self.nav;
        }
        self.nav = nav;
        self.focus = Focus::Content;
        self.settings_edit = None;
        self.settings_dropdown = None;
        self.settings_scroll = 0;
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
                self.scroll_active(1);
                true
            }
            MouseEventKind::ScrollUp => {
                self.scroll_active(-1);
                true
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(hit) = self.hit_action_at(new_pos) {
                    self.dispatch_hit(hit);
                    true
                } else {
                    hover_changed
                }
            }
            _ => hover_changed,
        }
    }

    pub(crate) fn hit_action_at(&self, pos: (u16, u16)) -> Option<HitAction> {
        self.hitboxes
            .iter()
            .find(|h| rect_contains(h.rect, pos))
            .map(|h| h.action)
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
        }
    }

    pub(crate) fn select_instance(&mut self, idx: usize) {
        if idx >= self.instances.len() {
            return;
        }
        self.instance_state.select(Some(idx));
        self.focus = Focus::Content;
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
                if self.grid_has_selection() || !self.instances.is_empty() {
                    // Wheel moves one visual row (section-aware).
                    let steps = delta.unsigned_abs().min(4) as i32;
                    let dir = if delta > 0 { steps } else { -steps };
                    self.move_grid_vert(dir);
                }
            }
            Nav::Browse => self.browse_scroll(delta),
            Nav::Versions => {}
            Nav::Modpacks => {
                let step = delta * 4;
                if self.selected_project.is_some() {
                    move_selection(&mut self.project_state, self.project_versions.len(), step);
                } else {
                    move_selection(&mut self.search_state, self.search_results.len(), step);
                }
            }
            Nav::Mods => {
                let step = delta * 4;
                move_selection(&mut self.mods_state, self.installed_mods.len(), step);
            }
            Nav::Logs => self.scroll_logs(delta * 3),
            Nav::Jvm => {
                let len = settings_field_count(Nav::Jvm);
                let next = (self.settings_field as i32 + delta).clamp(0, len as i32 - 1) as usize;
                self.settings_field = next;
            }
            Nav::Accounts => move_selection(
                &mut self.account_state,
                self.accounts.accounts().len(),
                delta * 4,
            ),
            Nav::Launcher => {
                let len = settings_field_count(Nav::Launcher);
                let next = (self.settings_field as i32 + delta).clamp(0, len as i32 - 1) as usize;
                self.settings_field = next;
            }
            Nav::ResourcePacks => {
                move_selection(
                    &mut self.resource_packs_state,
                    self.resource_packs.len(),
                    delta * 4,
                );
            }
            Nav::Shaders => {
                move_selection(&mut self.shaders_state, self.shaders.len(), delta * 4);
            }
            Nav::Worlds => {
                move_selection(&mut self.worlds_state, self.worlds.len(), delta);
            }
            Nav::Screenshots => {
                let cols = self.screenshot_cols.max(1);
                move_selection(
                    &mut self.screenshots_state,
                    self.screenshots.len(),
                    delta * cols as i32,
                );
            }
        }
    }

    pub(crate) fn ensure_tile_visible(&mut self, view_h: usize) {
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
                if y < self.tile_scroll {
                    self.tile_scroll = y;
                } else if tile_end > self.tile_scroll + view_h {
                    self.tile_scroll = tile_end.saturating_sub(view_h);
                }
            }
        }
        self.tile_scroll =
            crate::views::tiles::snap_scroll(&boxes, &sections, self.tile_scroll);
        self.tile_scroll = self.tile_scroll.min(max_scroll);
        // Approximate visible tile rows for PageUp/PageDown step size.
        let row_h = crate::views::tiles::TILE_H as usize + crate::views::tiles::GAP_Y as usize;
        self.tile_visible_rows = (view_h / row_h).max(1);
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
            Overlay::DeviceCode(_) => {
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
            Some(Overlay::Confirm { action, .. }) => {
                let action = action.clone();
                self.overlay = None;
                self.confirm(action);
            }
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
