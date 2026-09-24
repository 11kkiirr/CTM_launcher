use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use mc_core::auth::microsoft::MicrosoftAuth;
use mc_core::auth::{Account, AccountKind, AccountStore};
use mc_core::install::Installer;
use mc_core::instance::{Instance, JvmConfig, LoaderType};
use mc_core::launch::{java, resolve_instance_version, select_java, Launcher};
use mc_core::logs;
use mc_core::modpack::ModpackInstaller;
use mc_core::modrinth::{self, InstalledMod, SearchHit, SearchResults};
use mc_core::skins::{SkinClient, SkinVariant};
use mc_core::util::{Progress, ProgressCallback};
use mc_core::CoreError;
use ratatui::layout::Rect;

use crate::engine::{progress_event, EngineEvent};
use crate::forms::{
    gc_options, parse_gc, ConfirmAction, Form, FormAction, Overlay, PickerTarget, TextAction,
};
use crate::i18n::Lang;
use crate::views::browse::{BrowseFocus, BrowseKind, SideFilter};

use super::{
    gc_index, rect_contains, split_args, App, CLIENT_ID, Focus, HitAction, Hitbox, Nav, Toast,
};

    // ---------------------------------------------------------------------
    // Actions: instances
    // ---------------------------------------------------------------------

async fn install_modrinth_file(
    client: &reqwest::Client,
    modrinth: &mc_core::modrinth::ModrinthClient,
    version: &mc_core::modrinth::Version,
    dest_dir: &std::path::Path,
    resolve_deps: bool,
    game_version: Option<&str>,
    loader: Option<&str>,
) -> Result<Vec<String>, CoreError> {
    let file = version
        .primary_file()
        .ok_or_else(|| CoreError::Modrinth("version has no downloadable file".into()))?;
    modrinth::install_version_file(client, file, dest_dir, None).await?;
    let mut installed = vec![file.filename.clone()];
    if resolve_deps {
        let deps = modrinth::resolve_dependencies(modrinth, version, game_version, loader, 3).await?;
        for dep in deps {
            if let Some(dep_file) = dep.primary_file() {
                if modrinth::install_version_file(client, dep_file, dest_dir, None)
                    .await
                    .is_ok()
                {
                    installed.push(dep_file.filename.clone());
                }
            }
        }
    }
    Ok(installed)
}

impl App {
pub fn selected_instance(&self) -> Option<&Instance> {
    self.instance_state
        .selected()
        .and_then(|idx| self.instances.get(idx))
}

pub(crate) fn reload_instances(&mut self) {
    self.groups = self.instance_manager.groups();
    let mut all = self.instance_manager.list().unwrap_or_default();
    self.group_counts = self.instance_manager.group_counts();
    // Stable section order: ungrouped first, then by group name, then by instance name.
    all.sort_by(|a, b| {
        let ga = a.metadata.group.as_str();
        let gb = b.metadata.group.as_str();
        ga.cmp(gb)
            .then_with(|| a.name().to_lowercase().cmp(&b.name().to_lowercase()))
    });
    self.instances = all;
    if !self.selected_group.is_empty() && !self.groups.contains(&self.selected_group) {
        self.selected_group.clear();
    }
    if self.instances.is_empty() {
        self.instance_state.select(None);
    } else if self.instance_state.selected().unwrap_or(0) >= self.instances.len() {
        self.instance_state.select(Some(self.instances.len() - 1));
    } else if self.instance_state.selected().is_none() {
        self.instance_state.select(Some(0));
    }
}

pub(crate) fn open_create_instance_form(&mut self) {
    self.open_create_wizard();
}

/// Create an instance and install its loader in the background.
pub(crate) fn create_instance_async(
    &mut self,
    name: String,
    game_version: String,
    loader: LoaderType,
    loader_version: Option<String>,
) {
    let manager = self.instance_manager.clone();
    let installer = Installer::new(
        self.client.clone(),
        self.paths.clone(),
        self.progress_callback(),
    );
    let tx = self.engine_tx.clone();
    let min = self.settings.default_min_memory_mb;
    let max = self.settings.default_max_memory_mb;
    let gc = self.settings.default_gc;
    self.progress = Some((None, format!("Creating {name}")));

    tokio::spawn(async move {
        let result: Result<Instance, CoreError> = async {
            let mut instance = manager
                .create(&name, &game_version, loader, loader_version.clone())
                .await?;
            instance.metadata.jvm.min_memory_mb = min;
            instance.metadata.jvm.max_memory_mb = max;
            instance.metadata.jvm.gc = gc;
            instance.save().await?;

            let _ = tx.send(EngineEvent::Status(format!(
                "Installing {} ...",
                instance.metadata.descriptor()
            )));
            installer
                .install_loader(&game_version, loader, loader_version.as_deref())
                .await?;
            Ok(instance)
        }
        .await;

        match result {
            Ok(instance) => {
                let _ = tx.send(EngineEvent::ProgressDone);
                let _ = tx.send(EngineEvent::InstancesChanged);
                let _ = tx.send(EngineEvent::Toast(format!("Created {}", instance.name())));
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::ProgressDone);
                let _ = tx.send(EngineEvent::Error(format!("Create failed: {err}")));
            }
        }
    });
}

/// Fetch the Mojang version list for a picker.
pub(crate) fn request_game_versions(&mut self) {
    self.fetch_game_versions(PickerTarget::WizardGame);
}

/// Open the version picker to change the selected build's game version.
pub(crate) fn open_change_version_picker(&mut self) {
    self.fetch_game_versions(PickerTarget::ChangeGameVersion);
}

fn fetch_game_versions(&mut self, target: PickerTarget) {
    let client = self.client.clone();
    let tx = self.engine_tx.clone();
    self.progress = Some((None, "Loading Minecraft versions...".into()));
    tokio::spawn(async move {
        let result = mc_core::install::common::fetch_manifest(&client).await;
        let _ = tx.send(EngineEvent::ProgressDone);
        match result {
            Ok(manifest) => {
                let versions = manifest.versions.into_iter().map(|v| v.id).collect();
                let _ = tx.send(EngineEvent::VersionList { target, versions });
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::Error(format!("Version list failed: {err}")));
            }
        }
    });
}

/// Fetch loader versions for the wizard's current loader/game version.
pub(crate) fn request_loader_versions(&mut self) {
    let Some(wizard) = self.pending_wizard.as_ref() else {
        return;
    };
    let loader = wizard.loader();
    let game = wizard.game_version.clone();
    let installer = Installer::new(
        self.client.clone(),
        self.paths.clone(),
        self.progress_callback(),
    );
    let tx = self.engine_tx.clone();
    self.progress = Some((None, "Loading loader versions...".into()));
    tokio::spawn(async move {
        let result: Result<Vec<String>, CoreError> = match loader {
            LoaderType::Fabric => {
                mc_core::install::fabric::available_loaders(&installer, &game).await
            }
            LoaderType::Quilt => {
                mc_core::install::quilt::available_loaders(&installer, &game).await
            }
            LoaderType::Forge => {
                mc_core::install::forge::available_loaders(&installer, &game).await
            }
            LoaderType::NeoForge => {
                mc_core::install::neoforge::available_loaders(&installer, &game).await
            }
            _ => Ok(Vec::new()),
        };
        let _ = tx.send(EngineEvent::ProgressDone);
        match result {
            Ok(versions) => {
                let _ = tx.send(EngineEvent::VersionList {
                    target: PickerTarget::WizardLoader,
                    versions,
                });
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::Error(format!("Loader list failed: {err}")));
            }
        }
    });
}

/// Change the selected build's game version and reinstall it.
pub(crate) fn change_instance_version(&mut self, game_version: String) {
    let Some(instance) = self.selected_instance().cloned() else {
        return;
    };
    let Ok(mut inst) = self.instance_manager.get(instance.id()) else {
        return;
    };
    inst.metadata.game_version = game_version.clone();
    // The previous loader build may not exist for the new version.
    inst.metadata.loader_version = None;
    let loader = inst.metadata.loader;
    let installer = Installer::new(
        self.client.clone(),
        self.paths.clone(),
        self.progress_callback(),
    );
    let tx = self.engine_tx.clone();
    self.progress = Some((None, format!("Reinstalling {} ...", inst.name())));
    tokio::spawn(async move {
        let result: Result<(), CoreError> = async {
            inst.save().await?;
            installer
                .install_loader(&game_version, loader, None)
                .await?;
            Ok(())
        }
        .await;
        let _ = tx.send(EngineEvent::ProgressDone);
        match result {
            Ok(()) => {
                let _ = tx.send(EngineEvent::InstancesChanged);
                let _ = tx.send(EngineEvent::Toast("Version changed".into()));
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::Error(format!("Change failed: {err}")));
            }
        }
    });
}

/// Install a Modrinth modpack project version by downloading its `.mrpack`.
pub(crate) fn install_modrinth_modpack(
    &mut self,
    name: String,
    version: mc_core::modrinth::Version,
) {
    let Some(file) = version.primary_file().cloned() else {
        self.set_toast(self.tr("toast.no_file"), true);
        return;
    };
    let installer = Installer::new(
        self.client.clone(),
        self.paths.clone(),
        self.progress_callback(),
    );
    let instances = self.instance_manager.clone();
    let paths = self.paths.clone();
    let progress = self.progress_callback();
    let client = self.client.clone();
    let tx = self.engine_tx.clone();
    self.progress = Some((None, format!("Downloading {name}...")));

    tokio::spawn(async move {
        let result: Result<Instance, CoreError> = async {
            let archive = paths.downloads_dir().join(&file.filename);
            mc_core::util::download_file(&client, &file.url, &archive, file.sha1(), None)
                .await?;
            let importer = ModpackInstaller::new(installer, instances, paths, progress);
            importer.import(&archive, Some(&name)).await
        }
        .await;
        let _ = tx.send(EngineEvent::ProgressDone);
        match result {
            Ok(instance) => {
                let _ = tx.send(EngineEvent::InstancesChanged);
                let _ = tx.send(EngineEvent::Toast(format!("Imported {}", instance.name())));
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::Error(format!("Import failed: {err}")));
            }
        }
    });
}

#[allow(dead_code)]
pub(crate) fn open_edit_instance_form(&mut self) {
    let Some(instance) = self.selected_instance().cloned() else {
        self.set_toast(self.tr("toast.no_instance"), true);
        return;
    };
    let jvm = &instance.metadata.jvm;
    let form = Form::new(
        format!("Edit {}", instance.name()),
        FormAction::EditInstanceSettings,
    )
    .push_number("Min RAM (MB)", jvm.min_memory_mb)
    .push_number("Max RAM (MB)", jvm.max_memory_mb)
    .push_choice("Garbage Collector", gc_options(), gc_index(jvm.gc))
    .push_text(
        "Java Path",
        jvm.java_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
    )
    .with_hint("blank = auto-detect")
    .push_bool("Fullscreen", jvm.fullscreen)
    .push_text("Custom JVM Args", jvm.custom_jvm_args.join(" "))
    .push_text("Extra Game Args", jvm.extra_game_args.join(" "));
    self.overlay = Some(Overlay::Form(form));
}

/// Prompt for a new display name for the selected build.
pub(crate) fn open_rename_instance_form(&mut self) {
    let Some(instance) = self.selected_instance().cloned() else {
        self.set_toast(self.tr("toast.no_instance"), true);
        return;
    };
    self.overlay = Some(Overlay::text_with(
        "Rename Build",
        "New name: ",
        instance.name().to_string(),
        TextAction::RenameInstance,
    ));
}

pub(crate) fn rename_instance(&mut self, new_name: String) {
    let Some(instance) = self.selected_instance().cloned() else {
        return;
    };
    let trimmed = new_name.trim().to_string();
    if trimmed.is_empty() {
        self.set_toast(self.tr("toast.name_empty"), true);
        return;
    }
    let manager = self.instance_manager.clone();
    let tx = self.engine_tx.clone();
    let id = instance.id().to_string();
    tokio::spawn(async move {
        match manager.rename(&id, &trimmed).await {
            Ok(_) => {
                let _ = tx.send(EngineEvent::InstancesChanged);
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::Error(format!("Rename failed: {err}")));
            }
        }
    });
}

/// Mark the active group section (for rename/delete + header highlight).
/// Does not filter the instance list — all builds stay visible.
pub(crate) fn set_selected_group(&mut self, group: String) {
    self.selected_group = group;
}

/// Collapse/expand a group panel by name (`""` = Ungrouped).
pub(crate) fn toggle_group_collapsed(&mut self, name: &str) {
    if !self.collapsed_groups.remove(name) {
        self.collapsed_groups.insert(name.to_string());
    }
}

/// Collapse/expand the panel that owns the current selection (or the
/// explicitly selected group header).
pub(crate) fn toggle_selected_group_collapsed(&mut self) {
    let name = if !self.selected_group.is_empty() {
        self.selected_group.clone()
    } else if let Some(idx) = self.instance_state.selected() {
        self.instances
            .get(idx)
            .map(|i| i.metadata.group.clone())
            .unwrap_or_default()
    } else {
        String::new()
    };
    // Ungrouped with no selection: nothing to toggle unless explicitly set.
    if name.is_empty() && self.selected_group.is_empty() {
        // Allow toggling Ungrouped when it is the active header.
        if self.instance_state.selected().is_none() {
            return;
        }
    }
    self.toggle_group_collapsed(&name);
}

/// Open a prompt to create a standalone group (does not move an instance).
pub(crate) fn open_new_group_form(&mut self) {
    self.pending_move_on_new_group = false;
    self.overlay = Some(Overlay::text(
        "New Group",
        "Group name: ",
        TextAction::NewGroup,
    ));
}

/// Create a named group in the registry.
pub(crate) fn create_group(&mut self, name: String) {
    let trimmed = name.trim().to_string();
    if trimmed.is_empty() {
        self.set_toast(self.tr("toast.group_empty"), true);
        return;
    }
    let manager = self.instance_manager.clone();
    let tx = self.engine_tx.clone();
    tokio::spawn(async move {
        match manager.create_group(&trimmed).await {
            Ok(()) => {
                let _ = tx.send(EngineEvent::InstancesChanged);
                let _ = tx.send(EngineEvent::Toast(format!("Created group: {trimmed}")));
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::Error(format!("Create group failed: {err}")));
            }
        }
    });
}

/// Prompt to rename the active group section (falls back to the selected
/// instance's group).
pub(crate) fn open_rename_group_form(&mut self) {
    let old = if !self.selected_group.is_empty() {
        self.selected_group.clone()
    } else {
        self.selected_instance()
            .map(|i| i.metadata.group.clone())
            .unwrap_or_default()
    };
    if old.is_empty() {
        self.set_toast(self.tr("toast.select_group"), true);
        return;
    }
    self.overlay = Some(Overlay::text_with(
        "Rename Group",
        "New name: ",
        old.clone(),
        TextAction::RenameGroup(old),
    ));
}

/// Rename a group across the registry and all member instances.
pub(crate) fn rename_group(&mut self, old: String, new: String) {
    let new = new.trim().to_string();
    if new.is_empty() {
        self.set_toast(self.tr("toast.group_empty"), true);
        return;
    }
    if new == old {
        return;
    }
    if self.groups.contains(&new) {
        self.set_toast(self.tr("toast.group_exists"), true);
        return;
    }
    let manager = self.instance_manager.clone();
    let tx = self.engine_tx.clone();
    let old_for_spawn = old.clone();
    let new_for_spawn = new.clone();
    if self.collapsed_groups.remove(&old) {
        self.collapsed_groups.insert(new.clone());
    }
    tokio::spawn(async move {
        match manager.rename_group(&old_for_spawn, &new_for_spawn).await {
            Ok(()) => {
                let _ = tx.send(EngineEvent::InstancesChanged);
                let _ = tx.send(EngineEvent::Toast(format!("Renamed to: {new_for_spawn}")));
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::Error(format!("Rename failed: {err}")));
            }
        }
    });
    // Optimistically follow the rename so the filter stays valid.
    if self.selected_group == old {
        self.selected_group = new;
    }
}

/// Confirm deleting the active group section (falls back to the selected
/// instance's group).
pub(crate) fn confirm_delete_group(&mut self) {
    let name = if !self.selected_group.is_empty() {
        self.selected_group.clone()
    } else {
        self.selected_instance()
            .map(|i| i.metadata.group.clone())
            .unwrap_or_default()
    };
    if name.is_empty() {
        self.set_toast(self.tr("toast.select_group"), true);
        return;
    }
    let count = self.group_counts.get(&name).copied().unwrap_or(0);
    let message = if count == 0 {
        format!("Delete empty group '{name}'?")
    } else {
        format!("Delete group '{name}'?\n{count} instance(s) will move to Ungrouped.")
    };
    self.overlay = Some(Overlay::confirm(
        "Delete Group",
        message,
        ConfirmAction::DeleteGroup(name),
    ));
}

pub(crate) fn open_group_picker(&mut self) {
    let Some(instance) = self.selected_instance().cloned() else {
        self.set_toast(self.tr("toast.no_instance"), true);
        return;
    };
    let mut items: Vec<String> = Vec::new();
    items.push("All".to_string());
    for g in &self.groups {
        items.push(g.clone());
    }
    items.push("[New Group]".to_string());

    let current_label = if instance.metadata.group.is_empty() {
        "All".to_string()
    } else {
        instance.metadata.group.clone()
    };
    let selected = items.iter().position(|i| *i == current_label).unwrap_or(0);

    let picker = crate::forms::VersionPicker::new(
        format!("Move '{}' to group", instance.name()),
        items,
        crate::forms::PickerTarget::MoveToGroup,
    );
    let mut picker = picker;
    picker.selected = selected;
    self.overlay = Some(Overlay::Picker(picker));
}

/// Apply a group-picker choice for the selected instance.
/// `"[New Group]"` opens a name prompt that creates + moves.
pub(crate) fn apply_group_picker(&mut self, group: &str) {
    if group == "[New Group]" {
        self.pending_move_on_new_group = true;
        self.overlay = Some(Overlay::text(
            "New Group",
            "Group name: ",
            TextAction::NewGroup,
        ));
        return;
    }
    self.pending_move_on_new_group = false;
    self.move_instance_to_group(group);
}

pub(crate) fn move_instance_to_group(&mut self, group: &str) {
    let Some(instance) = self.selected_instance().cloned() else {
        return;
    };
    let actual_group = if group == "All" {
        ""
    } else {
        group
    };
    let manager = self.instance_manager.clone();
    let tx = self.engine_tx.clone();
    let id = instance.id().to_string();
    let group_owned = actual_group.to_string();
    tokio::spawn(async move {
        match manager.set_group(&id, &group_owned).await {
            Ok(()) => {
                let _ = tx.send(EngineEvent::InstancesChanged);
                let label = if group_owned.is_empty() {
                    "All".to_string()
                } else {
                    group_owned
                };
                let _ = tx.send(EngineEvent::Toast(format!(
                    "Moved to group: {label}"
                )));
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::Error(format!("Move failed: {err}")));
            }
        }
    });
}

pub(crate) fn save_instance_settings_from_form(&mut self, form: &Form) {
    let Some(instance) = self.selected_instance().cloned() else {
        return;
    };
    let min = form.number_value("Min RAM (MB)").unwrap_or(512);
    let max = form.number_value("Max RAM (MB)").unwrap_or(4096);
    let gc = parse_gc(form.choice_value("Garbage Collector").unwrap_or("G1GC"));
    let java_path = form
        .text_value("Java Path")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let fullscreen = form.bool_value("Fullscreen").unwrap_or(false);
    let jvm_args = split_args(form.text_value("Custom JVM Args").unwrap_or(""));
    let game_args = split_args(form.text_value("Extra Game Args").unwrap_or(""));

    self.update_instance_jvm(instance.id(), |jvm| {
        jvm.min_memory_mb = min;
        jvm.max_memory_mb = max;
        jvm.gc = gc;
        jvm.java_path = java_path;
        jvm.fullscreen = fullscreen;
        jvm.custom_jvm_args = jvm_args;
        jvm.extra_game_args = game_args;
    });
}

pub(crate) fn update_instance_jvm<F>(&mut self, id: &str, mutate: F)
where
    F: FnOnce(&mut JvmConfig),
{
    let Ok(mut instance) = self.instance_manager.get(id) else {
        return;
    };
    mutate(&mut instance.metadata.jvm);
    let tx = self.engine_tx.clone();
    tokio::spawn(async move {
        if instance.save().await.is_ok() {
            let _ = tx.send(EngineEvent::InstancesChanged);
            let _ = tx.send(EngineEvent::Toast("Instance updated".into()));
        } else {
            let _ = tx.send(EngineEvent::Error("Failed to save instance".into()));
        }
    });
}

pub(crate) fn confirm_delete_instance(&mut self) {
    let Some(instance) = self.selected_instance().cloned() else {
        return;
    };
    let (title, message) = if instance.is_linked() {
        (
            "Unlink Instance",
            format!(
                "Remove '{}' from the library without deleting the original files?",
                instance.name()
            ),
        )
    } else {
        (
            "Delete Instance",
            format!(
                "Delete '{}' and all of its files? This cannot be undone.",
                instance.name()
            ),
        )
    };
    self.overlay = Some(Overlay::confirm(
        title,
        message,
        ConfirmAction::DeleteInstance(instance.id().to_string()),
    ));
}

pub(crate) fn install_selected_instance(&mut self) {
    let Some(instance) = self.selected_instance().cloned() else {
        self.set_toast(self.tr("toast.no_instance"), true);
        return;
    };
    let installer = Installer::new(
        self.client.clone(),
        self.paths.clone(),
        self.progress_callback(),
    );
    let tx = self.engine_tx.clone();
    self.progress = Some((None, format!("Installing {}", instance.name())));
    tokio::spawn(async move {
        let result = installer
            .install_loader(
                &instance.metadata.game_version,
                instance.metadata.loader,
                instance.metadata.loader_version.as_deref(),
            )
            .await;
        let _ = tx.send(EngineEvent::ProgressDone);
        match result {
            Ok(version) => {
                let _ = tx.send(EngineEvent::Toast(format!("Installed {}", version.id)));
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::Error(format!("Install failed: {err}")));
            }
        }
    });
}

pub(crate) fn launch_selected(&mut self) {
    let Some(instance) = self.selected_instance().cloned() else {
        self.set_toast(self.tr("toast.no_instance"), true);
        return;
    };
    let Some(account) = self.accounts.active().cloned() else {
        self.set_toast(self.tr("toast.no_account"), true);
        return;
    };
    if self.running.is_some() {
        self.set_toast(self.tr("toast.game_running"), true);
        return;
    }

    let client = self.client.clone();
    let paths = self.paths.clone();
    let tx = self.engine_tx.clone();
    let progress = self.progress_callback();
    self.progress = Some((None, format!("Preparing {}", instance.name())));

    tokio::spawn(async move {
        let result: Result<_, CoreError> = async {
            // Refresh an expired Microsoft token before launching.
            let account = if account.kind == AccountKind::Microsoft
                && !account.token_valid(120)
                && account.refresh_token.is_some()
            {
                let auth = MicrosoftAuth::new(client.clone());
                match auth.refresh_account(&account).await {
                    Ok(updated) => {
                        let _ = tx.send(EngineEvent::Authenticated(Box::new(updated.clone())));
                        updated
                    }
                    Err(_) => account,
                }
            } else {
                account
            };

            let installer = Installer::new(client.clone(), paths.clone(), progress.clone());
            let resolved = match resolve_instance_version(&paths, &instance).await {
                Ok(resolved) => resolved,
                Err(_) => {
                    let _ = tx.send(EngineEvent::Status(format!(
                        "Installing {} ...",
                        instance.metadata.descriptor()
                    )));
                    installer
                        .install_loader(
                            &instance.metadata.game_version,
                            instance.metadata.loader,
                            instance.metadata.loader_version.as_deref(),
                        )
                        .await?;
                    resolve_instance_version(&paths, &instance).await?
                }
            };
            let version_id = resolved.id.clone();
            let required = resolved.details.required_java_major();
            let component = resolved.details.java_component().map(str::to_string);
            let java = select_java(
                &client,
                &paths,
                &instance,
                component.as_deref(),
                required,
                Some(progress.clone()),
            )
            .await?;

            let launcher = Launcher::new(client.clone(), paths.clone(), progress);
            let plan = launcher
                .prepare(&instance, &account, &java, &version_id, CLIENT_ID)
                .await?;
            let (handle, logs) = launcher.start(&plan).await?;
            Ok((plan, version_id, handle, logs))
        }
        .await;

        match result {
            Ok((plan, version_id, handle, logs)) => {
                let _ = tx.send(EngineEvent::Started {
                    command: plan.display(),
                    version: version_id,
                    handle: Box::new(handle),
                    logs: Box::new(logs),
                });
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::ProgressDone);
                let _ = tx.send(EngineEvent::Error(format!("Launch failed: {err}")));
            }
        }
    });
}

// ---------------------------------------------------------------------
// Actions: modpacks
// ---------------------------------------------------------------------

pub(crate) fn open_search_prompt(&mut self) {
    self.overlay = Some(Overlay::text(
        "Search Modpacks",
        "Query: ",
        TextAction::SearchModrinth,
    ));
}

pub(crate) fn open_import_prompt(&mut self) {
    self.overlay = Some(Overlay::text(
        "Import .mrpack",
        "Archive path: ",
        TextAction::ImportPath,
    ));
}

/// Open a native file dialog to pick a `.mrpack` file.
///
/// Uses `zenity`/`kdialog` when available; falls back to the manual path
/// prompt otherwise.
pub(crate) fn browse_for_mrpack(&mut self) {
    let tx = self.engine_tx.clone();
    tokio::spawn(async move {
        let result = tokio::process::Command::new("zenity")
            .arg("--file-selection")
            .arg("--title=Select .mrpack file")
            .arg("--file-filter=*.mrpack")
            .output()
            .await;
        match result {
            Ok(output) if output.status.success() => {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let _ = tx.send(EngineEvent::ImportPathPicked(
                    (!path.is_empty()).then_some(path),
                ));
            }
            _ => {
                let _ = tx.send(EngineEvent::ImportPathPicked(None));
            }
        }
    });
}

pub(crate) fn run_search(&mut self, query: String) {
    self.search_query = query.clone();
    self.selected_project = None;
    let modrinth = self.modrinth.clone();
    let tx = self.engine_tx.clone();
    self.progress = Some((None, format!("Searching '{query}'...")));
    tokio::spawn(async move {
        let result = modrinth
            .search(&query, Some("modpack"), None, None, None, 30, 0, &[])
            .await;
        let _ = tx.send(EngineEvent::ProgressDone);
        match result {
            Ok(results) => {
                let _ = tx.send(EngineEvent::SearchResults(results));
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::Error(format!("Search failed: {err}")));
            }
        }
    });
}

pub(crate) fn open_project(&mut self, hit: &SearchHit) {
    let modrinth = self.modrinth.clone();
    let tx = self.engine_tx.clone();
    let id = hit.project_id.clone();
    self.progress = Some((None, "Loading project...".into()));
    tokio::spawn(async move {
        let result = async {
            let project = modrinth.project(&id).await?;
            let versions = modrinth.versions_filtered(&project.id, None, None).await?;
            Ok::<_, CoreError>((project, versions))
        }
        .await;
        let _ = tx.send(EngineEvent::ProgressDone);
        match result {
            Ok((project, versions)) => {
                let _ = tx.send(EngineEvent::Project {
                    project: Box::new(project),
                    versions,
                });
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::Error(format!("Load failed: {err}")));
            }
        }
    });
}

pub(crate) fn install_selected_project(&mut self) {
    let Some(project) = self.selected_project.clone() else {
        self.set_toast(self.tr("toast.open_project_first"), true);
        return;
    };
    let Some(instance) = self.selected_instance().cloned() else {
        self.set_toast(self.tr("toast.select_instance_first"), true);
        return;
    };
    let version = self
        .project_state
        .selected()
        .and_then(|idx| self.project_versions.get(idx))
        .cloned();
    let Some(version) = version else {
        self.set_toast(self.tr("toast.no_version"), true);
        return;
    };

    let client = self.client.clone();
    let tx = self.engine_tx.clone();
    let mods_dir = instance.mods_dir();
    let loader = instance.metadata.loader.as_str().to_string();
    let game_version = instance.metadata.game_version.clone();
    let modrinth = self.modrinth.clone();
    self.progress = Some((None, format!("Installing {}", version.name)));

    tokio::spawn(async move {
        let result = install_modrinth_file(
            &client,
            &modrinth,
            &version,
            &mods_dir,
            true,
            Some(&game_version),
            Some(&loader),
        )
        .await;

        let _ = tx.send(EngineEvent::ProgressDone);
        match result {
            Ok(files) => {
                let _ = tx.send(EngineEvent::ModsChanged);
                let _ = tx.send(EngineEvent::Toast(format!(
                    "Installed {} file(s) from {}",
                    files.len(),
                    project.title
                )));
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::Error(format!("Install failed: {err}")));
            }
        }
    });
}

pub(crate) fn import_modpack(&mut self, path: PathBuf) {
    if path.as_os_str().is_empty() {
        return;
    }
    let installer = Installer::new(
        self.client.clone(),
        self.paths.clone(),
        self.progress_callback(),
    );
    let instances = self.instance_manager.clone();
    let paths = self.paths.clone();
    let progress = self.progress_callback();
    let tx = self.engine_tx.clone();
    self.progress = Some((None, "Importing modpack...".into()));

    tokio::spawn(async move {
        let importer = ModpackInstaller::new(installer, instances, paths, progress);
        let result = importer.import(&path, None).await;
        let _ = tx.send(EngineEvent::ProgressDone);
        match result {
            Ok(instance) => {
                let _ = tx.send(EngineEvent::InstancesChanged);
                let _ = tx.send(EngineEvent::Toast(format!("Imported {}", instance.name())));
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::Error(format!("Import failed: {err}")));
            }
        }
    });
}

// ---------------------------------------------------------------------
// Actions: mods
// ---------------------------------------------------------------------

pub(crate) fn reload_mods(&mut self) {
    let Some(instance) = self.selected_instance().cloned() else {
        self.installed_mods.clear();
        self.mods_state.select(None);
        self.mods_scanning = false;
        return;
    };
    self.mods_scanning = true;
    let tx = self.engine_tx.clone();
    let mods_dir = instance.mods_dir();
    tokio::spawn(async move {
        match modrinth::scan_installed_mods(&mods_dir).await {
            Ok(mods) => {
                let _ = tx.send(EngineEvent::InstalledMods(mods));
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::Error(format!("Scan failed: {err}")));
            }
        }
    });
}

pub(crate) fn reload_resource_packs(&self) {
    let Some(instance) = self.selected_instance().cloned() else { return };
    let tx = self.engine_tx.clone();
    let dir = instance.resourcepacks_dir();
    tokio::spawn(async move {
        // Resource packs are commonly zips (or unpacked folders).
        let items = Self::scan_pack_names(&dir, true).await;
        let _ = tx.send(EngineEvent::ResourcePacks(items));
    });
}

pub(crate) fn reload_shaders(&self) {
    let Some(instance) = self.selected_instance().cloned() else { return };
    let tx = self.engine_tx.clone();
    let dir = instance.shaders_dir();
    tokio::spawn(async move {
        // Shader packs are commonly zips (or unpacked folders).
        let items = Self::scan_pack_names(&dir, true).await;
        let _ = tx.send(EngineEvent::ShaderPacks(items));
    });
}

pub(crate) fn reload_worlds(&self) {
    let Some(instance) = self.selected_instance().cloned() else { return };
    let tx = self.engine_tx.clone();
    let dir = instance.saves_dir();
    tokio::spawn(async move {
        let items = Self::scan_world_names(&dir).await;
        let _ = tx.send(EngineEvent::Worlds(items));
    });
}

pub(crate) fn reload_screenshots(&self) {
    let Some(instance) = self.selected_instance().cloned() else { return };
    let tx = self.engine_tx.clone();
    let dir = instance.game_dir().join("screenshots");
    tokio::spawn(async move {
        let items = Self::scan_screenshot_names(&dir).await;
        let _ = tx.send(EngineEvent::Screenshots(items));
    });
}

/// Scan a pack directory: directories always, plus `.zip` files when
/// `allow_zips` (resource packs). Ignores stray files like `pack.png`.
async fn scan_pack_names(dir: &std::path::Path, allow_zips: bool) -> Vec<String> {
    let mut items = Vec::new();
    if let Ok(mut entries) = tokio::fs::read_dir(dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let Ok(name) = entry.file_name().into_string() else { continue };
            let Ok(file_type) = entry.file_type().await else { continue };
            if file_type.is_dir() {
                items.push(name);
            } else if allow_zips {
                let lower = name.to_ascii_lowercase();
                if lower.ends_with(".zip") || lower.ends_with(".zip.disabled") {
                    items.push(name);
                }
            }
        }
    }
    items.sort();
    items
}

/// Screenshot list: only image files.
async fn scan_screenshot_names(dir: &std::path::Path) -> Vec<String> {
    let mut items = Vec::new();
    if let Ok(mut entries) = tokio::fs::read_dir(dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let Ok(name) = entry.file_name().into_string() else { continue };
            let lower = name.to_ascii_lowercase();
            if lower.ends_with(".png") || lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
                items.push(name);
            }
        }
    }
    items.sort();
    items
}

/// Worlds list: directories only (ignore stray files in `saves/`).
async fn scan_world_names(dir: &std::path::Path) -> Vec<String> {
    let mut items = Vec::new();
    if let Ok(mut entries) = tokio::fs::read_dir(dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let Ok(name) = entry.file_name().into_string() else { continue };
            let Ok(file_type) = entry.file_type().await else { continue };
            if file_type.is_dir() {
                items.push(name);
            }
        }
    }
    items.sort();
    items
}

/// Remove a file or (recursively) a directory.
async fn remove_path(path: &std::path::Path) -> std::io::Result<()> {
    let meta = tokio::fs::symlink_metadata(path).await?;
    if meta.is_dir() {
        tokio::fs::remove_dir_all(path).await
    } else {
        tokio::fs::remove_file(path).await
    }
}

// ---------------------------------------------------------------------
// Actions: Modrinth browser
// ---------------------------------------------------------------------

/// Navigate to the browser, loading the first page of popular projects for
/// the active content type when nothing is loaded yet. Also refreshes the
/// installed list for the current kind so Browse can show Installed state.
pub(crate) fn open_browse(&mut self) {
    match self.browse.kind {
        BrowseKind::Mods => self.reload_mods(),
        BrowseKind::ResourcePacks => self.reload_resource_packs(),
        BrowseKind::Shaders => self.reload_shaders(),
    }
    if self.browse.results.is_empty() && !self.browse.loading {
        self.browse_load_first_page();
    }
}

pub(crate) fn run_browse_search(&mut self, query: String) {
    self.browse.query = query.trim().to_string();
    self.browse_close_detail();
    self.browse_load_first_page();
}

pub(crate) fn browse_load_first_page(&mut self) {
    self.browse.results.clear();
    self.browse.selected = 0;
    self.browse.offset = 0;
    self.browse.total = 0;
    self.browse_do_search(0);
}

pub(crate) fn browse_next_page(&mut self) {
    if self.browse.loading {
        return;
    }
    let next = self.browse.offset + 30;
    if next >= self.browse.total && self.browse.total > 0 {
        return;
    }
    self.browse.offset = next;
    self.browse.selected = 0;
    self.browse.results.clear();
    self.browse_do_search(next);
}

pub(crate) fn browse_prev_page(&mut self) {
    if self.browse.loading {
        return;
    }
    if self.browse.offset == 0 {
        return;
    }
    let prev = self.browse.offset.saturating_sub(30);
    self.browse.offset = prev;
    self.browse.selected = 0;
    self.browse.results.clear();
    self.browse_do_search(prev);
}

fn browse_do_search(&mut self, offset: u32) {
    let kind = self.browse.kind;
    let query = self.browse.query.clone();
    let sort = self.browse.sort;
    let filter_compat = self.browse.filter_compat;
    let filter_side = self.browse.filter_side;
    let filter_categories = self.browse.filter_categories.clone();
    let filter_loaders = self.browse.filter_loaders.clone();
    let game_version = if filter_compat {
        self.selected_instance()
            .map(|i| i.metadata.game_version.clone())
    } else {
        None
    };
    let loader = if filter_compat {
        self.selected_instance()
            .map(|i| i.metadata.loader.as_str().to_string())
    } else {
        None
    };
    let modrinth = self.modrinth.clone();
    let tx = self.engine_tx.clone();
    self.browse.loading = true;
    let label = if query.is_empty() {
        format!("Loading popular {}...", kind.label())
    } else {
        format!("Searching '{}'...", query)
    };
    self.progress = Some((None, label));
    tokio::spawn(async move {
        let mut categories = filter_categories;
        // Add selected loaders as category facets (Modrinth uses categories for loader filtering).
        for loader in &filter_loaders {
            categories.push(loader.clone());
        }
        match filter_side {
            SideFilter::Client => categories.push("client-side".to_string()),
            SideFilter::Server => categories.push("server-side".to_string()),
            SideFilter::All => {}
        }
        let gv = game_version.as_deref();
        let ld = loader.as_deref();
        let result = modrinth
            .search(
                &query,
                Some(kind.project_type()),
                gv,
                ld,
                Some(sort.as_str()),
                30,
                offset,
                &categories,
            )
            .await;
        let _ = tx.send(EngineEvent::ProgressDone);
        match result {
            Ok(results) => {
                let _ = tx.send(EngineEvent::BrowseResults(results));
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::BrowseResults(SearchResults {
                    hits: Vec::new(),
                    offset,
                    limit: 30,
                    total_hits: 0,
                }));
                let _ = tx.send(EngineEvent::Error(format!("Search failed: {err}")));
            }
        }
    });
}

pub(crate) fn browse_open_selected(&mut self) {
    let Some(hit) = self.browse.results.get(self.browse.selected).cloned() else {
        return;
    };
    self.browse_open_project(&hit.project_id);
}

fn browse_open_project(&mut self, id: &str) {
    let id = id.to_string();
    let modrinth = self.modrinth.clone();
    let tx = self.engine_tx.clone();
    self.progress = Some((None, "Loading project...".into()));
    tokio::spawn(async move {
        let result = async {
            let project = modrinth.project(&id).await?;
            let versions = modrinth.versions_filtered(&project.id, None, None).await?;
            Ok::<_, CoreError>((project, versions))
        }
        .await;
        let _ = tx.send(EngineEvent::ProgressDone);
        match result {
            Ok((project, versions)) => {
                let _ = tx.send(EngineEvent::BrowseProject {
                    project: Box::new(project),
                    versions,
                });
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::Error(format!("Load failed: {err}")));
            }
        }
    });
}

pub(crate) fn browse_close_detail(&mut self) {
    self.browse.detail = None;
    self.browse.versions.clear();
    self.browse.version_selected = 0;
    self.browse.focus = BrowseFocus::List;
    self.browse.body.clear();
    self.browse.body_for = String::new();
    self.browse.body_scroll = 0;
}

/// Kick off fetches for the project icon and a couple of gallery images.
pub(crate) fn browse_fetch_images(&mut self) {
    let Some(project) = self.browse.detail.clone() else {
        return;
    };
    let mut urls: Vec<String> = Vec::new();
    if let Some(url) = project.icon_url {
        urls.push(url);
    }
    for image in project.gallery.iter().take(2) {
        urls.push(image.url.clone());
    }
    for url in urls {
        self.browse_fetch_image(url.as_str());
    }
}

pub(crate) fn browse_fetch_image(&mut self, url: &str) {
    let key = url.to_string();
    if self.browse_images.contains_key(&key) {
        return;
    }
    if !self.browse.image_requested.insert(key.clone()) {
        return;
    }
    let modrinth = self.modrinth.clone();
    let tx = self.engine_tx.clone();
    tokio::spawn(async move {
        let data = match modrinth.get_bytes(&key, 6 * 1024 * 1024).await {
            Ok(bytes) => Some(bytes),
            Err(_) => None,
        };
        let _ = tx.send(EngineEvent::BrowseImage {
            url: key,
            data,
        });
    });
}

/// Maximum edge length for cached local thumbnails. Screenshot cards are
/// ~32 cells wide, so 512px is plenty and keeps 1080p shots from hogging RAM.
const THUMB_MAX: u32 = 512;

/// Load a local image file into `local_images` if not already cached.
///
/// Called every render frame per visible card — the requested set guards
/// against re-spawning a decode task while one is already in flight, and the
/// decode runs on the blocking pool downscaling to [`THUMB_MAX`] so full-res
/// screenshots never sit in the UI thread's cache.
pub(crate) fn load_local_image(&mut self, path: &str) {
    if self.local_images.contains_key(path) {
        return;
    }
    if !self.local_image_requested.insert(path.to_string()) {
        return;
    }
    let key = path.to_string();
    let path_buf = PathBuf::from(path);
    let tx = self.engine_tx.clone();
    tokio::task::spawn_blocking(move || {
        let img = std::fs::read(&path_buf).ok().and_then(|data| {
            mc_core::img::decode_image(&data).ok().or_else(|| {
                image::load_from_memory(&data).ok().map(|dynamic| {
                    let rgba = dynamic.to_rgba8();
                    mc_core::img::RgbaImage {
                        width: rgba.width(),
                        height: rgba.height(),
                        pixels: rgba.into_raw(),
                    }
                })
            })
        });
        let img = img.map(|img| mc_core::img::fit_image(&img, Self::THUMB_MAX, Self::THUMB_MAX));
        let _ = tx.send(EngineEvent::LocalImage { path: key, img });
    });
}

/// Install the selected version: modpacks import a new build, everything
/// else installs a file into the selected instance's folder.
pub(crate) fn browse_install(&mut self) {
    let Some(project) = self.browse.detail.clone() else {
        self.set_toast(self.tr("toast.open_project_first"), true);
        return;
    };
    let Some(version) = self
        .browse
        .versions
        .get(self.browse.version_selected)
        .cloned()
    else {
        self.set_toast(self.tr("toast.no_version"), true);
        return;
    };

    let Some(instance) = self.selected_instance().cloned() else {
        self.set_toast(self.tr("toast.select_instance_first"), true);
        return;
    };
    let dest_dir = match self.browse.kind {
        crate::views::browse::BrowseKind::Mods => instance.mods_dir(),
        crate::views::browse::BrowseKind::ResourcePacks => instance.resourcepacks_dir(),
        crate::views::browse::BrowseKind::Shaders => instance.shaders_dir(),
    };
    let reload_evt = match self.browse.kind {
        BrowseKind::Mods => EngineEvent::ModsChanged,
        BrowseKind::ResourcePacks => EngineEvent::ReloadList(Nav::ResourcePacks),
        BrowseKind::Shaders => EngineEvent::ReloadList(Nav::Shaders),
    };
    let game_version = instance.metadata.game_version.clone();
    let loader = instance.metadata.loader.as_str().to_string();
    let resolve_deps = true;

    let client = self.client.clone();
    let modrinth = self.modrinth.clone();
    let tx = self.engine_tx.clone();
    let title = project.title.clone();
    self.progress = Some((None, format!("Installing {}", version.name)));
    tokio::spawn(async move {
        let result = install_modrinth_file(
            &client,
            &modrinth,
            &version,
            &dest_dir,
            resolve_deps,
            Some(&game_version),
            Some(&loader),
        )
        .await;
        let _ = tx.send(EngineEvent::ProgressDone);
        match result {
            Ok(files) => {
                let _ = tx.send(reload_evt);
                let _ = tx.send(EngineEvent::Toast(format!(
                    "Installed {} file(s) from {}",
                    files.len(),
                    title
                )));
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::Error(format!("Install failed: {err}")));
            }
        }
    });
}

/// Quick-install the latest compatible release for a mod directly from the
/// browse list (without opening the detail view).
pub(crate) fn browse_quick_install(&mut self, idx: usize) {
    let Some(hit) = self.browse.results.get(idx).cloned() else {
        return;
    };
    let Some(instance) = self.selected_instance().cloned() else {
        self.set_toast(self.tr("toast.select_instance"), true);
        return;
    };
    let dest_dir = match self.browse.kind {
        crate::views::browse::BrowseKind::Mods => instance.mods_dir(),
        crate::views::browse::BrowseKind::ResourcePacks => instance.resourcepacks_dir(),
        crate::views::browse::BrowseKind::Shaders => instance.shaders_dir(),
    };
    let reload_evt = match self.browse.kind {
        BrowseKind::Mods => EngineEvent::ModsChanged,
        BrowseKind::ResourcePacks => EngineEvent::ReloadList(Nav::ResourcePacks),
        BrowseKind::Shaders => EngineEvent::ReloadList(Nav::Shaders),
    };
    let game_version = instance.metadata.game_version.clone();
    let loader = instance.metadata.loader.as_str().to_string();
    let compat = matches!(self.browse.kind, BrowseKind::Mods);
    let client = self.client.clone();
    let modrinth = self.modrinth.clone();
    let tx = self.engine_tx.clone();
    let title = hit.title.clone();
    let project_id = hit.project_id.clone();
    self.progress = Some((None, format!("Installing {}...", title)));
    tokio::spawn(async move {
        let result = async {
            let gv = if compat { Some(&*game_version) } else { None };
            let ld = if compat { Some(&*loader) } else { None };
            let Some(version) = modrinth
                .latest_version(&project_id, gv, ld)
                .await?
            else {
                return Err(CoreError::Modrinth(
                    "no compatible version found".into(),
                ));
            };
            install_modrinth_file(
                &client,
                &modrinth,
                &version,
                &dest_dir,
                true,
                gv,
                ld,
            )
            .await
        }
        .await;
        let _ = tx.send(EngineEvent::ProgressDone);
        match result {
            Ok(files) => {
                let _ = tx.send(reload_evt);
                let _ = tx.send(EngineEvent::Toast(format!(
                    "Installed {} file(s) from {}",
                    files.len(),
                    title
                )));
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::Error(format!("Install failed: {err}")));
            }
        }
    });
}

/// Handle a click on a sidebar filter item.
pub(crate) fn browse_filter_click(&mut self, item: crate::views::browse::FilterItem) {
    use crate::views::browse::FilterItem;
    match item {
        FilterItem::Sort => {
            self.browse.sort = self.browse.sort.cycle();
            self.browse_load_first_page();
        }
        FilterItem::Side => {
            self.browse.filter_side = self.browse.filter_side.cycle();
            self.browse_load_first_page();
        }
        FilterItem::Compat => {
            self.browse.filter_compat = !self.browse.filter_compat;
            self.browse_load_first_page();
        }
        FilterItem::Loader(idx) => {
            let loaders = self.browse.kind.loaders();
            if let Some(loader) = loaders.get(idx) {
                let loader = loader.to_string();
                if let Some(pos) = self.browse.filter_loaders.iter().position(|l| *l == loader) {
                    self.browse.filter_loaders.remove(pos);
                } else {
                    self.browse.filter_loaders.push(loader);
                }
                self.browse_load_first_page();
            }
        }
        FilterItem::Category(idx) => {
            let cats = self.browse.kind.categories();
            if let Some(cat) = cats.get(idx) {
                let cat = cat.to_string();
                if let Some(pos) = self.browse.filter_categories.iter().position(|c| *c == cat) {
                    self.browse.filter_categories.remove(pos);
                } else {
                    self.browse.filter_categories.push(cat);
                }
                self.browse_load_first_page();
            }
        }
    }
}

pub(crate) fn toggle_selected_mod(&mut self) {
    let Some(idx) = self.mods_state.selected() else {
        return;
    };
    let Some(module) = self.installed_mods.get(idx).cloned() else {
        return;
    };
    // Optimistic local flip so the toggle is instant; the async reload
    // confirms it once the rename completes.
    if let Some(current) = self.installed_mods.get_mut(idx) {
        current.enabled = !current.enabled;
    }
    let tx = self.engine_tx.clone();
    tokio::spawn(async move {
        match modrinth::toggle_mod(&module.path).await {
            Ok(_) => {
                let _ = tx.send(EngineEvent::ModsChanged);
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::Error(format!("Toggle failed: {err}")));
            }
        }
    });
}

pub(crate) fn confirm_delete_mod(&mut self) {
    let Some(idx) = self.mods_state.selected() else {
        return;
    };
    let Some(module) = self.installed_mods.get(idx).cloned() else {
        return;
    };
    self.overlay = Some(Overlay::confirm(
        "Delete Mod",
        format!("Delete '{}'?", module.file_name),
        ConfirmAction::DeleteMod(module.path),
    ));
}

/// Delete the selected entry in the current list view.
pub(crate) fn confirm_delete_selected(&mut self) {
    match self.nav {
        Nav::Worlds => self.confirm_delete_world(),
        Nav::Screenshots => self.confirm_delete_screenshot(),
        Nav::ResourcePacks => self.confirm_delete_resource_pack(),
        Nav::Shaders => self.confirm_delete_shader(),
        Nav::Mods => self.confirm_delete_mod(),
        _ => {}
    }
}

/// Toggle the selected entry in views that support enable/disable.
pub(crate) fn toggle_selected_entry(&mut self) {
    match self.nav {
        Nav::Mods => self.toggle_selected_mod(),
        Nav::ResourcePacks => self.toggle_selected_resource_pack(),
        Nav::Shaders => self.toggle_selected_shader(),
        _ => {}
    }
}

fn confirm_delete_world(&mut self) {
    let Some(idx) = self.worlds_state.selected() else { return };
    let Some(name) = self.worlds.get(idx).cloned() else { return };
    let Some(instance) = self.selected_instance().cloned() else { return };
    let path = instance.saves_dir().join(&name);
    self.overlay = Some(Overlay::confirm(
        "Delete World",
        format!("Delete world '{name}' and all of its saves? This cannot be undone."),
        ConfirmAction::DeleteWorld(path),
    ));
}

fn confirm_delete_screenshot(&mut self) {
    let Some(idx) = self.screenshots_state.selected() else { return };
    let Some(name) = self.screenshots.get(idx).cloned() else { return };
    let Some(instance) = self.selected_instance().cloned() else { return };
    let path = instance.game_dir().join("screenshots").join(&name);
    self.overlay = Some(Overlay::confirm(
        "Delete Screenshot",
        format!("Delete '{name}'?"),
        ConfirmAction::DeleteScreenshot(path),
    ));
}

fn confirm_delete_resource_pack(&mut self) {
    let Some(idx) = self.resource_packs_state.selected() else { return };
    let Some(name) = self.resource_packs.get(idx).cloned() else { return };
    let Some(instance) = self.selected_instance().cloned() else { return };
    let path = instance.resourcepacks_dir().join(&name);
    self.overlay = Some(Overlay::confirm(
        "Delete Resource Pack",
        format!("Delete resource pack '{name}'?"),
        ConfirmAction::DeleteResourcePack(path),
    ));
}

fn confirm_delete_shader(&mut self) {
    let Some(idx) = self.shaders_state.selected() else { return };
    let Some(name) = self.shaders.get(idx).cloned() else { return };
    let Some(instance) = self.selected_instance().cloned() else { return };
    let path = instance.shaders_dir().join(&name);
    self.overlay = Some(Overlay::confirm(
        "Delete Shader Pack",
        format!("Delete shader pack '{name}'?"),
        ConfirmAction::DeleteShader(path),
    ));
}

fn toggle_selected_resource_pack(&mut self) {
    let Some(idx) = self.resource_packs_state.selected() else { return };
    let Some(name) = self.resource_packs.get(idx).cloned() else { return };
    let Some(instance) = self.selected_instance().cloned() else { return };
    let path = instance.resourcepacks_dir().join(&name);
    let tx = self.engine_tx.clone();
    tokio::spawn(async move {
        match mc_core::modrinth::toggle_mod(&path).await {
            Ok(_) => {
                let _ = tx.send(EngineEvent::ReloadList(Nav::ResourcePacks));
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::Error(format!("Toggle failed: {err}")));
            }
        }
    });
}

fn toggle_selected_shader(&mut self) {
    let Some(idx) = self.shaders_state.selected() else { return };
    let Some(name) = self.shaders.get(idx).cloned() else { return };
    let Some(instance) = self.selected_instance().cloned() else { return };
    let path = instance.shaders_dir().join(&name);
    let tx = self.engine_tx.clone();
    tokio::spawn(async move {
        match mc_core::modrinth::toggle_mod(&path).await {
            Ok(_) => {
                let _ = tx.send(EngineEvent::ReloadList(Nav::Shaders));
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::Error(format!("Toggle failed: {err}")));
            }
        }
    });
}

pub(crate) fn check_mod_updates(&mut self) {
    let Some(instance) = self.selected_instance().cloned() else {
        self.set_toast(self.tr("toast.select_instance"), true);
        return;
    };
    if self.installed_mods.is_empty() {
        self.set_toast(self.tr("toast.no_updates_needed"), true);
        return;
    }
    let modrinth = self.modrinth.clone();
    let tx = self.engine_tx.clone();
    let mods: Vec<InstalledMod> = self.installed_mods.clone();
    let game_version = instance.metadata.game_version.clone();
    let loader = instance.metadata.loader.as_str().to_string();
    self.progress = Some((None, "Checking for updates...".into()));

    tokio::spawn(async move {
        let mut updates = Vec::new();
        for module in mods {
            // SHA-1 is computed lazily here so the initial mod scan stays fast.
            let sha1 = match modrinth::update_check_sha1(&module).await {
                Ok(hash) => hash,
                Err(_) => continue,
            };
            if sha1.is_empty() {
                continue;
            }
            if let Ok(Some((installed, latest))) = modrinth
                .check_update(&sha1, Some(&game_version), Some(&loader))
                .await
            {
                updates.push(format!(
                    "{}: {} -> {}",
                    module.file_name, installed.version_number, latest.version_number
                ));
            }
        }
        let _ = tx.send(EngineEvent::ProgressDone);
        if updates.is_empty() {
            let _ = tx.send(EngineEvent::Toast("All mods are up to date".into()));
        } else {
            let _ = tx.send(EngineEvent::Message(updates));
        }
    });
}

// ---------------------------------------------------------------------
// Actions: accounts
// ---------------------------------------------------------------------

pub(crate) fn reload_accounts(&mut self) {
    let path = self.paths.accounts_file();
    let tx = self.engine_tx.clone();
    tokio::spawn(async move {
        if let Ok(store) = AccountStore::load(path).await {
            let _ = tx.send(EngineEvent::AccountsReloaded(store));
        }
    });
}

pub(crate) fn open_offline_login(&mut self) {
    let form = Form::new("Offline Account", FormAction::OfflineLogin)
        .push_text("Username", "Player")
        .with_hint("3-16 chars, A-Z 0-9 _");
    self.overlay = Some(Overlay::Form(form));
}

pub(crate) fn offline_login(&mut self, name: &str) {
    if !mc_core::auth::offline::valid_username(name) {
        self.set_toast(self.tr("toast.invalid_username"), true);
        return;
    }
    let account = Account::offline(name);
    let path = self.paths.accounts_file();
    let tx = self.engine_tx.clone();
    let name = name.to_string();
    tokio::spawn(async move {
        match AccountStore::load(&path).await {
            Ok(mut store) => {
                if store.upsert(account).await.is_ok() {
                    let _ = tx.send(EngineEvent::AccountsChanged);
                    let _ =
                        tx.send(EngineEvent::Toast(format!("Added offline account {name}")));
                } else {
                    let _ = tx.send(EngineEvent::Error("Failed to save account".into()));
                }
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::Error(format!("Account store error: {err}")));
            }
        }
    });
}

pub(crate) fn start_microsoft_login(&mut self) {
    let auth = MicrosoftAuth::new(self.client.clone());
    let tx = self.engine_tx.clone();
    self.progress = Some((None, "Requesting device code...".into()));
    tokio::spawn(async move {
        let prompt = match auth.request_device_code().await {
            Ok(prompt) => prompt,
            Err(err) => {
                let _ = tx.send(EngineEvent::ProgressDone);
                let _ = tx.send(EngineEvent::Error(format!("Device code failed: {err}")));
                return;
            }
        };
        let _ = tx.send(EngineEvent::ProgressDone);
        let _ = tx.send(EngineEvent::DeviceCode(Box::new(prompt.clone())));

        let _ = tx.send(EngineEvent::Status(
            "Completing Xbox/Minecraft login...".into(),
        ));
        match auth.login_device_code(&prompt).await {
            Ok(account) => {
                let _ = tx.send(EngineEvent::Authenticated(Box::new(account)));
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::Error(format!("Login failed: {err}")));
            }
        }
    });
}

pub(crate) fn set_active_account(&mut self) {
    let Some(idx) = self.account_state.selected() else {
        return;
    };
    let Some(account) = self.accounts.accounts().get(idx).cloned() else {
        return;
    };
    let path = self.paths.accounts_file();
    let tx = self.engine_tx.clone();
    let name = account.username.clone();
    tokio::spawn(async move {
        if let Ok(mut store) = AccountStore::load(&path).await {
            let _ = store.set_active(&account.id).await;
            let _ = tx.send(EngineEvent::AccountsChanged);
            let _ = tx.send(EngineEvent::Toast(format!("Active account: {name}")));
        }
    });
}

pub(crate) fn confirm_delete_account(&mut self) {
    let Some(idx) = self.account_state.selected() else {
        return;
    };
    let Some(account) = self.accounts.accounts().get(idx).cloned() else {
        return;
    };
    self.overlay = Some(Overlay::confirm(
        "Remove Account",
        format!("Remove account '{}'?", account.username),
        ConfirmAction::DeleteAccount(account.id),
    ));
}

pub(crate) fn open_skin_prompt(&mut self) {
    self.overlay = Some(Overlay::text(
        "Change Skin",
        "Image URL or local .png path: ",
        TextAction::SkinUrl,
    ));
}

pub(crate) fn change_skin(&mut self, input: String) {
    let input = input.trim().to_string();
    if input.is_empty() {
        return;
    }
    let Some(account) = self.accounts.active().cloned() else {
        self.set_toast(self.tr("toast.no_active_account"), true);
        return;
    };
    let Some(token) = account.access_token.clone() else {
        self.set_toast(self.tr("toast.skin_ms_only"), true);
        return;
    };
    let client = self.client.clone();
    let tx = self.engine_tx.clone();
    self.progress = Some((None, "Updating skin...".into()));

    tokio::spawn(async move {
        let skin = SkinClient::new(client);
        let result = if input.starts_with("http://") || input.starts_with("https://") {
            skin.set_skin_url(&token, &input, SkinVariant::Classic)
                .await
        } else {
            let filename = std::path::Path::new(&input)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("skin.png")
                .to_string();
            match tokio::fs::read(&input).await {
                Ok(bytes) => {
                    skin.upload_skin(&token, bytes, &filename, SkinVariant::Classic)
                        .await
                }
                Err(err) => Err(CoreError::Skin(format!("cannot read file: {err}"))),
            }
        };
        let _ = tx.send(EngineEvent::ProgressDone);
        match result {
            Ok(()) => {
                let _ = tx.send(EngineEvent::AccountsChanged);
                let _ = tx.send(EngineEvent::Toast("Skin updated".into()));
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::Error(format!("Skin update failed: {err}")));
            }
        }
    });
}

// ---------------------------------------------------------------------
// Actions: logs & settings
// ---------------------------------------------------------------------

pub(crate) fn load_latest_log(&mut self) {
    let Some(instance) = self.selected_instance().cloned() else {
        return;
    };
    let path = instance.logs_dir().join("latest.log");
    let tx = self.engine_tx.clone();
    tokio::spawn(async move {
        if let Ok(contents) = tokio::fs::read_to_string(&path).await {
            let lines: Vec<String> = contents
                .lines()
                .rev()
                .take(2000)
                .map(str::to_string)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let _ = tx.send(EngineEvent::LogLines(lines));
        }
    });
}

pub(crate) fn analyze_crash(&mut self) {
    if self.crash_analysis.is_some() {
        if let Some(analysis) = self.crash_analysis.clone() {
            let mut lines = vec![format!("Headline: {}", analysis.headline), String::new()];
            for cause in &analysis.causes {
                lines.push(format!("• {cause}"));
            }
            for rec in &analysis.recommendations {
                lines.push(format!("→ {rec}"));
            }
            if let Some(mismatch) = &analysis.java_mismatch {
                lines.push(format!("Java: {}", mismatch.detail));
            }
            if !analysis.missing_dependencies.is_empty() {
                lines.push(format!(
                    "Missing: {}",
                    analysis.missing_dependencies.join(", ")
                ));
            }
            self.overlay = Some(Overlay::message("Crash Analysis", lines));
        }
        return;
    }
    let Some(instance) = self.selected_instance().cloned() else {
        return;
    };
    let tx = self.engine_tx.clone();
    tokio::spawn(async move {
        let result = tokio::task::spawn_blocking(move || logs::analyze_latest(&instance))
            .await
            .ok()
            .and_then(|r| r.ok())
            .flatten();
        let _ = tx.send(EngineEvent::Crash(result));
    });
}

pub(crate) fn save_settings(&mut self) {
    self.save_settings_async();
    self.set_toast(self.tr("toast.settings_saved"), false);
}

/// Begin inline text/number editing for the selected launcher settings row.
pub(crate) fn begin_settings_edit(&mut self) {
    let field = self.settings_field;
    let buffer = match field {
        0 => self
            .settings
            .java_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        1 => self.settings.default_min_memory_mb.to_string(),
        2 => self.settings.default_max_memory_mb.to_string(),
        _ => return,
    };
    self.settings_edit = Some((field, buffer));
}

/// Mouse click helper: start editing whichever text/slider row is selected.
pub(crate) fn begin_settings_row_edit(&mut self) {
    if self.nav == Nav::Jvm {
        self.begin_instance_settings_edit();
    } else {
        self.begin_settings_edit();
    }
}

/// Mouse click helper: toggle the bool under the selected row.
pub(crate) fn toggle_selected_bool_row(&mut self) {
    if self.nav == Nav::Jvm {
        self.toggle_instance_setting(self.settings_field);
    } else {
        self.toggle_setting(self.settings_field);
    }
}

/// Commit an inline edit of a launcher settings text/number field.
pub(crate) fn commit_launcher_setting(&mut self, field: usize, buffer: &str) {
    use crate::views::settings_ui::{is_ram_over_half, max_ram_range, min_ram_range, snap_ram, RAM_STEP};
    match field {
        0 => {
            let path = buffer.trim();
            self.settings.java_path = if path.is_empty() {
                None
            } else {
                Some(PathBuf::from(path))
            };
        }
        1 => {
            if let Ok(v) = buffer.trim().parse::<u32>() {
                let (lo, hi) = min_ram_range(self.settings.default_max_memory_mb);
                let v = snap_ram(v, lo, hi);
                self.settings.default_min_memory_mb = v;
                if is_ram_over_half(v) {
                    self.warn_ram_over_half(v);
                }
            }
        }
        2 => {
            if let Ok(v) = buffer.trim().parse::<u32>() {
                let (lo, hi) = max_ram_range(self.settings.default_min_memory_mb);
                let v = snap_ram(v, lo, hi);
                self.settings.default_max_memory_mb = v;
                if is_ram_over_half(v) {
                    self.warn_ram_over_half(v);
                }
            }
            // Keep min ≤ max − step after max edits.
            let max = self.settings.default_max_memory_mb;
            let (lo, _) = min_ram_range(max);
            if self.settings.default_min_memory_mb > max.saturating_sub(RAM_STEP) {
                self.settings.default_min_memory_mb = lo;
            }
        }
        _ => return,
    }
    self.save_settings_async();
}

/// ←/→ adjust the selected launcher settings row without opening a form.
pub(crate) fn settings_adjust(&mut self, forward: bool) {
    use crate::views::settings_ui::{is_ram_over_half, max_ram_range, min_ram_range, snap_ram, RAM_STEP};
    match self.settings_field {
        1 => {
            let (lo, hi) = min_ram_range(self.settings.default_max_memory_mb);
            let cur = self.settings.default_min_memory_mb.clamp(lo, hi);
            let next = if forward {
                snap_ram(cur.saturating_add(RAM_STEP), lo, hi)
            } else {
                snap_ram(cur.saturating_sub(RAM_STEP), lo, hi)
            };
            self.settings.default_min_memory_mb = next;
            self.save_settings_async();
            if is_ram_over_half(next) {
                self.warn_ram_over_half(next);
            }
        }
        2 => {
            let (lo, hi) = max_ram_range(self.settings.default_min_memory_mb);
            let cur = self.settings.default_max_memory_mb.clamp(lo, hi);
            let next = if forward {
                snap_ram(cur.saturating_add(RAM_STEP), lo, hi)
            } else {
                snap_ram(cur.saturating_sub(RAM_STEP), lo, hi)
            };
            self.settings.default_max_memory_mb = next;
            self.save_settings_async();
            if is_ram_over_half(next) {
                self.warn_ram_over_half(next);
            }
        }
        3 => self.cycle_setting_gc(forward),
        7 => self.cycle_ascii_bg_anchor(forward),
        8 => self.cycle_setting_language(forward),
        _ => {}
    }
}

/// Begin inline editing for the selected instance JVM settings row.
pub(crate) fn begin_instance_settings_edit(&mut self) {
    let Some(instance) = self.selected_instance().cloned() else {
        return;
    };
    let jvm = &instance.metadata.jvm;
    let field = self.settings_field;
    let buffer = match field {
        0 => jvm.min_memory_mb.to_string(),
        1 => jvm.max_memory_mb.to_string(),
        3 => jvm
            .java_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        5 => jvm.custom_jvm_args.join(" "),
        6 => jvm.extra_game_args.join(" "),
        _ => return,
    };
    self.settings_edit = Some((field, buffer));
}

/// Commit an inline edit of an instance JVM settings field.
pub(crate) fn commit_instance_setting(&mut self, field: usize, buffer: &str) {
    use crate::views::settings_ui::{
        is_ram_over_half, max_ram_range, min_ram_range, snap_ram, RAM_STEP,
    };
    let Some(instance) = self.selected_instance().cloned() else {
        return;
    };
    let id = instance.id().to_string();
    match field {
        0 => {
            let raw: u32 = buffer.trim().parse().unwrap_or(512);
            let (lo, hi) = min_ram_range(instance.metadata.jvm.max_memory_mb);
            let v = snap_ram(raw, lo, hi);
            self.update_instance_jvm(&id, |jvm| jvm.min_memory_mb = v);
            if is_ram_over_half(v) {
                self.warn_ram_over_half(v);
            }
        }
        1 => {
            let raw: u32 = buffer.trim().parse().unwrap_or(4096);
            let (lo, hi) = max_ram_range(instance.metadata.jvm.min_memory_mb);
            let v = snap_ram(raw, lo, hi);
            let min = instance.metadata.jvm.min_memory_mb;
            self.update_instance_jvm(&id, |jvm| {
                jvm.max_memory_mb = v;
                if jvm.min_memory_mb > v.saturating_sub(RAM_STEP) {
                    jvm.min_memory_mb = min_ram_range(v).0.min(min).max(512);
                }
            });
            if is_ram_over_half(v) {
                self.warn_ram_over_half(v);
            }
        }
        3 => {
            let path = buffer.trim();
            let p = if path.is_empty() {
                None
            } else {
                Some(PathBuf::from(path))
            };
            self.update_instance_jvm(&id, |jvm| jvm.java_path = p);
        }
        5 => {
            let args = split_args(buffer);
            self.update_instance_jvm(&id, |jvm| jvm.custom_jvm_args = args);
        }
        6 => {
            let args = split_args(buffer);
            self.update_instance_jvm(&id, |jvm| jvm.extra_game_args = args);
        }
        _ => {}
    }
}

/// ←/→ adjust the selected instance JVM settings row.
pub(crate) fn instance_settings_adjust(&mut self, forward: bool) {
    use crate::views::settings_ui::{
        is_ram_over_half, max_ram_range, min_ram_range, snap_ram, RAM_STEP,
    };
    let Some(instance) = self.selected_instance().cloned() else {
        return;
    };
    let id = instance.id().to_string();
    match self.settings_field {
        0 => {
            let (lo, hi) = min_ram_range(instance.metadata.jvm.max_memory_mb);
            let cur = instance.metadata.jvm.min_memory_mb.clamp(lo, hi);
            let next = if forward {
                snap_ram(cur.saturating_add(RAM_STEP), lo, hi)
            } else {
                snap_ram(cur.saturating_sub(RAM_STEP), lo, hi)
            };
            self.update_instance_jvm(&id, |jvm| jvm.min_memory_mb = next);
            if is_ram_over_half(next) {
                self.warn_ram_over_half(next);
            }
        }
        1 => {
            let (lo, hi) = max_ram_range(instance.metadata.jvm.min_memory_mb);
            let cur = instance.metadata.jvm.max_memory_mb.clamp(lo, hi);
            let next = if forward {
                snap_ram(cur.saturating_add(RAM_STEP), lo, hi)
            } else {
                snap_ram(cur.saturating_sub(RAM_STEP), lo, hi)
            };
            self.update_instance_jvm(&id, |jvm| jvm.max_memory_mb = next);
            if is_ram_over_half(next) {
                self.warn_ram_over_half(next);
            }
        }
        2 => {
            let options = mc_core::instance::GcPreset::all();
            let current = options
                .iter()
                .position(|g| *g == instance.metadata.jvm.gc)
                .unwrap_or(0);
            let next = if forward {
                (current + 1) % options.len()
            } else {
                (current + options.len() - 1) % options.len()
            };
            let gc = options[next];
            self.update_instance_jvm(&id, |jvm| jvm.gc = gc);
        }
        4 => {
            let v = !instance.metadata.jvm.fullscreen;
            self.update_instance_jvm(&id, |jvm| jvm.fullscreen = v);
        }
        _ => {}
    }
}

/// Space toggle for the selected instance JVM settings row.
pub(crate) fn toggle_instance_setting(&mut self, field: usize) {
    if field != 4 {
        return;
    }
    if let Some(instance) = self.selected_instance().cloned() {
        let id = instance.id().to_string();
        let v = !instance.metadata.jvm.fullscreen;
        self.update_instance_jvm(&id, |jvm| jvm.fullscreen = v);
    }
}

/// Persist instance JVM settings immediately (Save button).
pub(crate) fn save_instance_settings_now(&mut self) {
    // update_instance_jvm already persists; just toast.
    self.set_toast(self.tr("toast.settings_saved"), false);
}

#[allow(dead_code)]
pub(crate) fn open_settings_form(&mut self) {
    use crate::settings::AsciiBgAnchor;
    let s = &self.settings;
    let lang = s.language;
    let anchor_index = AsciiBgAnchor::ALL
        .iter()
        .position(|a| *a == s.ascii_bg_anchor)
        .unwrap_or(0);
    let anchor_options: Vec<String> = AsciiBgAnchor::ALL
        .iter()
        .map(|a| a.label_lang(lang).to_string())
        .collect();
    let lang_index = Lang::ALL
        .iter()
        .position(|l| *l == s.language)
        .unwrap_or(0);
    let lang_options: Vec<String> = Lang::ALL
        .iter()
        .map(|l| l.native_label().to_string())
        .collect();
    let tr = |k: &str| crate::i18n::tr(lang, k).to_string();
    let form = Form::new(tr("settings.title"), FormAction::EditLauncherSettings)
        .push_text(
            tr("settings.java_path"),
            s.java_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
        )
        .with_hint(tr("settings.blank_auto"))
        .push_number(
            format!("{} ({})", tr("settings.min_ram"), tr("settings.mb")),
            s.default_min_memory_mb,
        )
        .push_number(
            format!("{} ({})", tr("settings.max_ram"), tr("settings.mb")),
            s.default_max_memory_mb,
        )
        .push_choice(tr("settings.gc"), gc_options(), gc_index(s.default_gc))
        .push_bool(tr("settings.show_progress"), s.show_progress)
        .push_bool(tr("settings.confirm_quit"), s.confirm_quit)
        .push_bool(tr("settings.auto_scroll"), s.log_auto_scroll)
        .push_choice(tr("settings.ascii_anchor"), anchor_options, anchor_index)
        .push_choice(tr("settings.language"), lang_options, lang_index);
    self.overlay = Some(Overlay::Form(form));
}

pub(crate) fn apply_launcher_settings_form(&mut self, form: &Form) {
    use crate::settings::AsciiBgAnchor;
    // Field order must match `open_settings_form`.
    let f = &form.fields;
    if f.len() >= 9 {
        let path = f[0].value.trim();
        self.settings.java_path = if path.is_empty() {
            None
        } else {
            Some(PathBuf::from(path))
        };
        self.settings.default_min_memory_mb =
            f[1].value.trim().parse().unwrap_or(512);
        self.settings.default_max_memory_mb =
            f[2].value.trim().parse().unwrap_or(4096);
        if let crate::forms::FieldKind::Choice { selected, .. } = f[3].kind {
            let opts = mc_core::instance::GcPreset::all();
            if let Some(gc) = opts.get(selected) {
                self.settings.default_gc = *gc;
            }
        }
        if let crate::forms::FieldKind::Bool(v) = f[4].kind {
            self.settings.show_progress = v;
        }
        if let crate::forms::FieldKind::Bool(v) = f[5].kind {
            self.settings.confirm_quit = v;
        }
        if let crate::forms::FieldKind::Bool(v) = f[6].kind {
            self.settings.log_auto_scroll = v;
        }
        if let Some(a) = AsciiBgAnchor::from_any_label(&f[7].value) {
            self.settings.ascii_bg_anchor = a;
        }
        if let Some(l) = Lang::from_native_label(&f[8].value) {
            self.settings.language = l;
        }
    }
    self.save_settings_async();
    self.set_toast(self.tr("toast.settings_saved"), false);
}

/// Toggle a boolean launcher setting by field index.
pub(crate) fn toggle_setting(&mut self, field: usize) {
    match field {
        4 => self.settings.show_progress = !self.settings.show_progress,
        5 => self.settings.confirm_quit = !self.settings.confirm_quit,
        6 => self.settings.log_auto_scroll = !self.settings.log_auto_scroll,
        _ => return,
    }
    self.save_settings_async();
}

/// Cycle the default GC preset.
pub(crate) fn cycle_setting_gc(&mut self, forward: bool) {
    let options = mc_core::instance::GcPreset::all();
    let current = options
        .iter()
        .position(|g| *g == self.settings.default_gc)
        .unwrap_or(0);
    let next = if forward {
        (current + 1) % options.len()
    } else {
        (current + options.len() - 1) % options.len()
    };
    self.settings.default_gc = options[next];
    self.save_settings_async();
}

pub(crate) fn cycle_ascii_bg_anchor(&mut self, forward: bool) {
    use crate::settings::AsciiBgAnchor;
    let options = AsciiBgAnchor::ALL;
    let current = options
        .iter()
        .position(|a| *a == self.settings.ascii_bg_anchor)
        .unwrap_or(0);
    let next = if forward {
        (current + 1) % options.len()
    } else {
        (current + options.len() - 1) % options.len()
    };
    self.settings.ascii_bg_anchor = options[next];
    self.save_settings_async();
}

pub(crate) fn cycle_setting_language(&mut self, forward: bool) {
    self.settings.language = self.settings.language.cycle(forward);
    self.save_settings_async();
}

pub(crate) fn save_settings_async(&self) {
    let settings = self.settings.clone();
    let paths = self.paths.clone();
    tokio::spawn(async move {
        let _ = settings.save(&paths).await;
    });
}

/// Select a settings field (commits any open edit, closes dropdown).
pub(crate) fn settings_select_field(&mut self, idx: usize) {
    if let Some((field, buffer)) = self.settings_edit.take() {
        let buf = buffer;
        if self.nav == Nav::Jvm {
            self.commit_instance_setting(field, &buf);
        } else {
            self.commit_launcher_setting(field, &buf);
        }
    }
    self.settings_dropdown = None;
    self.settings_field = idx;
    self.focus = Focus::Content;
}

/// Open the choice dropdown for the selected field (if it is a choice).
pub(crate) fn settings_open_dropdown(&mut self) {
    let field = self.settings_field;
    if self.nav == Nav::Jvm {
        if field != 2 {
            return;
        }
        let cur = self
            .selected_instance()
            .map(|i| i.metadata.jvm.gc)
            .unwrap_or(mc_core::instance::GcPreset::G1);
        let opts = mc_core::instance::GcPreset::all();
        let sel = opts.iter().position(|g| *g == cur).unwrap_or(0);
        self.settings_dropdown = Some((field, sel));
    } else {
        let sel = match field {
            3 => self.gc_dropdown_index(),
            7 => self.anchor_dropdown_index(),
            8 => self.lang_dropdown_index(),
            _ => return,
        };
        self.settings_dropdown = Some((field, sel));
    }
}

fn gc_dropdown_index(&self) -> usize {
    mc_core::instance::GcPreset::all()
        .iter()
        .position(|g| *g == self.settings.default_gc)
        .unwrap_or(0)
}

fn anchor_dropdown_index(&self) -> usize {
    crate::settings::AsciiBgAnchor::ALL
        .iter()
        .position(|a| *a == self.settings.ascii_bg_anchor)
        .unwrap_or(0)
}

fn lang_dropdown_index(&self) -> usize {
    Lang::ALL
        .iter()
        .position(|l| *l == self.settings.language)
        .unwrap_or(0)
}

/// Move the highlight inside an open dropdown.
pub(crate) fn settings_dropdown_move(&mut self, delta: i32) {
    let Some((field, sel)) = self.settings_dropdown else {
        return;
    };
    let len = self.settings_dropdown_options(field).len();
    if len == 0 {
        return;
    }
    let next = (sel as i32 + delta).rem_euclid(len as i32) as usize;
    self.settings_dropdown = Some((field, next));
}

/// Apply option `opt` for `field` and close the dropdown.
pub(crate) fn settings_pick_option(&mut self, field: usize, opt: usize) {
    self.settings_dropdown = None;
    if self.nav == Nav::Jvm {
        let opts = mc_core::instance::GcPreset::all();
        if field == 2 && opt < opts.len() {
            let gc = opts[opt];
            if let Some(instance) = self.selected_instance().cloned() {
                let id = instance.id().to_string();
                self.update_instance_jvm(&id, |jvm| jvm.gc = gc);
            }
        }
        return;
    }
    match field {
        3 => {
            let opts = mc_core::instance::GcPreset::all();
            if opt < opts.len() {
                self.settings.default_gc = opts[opt];
                self.save_settings_async();
            }
        }
        7 => {
            let opts = crate::settings::AsciiBgAnchor::ALL;
            if opt < opts.len() {
                self.settings.ascii_bg_anchor = opts[opt];
                self.save_settings_async();
            }
        }
        8 => {
            let opts = Lang::ALL;
            if opt < opts.len() {
                self.settings.language = opts[opt];
                self.save_settings_async();
            }
        }
        _ => {}
    }
}

/// Labels for the dropdown attached to `field`.
pub(crate) fn settings_dropdown_options(&self, field: usize) -> Vec<String> {
    if self.nav == Nav::Jvm {
        if field == 2 {
            return mc_core::instance::GcPreset::all()
                .iter()
                .map(|g| g.label().to_string())
                .collect();
        }
        return Vec::new();
    }
    match field {
        3 => mc_core::instance::GcPreset::all()
            .iter()
            .map(|g| g.label().to_string())
            .collect(),
        7 => crate::settings::AsciiBgAnchor::ALL
            .iter()
            .map(|a| a.label_lang(self.settings.language))
            .collect(),
        8 => Lang::ALL.iter().map(|l| l.native_label().to_string()).collect(),
        _ => Vec::new(),
    }
}

/// Close an open dropdown without applying.
pub(crate) fn settings_close_dropdown(&mut self) {
    self.settings_dropdown = None;
}

/// Set a slider value for field `field` (RAM snapped to grid, min≤max−step).
pub(crate) fn settings_set_slider(&mut self, field: usize, value: u32) {
    use crate::views::settings_ui::{
        is_ram_over_half, max_ram_range, min_ram_range, snap_ram, RAM_STEP,
    };
    if self.nav == Nav::Jvm {
        if let Some(instance) = self.selected_instance().cloned() {
            let id = instance.id().to_string();
            match field {
                0 => {
                    let (lo, hi) = min_ram_range(instance.metadata.jvm.max_memory_mb);
                    let v = snap_ram(value, lo, hi);
                    self.update_instance_jvm(&id, |j| j.min_memory_mb = v);
                    if is_ram_over_half(v) {
                        self.warn_ram_over_half(v);
                    }
                }
                1 => {
                    let (lo, hi) = max_ram_range(instance.metadata.jvm.min_memory_mb);
                    let v = snap_ram(value, lo, hi);
                    let min = instance.metadata.jvm.min_memory_mb;
                    self.update_instance_jvm(&id, |j| {
                        j.max_memory_mb = v;
                        if j.min_memory_mb > v.saturating_sub(RAM_STEP) {
                            j.min_memory_mb = min_ram_range(v).0.min(min).max(512);
                        }
                    });
                    if is_ram_over_half(v) {
                        self.warn_ram_over_half(v);
                    }
                }
                _ => {}
            }
        }
        return;
    }
    match field {
        1 => {
            let (lo, hi) = min_ram_range(self.settings.default_max_memory_mb);
            let v = snap_ram(value, lo, hi);
            self.settings.default_min_memory_mb = v;
            self.save_settings_async();
            if is_ram_over_half(v) {
                self.warn_ram_over_half(v);
            }
        }
        2 => {
            let (lo, hi) = max_ram_range(self.settings.default_min_memory_mb);
            let v = snap_ram(value, lo, hi);
            self.settings.default_max_memory_mb = v;
            let max = self.settings.default_max_memory_mb;
            if self.settings.default_min_memory_mb > max.saturating_sub(RAM_STEP) {
                self.settings.default_min_memory_mb = min_ram_range(max).0;
            }
            self.save_settings_async();
            if is_ram_over_half(v) {
                self.warn_ram_over_half(v);
            }
        }
        _ => {}
    }
}

/// Toast: selected RAM is over ~60% of physical memory.
fn warn_ram_over_half(&mut self, mb: u32) {
    use crate::views::settings_ui::{ram_warn_threshold, system_ram_mb};
    let msg = crate::i18n::tr_string(self.lang(), "toast.ram_over_half")
        .replacen("{}", &mb.to_string(), 1)
        .replacen("{}", &system_ram_mb().to_string(), 1)
        .replacen("{}", &ram_warn_threshold().to_string(), 1);
    self.set_toast(msg, true);
}

pub(crate) fn spawn_java_discovery(&self) {
    let tx = self.engine_tx.clone();
    tokio::spawn(async move {
        let list = java::discover().await;
        let _ = tx.send(EngineEvent::Java(list));
    });
}

/// Launch the platform file manager for `dir`.
fn open_dir_in_fm(dir: &std::path::Path) {
    #[cfg(target_os = "linux")]
    let _ = tokio::process::Command::new("xdg-open").arg(dir).spawn();
    #[cfg(target_os = "macos")]
    let _ = tokio::process::Command::new("open").arg(dir).spawn();
    #[cfg(target_os = "windows")]
    let _ = tokio::process::Command::new("explorer").arg(dir).spawn();
}

/// Open the folder backing the current view in the system file manager.
///
/// Worlds → `saves/`, Screenshots → `screenshots/`, ResourcePacks/Shaders/Mods
/// → their respective game subdirectories. Creates the directory first so
/// empty instances still open something useful.
pub(crate) fn open_current_folder(&self) {
    let Some(instance) = self.selected_instance().cloned() else {
        let tx = self.engine_tx.clone();
        let _ = tx.send(EngineEvent::Error("No instance selected".into()));
        return;
    };
    let dir = match self.nav {
        Nav::Worlds => instance.saves_dir(),
        Nav::Screenshots => instance.game_dir().join("screenshots"),
        Nav::ResourcePacks => instance.resourcepacks_dir(),
        Nav::Shaders => instance.shaders_dir(),
        Nav::Mods => instance.mods_dir(),
        _ => instance.game_dir(),
    };
    let tx = self.engine_tx.clone();
    tokio::spawn(async move {
        let _ = tokio::fs::create_dir_all(&dir).await;
        Self::open_dir_in_fm(&dir);
        let _ = tx.send(EngineEvent::Toast(format!("Opened {}", dir.display())));
    });
}

/// Open the config directory and ensure `ascii_bg.txt` exists inside it.
pub(crate) fn open_ascii_bg_folder(&self) {
    let dir = self.paths.config_dir.clone();
    let file = self.paths.ascii_bg_file();
    let tx = self.engine_tx.clone();
    tokio::spawn(async move {
        let _ = tokio::fs::create_dir_all(&dir).await;
        if !file.exists() {
            let sample = concat!(
                "    /\\_/\\\n",
                "   ( o.o )\n",
                "    > ^ <\n",
            );
            let _ = tokio::fs::write(&file, sample).await;
        }
        Self::open_dir_in_fm(&dir);
        let _ = tx.send(EngineEvent::Toast(format!(
            "Opened {}", dir.display()
        )));
    });
}

pub(crate) fn confirm(&mut self, action: ConfirmAction) {
    match action {
        ConfirmAction::DeleteInstance(id) => {
            let manager = self.instance_manager.clone();
            let tx = self.engine_tx.clone();
            tokio::spawn(async move {
                match manager.delete(&id).await {
                    Ok(()) => {
                        let _ = tx.send(EngineEvent::InstancesChanged);
                        let _ = tx.send(EngineEvent::Toast("Instance deleted".into()));
                    }
                    Err(err) => {
                        let _ = tx.send(EngineEvent::Error(format!("Delete failed: {err}")));
                    }
                }
            });
        }
        ConfirmAction::DeleteMod(path) => {
            let tx = self.engine_tx.clone();
            tokio::spawn(async move {
                match modrinth::remove_mod(&path).await {
                    Ok(()) => {
                        let _ = tx.send(EngineEvent::ModsChanged);
                        let _ = tx.send(EngineEvent::Toast("Mod deleted".into()));
                    }
                    Err(err) => {
                        let _ = tx.send(EngineEvent::Error(format!("Delete failed: {err}")));
                    }
                }
            });
        }
        ConfirmAction::DeleteAccount(id) => {
            let path = self.paths.accounts_file();
            let tx = self.engine_tx.clone();
            tokio::spawn(async move {
                if let Ok(mut store) = AccountStore::load(&path).await {
                    let _ = store.remove(&id).await;
                    let _ = tx.send(EngineEvent::AccountsChanged);
                }
            });
        }
        ConfirmAction::DeleteWorld(path) => {
            let icon_key = path.join("icon.png").to_string_lossy().to_string();
            self.local_images.remove(&icon_key);
            self.local_protocols.remove(&icon_key);
            let tx = self.engine_tx.clone();
            tokio::spawn(async move {
                match Self::remove_path(&path).await {
                    Ok(()) => {
                        let _ = tx.send(EngineEvent::ReloadList(Nav::Worlds));
                        let _ = tx.send(EngineEvent::Toast("World deleted".into()));
                    }
                    Err(err) => {
                        let _ = tx.send(EngineEvent::Error(format!("Delete failed: {err}")));
                    }
                }
            });
        }
        ConfirmAction::DeleteScreenshot(path) => {
            let key = path.to_string_lossy().to_string();
            self.local_images.remove(&key);
            self.local_protocols.remove(&key);
            self.local_image_requested.remove(&key);
            let tx = self.engine_tx.clone();
            tokio::spawn(async move {
                match Self::remove_path(&path).await {
                    Ok(()) => {
                        let _ = tx.send(EngineEvent::ReloadList(Nav::Screenshots));
                        let _ = tx.send(EngineEvent::Toast("Screenshot deleted".into()));
                    }
                    Err(err) => {
                        let _ = tx.send(EngineEvent::Error(format!("Delete failed: {err}")));
                    }
                }
            });
        }
        ConfirmAction::DeleteResourcePack(path) => {
            let tx = self.engine_tx.clone();
            tokio::spawn(async move {
                match Self::remove_path(&path).await {
                    Ok(()) => {
                        let _ = tx.send(EngineEvent::ReloadList(Nav::ResourcePacks));
                        let _ = tx.send(EngineEvent::Toast("Resource pack deleted".into()));
                    }
                    Err(err) => {
                        let _ = tx.send(EngineEvent::Error(format!("Delete failed: {err}")));
                    }
                }
            });
        }
        ConfirmAction::DeleteShader(path) => {
            let tx = self.engine_tx.clone();
            tokio::spawn(async move {
                match Self::remove_path(&path).await {
                    Ok(()) => {
                        let _ = tx.send(EngineEvent::ReloadList(Nav::Shaders));
                        let _ = tx.send(EngineEvent::Toast("Shader pack deleted".into()));
                    }
                    Err(err) => {
                        let _ = tx.send(EngineEvent::Error(format!("Delete failed: {err}")));
                    }
                }
            });
        }
        ConfirmAction::DeleteGroup(name) => {
            let manager = self.instance_manager.clone();
            let tx = self.engine_tx.clone();
            if self.selected_group == name {
                self.selected_group.clear();
            }
            self.collapsed_groups.remove(&name);
            tokio::spawn(async move {
                match manager.delete_group(&name).await {
                    Ok(()) => {
                        let _ = tx.send(EngineEvent::InstancesChanged);
                        let _ = tx.send(EngineEvent::Toast(format!("Deleted group: {name}")));
                    }
                    Err(err) => {
                        let _ = tx.send(EngineEvent::Error(format!("Delete failed: {err}")));
                    }
                }
            });
        }
        ConfirmAction::Quit => self.should_quit = true,
        ConfirmAction::None => {}
    }
}

pub(crate) fn request_quit(&mut self) {
    if self.settings.confirm_quit {
        self.overlay = Some(Overlay::confirm(
            "Quit",
            "Quit CTMLauncher?",
            ConfirmAction::Quit,
        ));
    } else {
        self.should_quit = true;
    }
}

pub(crate) fn show_help(&mut self) {
    let lines = vec![
        "Navigation (right panel)".to_string(),
        "  ↑↓ / jk      move through the navigation menu".to_string(),
        "  1-9          Instances/Mods/Packs/Shaders/Worlds/Screenshots/Versions/JVM/Logs".to_string(),
        "  F2 / F3 / F4 Accounts / Launcher Settings / Modpacks".to_string(),
        "  Esc          back to the Instances page".to_string(),
        "  Tab          toggle panel/content focus".to_string(),
        "  Enter        primary action".to_string(),
        "  q / Ctrl-C   quit".to_string(),
        String::new(),
        "Instances (main area + toolbar)".to_string(),
        "  arrows/hjkl  move between build cards (section-aware)".to_string(),
        "  Enter        launch the selected build".to_string(),
        "  n new · e edit · i install · p import · v versions · g move group".to_string(),
        "  c collapse/expand group panel · click header to toggle".to_string(),
        "  y new group · Y rename group · D delete group · r rename · d delete".to_string(),
        String::new(),
        "Mods / Modpacks / Versions / JVM".to_string(),
        "  Mods: t pane · Space toggle · s search · u updates · d delete".to_string(),
        "  Modpacks: / search · Enter open · i install · m import .mrpack".to_string(),
        "  Versions: c change version · r reinstall".to_string(),
        String::new(),
        "Logs".to_string(),
        "  j/k or wheel scroll · PgUp/PgDn page · g/G top/bottom".to_string(),
        "  f follow · p pause · c clear · / filter · a analyze crash · l level".to_string(),
        String::new(),
        "Mouse: hover highlights; click cards, nav blocks, lists and buttons.".to_string(),
    ];
    self.overlay = Some(Overlay::message("Help", lines));
}

// ---------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------

pub(crate) fn progress_callback(&self) -> ProgressCallback {
    let tx = self.engine_tx.clone();
    Arc::new(move |progress: Progress| {
        let _ = tx.send(progress_event(progress));
    }) as ProgressCallback
}

pub(crate) fn set_toast(&mut self, message: impl Into<String>, error: bool) {
    self.toast = Some(Toast {
        message: message.into(),
        error,
        at: Instant::now(),
    });
}

pub(crate) fn push_hitbox(&mut self, rect: Rect, action: HitAction) {
    if rect.width > 0 && rect.height > 0 {
        self.hitboxes.push(Hitbox { rect, action });
    }
}

/// Whether the mouse currently hovers `rect`.
pub(crate) fn is_hovered(&self, rect: Rect) -> bool {
    self.mouse_pos
        .map(|pos| rect_contains(rect, pos))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------
// Actions: external launcher import
// ---------------------------------------------------------------------

/// Scan for Modrinth App instances in the default location.
pub(crate) fn scan_modrinth_app_instances(&mut self) {
    let tx = self.engine_tx.clone();
    self.progress = Some((None, "Scanning Modrinth App...".into()));
    tokio::spawn(async move {
        let result = async {
            let dir = mc_core::import::modrinth_app::modrinth_app_dir()
                .ok_or_else(|| CoreError::other("Modrinth App not found"))?;
            let profiles = dir.join("profiles");
            let instances = if dir.join("app.db").exists() {
                mc_core::import::modrinth_app::list_instances(&dir)?
            } else {
                mc_core::import::modrinth_app::list_from_profile_json(&profiles)?
            };
            Ok::<_, CoreError>(instances)
        }
        .await;
        let _ = tx.send(EngineEvent::ProgressDone);
        match result {
            Ok(instances) => {
                let _ = tx.send(EngineEvent::ExternalInstances(instances));
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::Error(format!("Scan failed: {err}")));
            }
        }
    });
}

/// Import an external instance by scanning from a user-provided path.
pub(crate) fn scan_from_path(&mut self, path: String) {
    let path = std::path::PathBuf::from(path.trim());
    if !path.exists() {
        self.set_toast(self.tr("toast.path_missing"), true);
        return;
    }
    let tx = self.engine_tx.clone();
    self.progress = Some((None, "Scanning directory...".into()));
    tokio::spawn(async move {
        let result = async {
            let detected = mc_core::import::detect::detect_metadata(&path);
            let launcher = mc_core::import::detect_launcher(&path);
            let name = detected
                .name
                .unwrap_or_else(|| path.file_name().unwrap_or_default().to_string_lossy().to_string());
            let game_version = detected.game_version.unwrap_or_else(|| "1.20.1".to_string());
            let loader = detected.loader.unwrap_or(mc_core::instance::LoaderType::Vanilla);
            Ok::<_, CoreError>(vec![mc_core::import::ExternalInstance {
                name,
                game_version,
                loader,
                loader_version: detected.loader_version,
                source_path: path,
                icon_path: detected.icon_path,
                launcher,
            }])
        }
        .await;
        let _ = tx.send(EngineEvent::ProgressDone);
        match result {
            Ok(instances) => {
                let _ = tx.send(EngineEvent::ExternalInstances(instances));
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::Error(format!("Scan failed: {err}")));
            }
        }
    });
}

/// Import an external instance by name lookup (used by picker).
pub(crate) fn import_external_by_name(&mut self, name: &str) {
    let Some(external) = self.external_instances.iter().find(|e| e.name == name).cloned() else {
        self.set_toast(&crate::i18n::tr_string(self.lang(), "toast.instance_not_found").replace("{}", name), true);
        return;
    };
    let manager = self.instance_manager.clone();
    let paths = self.paths.clone();
    let tx = self.engine_tx.clone();
    self.progress = Some((None, format!("Importing '{}'...", external.name)));
    tokio::spawn(async move {
        let result = mc_core::import::import_external(&external, &manager, &paths).await;
        let _ = tx.send(EngineEvent::ProgressDone);
        match result {
            Ok(instance) => {
                let _ = tx.send(EngineEvent::InstancesChanged);
                let _ = tx.send(EngineEvent::Toast(format!(
                    "Imported '{}' from {}",
                    instance.name(),
                    external.launcher.label()
                )));
            }
            Err(err) => {
                let _ = tx.send(EngineEvent::Error(format!("Import failed: {err}")));
            }
        }
    });
}

}
