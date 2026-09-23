//! Detect and extract instance metadata from common launcher formats.

use std::path::{Path, PathBuf};

use crate::instance::LoaderType;

/// Metadata extracted from a detected instance directory.
#[derive(Debug, Clone)]
pub struct DetectedMetadata {
    pub name: Option<String>,
    pub game_version: Option<String>,
    pub loader: Option<LoaderType>,
    pub loader_version: Option<String>,
    pub icon_path: Option<PathBuf>,
}

/// Check if a directory contains a `modrinth.index.json` (mrpack manifest).
pub fn has_modrinth_index(path: &Path) -> bool {
    path.join("modrinth.index.json").exists()
}

/// Try to extract metadata from a `modrinth.index.json` file.
pub fn from_modrinth_index(path: &Path) -> Option<DetectedMetadata> {
    let index_path = path.join("modrinth.index.json");
    let data = std::fs::read_to_string(&index_path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&data).ok()?;

    let name = value.get("name").and_then(|v| v.as_str()).map(String::from);

    let deps = value.get("dependencies")?.as_object()?;

    let game_version = deps
        .get("minecraft")
        .and_then(|v| v.as_str())
        .map(String::from);

    let (loader, loader_version) = detect_loader_from_deps(deps);

    Some(DetectedMetadata {
        name,
        game_version,
        loader,
        loader_version,
        icon_path: None,
    })
}

/// Detect loader from a `dependencies` JSON object (mrpack format).
fn detect_loader_from_deps(
    deps: &serde_json::Map<String, serde_json::Value>,
) -> (Option<LoaderType>, Option<String>) {
    if let Some(v) = deps.get("fabric-loader").and_then(|v| v.as_str()) {
        return (Some(LoaderType::Fabric), Some(v.to_string()));
    }
    if let Some(v) = deps.get("quilt-loader").and_then(|v| v.as_str()) {
        return (Some(LoaderType::Quilt), Some(v.to_string()));
    }
    if let Some(v) = deps.get("neoforge").and_then(|v| v.as_str()) {
        return (Some(LoaderType::NeoForge), Some(v.to_string()));
    }
    if let Some(v) = deps.get("forge").and_then(|v| v.as_str()) {
        return (Some(LoaderType::Forge), Some(v.to_string()));
    }
    (None, None)
}

/// Try to extract metadata from a legacy Modrinth App `profile.json`.
pub fn from_profile_json(path: &Path) -> Option<DetectedMetadata> {
    let profile_path = path.join("profile.json");
    let data = std::fs::read_to_string(&profile_path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&data).ok()?;

    let metadata = value.get("metadata")?;

    let name = metadata
        .get("name")
        .and_then(|v| v.as_str())
        .map(String::from);
    let game_version = metadata
        .get("game_version")
        .and_then(|v| v.as_str())
        .map(String::from);

    let loader_str = metadata.get("loader").and_then(|v| v.as_str())?;
    let loader = parse_loader(loader_str)?;

    let loader_version = metadata
        .get("loader_version")
        .and_then(|v| match v {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Object(o) => o.get("id").and_then(|v| v.as_str()).map(String::from),
            _ => None,
        });

    let icon_path = metadata
        .get("icon")
        .and_then(|v| v.as_str())
        .map(PathBuf::from);

    Some(DetectedMetadata {
        name,
        game_version,
        loader: Some(loader),
        loader_version,
        icon_path,
    })
}

/// Try to extract metadata from a MultiMC/Prism `instance.cfg`.
pub fn from_instance_cfg(path: &Path) -> Option<DetectedMetadata> {
    let cfg_path = path.join("instance.cfg");
    let data = std::fs::read_to_string(&cfg_path).ok()?;

    let mut name = None;
    let mut game_version = None;

    for line in data.lines() {
        if let Some(val) = line.strip_prefix("name=") {
            name = Some(val.to_string());
        }
        if let Some(val) = line.strip_prefix("IntendedVersion=").or_else(|| line.strip_prefix("OverridesVersion=")) {
            game_version = Some(val.to_string());
        }
    }

    if name.is_some() || game_version.is_some() {
        Some(DetectedMetadata {
            name,
            game_version,
            loader: None,
            loader_version: None,
            icon_path: None,
        })
    } else {
        None
    }
}

/// Try to extract metadata from a `.mrpack` file's `modrinth.index.json`
/// stored alongside overrides (common pattern for mrpack-based instances).
pub fn from_mrpack_sidecar(path: &Path) -> Option<DetectedMetadata> {
    // Check for a .mrpack file in the directory.
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            if entry.path().extension().and_then(|e| e.to_str()) == Some("mrpack") {
                return from_modrinth_index(path);
            }
        }
    }
    None
}

/// Comprehensive detection: try all known formats in priority order.
pub fn detect_metadata(path: &Path) -> DetectedMetadata {
    // 1. mrpack manifest (modrinth.index.json)
    if let Some(m) = from_modrinth_index(path) {
        return m;
    }
    // 2. Legacy Modrinth App profile.json
    if let Some(m) = from_profile_json(path) {
        return m;
    }
    // 3. MultiMC/Prism instance.cfg
    if let Some(m) = from_instance_cfg(path) {
        return m;
    }
    // 4. Fallback: use directory name
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string());
    DetectedMetadata {
        name,
        game_version: None,
        loader: None,
        loader_version: None,
        icon_path: None,
    }
}

/// Parse a loader string into a `LoaderType`.
fn parse_loader(s: &str) -> Option<LoaderType> {
    match s.to_ascii_lowercase().as_str() {
        "vanilla" => Some(LoaderType::Vanilla),
        "fabric" => Some(LoaderType::Fabric),
        "quilt" => Some(LoaderType::Quilt),
        "forge" => Some(LoaderType::Forge),
        "neoforge" => Some(LoaderType::NeoForge),
        "paper" => Some(LoaderType::Paper),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parse_loader_variants() {
        assert_eq!(parse_loader("fabric"), Some(LoaderType::Fabric));
        assert_eq!(parse_loader("FABRIC"), Some(LoaderType::Fabric));
        assert_eq!(parse_loader("forge"), Some(LoaderType::Forge));
        assert_eq!(parse_loader("neoforge"), Some(LoaderType::NeoForge));
        assert_eq!(parse_loader("quilt"), Some(LoaderType::Quilt));
        assert_eq!(parse_loader("vanilla"), Some(LoaderType::Vanilla));
        assert_eq!(parse_loader("unknown"), None);
    }

    #[test]
    fn detect_metadata_fallback() {
        let dir = std::env::temp_dir().join(format!("ctm-detect-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let meta = detect_metadata(&dir);
        assert!(meta.name.is_some());
        assert!(meta.game_version.is_none());
        fs::remove_dir_all(&dir).unwrap();
    }
}
