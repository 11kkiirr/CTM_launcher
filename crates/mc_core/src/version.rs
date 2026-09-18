//! Mojang version metadata models (`version_manifest_v2.json` and per-version
//! `*.json` documents), plus rule evaluation and Maven path helpers.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

/// Manifest URL for all published game versions.
pub const VERSION_MANIFEST_URL: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

/// Top-level version manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionManifest {
    pub latest: LatestVersions,
    pub versions: Vec<VersionSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatestVersions {
    pub release: String,
    pub snapshot: String,
}

/// A single entry in the version manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionSummary {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    #[serde(default)]
    pub time: String,
    #[serde(default, rename = "releaseTime")]
    pub release_time: String,
    #[serde(default)]
    pub sha1: String,
    #[serde(default, rename = "complianceLevel")]
    pub compliance_level: u32,
}

impl VersionSummary {
    /// `true` when this is a stable release (as opposed to a snapshot).
    pub fn is_release(&self) -> bool {
        self.kind == "release"
    }
}

/// A fully resolved version document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionDetails {
    pub id: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(rename = "mainClass", default)]
    pub main_class: String,
    #[serde(default)]
    pub assets: String,
    #[serde(rename = "assetIndex", default)]
    pub asset_index: Option<AssetIndexRef>,
    #[serde(default)]
    pub downloads: VersionDownloads,
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(default)]
    pub arguments: Option<Arguments>,
    #[serde(rename = "minecraftArguments", default)]
    pub minecraft_arguments: Option<String>,
    #[serde(rename = "javaVersion", default)]
    pub java_version: Option<JavaVersion>,
    #[serde(rename = "inheritsFrom", default)]
    pub inherits_from: Option<String>,
    #[serde(rename = "releaseTime", default)]
    pub release_time: Option<String>,
    #[serde(rename = "complianceLevel", default)]
    pub compliance_level: Option<u32>,
    #[serde(default)]
    pub logging: Option<Logging>,
}

impl VersionDetails {
    /// Download descriptor for the client jar.
    pub fn client_download(&self) -> Result<&DownloadArtifact> {
        self.downloads
            .client
            .as_ref()
            .ok_or_else(|| CoreError::Install(format!("version {} has no client jar", self.id)))
    }

    /// The Java major version this game version expects.
    pub fn required_java_major(&self) -> u32 {
        self.java_version
            .as_ref()
            .map(|j| j.major_version)
            .unwrap_or(8)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JavaVersion {
    #[serde(default)]
    pub component: String,
    #[serde(rename = "majorVersion", default)]
    pub major_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetIndexRef {
    pub id: String,
    #[serde(default)]
    pub sha1: String,
    #[serde(default)]
    pub size: u64,
    #[serde(rename = "totalSize", default)]
    pub total_size: u64,
    pub url: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VersionDownloads {
    #[serde(default)]
    pub client: Option<DownloadArtifact>,
    #[serde(default)]
    pub server: Option<DownloadArtifact>,
    #[serde(default, rename = "client_mappings")]
    pub client_mappings: Option<DownloadArtifact>,
}

/// A downloadable artifact with integrity metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadArtifact {
    #[serde(default)]
    pub sha1: String,
    #[serde(default)]
    pub size: u64,
    pub url: String,
    #[serde(default)]
    pub path: Option<String>,
}

/// A game library dependency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Library {
    pub name: String,
    #[serde(default)]
    pub downloads: LibDownloads,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub natives: Option<HashMap<String, String>>,
    #[serde(default)]
    pub extract: Option<Extract>,
    /// Legacy base URL for libraries without an explicit `downloads.artifact`.
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LibDownloads {
    #[serde(default)]
    pub artifact: Option<DownloadArtifact>,
    #[serde(default)]
    pub classifiers: Option<HashMap<String, DownloadArtifact>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Extract {
    #[serde(default)]
    pub exclude: Vec<String>,
}

/// A conditional rule controlling whether a library/argument applies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub action: String,
    #[serde(default)]
    pub os: Option<OsRule>,
    #[serde(default)]
    pub features: Option<HashMap<String, bool>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsRule {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub arch: Option<String>,
}

/// JVM/game argument lists for modern versions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Arguments {
    #[serde(default)]
    pub game: Vec<Argument>,
    #[serde(default)]
    pub jvm: Vec<Argument>,
}

/// Either a plain string argument or a rules-gated one.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Argument {
    Plain(String),
    Conditional {
        rules: Vec<Rule>,
        value: StringOrVec,
    },
}

/// A string or list of strings (Mojang uses both).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StringOrVec {
    One(String),
    Many(Vec<String>),
}

impl StringOrVec {
    pub fn into_vec(self) -> Vec<String> {
        match self {
            StringOrVec::One(s) => vec![s],
            StringOrVec::Many(v) => v,
        }
    }

    pub fn as_slice(&self) -> &[String] {
        match self {
            StringOrVec::One(s) => std::slice::from_ref(s),
            StringOrVec::Many(v) => v.as_slice(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Logging {
    #[serde(default)]
    pub client: Option<LoggingClient>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingClient {
    #[serde(default)]
    pub argument: String,
    #[serde(default)]
    pub file: Option<DownloadArtifact>,
    #[serde(rename = "type", default)]
    pub kind: String,
}

/// Context used to evaluate [`Rule`]s for the current machine.
#[derive(Debug, Clone)]
pub struct RuleContext {
    /// One of `windows`, `linux`, `osx`.
    pub os: String,
    pub arch: String,
    pub version: String,
    pub features: HashMap<String, bool>,
}

impl RuleContext {
    /// Build a context for the machine currently running the launcher.
    pub fn current() -> Self {
        let os = match std::env::consts::OS {
            "macos" => "osx",
            other => other,
        }
        .to_string();
        let arch = match std::env::consts::ARCH {
            "x86_64" => "x86_64",
            "aarch64" => "arm64",
            other => other,
        }
        .to_string();
        Self {
            os,
            arch,
            version: String::new(),
            features: HashMap::new(),
        }
    }

    pub fn with_feature(mut self, key: impl Into<String>, value: bool) -> Self {
        self.features.insert(key.into(), value);
        self
    }

    /// Whether the given rule list permits the element (defaults to allow).
    pub fn allows(&self, rules: &[Rule]) -> bool {
        if rules.is_empty() {
            return true;
        }
        let mut allowed = false;
        for rule in rules {
            if rule_matches(rule, self) {
                allowed = rule.action == "allow";
            }
        }
        allowed
    }
}

fn rule_matches(rule: &Rule, ctx: &RuleContext) -> bool {
    if let Some(os) = &rule.os {
        if let Some(name) = &os.name {
            if name != &ctx.os {
                return false;
            }
        }
        if let Some(arch) = &os.arch {
            if arch != &ctx.arch {
                return false;
            }
        }
        if let Some(version) = &os.version {
            // Mojang uses regexes here; fall back to substring match.
            let matched = regex::Regex::new(version)
                .map(|re| re.is_match(&ctx.version))
                .unwrap_or_else(|_| ctx.version.contains(version));
            if !matched {
                return false;
            }
        }
    }
    if let Some(features) = &rule.features {
        for (key, expected) in features {
            let actual = ctx.features.get(key).copied().unwrap_or(false);
            if actual != *expected {
                return false;
            }
        }
    }
    true
}

impl Library {
    /// Split a Maven coordinate `group:artifact:version[:classifier]`.
    pub fn coordinate(&self) -> Result<(&str, &str, &str, Option<&str>)> {
        let parts: Vec<&str> = self.name.split(':').collect();
        match parts.as_slice() {
            [group, artifact, version] => Ok((group, artifact, version, None)),
            [group, artifact, version, classifier] => {
                Ok((group, artifact, version, Some(classifier)))
            }
            _ => Err(CoreError::Install(format!(
                "invalid maven coordinate: {}",
                self.name
            ))),
        }
    }

    /// Relative Maven path for the main artifact (e.g. `com/foo/bar/1.0/bar-1.0.jar`).
    pub fn relative_path(&self) -> Result<String> {
        let (group, artifact, version, classifier) = self.coordinate()?;
        let group_path = group.replace('.', "/");
        let file = match classifier {
            Some(c) => format!("{artifact}-{version}-{c}.jar"),
            None => format!("{artifact}-{version}.jar"),
        };
        Ok(format!("{group_path}/{artifact}/{version}/{file}"))
    }

    /// Resolve the native classifier for the current OS, if any.
    pub fn native_classifier(&self, ctx: &RuleContext) -> Option<String> {
        let natives = self.natives.as_ref()?;
        let key = natives.get(&ctx.os)?;
        let arch = if ctx.arch == "x86" { "32" } else { "64" };
        Some(key.replace("${arch}", arch))
    }

    /// Whether this library applies on the current machine.
    pub fn applies(&self, ctx: &RuleContext) -> bool {
        ctx.allows(&self.rules)
    }

    /// Maven repository base URL for this library.
    pub fn base_url(&self) -> String {
        self.url
            .clone()
            .unwrap_or_else(|| "https://libraries.minecraft.net/".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_coordinate_and_path() {
        let lib = Library {
            name: "org.ow2.asm:asm:9.6".into(),
            downloads: LibDownloads::default(),
            rules: vec![],
            natives: None,
            extract: None,
            url: None,
        };
        assert_eq!(
            lib.relative_path().unwrap(),
            "org/ow2/asm/asm/9.6/asm-9.6.jar"
        );
    }

    #[test]
    fn rule_evaluation_allow_deny() {
        let ctx = RuleContext {
            os: "linux".into(),
            arch: "x86_64".into(),
            version: String::new(),
            features: HashMap::new(),
        };
        let allow_linux = vec![Rule {
            action: "allow".into(),
            os: Some(OsRule {
                name: Some("linux".into()),
                version: None,
                arch: None,
            }),
            features: None,
        }];
        assert!(ctx.allows(&allow_linux));

        let allow_windows = vec![Rule {
            action: "allow".into(),
            os: Some(OsRule {
                name: Some("windows".into()),
                version: None,
                arch: None,
            }),
            features: None,
        }];
        assert!(!ctx.allows(&allow_windows));
    }

    #[test]
    fn default_allow_without_rules() {
        let ctx = RuleContext::current();
        assert!(ctx.allows(&[]));
    }

    #[test]
    fn parses_conditional_arguments() {
        let raw = r#"{
            "game": ["--username", {"rules":[{"action":"allow","os":{"name":"linux"}}],"value":"--demo"}],
            "jvm": []
        }"#;
        let args: Arguments = serde_json::from_str(raw).unwrap();
        assert_eq!(args.game.len(), 2);
    }
}
