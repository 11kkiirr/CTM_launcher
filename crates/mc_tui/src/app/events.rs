use std::time::Instant;

use mc_core::auth::AccountStore;

use crate::engine::EngineEvent;
use crate::forms::Overlay;

use super::{App, Nav, RunningProcess};

impl App {
    pub(crate) fn handle_engine_event(&mut self, event: EngineEvent) {
        match event {
            EngineEvent::Status(message) => self.status = message,
            EngineEvent::Progress { ratio, label } => {
                self.progress = Some((ratio, label));
            }
            EngineEvent::ProgressDone => self.progress = None,
            EngineEvent::LogLines(lines) => {
                for line in lines {
                    self.log_buffer.push_line(&line);
                }
            }
            EngineEvent::Started {
                command,
                version,
                handle,
                logs,
            } => {
                self.last_command = Some(command);
                self.log_buffer.clear();
                self.progress = None;
                self.running = Some(RunningProcess {
                    handle: *handle,
                    logs: *logs,
                    version: version.clone(),
                    started: Instant::now(),
                });
                self.set_toast(format!("Launched {version}"), false);
                self.nav = Nav::Logs;
            }
            EngineEvent::InstancesChanged => {
                self.reload_instances();
                self.reload_mods();
            }
            EngineEvent::ImportPathPicked(Some(path)) => {
                self.import_modpack(std::path::PathBuf::from(path.trim()));
            }
            EngineEvent::ImportPathPicked(None) => {
                self.open_import_prompt();
            }
            EngineEvent::AccountsChanged => {
                self.reload_accounts();
            }
            EngineEvent::AccountsReloaded(store) => {
                self.accounts = store;
                if self.accounts.accounts().is_empty() {
                    self.account_state.select(None);
                } else if self.account_state.selected().unwrap_or(0)
                    >= self.accounts.accounts().len()
                {
                    self.account_state
                        .select(Some(self.accounts.accounts().len() - 1));
                } else if self.account_state.selected().is_none() {
                    self.account_state.select(Some(0));
                }
            }
            EngineEvent::Message(lines) => {
                self.overlay = Some(Overlay::message("Updates", lines));
            }
            EngineEvent::SearchResults(results) => {
                self.search_results = results.hits;
                self.search_state.select(if self.search_results.is_empty() {
                    None
                } else {
                    Some(0)
                });
            }
            EngineEvent::BrowseResults(results) => {
                self.browse.total = results.total_hits;
                self.browse.results = results.hits;
                self.browse.loading = false;
                if self.browse.selected >= self.browse.results.len() {
                    self.browse.selected = self.browse.results.len().saturating_sub(1);
                }
            }
            EngineEvent::BrowseProject { project, versions } => {
                self.browse.detail = Some(*project);
                self.browse.versions = versions;
                self.browse.version_selected = 0;
                self.browse.focus = crate::views::browse::BrowseFocus::Body;
                self.browse.body = Vec::new();
                self.browse.body_for = String::new();
                self.browse.body_scroll = 0;
                self.browse_protocols.clear();
                self.browse_fetch_images();
            }
            EngineEvent::BrowseImage { url, data } => {
                if let Some(data) = data {
                    if let Ok(img) = mc_core::img::decode_image(&data) {
                        self.browse_images.insert(url, img);
                    } else if let Ok(dynamic) = image::load_from_memory(&data) {
                        let rgba = dynamic.to_rgba8();
                        let w = rgba.width();
                        let h = rgba.height();
                        self.browse_images.insert(
                            url,
                            mc_core::img::RgbaImage {
                                width: w,
                                height: h,
                                pixels: rgba.into_raw(),
                            },
                        );
                    }
                }
            }
            EngineEvent::VersionList { target, versions } => {
                self.show_version_picker(target, versions);
            }
            EngineEvent::WizardSearch(results) => {
                if let Some(mut wizard) = self.pending_wizard.take() {
                    wizard.results = results.hits;
                    wizard.selected = 0;
                    wizard.step = crate::wizard::WizardStep::ModrinthSearch;
                    self.overlay = Some(Overlay::Wizard(wizard));
                }
            }
            EngineEvent::WizardProject { project, versions } => {
                if let Some(mut wizard) = self.pending_wizard.take() {
                    wizard.project = Some(*project);
                    wizard.project_versions = versions;
                    wizard.selected = 0;
                    wizard.step = crate::wizard::WizardStep::ModrinthProject;
                    self.overlay = Some(Overlay::Wizard(wizard));
                }
            }
            EngineEvent::Project { project, versions } => {
                self.selected_project = Some(project);
                self.project_versions = versions;
                self.project_state
                    .select(if self.project_versions.is_empty() {
                        None
                    } else {
                        Some(0)
                    });
            }
            EngineEvent::ModsChanged => {
                self.reload_mods();
            }
            EngineEvent::InstalledMods(mods) => {
                self.installed_mods = mods;
                self.mods_scanning = false;
                if self.installed_mods.is_empty() {
                    self.mods_state.select(None);
                } else if self.mods_state.selected().is_none() {
                    self.mods_state.select(Some(0));
                }
            }
            EngineEvent::Crash(analysis) => {
                self.crash_analysis = analysis;
            }
            EngineEvent::DeviceCode(prompt) => {
                self.overlay = Some(Overlay::DeviceCode(prompt));
            }
            EngineEvent::Authenticated(account) => {
                self.overlay = None;
                let account = *account;
                let name = account.username.clone();
                tokio::spawn({
                    let paths = self.paths.clone();
                    let tx = self.engine_tx.clone();
                    async move {
                        if let Ok(mut store) = AccountStore::load(paths.accounts_file()).await {
                            let _ = store.upsert(account).await;
                        }
                        let _ = tx.send(EngineEvent::AccountsChanged);
                        let _ = tx.send(EngineEvent::Toast(format!("Signed in as {name}")));
                    }
                });
            }
            EngineEvent::Java(list) => {
                self.java_installations = list;
            }
            EngineEvent::Toast(message) => self.set_toast(message, false),
            EngineEvent::Error(message) => self.set_toast(message, true),
            EngineEvent::ResourcePacks(items) => {
                self.resource_packs = items;
                if self.resource_packs_state.selected().is_none() && !self.resource_packs.is_empty() {
                    self.resource_packs_state.select(Some(0));
                }
            }
            EngineEvent::ShaderPacks(items) => {
                self.shaders = items;
                if self.shaders_state.selected().is_none() && !self.shaders.is_empty() {
                    self.shaders_state.select(Some(0));
                }
            }
            EngineEvent::Worlds(items) => {
                self.worlds = items;
                if self.worlds_state.selected().is_none() && !self.worlds.is_empty() {
                    self.worlds_state.select(Some(0));
                }
            }
            EngineEvent::Screenshots(items) => {
                self.screenshots = items;
                if self.screenshots_state.selected().is_none() && !self.screenshots.is_empty() {
                    self.screenshots_state.select(Some(0));
                }
            }
            EngineEvent::LocalImage { path, img } => {
                if let Some(img) = img {
                    self.local_images.insert(path, img);
                }
            }
            EngineEvent::ExternalInstances(instances) => {
                self.external_instances = instances.clone();
                if instances.is_empty() {
                    self.external_state.select(None);
                    self.set_toast("No external instances found", true);
                } else {
                    self.external_state.select(Some(0));
                    let names: Vec<String> = instances.iter().map(|i| i.name.clone()).collect();
                    let picker = crate::forms::VersionPicker::new(
                        "Select Instance to Import",
                        names,
                        crate::forms::PickerTarget::ImportExternal,
                    );
                    self.overlay = Some(Overlay::Picker(picker));
                }
            }
        }
    }
}
