//! Java runtime discovery and version probing.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

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
}
