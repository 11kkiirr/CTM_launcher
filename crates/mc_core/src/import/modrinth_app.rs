//! Read instance data from the Modrinth App's SQLite database (`app.db`).

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::instance::LoaderType;
use crate::{CoreError, Result};

use super::{ExternalInstance, LauncherKind};

/// Detect the Modrinth App data directory on the current platform.
pub fn modrinth_app_dir() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        let home = std::env::var("HOME").ok()?;
        let xdg = std::env::var("XDG_DATA_HOME")
            .unwrap_or_else(|_| format!("{home}/.local/share"));
        let dir = PathBuf::from(&xdg).join("ModrinthApp");
        if dir.exists() {
            return Some(dir);
        }
        // Legacy directory name
        let legacy = PathBuf::from(xdg).join("com.modrinth.theseus");
        if legacy.exists() {
            return Some(legacy);
        }
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").ok()?;
        let dir = PathBuf::from(home)
            .join("Library/Application Support/ModrinthApp");
        if dir.exists() {
            return Some(dir);
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            let dir = PathBuf::from(appdata).join("ModrinthApp");
            if dir.exists() {
                return Some(dir);
            }
        }
    }
    None
}

/// Check if a directory looks like a Modrinth App data directory.
pub fn is_modrinth_app_dir(path: &Path) -> bool {
    path.join("app.db").exists() || path.join("profiles").exists()
}

/// Read the Modrinth App database and list all instances.
pub fn list_instances(app_dir: &Path) -> Result<Vec<ExternalInstance>> {
    let db_path = app_dir.join("app.db");
    if !db_path.exists() {
        return Err(CoreError::Other(format!(
            "Modrinth App database not found at {}",
            db_path.display()
        )));
    }

    let conn = Connection::open(&db_path)
        .map_err(|e| CoreError::Other(format!("failed to open Modrinth App DB: {e}")))?;

    let profiles_dir = app_dir.join("profiles");

    // Try the current v2 schema first (instances + instance_content_sets).
    let instances = try_query_v2(&conn, &profiles_dir)
        .or_else(|_| try_query_v1(&conn, &profiles_dir))
        .unwrap_or_default();

    Ok(instances)
}

/// Query the newer "instances v2" schema with separate tables.
fn try_query_v2(conn: &Connection, profiles_dir: &Path) -> Result<Vec<ExternalInstance>> {
    let mut stmt = conn
        .prepare(
            "SELECT
                i.id,
                i.path,
                i.name,
                i.icon_path,
                i.last_played,
                ics.game_version,
                ics.loader,
                ics.loader_version
            FROM instances i
            LEFT JOIN instance_content_sets ics ON i.id = ics.instance_id
            WHERE i.install_stage = 'installed'
            ORDER BY i.name",
        )
        .map_err(|e| CoreError::Other(format!("DB query prep failed: {e}")))?;

    let rows = stmt
        .query_map([], |row| {
            let path: String = row.get(1)?;
            let name: String = row.get(2)?;
            let icon_path: Option<String> = row.get(3)?;
            let game_version: Option<String> = row.get(5)?;
            let loader: Option<String> = row.get(6)?;
            let loader_version: Option<String> = row.get(7)?;
            Ok((path, name, icon_path, game_version, loader, loader_version))
        })
        .map_err(|e| CoreError::Other(format!("DB query failed: {e}")))?;

    let mut instances = Vec::new();
    for row in rows.flatten() {
        let (path, name, icon_path, game_version, loader, loader_version) = row;
        let source_path = profiles_dir.join(&path);
        if !source_path.exists() {
            continue;
        }

        let loader_type = loader
            .as_deref()
            .and_then(parse_loader)
            .unwrap_or(LoaderType::Vanilla);

        let icon = icon_path.map(PathBuf::from);

        instances.push(ExternalInstance {
            name,
            game_version: game_version.unwrap_or_else(|| "unknown".to_string()),
            loader: loader_type,
            loader_version,
            source_path,
            icon_path: icon,
            launcher: LauncherKind::ModrinthApp,
        });
    }

    Ok(instances)
}

/// Fallback: query a single-table schema (legacy Theseus).
fn try_query_v1(conn: &Connection, profiles_dir: &Path) -> Result<Vec<ExternalInstance>> {
    let mut stmt = conn
        .prepare(
            "SELECT path, name, icon_path, game_version, loader, loader_version
             FROM instances
             WHERE install_stage = 'installed'
             ORDER BY name",
        )
        .map_err(|e| CoreError::Other(format!("DB v1 query prep failed: {e}")))?;

    let rows = stmt
        .query_map([], |row| {
            let path: String = row.get(0)?;
            let name: String = row.get(1)?;
            let icon_path: Option<String> = row.get(2)?;
            let game_version: Option<String> = row.get(3)?;
            let loader: Option<String> = row.get(4)?;
            let loader_version: Option<String> = row.get(5)?;
            Ok((path, name, icon_path, game_version, loader, loader_version))
        })
        .map_err(|e| CoreError::Other(format!("DB v1 query failed: {e}")))?;

    let mut instances = Vec::new();
    for row in rows.flatten() {
        let (path, name, icon_path, game_version, loader, loader_version) = row;
        let source_path = profiles_dir.join(&path);
        if !source_path.exists() {
            continue;
        }

        let loader_type = loader
            .as_deref()
            .and_then(parse_loader)
            .unwrap_or(LoaderType::Vanilla);

        instances.push(ExternalInstance {
            name,
            game_version: game_version.unwrap_or_else(|| "unknown".to_string()),
            loader: loader_type,
            loader_version,
            source_path,
            icon_path: icon_path.map(PathBuf::from),
            launcher: LauncherKind::ModrinthApp,
        });
    }

    Ok(instances)
}

/// Also try to read metadata directly from a profile.json file
/// (legacy Modrinth App format, before v0.8.0).
pub fn list_from_profile_json(profiles_dir: &Path) -> Result<Vec<ExternalInstance>> {
    let mut instances = Vec::new();
    if !profiles_dir.exists() {
        return Ok(instances);
    }

    for entry in std::fs::read_dir(profiles_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let profile_path = entry.path().join("profile.json");
        if !profile_path.exists() {
            continue;
        }

        if let Some(meta) = super::detect::from_profile_json(&entry.path()) {
            let icon = meta.icon_path.clone();
            instances.push(ExternalInstance {
                name: meta.name.unwrap_or_else(|| entry.file_name().to_string_lossy().to_string()),
                game_version: meta.game_version.unwrap_or_else(|| "unknown".to_string()),
                loader: meta.loader.unwrap_or(LoaderType::Vanilla),
                loader_version: meta.loader_version,
                source_path: entry.path(),
                icon_path: icon,
                launcher: LauncherKind::ModrinthApp,
            });
        }
    }

    Ok(instances)
}

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

    #[test]
    fn detect_modrinth_app_dir_nonexistent() {
        let fake = std::env::temp_dir().join("no-such-dir-ctm-test");
        assert!(!is_modrinth_app_dir(&fake));
    }
}
