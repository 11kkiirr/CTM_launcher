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
    pub group: String,
    #[serde(default)]
    pub modpack: Option<ModpackOrigin>,
    #[serde(default)]
    pub linked_source: Option<PathBuf>,
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
            group: String::new(),
            modpack: None,
            linked_source: None,
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

    /// Shader packs live in `shaderpacks/` (Iris, OptiFine, Modrinth App).
    /// Note: `shaders/` is a different directory used for core shaders.
    pub fn shaders_dir(&self) -> PathBuf {
        self.game_dir().join("shaderpacks")
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

    /// Whether this instance is a symlink to an external launcher's directory.
    pub fn is_linked(&self) -> bool {
        self.metadata.linked_source.is_some()
    }

    /// The original source path if this instance is linked.
    pub fn linked_source(&self) -> Option<&std::path::Path> {
        self.metadata.linked_source.as_deref()
    }

    /// Create the standard instance subdirectory tree.
    pub async fn scaffold(&self) -> Result<()> {
        for dir in [
            self.game_dir(),
            self.mods_dir(),
            self.config_dir(),
            self.resourcepacks_dir(),
            self.shaders_dir(),
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

    /// List every instance whose `group` field matches the given value.
    /// Pass an empty string to list all ungrouped instances.
    pub fn list_in_group(&self, group: &str) -> Result<Vec<Instance>> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|i| i.metadata.group == group)
            .collect())
    }

    /// Known group names: the registry (`groups.json`) merged with any
    /// group still referenced by an instance (migration), sorted + deduped.
    /// Empty groups are preserved so they can exist before instances move in.
    pub fn groups(&self) -> Vec<String> {
        let mut names = self.load_stored_groups();
        names.extend(
            self.list()
                .unwrap_or_default()
                .into_iter()
                .map(|i| i.metadata.group)
                .filter(|g| !g.is_empty()),
        );
        names.sort();
        names.dedup();
        names
    }

    /// Number of instances in each group; key `""` holds the ungrouped count.
    pub fn group_counts(&self) -> std::collections::HashMap<String, usize> {
        let mut counts = std::collections::HashMap::new();
        for instance in self.list().unwrap_or_default() {
            *counts.entry(instance.metadata.group).or_insert(0) += 1;
        }
        counts
    }

    /// Assign an instance to a group (empty string = ungrouped).
    /// Non-empty groups are ensured in the registry so empty groups persist.
    pub async fn set_group(&self, id: &str, group: &str) -> Result<()> {
        let instance = self.get(id)?;
        let meta_file = instance.metadata_file();
        let mut metadata = instance.metadata;
        metadata.group = group.to_string();
        write_json(meta_file, &metadata).await?;
        if !group.is_empty() {
            self.ensure_group_in_registry(group).await?;
        }
        Ok(())
    }

    /// Create a named group (no-op if the name already exists).
    pub async fn create_group(&self, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(CoreError::other("group name cannot be empty"));
        }
        let mut groups = self.load_stored_groups();
        if groups.iter().any(|g| g == name) {
            return Ok(());
        }
        groups.push(name.to_string());
        self.save_stored_groups(groups).await
    }

    /// Rename a group across the registry and every member instance.
    pub async fn rename_group(&self, old: &str, new: &str) -> Result<()> {
        let old = old.trim();
        let new = new.trim();
        if old.is_empty() || new.is_empty() {
            return Err(CoreError::other("group name cannot be empty"));
        }
        if old == new {
            return Ok(());
        }

        let mut groups = self.load_stored_groups();
        // Also pull any derived-only names so rename works pre-registry.
        for instance in self.list().unwrap_or_default() {
            let g = instance.metadata.group;
            if !g.is_empty() && !groups.contains(&g) {
                groups.push(g);
            }
        }
        if let Some(pos) = groups.iter().position(|g| g == old) {
            groups[pos] = new.to_string();
        } else {
            groups.push(new.to_string());
        }
        // Drop the old name if it remains (shouldn't after replace).
        groups.retain(|g| g != old);
        groups.sort();
        groups.dedup();
        self.save_stored_groups(groups).await?;

        for instance in self.list().unwrap_or_default() {
            if instance.metadata.group == old {
                let meta_file = instance.metadata_file();
                let mut metadata = instance.metadata;
                metadata.group = new.to_string();
                write_json(meta_file, &metadata).await?;
            }
        }
        Ok(())
    }

    /// Delete a group: remove it from the registry and ungroup its members.
    pub async fn delete_group(&self, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(CoreError::other("group name cannot be empty"));
        }

        let mut groups = self.load_stored_groups();
        for instance in self.list().unwrap_or_default() {
            let g = instance.metadata.group;
            if !g.is_empty() && !groups.contains(&g) {
                groups.push(g);
            }
        }
        groups.retain(|g| g != name);
        self.save_stored_groups(groups).await?;

        for instance in self.list().unwrap_or_default() {
            if instance.metadata.group == name {
                let meta_file = instance.metadata_file();
                let mut metadata = instance.metadata;
                metadata.group = String::new();
                write_json(meta_file, &metadata).await?;
            }
        }
        Ok(())
    }

    fn load_stored_groups(&self) -> Vec<String> {
        let path = self.paths.groups_file();
        if !path.exists() {
            return Vec::new();
        }
        let raw = std::fs::read_to_string(&path).unwrap_or_default();
        #[derive(serde::Deserialize)]
        struct File {
            #[serde(default)]
            groups: Vec<String>,
        }
        serde_json::from_str::<File>(&raw)
            .map(|f| f.groups)
            .unwrap_or_default()
    }

    async fn save_stored_groups(&self, groups: Vec<String>) -> Result<()> {
        #[derive(serde::Serialize)]
        struct File {
            groups: Vec<String>,
        }
        let path = self.paths.groups_file();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        write_json(path, &File { groups }).await
    }

    async fn ensure_group_in_registry(&self, name: &str) -> Result<()> {
        let mut groups = self.load_stored_groups();
        if groups.iter().any(|g| g == name) {
            return Ok(());
        }
        groups.push(name.to_string());
        self.save_stored_groups(groups).await
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
        // For linked instances, remove the .minecraft symlink itself so we
        // don't accidentally follow it and delete the original game files.
        let game_dir = root.join(".minecraft");
        if game_dir.is_symlink() {
            tokio::fs::remove_file(&game_dir).await?;
        }
        tokio::fs::remove_dir_all(&root).await?;
        Ok(())
    }

    /// Remove a linked instance from the library without deleting the original files.
    ///
    /// This is the safe way to remove an instance that points at an external
    /// launcher's directory.  It removes the CTMLauncher metadata and symlink
    /// but leaves the source directory untouched.
    pub async fn unlink(&self, id: &str) -> Result<()> {
        self.delete(id).await
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

    #[test]
    fn shaders_dir_is_shaderpacks_not_shaders() {
        let root = std::path::PathBuf::from("/tmp/instance-root");
        let meta = InstanceMetadata::new("id", "Test", "1.21.1", LoaderType::Fabric, None);
        let instance = Instance::new(root, meta);
        // Minecraft/Iris/OptiFine/Modrinth App use `shaderpacks/`,
        // not `shaders/` (that's for core shaders inside resource packs).
        assert_eq!(
            instance.shaders_dir(),
            instance.game_dir().join("shaderpacks")
        );
        assert!(!instance.shaders_dir().ends_with("shaders"));
    }

    #[tokio::test]
    async fn group_round_trip() {
        let dir = std::env::temp_dir().join(format!("ctm-inst-{}", uuid::Uuid::new_v4()));
        let paths = Paths::rooted_at(&dir);
        paths.ensure_layout().unwrap();
        let manager = InstanceManager::new(paths);

        let inst1 = manager
            .create("A", "1.20.1", LoaderType::Fabric, Some("0.15.7".into()))
            .await
            .unwrap();
        let inst2 = manager
            .create("B", "1.21.1", LoaderType::Vanilla, None)
            .await
            .unwrap();

        assert!(manager.groups().is_empty());

        manager.set_group(inst1.id(), "Modded").await.unwrap();
        manager.set_group(inst2.id(), "Vanilla").await.unwrap();
        let groups = manager.groups();
        assert_eq!(groups, vec!["Modded".to_string(), "Vanilla".to_string()]);

        let modded = manager.list_in_group("Modded").unwrap();
        assert_eq!(modded.len(), 1);
        assert_eq!(modded[0].name(), "A");

        let counts = manager.group_counts();
        assert_eq!(counts.get("Modded"), Some(&1));
        assert_eq!(counts.get("Vanilla"), Some(&1));
        // No ungrouped instances yet — key may be absent.
        assert_ne!(counts.get(""), Some(&1));

        // Ungrouping keeps the name in the registry (empty groups persist).
        manager.set_group(inst1.id(), "").await.unwrap();
        let groups = manager.groups();
        assert_eq!(groups, vec!["Modded".to_string(), "Vanilla".to_string()]);
        assert_eq!(manager.list_in_group("").unwrap().len(), 1);

        // Rename rewrites registry + members.
        manager.rename_group("Modded", "Pack").await.unwrap();
        assert!(manager.groups().contains(&"Pack".to_string()));
        assert!(!manager.groups().contains(&"Modded".to_string()));

        // Delete removes from registry and ungroups members.
        manager.delete_group("Pack").await.unwrap();
        manager.delete_group("Vanilla").await.unwrap();
        assert!(manager.groups().is_empty());
        assert!(manager
            .list()
            .unwrap()
            .iter()
            .all(|i| i.metadata.group.is_empty()));

        manager.delete(inst1.id()).await.unwrap();
        manager.delete(inst2.id()).await.unwrap();
        assert!(manager.list().unwrap().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn create_empty_group_persists() {
        let dir = std::env::temp_dir().join(format!("ctm-inst-{}", uuid::Uuid::new_v4()));
        let paths = Paths::rooted_at(&dir);
        paths.ensure_layout().unwrap();
        let manager = InstanceManager::new(paths);

        manager.create_group("Empty").await.unwrap();
        assert_eq!(manager.groups(), vec!["Empty".to_string()]);

        // Duplicate create is a no-op.
        manager.create_group("Empty").await.unwrap();
        assert_eq!(manager.groups().len(), 1);

        manager.delete_group("Empty").await.unwrap();
        assert!(manager.groups().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
