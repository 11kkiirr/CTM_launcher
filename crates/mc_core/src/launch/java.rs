//! Java runtime discovery, probing and Mojang runtime provisioning.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::util::{download_file, ensure_dir, fetch_json, Paths, Progress, ProgressCallback};
use crate::version::DownloadArtifact;

/// Mojang's Java runtime manifest (same one the official launcher uses).
pub const JAVA_RUNTIME_MANIFEST_URL: &str =
    "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

/// A discovered Java runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JavaInstallation {
    pub path: PathBuf,
    /// Full version string, e.g. `21.0.1` or `1.8.0_392`.
    pub version: String,
    /// Major version, e.g. `21` or `8`.
    pub major: u32,
    /// Vendor string when detectable, e.g. `Temurin`.
    pub vendor: Option<String>,
}

impl JavaInstallation {
    pub fn label(&self) -> String {
        match &self.vendor {
            Some(v) => format!("Java {} ({v}) — {}", self.version, self.path.display()),
            None => format!("Java {} — {}", self.version, self.path.display()),
        }
    }

    /// Whether this runtime can run a version requiring `required` major.
    pub fn satisfies(&self, required: u32) -> bool {
        // Java 8 is reported as 8; runtimes are backwards compatible from 8 up
        // for our purposes, but never below the requirement.
        self.major >= required
    }
}

/// Return the executable name for the platform.
fn java_exe_name() -> &'static str {
    if cfg!(windows) {
        "java.exe"
    } else {
        "java"
    }
}

/// Probe a candidate `java` executable and read its version.
pub async fn probe(path: impl AsRef<Path>) -> Result<JavaInstallation> {
    let path = path.as_ref().to_path_buf();
    let output = tokio::process::Command::new(&path)
        .arg("-version")
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .output()
        .await
        .map_err(|e| CoreError::Launch(format!("failed to run {}: {e}", path.display())))?;

    // `java -version` writes to stderr on most distributions.
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let version = parse_version_string(&text).ok_or_else(|| {
        CoreError::Launch(format!(
            "could not parse java version for {}",
            path.display()
        ))
    })?;
    let major = parse_major(&version)
        .ok_or_else(|| CoreError::Launch(format!("could not parse java major for {version}")))?;
    let vendor = parse_vendor(&text);

    Ok(JavaInstallation {
        path,
        version,
        major,
        vendor,
    })
}

/// Extract the quoted version token from `java -version` output.
pub fn parse_version_string(output: &str) -> Option<String> {
    // e.g. openjdk version "21.0.1" 2023-10-17
    if let Some(start) = output.find('"') {
        if let Some(end) = output[start + 1..].find('"') {
            return Some(output[start + 1..start + 1 + end].to_string());
        }
    }
    None
}

/// Derive the major version from a Java version string.
pub fn parse_major(version: &str) -> Option<u32> {
    let version = version.trim();
    if let Some(rest) = version.strip_prefix("1.") {
        // Legacy scheme: 1.8.0_392 -> 8
        return rest.split(['.', '_']).next()?.parse().ok();
    }
    version.split(['.', '_', '-']).next()?.parse().ok()
}

fn parse_vendor(output: &str) -> Option<String> {
    let lower = output.to_ascii_lowercase();
    let known = [
        ("temurin", "Temurin"),
        ("zulu", "Zulu"),
        ("graalvm", "GraalVM"),
        ("corretto", "Corretto"),
        ("microsoft", "Microsoft"),
        ("oracle", "Oracle"),
        ("adoptopenjdk", "AdoptOpenJDK"),
        ("openjdk", "OpenJDK"),
    ];
    for (needle, label) in known {
        if lower.contains(needle) {
            return Some(label.to_string());
        }
    }
    None
}

/// Directories commonly containing JDK/JRE installations.
fn candidate_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if cfg!(windows) {
        for base in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
            if let Ok(pf) = std::env::var(base) {
                roots.push(PathBuf::from(&pf).join("Java"));
                roots.push(PathBuf::from(&pf).join("Eclipse Adoptium"));
                roots.push(PathBuf::from(&pf).join("Microsoft"));
                roots.push(PathBuf::from(&pf).join("Zulu"));
            }
        }
    } else if cfg!(target_os = "macos") {
        roots.push(PathBuf::from("/Library/Java/JavaVirtualMachines"));
        if let Ok(home) = std::env::var("HOME") {
            roots.push(PathBuf::from(&home).join("Library/Java/JavaVirtualMachines"));
        }
    } else {
        roots.push(PathBuf::from("/usr/lib/jvm"));
        roots.push(PathBuf::from("/usr/java"));
        roots.push(PathBuf::from("/opt/java"));
        if let Ok(home) = std::env::var("HOME") {
            roots.push(PathBuf::from(&home).join(".sdkman/candidates/java"));
            roots.push(PathBuf::from(&home).join(".jdks"));
        }
    }
    // Runtimes provisioned by the launcher itself.
    if let Ok(dirs) = crate::util::project_dirs() {
        roots.push(dirs.data_dir().join("java"));
    }
    roots
}

/// Recursively look for `bin/java` beneath the well-known roots (bounded depth).
fn find_under(root: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth == 0 || out.len() > 64 {
        return;
    }
    let direct = root.join("bin").join(java_exe_name());
    if direct.exists() {
        out.push(direct);
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            find_under(&entry.path(), depth - 1, out);
        }
    }
}

/// Discover every Java runtime reachable on this machine.
pub async fn discover() -> Vec<JavaInstallation> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    // Explicit override wins.
    if let Ok(home) = std::env::var("JAVA_HOME") {
        candidates.push(PathBuf::from(home).join("bin").join(java_exe_name()));
    }

    // PATH lookup.
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(java_exe_name());
            if candidate.exists() {
                candidates.push(candidate);
            }
        }
    }

    for root in candidate_roots() {
        find_under(&root, 3, &mut candidates);
    }

    let mut installations = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for candidate in candidates {
        let canonical = std::fs::canonicalize(&candidate).unwrap_or(candidate.clone());
        if !seen.insert(canonical.clone()) {
            continue;
        }
        if let Ok(inst) = probe(&canonical).await {
            installations.push(inst);
        }
    }
    installations.sort_by_key(|j| std::cmp::Reverse(j.major));
    installations
}

/// Pick the best runtime satisfying `required_major`, if any.
pub async fn find_for_major(required_major: u32) -> Option<JavaInstallation> {
    let all = discover().await;
    all.into_iter().find(|j| j.satisfies(required_major))
}

// -------------------------------------------------------------------------
// Mojang runtime provisioning
// -------------------------------------------------------------------------

/// The Mojang runtime component name for a Java major version.
pub fn component_for_major(major: u32) -> &'static str {
    match major {
        0..=8 => "jre-legacy",
        9..=16 => "java-runtime-alpha",
        17 => "java-runtime-gamma",
        18..=21 => "java-runtime-delta",
        _ => "java-runtime-epsilon",
    }
}

/// The Mojang runtime manifest platform key for this machine.
pub fn platform_key() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86") => "linux-i386",
        ("linux", _) => "linux",
        ("windows", "x86") => "windows-x86",
        ("windows", "aarch64") => "windows-arm64",
        ("windows", _) => "windows-x64",
        ("macos", "aarch64") => "mac-os-arm64",
        ("macos", _) => "mac-os",
        _ => "linux",
    }
}

#[derive(Debug, Clone, Deserialize)]
struct RuntimeAvailability {
    #[serde(default)]
    progress: u32,
}

impl RuntimeAvailability {
    fn is_available(&self) -> bool {
        self.progress > 0
    }
}

#[derive(Debug, Clone, Deserialize)]
struct RuntimeVersion {
    #[serde(default)]
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct RuntimeEntry {
    manifest: DownloadArtifact,
    #[serde(default)]
    version: Option<RuntimeVersion>,
    #[serde(default)]
    availability: Option<RuntimeAvailability>,
}

type RuntimeManifest = HashMap<String, HashMap<String, Vec<RuntimeEntry>>>;

#[derive(Debug, Clone, Deserialize)]
struct RuntimeDownloads {
    #[serde(default)]
    raw: Option<DownloadArtifact>,
}

#[derive(Debug, Clone, Deserialize)]
struct RuntimeFile {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    downloads: Option<RuntimeDownloads>,
    #[serde(default)]
    executable: Option<bool>,
    #[serde(default)]
    target: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RuntimeFiles {
    #[serde(default)]
    files: HashMap<String, RuntimeFile>,
}

/// Ensure the Mojang-provided Java runtime for `component` (and `major`) is
/// installed locally, downloading it if necessary.
pub async fn ensure_runtime(
    client: &reqwest::Client,
    paths: &Paths,
    component: Option<&str>,
    major: u32,
    progress: Option<ProgressCallback>,
) -> Result<JavaInstallation> {
    let component = component
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| component_for_major(major));
    let root = paths.java_dir().join(component);
    let exe = root.join("bin").join(java_exe_name());

    if exe.exists() {
        if let Ok(inst) = probe(&exe).await {
            if inst.satisfies(major) {
                return Ok(inst);
            }
        }
    }

    let manifest: RuntimeManifest = fetch_json(client, JAVA_RUNTIME_MANIFEST_URL).await?;
    let platform = platform_key();
    let entry = manifest
        .get(platform)
        .and_then(|platforms| platforms.get(component))
        .and_then(|entries| {
            entries
                .iter()
                .find(|e| {
                    e.availability
                        .as_ref()
                        .map(|a| a.is_available())
                        .unwrap_or(true)
                })
                .or_else(|| entries.first())
        })
        .ok_or_else(|| {
            CoreError::Launch(format!(
                "Mojang runtime '{component}' is unavailable for {platform}"
            ))
        })?;

    if let Some(cb) = &progress {
        let name = entry
            .version
            .as_ref()
            .map(|v| v.name.clone())
            .unwrap_or_default();
        cb(Progress::new(format!(
            "Downloading Java {name} ({component})..."
        )));
    }

    let files: RuntimeFiles = fetch_json(client, &entry.manifest.url).await?;

    // Directories and symlinks first; files are downloaded concurrently.
    for (rel, file) in &files.files {
        let Ok(rel_path) = crate::util::sanitize_archive_path(Path::new(rel)) else {
            continue;
        };
        let dest = root.join(&rel_path);
        match file.kind.as_str() {
            "directory" => {
                ensure_dir(&dest).await?;
            }
            "link" => {
                if let Some(target) = &file.target {
                    let _ = create_link(&dest, target);
                }
            }
            _ => {}
        }
    }

    let entries: Vec<(String, RuntimeFile)> = files
        .files
        .into_iter()
        .filter(|(_, file)| file.kind == "file")
        .collect();
    let total = entries.len();
    let done = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let results: Vec<Result<()>> = stream::iter(entries)
        .map(|(rel, file)| {
            let root = root.clone();
            let client = client.clone();
            let progress = progress.clone();
            let done = done.clone();
            async move {
                let rel_path = crate::util::sanitize_archive_path(Path::new(&rel))?;
                let dest = root.join(rel_path);
                let Some(raw) = file.downloads.as_ref().and_then(|d| d.raw.as_ref()) else {
                    return Ok(());
                };
                let sha1 = (!raw.sha1.is_empty()).then_some(raw.sha1.as_str());
                download_file(&client, &raw.url, &dest, sha1, None).await?;
                if file.executable.unwrap_or(false) {
                    set_executable(&dest);
                }
                let count = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                if let Some(cb) = &progress {
                    if count % 25 == 0 || count == total {
                        cb(Progress {
                            current: count as u64,
                            total: Some(total as u64),
                            message: format!("Java runtime {count}/{total}"),
                        });
                    }
                }
                Ok(())
            }
        })
        .buffer_unordered(16)
        .collect()
        .await;

    for result in results {
        result?;
    }

    probe(&exe).await
}

#[cfg(unix)]
fn create_link(dest: &Path, target: &str) -> std::io::Result<()> {
    use std::os::unix::fs::symlink;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if dest.symlink_metadata().is_ok() {
        let _ = std::fs::remove_file(dest);
    }
    symlink(target, dest)
}

#[cfg(not(unix))]
fn create_link(dest: &Path, target: &str) -> std::io::Result<()> {
    let resolved = dest.parent().unwrap_or_else(|| Path::new(".")).join(target);
    if resolved.is_dir() {
        std::fs::create_dir_all(dest)
    } else {
        std::fs::copy(&resolved, dest).map(|_| ())
    }
}

#[cfg(unix)]
fn set_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        perms.set_mode(perms.mode() | 0o755);
        let _ = std::fs::set_permissions(path, perms);
    }
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modern_version() {
        let out = "openjdk version \"21.0.1\" 2023-10-17\nOpenJDK Runtime Environment";
        assert_eq!(parse_version_string(out).as_deref(), Some("21.0.1"));
        assert_eq!(parse_major("21.0.1"), Some(21));
    }

    #[test]
    fn parses_legacy_version() {
        let out = "java version \"1.8.0_392\"\nJava(TM) SE Runtime Environment";
        assert_eq!(parse_version_string(out).as_deref(), Some("1.8.0_392"));
        assert_eq!(parse_major("1.8.0_392"), Some(8));
    }

    #[test]
    fn detects_vendor() {
        assert_eq!(
            parse_vendor("OpenJDK 64-Bit Server VM Temurin-21").as_deref(),
            Some("Temurin")
        );
        assert_eq!(
            parse_vendor("openjdk version 21").as_deref(),
            Some("OpenJDK")
        );
    }

    #[test]
    fn maps_java_major_to_component() {
        assert_eq!(component_for_major(8), "jre-legacy");
        assert_eq!(component_for_major(17), "java-runtime-gamma");
        assert_eq!(component_for_major(21), "java-runtime-delta");
        assert_eq!(component_for_major(25), "java-runtime-epsilon");
    }

    #[test]
    fn platform_key_is_known() {
        let key = platform_key();
        assert!(
            [
                "linux",
                "linux-i386",
                "windows-x64",
                "windows-x86",
                "windows-arm64",
                "mac-os",
                "mac-os-arm64"
            ]
            .contains(&key),
            "unexpected platform key: {key}"
        );
    }

    #[test]
    fn parses_runtime_manifest_shapes() {
        let raw = r#"{
            "linux": {
                "java-runtime-epsilon": [{
                    "manifest": {"sha1":"abc","size":1,"url":"https://example/manifest.json"},
                    "version": {"name":"25.0.1","released":"2025-12-10T14:20:17+00:00"},
                    "availability": {"group":3012,"progress":100}
                }]
            }
        }"#;
        let manifest: RuntimeManifest = serde_json::from_str(raw).unwrap();
        let entry = &manifest["linux"]["java-runtime-epsilon"][0];
        assert!(entry.availability.as_ref().unwrap().is_available());
        assert_eq!(entry.version.as_ref().unwrap().name, "25.0.1");
    }
}
