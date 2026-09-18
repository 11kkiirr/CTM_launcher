//! `.mrpack` (Modrinth modpack) parsing and override extraction.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::instance::LoaderType;
use crate::util::sanitize_archive_path;

/// The metadata document at the root of a `.mrpack` archive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MrpackIndex {
    #[serde(default = "default_format_version", rename = "formatVersion")]
    pub format_version: u32,
    #[serde(default)]
    pub game: String,
    #[serde(default, rename = "versionId")]
    pub version_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub files: Vec<MrpackFile>,
    #[serde(default)]
    pub dependencies: HashMap<String, String>,
}

fn default_format_version() -> u32 {
    1
}

impl MrpackIndex {
    /// The Minecraft version the pack targets.
    pub fn game_version(&self) -> Option<String> {
        self.dependencies
            .get("minecraft")
            .cloned()
            .or_else(|| (!self.version_id.is_empty()).then(|| self.version_id.clone()))
    }

    /// Determine the modloader and its version from the dependency table.
    pub fn loader(&self) -> (LoaderType, Option<String>) {
        let candidates = [
            ("fabric-loader", LoaderType::Fabric),
            ("quilt-loader", LoaderType::Quilt),
            ("neoforge", LoaderType::NeoForge),
            ("forge", LoaderType::Forge),
        ];
        for (key, loader) in candidates {
            if let Some(version) = self.dependencies.get(key) {
                return (loader, Some(version.clone()));
            }
        }
        (LoaderType::Vanilla, None)
    }
}

/// A single file to download as part of a modpack.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MrpackFile {
    /// Path relative to the game directory, e.g. `mods/sodium.jar`.
    pub path: String,
    #[serde(default)]
    pub hashes: HashMap<String, String>,
    #[serde(default)]
    pub env: MrpackEnv,
    #[serde(default)]
    pub downloads: Vec<String>,
    #[serde(default, rename = "fileSize")]
    pub file_size: u64,
}

impl MrpackFile {
    pub fn sha1(&self) -> Option<&str> {
        self.hashes.get("sha1").map(String::as_str)
    }

    pub fn sha512(&self) -> Option<&str> {
        self.hashes.get("sha512").map(String::as_str)
    }

    /// Whether the client should download this file.
    pub fn applies_to_client(&self) -> bool {
        self.env.client != "unsupported"
    }
}

/// Client/server side requirements for a modpack file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MrpackEnv {
    #[serde(default = "default_env")]
    pub client: String,
    #[serde(default = "default_env")]
    pub server: String,
}

impl Default for MrpackEnv {
    fn default() -> Self {
        Self {
            client: default_env(),
            server: default_env(),
        }
    }
}

fn default_env() -> String {
    "required".to_string()
}

/// Read and parse `modrinth.index.json` from a `.mrpack` archive.
pub fn parse_mrpack(path: impl AsRef<Path>) -> Result<MrpackIndex> {
    let file = std::fs::File::open(path.as_ref())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| CoreError::Archive(e.to_string()))?;

    let mut entry = archive
        .by_name("modrinth.index.json")
        .map_err(|_| CoreError::Modpack("archive is missing modrinth.index.json".into()))?;
    let mut contents = String::new();
    entry.read_to_string(&mut contents)?;
    let index: MrpackIndex = serde_json::from_str(&contents)?;
    Ok(index)
}

/// Extract `overrides/` and `client-overrides/` into `game_dir`.
///
/// Returns the number of files written. Paths are sanitized to prevent zip
/// slip attacks.
pub fn extract_overrides(
    archive_path: impl AsRef<Path>,
    game_dir: impl AsRef<Path>,
) -> Result<usize> {
    let file = std::fs::File::open(archive_path.as_ref())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| CoreError::Archive(e.to_string()))?;
    let game_dir = game_dir.as_ref();
    std::fs::create_dir_all(game_dir)?;
    let mut count = 0;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| CoreError::Archive(e.to_string()))?;
        let name = entry.name().to_string();

        let relative = if let Some(rest) = name.strip_prefix("overrides/") {
            rest
        } else if let Some(rest) = name.strip_prefix("client-overrides/") {
            rest
        } else {
            continue;
        };
        if relative.is_empty() {
            continue;
        }

        let safe = match sanitize_archive_path(Path::new(relative)) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let out_path = game_dir.join(safe);
        if name.ends_with('/') {
            std::fs::create_dir_all(&out_path)?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&out_path)?;
        std::io::copy(&mut entry, &mut out)?;
        count += 1;
    }
    Ok(count)
}

/// Destination path for a modpack file, sanitized against traversal.
pub fn file_destination(game_dir: impl AsRef<Path>, file: &MrpackFile) -> Result<PathBuf> {
    let safe = sanitize_archive_path(Path::new(&file.path))?;
    Ok(game_dir.as_ref().join(safe))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_index_and_detects_fabric() {
        let raw = r#"{
            "formatVersion": 1,
            "game": "minecraft",
            "versionId": "1.20.1",
            "name": "Test Pack",
            "files": [
                {"path":"mods/sodium.jar","hashes":{"sha1":"abc","sha512":"def"},"env":{"client":"required","server":"unsupported"},"downloads":["https://example.com/sodium.jar"],"fileSize":100}
            ],
            "dependencies": {"minecraft":"1.20.1","fabric-loader":"0.15.7"}
        }"#;
        let index: MrpackIndex = serde_json::from_str(raw).unwrap();
        assert_eq!(index.game_version().as_deref(), Some("1.20.1"));
        let (loader, version) = index.loader();
        assert_eq!(loader, LoaderType::Fabric);
        assert_eq!(version.as_deref(), Some("0.15.7"));
        assert!(index.files[0].applies_to_client());
        assert_eq!(index.files[0].sha1(), Some("abc"));
    }

    #[test]
    fn detects_neoforge_and_forge() {
        let neo: MrpackIndex =
            serde_json::from_str(r#"{"dependencies":{"minecraft":"1.21.1","neoforge":"21.1.77"}}"#)
                .unwrap();
        assert_eq!(neo.loader().0, LoaderType::NeoForge);

        let forge: MrpackIndex =
            serde_json::from_str(r#"{"dependencies":{"minecraft":"1.20.1","forge":"47.2.0"}}"#)
                .unwrap();
        assert_eq!(forge.loader().0, LoaderType::Forge);
    }

    #[test]
    fn default_env_is_required() {
        let env = MrpackEnv::default();
        assert_eq!(env.client, "required");
    }
}
