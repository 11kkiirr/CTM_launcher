//! Isolated instance directories and their metadata (`instance.json`).

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::util::{ensure_dir, write_json, Paths};

/// Supported modloaders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LoaderType {
    Vanilla,
    Fabric,
    Quilt,
    Forge,
    #[serde(rename = "neoforge")]
    NeoForge,
    Paper,
}

impl LoaderType {
    pub fn as_str(&self) -> &'static str {
        match self {
            LoaderType::Vanilla => "vanilla",
            LoaderType::Fabric => "fabric",
            LoaderType::Quilt => "quilt",
            LoaderType::Forge => "forge",
            LoaderType::NeoForge => "neoforge",
            LoaderType::Paper => "paper",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            LoaderType::Vanilla => "Vanilla",
            LoaderType::Fabric => "Fabric",
            LoaderType::Quilt => "Quilt",
            LoaderType::Forge => "Forge",
            LoaderType::NeoForge => "NeoForge",
            LoaderType::Paper => "Paper",
        }
    }

    /// Whether this loader installs mods into `mods/`.
    pub fn supports_mods(&self) -> bool {
        !matches!(self, LoaderType::Vanilla | LoaderType::Paper)
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "vanilla" => Some(LoaderType::Vanilla),
            "fabric" => Some(LoaderType::Fabric),
            "quilt" => Some(LoaderType::Quilt),
            "forge" => Some(LoaderType::Forge),
            "neoforge" => Some(LoaderType::NeoForge),
            "paper" => Some(LoaderType::Paper),
            _ => None,
        }
    }

    pub fn all() -> &'static [LoaderType] {
        &[
            LoaderType::Vanilla,
            LoaderType::Fabric,
            LoaderType::Quilt,
            LoaderType::Forge,
            LoaderType::NeoForge,
            LoaderType::Paper,
        ]
    }
}

impl std::fmt::Display for LoaderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Garbage collector presets exposed to the JVM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum GcPreset {
    #[default]
    G1,
    Zgc,
    Shenandoah,
    Parallel,
    Serial,
    /// No GC flags are added; rely on `custom_args`.
    None,
}

impl GcPreset {
    /// The JVM flags that select this collector.
    pub fn flags(&self) -> &'static [&'static str] {
        match self {
            GcPreset::G1 => &[
                "-XX:+UseG1GC",
                "-XX:+ParallelRefProcEnabled",
                "-XX:MaxGCPauseMillis=200",
                "-XX:+UnlockExperimentalVMOptions",
                "-XX:+DisableExplicitGC",
                "-XX:G1NewSizePercent=30",
                "-XX:G1MaxNewSizePercent=40",
                "-XX:G1HeapRegionSize=8M",
                "-XX:G1ReservePercent=20",
            ],
            GcPreset::Zgc => &["-XX:+UseZGC", "-XX:+UnlockExperimentalVMOptions"],
            GcPreset::Shenandoah => &["-XX:+UseShenandoahGC", "-XX:+UnlockExperimentalVMOptions"],
            GcPreset::Parallel => &["-XX:+UseParallelGC"],
            GcPreset::Serial => &["-XX:+UseSerialGC"],
            GcPreset::None => &[],
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            GcPreset::G1 => "G1GC",
            GcPreset::Zgc => "ZGC",
            GcPreset::Shenandoah => "Shenandoah",
            GcPreset::Parallel => "Parallel",
            GcPreset::Serial => "Serial",
            GcPreset::None => "None",
        }
    }

    pub fn all() -> &'static [GcPreset] {
        &[
            GcPreset::G1,
            GcPreset::Zgc,
            GcPreset::Shenandoah,
            GcPreset::Parallel,
            GcPreset::Serial,
            GcPreset::None,
        ]
    }
}

impl std::fmt::Display for GcPreset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Memory, GC, Java and extra-argument configuration for an instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JvmConfig {
    #[serde(default = "default_min_memory")]
    pub min_memory_mb: u32,
    #[serde(default = "default_max_memory")]
    pub max_memory_mb: u32,
    #[serde(default)]
    pub gc: GcPreset,
    /// Explicit Java executable, or `None` to auto-detect.
    #[serde(default)]
    pub java_path: Option<PathBuf>,
    /// Extra JVM flags appended after the preset flags.
    #[serde(default)]
    pub custom_jvm_args: Vec<String>,
    /// Extra game arguments appended at the end of the command line.
    #[serde(default)]
    pub extra_game_args: Vec<String>,
    /// Whether to use the system proxy settings for the game.
    #[serde(default)]
    pub fullscreen: bool,
}

fn default_min_memory() -> u32 {
    512
}
fn default_max_memory() -> u32 {
    4096
}

impl Default for JvmConfig {
    fn default() -> Self {
        Self {
            min_memory_mb: default_min_memory(),
            max_memory_mb: default_max_memory(),
            gc: GcPreset::default(),
            java_path: None,
            custom_jvm_args: Vec::new(),
            extra_game_args: Vec::new(),
            fullscreen: false,
        }
    }
}

/// Provenance information for instances imported from a modpack.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModpackOrigin {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
}

/// Contents of `instance.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceMetadata {
    pub id: String,
    pub name: String,
    pub game_version: String,
    pub loader: LoaderType,
    #[serde(default)]
    pub loader_version: Option<String>,
    #[serde(default = "Utc::now")]
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub last_played: Option<DateTime<Utc>>,
    #[serde(default)]
    pub jvm: JvmConfig,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub modpack: Option<ModpackOrigin>,
}

impl InstanceMetadata {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        game_version: impl Into<String>,
        loader: LoaderType,
        loader_version: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            game_version: game_version.into(),
            loader,
            loader_version,
            created_at: Utc::now(),
            last_played: None,
            jvm: JvmConfig::default(),
            icon: None,
            modpack: None,
        }
    }

    /// A compact `"1.20.1 · Fabric 0.15.7"` descriptor for the UI.
    pub fn descriptor(&self) -> String {
        match &self.loader_version {
            Some(v) if self.loader != LoaderType::Vanilla => {
                format!("{} · {} {}", self.game_version, self.loader, v)
            }
            _ => format!("{} · {}", self.game_version, self.loader),
        }
    }
}

/// A materialized instance rooted at its own directory.
#[derive(Debug, Clone)]
pub struct Instance {
    pub root: PathBuf,
    pub metadata: InstanceMetadata,
}

impl Instance {
    pub fn new(root: PathBuf, metadata: InstanceMetadata) -> Self {
        Self { root, metadata }
    }

    pub fn id(&self) -> &str {
        &self.metadata.id
    }

    pub fn name(&self) -> &str {
        &self.metadata.name
    }

    /// The `.minecraft` game directory.
    pub fn game_dir(&self) -> PathBuf {
        self.root.join(".minecraft")
    }

    pub fn mods_dir(&self) -> PathBuf {
        self.game_dir().join("mods")
    }

    pub fn config_dir(&self) -> PathBuf {
        self.game_dir().join("config")
    }

    pub fn resourcepacks_dir(&self) -> PathBuf {
        self.game_dir().join("resourcepacks")
    }

    pub fn saves_dir(&self) -> PathBuf {
        self.game_dir().join("saves")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.game_dir().join("logs")
    }

    pub fn crash_reports_dir(&self) -> PathBuf {
        self.game_dir().join("crash-reports")
    }

    pub fn options_file(&self) -> PathBuf {
        self.game_dir().join("options.txt")
    }

    pub fn metadata_file(&self) -> PathBuf {
        self.root.join("instance.json")
    }

    /// Create the standard instance subdirectory tree.
    pub async fn scaffold(&self) -> Result<()> {
        for dir in [
            self.game_dir(),
            self.mods_dir(),
            self.config_dir(),
            self.resourcepacks_dir(),
            self.saves_dir(),
            self.logs_dir(),
            self.crash_reports_dir(),
        ] {
            ensure_dir(dir).await?;
        }
        Ok(())
    }

    /// Persist metadata to `instance.json`.
    pub async fn save(&self) -> Result<()> {
        write_json(self.metadata_file(), &self.metadata).await
    }

    /// Update `last_played` and persist.
    pub async fn mark_played(&mut self) -> Result<()> {
        self.metadata.last_played = Some(Utc::now());
        self.save().await
    }
}

/// Creates, lists and deletes instances under [`Paths::instances_dir`].
#[derive(Debug, Clone)]
pub struct InstanceManager {
    paths: Paths,
}

impl InstanceManager {
    pub fn new(paths: Paths) -> Self {
        Self { paths }
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    /// List every instance that has a readable `instance.json`.
    pub fn list(&self) -> Result<Vec<Instance>> {
        let root = self.paths.instances_dir();
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let metadata_file = entry.path().join("instance.json");
            if !metadata_file.exists() {
                continue;
            }
            let bytes = std::fs::read(&metadata_file)?;
            let metadata: InstanceMetadata = serde_json::from_slice(&bytes)?;
            out.push(Instance::new(entry.path(), metadata));
        }
        out.sort_by_key(|a| a.name().to_lowercase());
        Ok(out)
    }

    /// Load a single instance by id.
    pub fn get(&self, id: &str) -> Result<Instance> {
        let root = self.paths.instance_dir(id);
        let metadata_file = root.join("instance.json");
        if !metadata_file.exists() {
            return Err(CoreError::NotFound(format!("instance '{id}'")));
        }
        let metadata: InstanceMetadata = read_json_sync(&metadata_file)?;
        Ok(Instance::new(root, metadata))
    }

    /// Create a new instance, returning it with its directory scaffolded.
    pub async fn create(
        &self,
        name: &str,
        game_version: &str,
        loader: LoaderType,
        loader_version: Option<String>,
    ) -> Result<Instance> {
        let id = self.unique_id(name)?;
        let root = self.paths.instance_dir(&id);
        if root.exists() {
            return Err(CoreError::Instance(format!(
                "instance directory already exists: {}",
                root.display()
            )));
        }
        let metadata = InstanceMetadata::new(&id, name, game_version, loader, loader_version);
        let instance = Instance::new(root, metadata);
        instance.scaffold().await?;
        instance.save().await?;
        Ok(instance)
    }

    /// Delete an instance directory and all of its contents.
    pub async fn delete(&self, id: &str) -> Result<()> {
        let root = self.paths.instance_dir(id);
        if !root.exists() {
            return Err(CoreError::NotFound(format!("instance '{id}'")));
        }
        tokio::fs::remove_dir_all(&root).await?;
        Ok(())
    }

    /// Rename an instance's display name, persisting `instance.json`.
    pub async fn rename(&self, id: &str, new_name: &str) -> Result<()> {
        let trimmed = new_name.trim().to_string();
        if trimmed.is_empty() {
            return Err(CoreError::Other("instance name cannot be empty".into()));
        }
        let instance = self.get(id)?;
        let mut metadata = instance.metadata.clone();
        metadata.name = trimmed;
        write_json(instance.metadata_file(), &metadata).await?;
        Ok(())
    }

    /// Build a filesystem-safe, unique id from a display name.
    fn unique_id(&self, name: &str) -> Result<String> {
        let base = slugify(name);
        let base = if base.is_empty() {
            "instance".to_string()
        } else {
            base
        };
        let mut candidate = base.clone();
        let mut counter = 2;
        while self.paths.instance_dir(&candidate).exists() {
            candidate = format!("{base}-{counter}");
            counter += 1;
        }
        Ok(candidate)
    }
}

fn read_json_sync<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = std::fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Lowercase, dash-separated slug safe for directory names.
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_variants() {
        assert_eq!(slugify("My Cool Pack!"), "my-cool-pack");
        assert_eq!(slugify("  ---  "), "");
        assert_eq!(slugify("1.20.1 Fabric"), "1-20-1-fabric");
    }

    #[test]
    fn gc_flags() {
        assert!(GcPreset::G1.flags().contains(&"-XX:+UseG1GC"));
        assert!(GcPreset::Zgc.flags().contains(&"-XX:+UseZGC"));
        assert!(GcPreset::None.flags().is_empty());
    }

    #[tokio::test]
    async fn create_list_delete_instance() {
        let dir = std::env::temp_dir().join(format!("ctm-inst-{}", uuid::Uuid::new_v4()));
        let paths = Paths::rooted_at(&dir);
        paths.ensure_layout().unwrap();
        let manager = InstanceManager::new(paths);

        let instance = manager
            .create(
                "Test Pack",
                "1.20.1",
                LoaderType::Fabric,
                Some("0.15.7".into()),
            )
            .await
            .unwrap();
        assert!(instance.mods_dir().exists());
        assert_eq!(instance.metadata.descriptor(), "1.20.1 · Fabric 0.15.7");

        let listed = manager.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name(), "Test Pack");

        manager.delete(instance.id()).await.unwrap();
        assert!(manager.list().unwrap().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn rename_updates_display_name() {
        let dir = std::env::temp_dir().join(format!("ctm-inst-{}", uuid::Uuid::new_v4()));
        let paths = Paths::rooted_at(&dir);
        paths.ensure_layout().unwrap();
        let manager = InstanceManager::new(paths);

        let instance = manager
            .create(
                "Old Name",
                "1.20.1",
                LoaderType::Fabric,
                Some("0.15.7".into()),
            )
            .await
            .unwrap();
        manager.rename(instance.id(), "New Name").await.unwrap();
        let reloaded = manager.get(instance.id()).unwrap();
        assert_eq!(reloaded.name(), "New Name");

        manager.delete(instance.id()).await.unwrap();
        assert!(manager.list().unwrap().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn loader_round_trip() {
        for loader in LoaderType::all() {
            assert_eq!(LoaderType::parse(loader.as_str()), Some(*loader));
        }
    }
}
