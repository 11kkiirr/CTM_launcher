//! Modrinth API v2 models.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A project (mod, resourcepack, shader, modpack...) as returned by search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub project_id: String,
    #[serde(default)]
    pub project_type: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub author: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub display_categories: Vec<String>,
    #[serde(default)]
    pub versions: Vec<String>,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub follows: u64,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub date_created: String,
    #[serde(default)]
    pub date_modified: String,
    #[serde(default)]
    pub latest_version: Option<String>,
    #[serde(default)]
    pub client_side: String,
    #[serde(default)]
    pub server_side: String,
    #[serde(default)]
    pub color: Option<u32>,
}

/// Paginated search response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResults {
    #[serde(default)]
    pub hits: Vec<SearchHit>,
    #[serde(default)]
    pub offset: u32,
    #[serde(default)]
    pub limit: u32,
    #[serde(default)]
    pub total_hits: u32,
}

/// A full project document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    #[serde(default)]
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub project_type: String,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub additional_categories: Vec<String>,
    #[serde(default)]
    pub client_side: String,
    #[serde(default)]
    pub server_side: String,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub followers: u64,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub color: Option<u32>,
    #[serde(default)]
    pub issues_url: Option<String>,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub wiki_url: Option<String>,
    #[serde(default)]
    pub discord_url: Option<String>,
    #[serde(default)]
    pub game_versions: Vec<String>,
    #[serde(default)]
    pub loaders: Vec<String>,
    #[serde(default)]
    pub versions: Vec<String>,
    #[serde(default)]
    pub published: String,
    #[serde(default)]
    pub updated: String,
    #[serde(default)]
    pub license: Option<License>,
    #[serde(default)]
    pub gallery: Vec<GalleryImage>,
}

/// A team member of a project (`/project/{id}/members`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub user: MemberUser,
}

/// The user behind a team membership.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemberUser {
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub avatar_url: Option<String>,
}

/// Compact a Modrinth `game_versions` list for display: snapshots and
/// pre-releases are dropped, bare `1.21` becomes `1.21.x`, and consecutive
/// patches collapse into ranges (`1.21.4-1.21.8`).
pub fn compact_game_versions(versions: &[String]) -> Vec<String> {
    let mut parsed: Vec<(u64, u64, Option<u64>)> = Vec::new();
    for v in versions {
        let parts: Vec<&str> = v.split('.').collect();
        let nums: Option<Vec<u64>> = parts.iter().map(|p| p.parse().ok()).collect();
        let Some(nums) = nums else { continue };
        match nums.as_slice() {
            [major, minor] => parsed.push((*major, *minor, None)),
            [major, minor, patch] => parsed.push((*major, *minor, Some(*patch))),
            _ => continue,
        }
    }
    parsed.sort();
    parsed.dedup();

    let mut out = Vec::new();
    let mut i = 0;
    while i < parsed.len() {
        let (major, minor, _) = parsed[i];
        let mut j = i;
        while j < parsed.len() && parsed[j].0 == major && parsed[j].1 == minor {
            j += 1;
        }
        let group = &parsed[i..j];
        if group.iter().any(|(_, _, p)| p.is_none()) {
            out.push(format!("{major}.{minor}.x"));
        } else {
            let mut patches: Vec<u64> = group.iter().map(|(_, _, p)| p.unwrap_or(0)).collect();
            patches.sort();
            patches.dedup();
            let mut start = patches[0];
            let mut prev = patches[0];
            for &p in &patches[1..] {
                if p == prev + 1 {
                    prev = p;
                } else {
                    push_patch_run(&mut out, major, minor, start, prev);
                    start = p;
                    prev = p;
                }
            }
            push_patch_run(&mut out, major, minor, start, prev);
        }
        i = j;
    }
    out
}

fn push_patch_run(out: &mut Vec<String>, major: u64, minor: u64, start: u64, end: u64) {
    if start == end {
        out.push(format!("{major}.{minor}.{start}"));
    } else {
        out.push(format!("{major}.{minor}.{start}-{major}.{minor}.{end}"));
    }
}

/// A screenshot/preview attached to a project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GalleryImage {
    /// 350px CDN thumbnail — fine for the strip, too small for the preview.
    pub url: String,
    /// Original full-resolution upload; prefer it for the large preview.
    /// Falls back to [`Self::url`] when the API omits it.
    #[serde(default)]
    pub raw_url: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub featured: bool,
}

impl GalleryImage {
    /// Best URL for a full-size render: the original upload when present.
    pub fn full_url(&self) -> &str {
        self.raw_url.as_deref().unwrap_or(&self.url)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct License {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
}

/// A specific project version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Version {
    pub id: String,
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version_number: String,
    #[serde(default)]
    pub changelog: Option<String>,
    #[serde(default)]
    pub date_published: String,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub version_type: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub files: Vec<VersionFile>,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
    #[serde(default)]
    pub game_versions: Vec<String>,
    #[serde(default)]
    pub loaders: Vec<String>,
}

impl Version {
    /// The primary file, falling back to the first available.
    pub fn primary_file(&self) -> Option<&VersionFile> {
        self.files
            .iter()
            .find(|f| f.primary)
            .or_else(|| self.files.first())
    }

    /// Required dependency project ids.
    pub fn required_dependencies(&self) -> Vec<&Dependency> {
        self.dependencies
            .iter()
            .filter(|d| d.dependency_type == DependencyType::Required)
            .collect()
    }
}

/// A downloadable file attached to a version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionFile {
    #[serde(default)]
    pub hashes: HashMap<String, String>,
    pub url: String,
    pub filename: String,
    #[serde(default)]
    pub primary: bool,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub file_type: Option<String>,
}

impl VersionFile {
    pub fn sha1(&self) -> Option<&str> {
        self.hashes.get("sha1").map(String::as_str)
    }

    pub fn sha512(&self) -> Option<&str> {
        self.hashes.get("sha512").map(String::as_str)
    }
}

/// How a version depends on another project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DependencyType {
    Required,
    Optional,
    Incompatible,
    Embedded,
}

/// A dependency edge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    #[serde(default)]
    pub version_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub file_name: Option<String>,
    pub dependency_type: DependencyType,
}

/// A locally installed mod file discovered in an instance's `mods/` folder.
#[derive(Debug, Clone)]
pub struct InstalledMod {
    pub path: std::path::PathBuf,
    pub file_name: String,
    pub enabled: bool,
    /// SHA-1 of the file contents.
    pub sha1: String,
    pub size: u64,
    /// Human-readable mod name parsed from jar manifest (e.g. "Sodium").
    pub mod_name: String,
    /// Mod version parsed from jar manifest (e.g. "0.5.8").
    pub version: String,
    /// Mod id from `fabric.mod.json` / `quilt.mod.json` (e.g. "sodium").
    /// Often matches the Modrinth slug; empty when not present.
    pub mod_id: String,
    /// File modification time used as install/update date (YYYY-MM-DD HH:MM).
    pub install_date: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn versions(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn compacts_consecutive_patches_into_ranges() {
        assert_eq!(
            compact_game_versions(&versions(&["1.21.4", "1.21.8", "1.21.5", "1.21.6", "1.21.7"])),
            vec!["1.21.4-1.21.8"]
        );
        assert_eq!(
            compact_game_versions(&versions(&["1.20.1", "1.20.4", "1.20.6", "1.20.5"])),
            vec!["1.20.1", "1.20.4-1.20.6"]
        );
    }

    #[test]
    fn drops_snapshots_and_prereleases() {
        assert_eq!(
            compact_game_versions(&versions(&["24w14a", "1.20.1", "1.21.1", "1.20.1-rc1", "3D Shareware v1.34"])),
            vec!["1.20.1", "1.21.1"]
        );
    }

    #[test]
    fn bare_minor_becomes_x() {
        assert_eq!(
            compact_game_versions(&versions(&["1.21", "1.21.4"])),
            vec!["1.21.x"]
        );
    }

    #[test]
    fn parses_members() {
        let raw = r#"[{"role":"Owner","user":{"username":"jellysquid3","avatar_url":"https://cdn.modrinth.com/u.png"}}]"#;
        let members: Vec<Member> = serde_json::from_str(raw).unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].role, "Owner");
        assert_eq!(members[0].user.username, "jellysquid3");
    }

    #[test]
    fn gallery_prefers_raw_url_for_full_size() {
        let raw = r#"{"url":"https://cdn.modrinth.com/data/x/images/h_350.webp","raw_url":"https://cdn.modrinth.com/data/x/images/h.png","description":"d","featured":true}"#;
        let img: GalleryImage = serde_json::from_str(raw).unwrap();
        assert_eq!(img.full_url(), "https://cdn.modrinth.com/data/x/images/h.png");

        let bare = r#"{"url":"https://cdn.modrinth.com/data/x/images/h_350.webp"}"#;
        let img: GalleryImage = serde_json::from_str(bare).unwrap();
        assert_eq!(img.full_url(), img.url);
    }
}
