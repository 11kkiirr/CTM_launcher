//! Import instances from external launchers.
//!
//! Provides detection, metadata extraction, and symlink-based linking for
//! integrating instances from other Minecraft launchers into CTMLauncher.

pub mod detect;
pub mod modrinth_app;

use std::path::PathBuf;

use crate::instance::{InstanceManager, LoaderType};
use crate::util::Paths;
use crate::Result;

/// Launcher type detected from the filesystem layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LauncherKind {
    ModrinthApp,
    MultiMc,
    Prism,
    ATLauncher,
    Unknown,
}

impl LauncherKind {
    pub fn label(&self) -> &'static str {
        match self {
            LauncherKind::ModrinthApp => "Modrinth App",
            LauncherKind::MultiMc => "MultiMC",
            LauncherKind::Prism => "Prism Launcher",
            LauncherKind::ATLauncher => "ATLauncher",
            LauncherKind::Unknown => "Unknown",
        }
    }
}

/// Metadata extracted from an external launcher's instance.
#[derive(Debug, Clone)]
pub struct ExternalInstance {
    pub name: String,
    pub game_version: String,
    pub loader: LoaderType,
    pub loader_version: Option<String>,
    pub source_path: PathBuf,
    pub icon_path: Option<PathBuf>,
    pub launcher: LauncherKind,
}

/// Attempt to auto-detect the launcher kind from a directory path.
pub fn detect_launcher(path: &std::path::Path) -> LauncherKind {
    if detect::has_modrinth_index(path) {
        return LauncherKind::ModrinthApp;
    }
    if path.join("instance.json").exists() || path.join("instance.cfg").exists() {
        // Could be MultiMC/Prism — but CTMLauncher also uses instance.json,
        // so only flag as external if it doesn't look like our own.
        return LauncherKind::Unknown;
    }
    if path.join("profile.json").exists() {
        return LauncherKind::ModrinthApp;
    }
    LauncherKind::Unknown
}

/// Import an external instance into CTMLauncher by symlinking to its game directory.
///
/// Creates a new CTMLauncher instance with metadata pointing at the source,
/// then symlinks `.minecraft/` to the original directory so no files are duplicated.
pub async fn import_external(
    external: &ExternalInstance,
    instance_manager: &InstanceManager,
    _paths: &Paths,
) -> Result<crate::instance::Instance> {
    let instance = instance_manager
        .create(
            &external.name,
            &external.game_version,
            external.loader,
            external.loader_version.clone(),
        )
        .await?;

    // Remove the scaffolded .minecraft/ so we can replace it with a symlink.
    let game_dir = instance.game_dir();
    if game_dir.exists() {
        tokio::fs::remove_dir_all(&game_dir).await?;
    }

    // Create symlink: .minecraft/ -> source directory.
    #[cfg(unix)]
    std::os::unix::fs::symlink(&external.source_path, &game_dir)?;
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&external.source_path, &game_dir)?;

    // Copy icon if present.
    if let Some(icon) = &external.icon_path {
        if icon.exists() {
            let dest = instance.root.join("icon.png");
            let _ = tokio::fs::copy(icon, &dest).await;
        }
    }

    // Record provenance + linked source.
    let mut metadata = instance.metadata.clone();
    metadata.linked_source = Some(external.source_path.clone());
    metadata.modpack = Some(crate::instance::ModpackOrigin {
        name: format!("{} import", external.launcher.label()),
        version: None,
        source: Some(external.launcher.label().to_string()),
    });
    let updated = crate::instance::Instance::new(instance.root.clone(), metadata);
    updated.save().await?;

    Ok(instance)
}
