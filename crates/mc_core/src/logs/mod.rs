//! Log parsing, filtering and crash-report analysis.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::instance::Instance;

/// Severity of a log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
    Unknown,
}

impl LogLevel {
    pub fn parse(value: &str) -> Self {
        match value.to_ascii_uppercase().as_str() {
            "TRACE" => LogLevel::Trace,
            "DEBUG" => LogLevel::Debug,
            "INFO" => LogLevel::Info,
            "WARN" | "WARNING" => LogLevel::Warn,
            "ERROR" | "SEVERE" => LogLevel::Error,
            "FATAL" => LogLevel::Fatal,
            _ => LogLevel::Unknown,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            LogLevel::Trace => "TRACE",
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
            LogLevel::Fatal => "FATAL",
            LogLevel::Unknown => "LOG",
        }
    }

    /// Lower is more verbose. Used for "minimum level" filtering.
    pub fn severity(&self) -> u8 {
        match self {
            LogLevel::Trace => 0,
            LogLevel::Debug => 1,
            LogLevel::Info => 2,
            LogLevel::Warn => 3,
            LogLevel::Error => 4,
            LogLevel::Fatal => 5,
            LogLevel::Unknown => 2,
        }
    }
}

/// A parsed log line.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub level: LogLevel,
    pub timestamp: Option<String>,
    pub thread: Option<String>,
    pub target: Option<String>,
    pub message: String,
    pub raw: String,
}

/// Remove ANSI/VT escape sequences and stray control bytes from a raw log line.
///
/// Games and wrappers sometimes emit coloured output or control codes; leaving
/// them in place corrupts the TUI buffer with stray bytes.
pub fn strip_ansi(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\x1b' {
            // Escape sequence: handle CSI / OSC / two-char escapes.
            let Some(next) = chars.get(i + 1) else {
                break;
            };
            i += 2;
            if *next == '[' {
                // CSI sequence: drop until the final byte (0x40..0x7E).
                while i < chars.len() {
                    let b = chars[i];
                    i += 1;
                    if matches!(b, '@'..='~') {
                        break;
                    }
                }
            } else if *next == ']' {
                // OSC sequence: drop until BEL or ST (ESC \).
                while i < chars.len() {
                    let b = chars[i];
                    i += 1;
                    if b == '\x07' {
                        break;
                    }
                    if b == '\x1b' {
                        if chars.get(i).cloned() == Some('\\') {
                            i += 1;
                        }
                        break;
                    }
                }
            } else if *next >= ' ' && *next <= '/' {
                // ESC followed by an intermediate byte (e.g. `(B`): consume the
                // final byte too.
                if i < chars.len() {
                    i += 1;
                }
            }
            // Any other two-character escape is simply dropped.
            continue;
        }
        // Drop all other C0 control bytes (carriage returns, bell, ...) and
        // DEL, but keep horizontal tab and newline.
        if ('\x00'..='\x1f').contains(&c) && c != '\t' && c != '\n' || c == '\x7f' {
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

fn log_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^\[(?P<time>[^\]]*)\]\s+\[(?P<thread>[^/\]]+)/(?P<level>[A-Za-z]+)\](?:\s+\[(?P<target>[^\]]*)\])?\s*:?\s*(?P<msg>.*)$",
        )
        .expect("valid log regex")
    })
}

/// Parse a raw log line into a structured entry.
///
/// ANSI escapes are stripped first so control bytes never reach the UI.
/// Falls back to a plain [`LogLevel::Unknown`] entry when the line does not
/// match the standard Minecraft/Log4j layout.
pub fn parse_line(raw: &str) -> LogEntry {
    let clean = strip_ansi(raw);
    if let Some(caps) = log_regex().captures(&clean) {
        let get = |name: &str| caps.name(name).map(|m| m.as_str().to_string());
        return LogEntry {
            level: LogLevel::parse(&get("level").unwrap_or_default()),
            timestamp: get("time"),
            thread: get("thread"),
            target: get("target").filter(|s| !s.is_empty()),
            message: get("msg").unwrap_or_default(),
            raw: clean,
        };
    }
    LogEntry {
        level: LogLevel::Unknown,
        timestamp: None,
        thread: None,
        target: None,
        message: clean.clone(),
        raw: clean,
    }
}

/// Predicate used to select which log entries the UI displays.
#[derive(Debug, Clone)]
pub struct LogFilter {
    pub min_level: LogLevel,
    /// Case-insensitive substring search over the message/target.
    pub search: Option<String>,
}

impl Default for LogFilter {
    fn default() -> Self {
        Self {
            min_level: LogLevel::Info,
            search: None,
        }
    }
}

impl LogFilter {
    pub fn all() -> Self {
        Self {
            min_level: LogLevel::Trace,
            search: None,
        }
    }

    /// Whether an entry passes the filter.
    pub fn matches(&self, entry: &LogEntry) -> bool {
        if entry.level.severity() < self.min_level.severity() {
            return false;
        }
        if let Some(query) = &self.search {
            if query.is_empty() {
                return true;
            }
            let query = query.to_ascii_lowercase();
            let haystack = format!(
                "{} {} {}",
                entry.message,
                entry.target.clone().unwrap_or_default(),
                entry.raw
            )
            .to_ascii_lowercase();
            if !haystack.contains(&query) {
                return false;
            }
        }
        true
    }
}

/// A bounded ring buffer of parsed log entries with filtering and pause.
#[derive(Debug)]
pub struct LogBuffer {
    entries: VecDeque<LogEntry>,
    capacity: usize,
    pub filter: LogFilter,
    pub paused: bool,
}

impl LogBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity.min(4096)),
            capacity,
            filter: LogFilter::default(),
            paused: false,
        }
    }

    /// Push an already-parsed entry (dropped while paused).
    pub fn push(&mut self, entry: LogEntry) {
        if self.paused {
            return;
        }
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    /// Parse and push a raw line.
    pub fn push_line(&mut self, raw: &str) {
        self.push(parse_line(raw));
    }

    /// Iterate entries that pass the current filter.
    pub fn visible(&self) -> impl Iterator<Item = &LogEntry> {
        let filter = &self.filter;
        self.entries.iter().filter(move |e| filter.matches(e))
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Count of entries at or above `ERROR`.
    pub fn error_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.level.severity() >= LogLevel::Error.severity())
            .count()
    }
}

/// A detected Java version mismatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JavaMismatch {
    /// The class file version found in the error, mapped to a Java major.
    pub required_java: Option<u32>,
    /// The maximum class file version the running JVM understands, mapped to Java.
    pub running_java: Option<u32>,
    pub detail: String,
}

/// Structured result of analyzing a crash report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CrashAnalysis {
    pub headline: String,
    pub causes: Vec<String>,
    pub recommendations: Vec<String>,
    pub missing_dependencies: Vec<String>,
    pub java_mismatch: Option<JavaMismatch>,
    pub out_of_memory: bool,
    pub graphics_issue: bool,
    pub duplicate_mods: Vec<String>,
    pub stack_frames: Vec<String>,
}

impl CrashAnalysis {
    /// Whether any actionable cause was identified.
    pub fn has_findings(&self) -> bool {
        !self.causes.is_empty()
    }
}

/// Map a JVM class-file major version to a Java release.
pub fn class_file_to_java(class_file: u32) -> Option<u32> {
    match class_file {
        45..=48 => Some(8),
        49 => Some(5),
        50 => Some(6),
        51 => Some(7),
        52 => Some(8),
        53 => Some(9),
        54 => Some(10),
        55 => Some(11),
        56 => Some(12),
        57 => Some(13),
        58 => Some(14),
        59 => Some(15),
        60 => Some(16),
        61 => Some(17),
        62 => Some(18),
        63 => Some(19),
        64 => Some(20),
        65 => Some(21),
        66 => Some(22),
        67 => Some(23),
        _ => None,
    }
}

/// Analyze a crash report or game log for common failure signatures.
pub fn analyze_crash(text: &str) -> CrashAnalysis {
    let mut analysis = CrashAnalysis::default();
    let lower = text.to_ascii_lowercase();

    // --- Java version mismatch -----------------------------------------
    if let Some(caps) = Regex::new(
        r"class file version (?P<found>[0-9]+)(?:\.0)?[^0-9]+(?:only recognizes class file versions up to|up to) (?P<max>[0-9]+)(?:\.0)?",
    )
    .ok()
    .and_then(|re| re.captures(text))
    {
        let found: u32 = caps["found"].parse().unwrap_or(0);
        let max: u32 = caps["max"].parse().unwrap_or(0);
        let required_java = class_file_to_java(found);
        let running_java = class_file_to_java(max);
        analysis.java_mismatch = Some(JavaMismatch {
            required_java,
            running_java,
            detail: format!(
                "compiled for Java {} but the running JVM is Java {}",
                required_java.map(|j| j.to_string()).unwrap_or_else(|| format!("class file {found}")),
                running_java.map(|j| j.to_string()).unwrap_or_else(|| format!("class file {max}")),
            ),
        });
        analysis.causes.push("Java version mismatch".into());
        analysis.recommendations.push(
            "Select a newer Java runtime in Settings (or point the instance at the right JDK)."
                .into(),
        );
    }

    // --- Out of memory ---------------------------------------------------
    if lower.contains("java.lang.outofmemoryerror") {
        analysis.out_of_memory = true;
        analysis.causes.push("Out of memory".into());
        analysis
            .recommendations
            .push("Increase the instance's maximum RAM allocation in Settings.".into());
    }

    // --- Missing dependencies -------------------------------------------
    if lower.contains("missing or unsupported mandatory dependencies")
        || lower.contains("modresolutionexception")
        || lower.contains("incompatible mod set")
    {
        analysis
            .causes
            .push("Missing or incompatible mod dependencies".into());
        analysis.recommendations.push(
            "Install the missing mods with the Mod Manager, or remove the conflicting mod.".into(),
        );
        let dep_re = Regex::new(r"(?i)(?:Mod ID|requires|missing)[^'\n]*'([^']+)'").ok();
        if let Some(re) = dep_re {
            for caps in re.captures_iter(text) {
                if let Some(id) = caps.get(1) {
                    let id = id.as_str().to_string();
                    if !analysis.missing_dependencies.contains(&id) {
                        analysis.missing_dependencies.push(id);
                    }
                }
            }
        }
    }

    // --- Graphics / drivers ----------------------------------------------
    for needle in [
        "pixel format not accelerated",
        "the driver does not appear to support opengl",
        "glfw error 65542",
        "failed to create window",
    ] {
        if lower.contains(needle) {
            analysis.graphics_issue = true;
            analysis
                .causes
                .push("Graphics/OpenGL driver problem".into());
            analysis.recommendations.push(
                "Update your GPU drivers or try a different OpenGL/LWJGL configuration.".into(),
            );
            break;
        }
    }

    // --- Duplicate mods ---------------------------------------------------
    if lower.contains("duplicate mods") || lower.contains("duplicate mod") {
        analysis.causes.push("Duplicate mods installed".into());
        analysis
            .recommendations
            .push("Remove one of each duplicated mod from the Mods view.".into());
        if let Ok(re) = Regex::new(r"(?i)duplicate mods?[:\s]+(.+)") {
            if let Some(caps) = re.captures(text) {
                analysis.duplicate_mods.push(caps[1].trim().to_string());
            }
        }
    }

    // --- Mixin failures ---------------------------------------------------
    if lower.contains("mixin apply failed") || lower.contains("mixin transformation") {
        analysis.causes.push("Mixin injection failure".into());
        analysis.recommendations.push(
            "A mod is incompatible with this loader version; update or remove the offending mod."
                .into(),
        );
    }

    // --- Stack frames -----------------------------------------------------
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("at ") && analysis.stack_frames.len() < 64 {
            analysis.stack_frames.push(trimmed.to_string());
        }
    }

    // --- Headline ---------------------------------------------------------
    analysis.headline = if analysis.causes.is_empty() {
        "No known crash signature detected".to_string()
    } else {
        analysis.causes[0].clone()
    };

    analysis
}

/// Locate the most recently modified crash report for an instance.
pub fn find_latest_crash_report(instance: &Instance) -> Result<Option<PathBuf>> {
    let dir = instance.crash_reports_dir();
    if !dir.exists() {
        return Ok(None);
    }
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("txt") {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        if newest.as_ref().map(|(t, _)| modified > *t).unwrap_or(true) {
            newest = Some((modified, path));
        }
    }
    Ok(newest.map(|(_, p)| p))
}

/// Read a crash report file as UTF-8 text.
pub async fn read_crash_report(path: impl AsRef<Path>) -> Result<String> {
    Ok(tokio::fs::read_to_string(path.as_ref()).await?)
}

/// Analyze the newest crash report for an instance, if present.
pub fn analyze_latest(instance: &Instance) -> Result<Option<CrashAnalysis>> {
    match find_latest_crash_report(instance)? {
        Some(path) => {
            let text = std::fs::read_to_string(path)?;
            Ok(Some(analyze_crash(&text)))
        }
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_line() {
        let entry = parse_line("[12:34:56] [main/INFO]: Loading Minecraft");
        assert_eq!(entry.level, LogLevel::Info);
        assert_eq!(entry.thread.as_deref(), Some("main"));
        assert_eq!(entry.message, "Loading Minecraft");
    }

    #[test]
    fn parses_line_with_target() {
        let entry = parse_line(
            "[12:34:56] [Render thread/WARN] [net.minecraft.client.Minecraft/]: Something odd",
        );
        assert_eq!(entry.level, LogLevel::Warn);
        assert_eq!(entry.thread.as_deref(), Some("Render thread"));
        assert_eq!(
            entry.target.as_deref(),
            Some("net.minecraft.client.Minecraft/")
        );
        assert_eq!(entry.message, "Something odd");
    }

    #[test]
    fn strips_ansi_escapes() {
        let entry = parse_line("\x1b[31m\x1b[1m[12:34:56] [main/INFO]: \x1b[0mcoloured");
        assert_eq!(entry.message, "coloured");
        assert!(!entry.raw.contains('\x1b'));
        assert_eq!(entry.level, LogLevel::Info);
    }

    #[test]
    fn strips_stray_control_bytes() {
        let clean = strip_ansi("line one\rline two\x07\x1b(B");
        assert_eq!(clean, "line oneline two");
    }

    #[test]
    fn unparsed_lines_are_kept() {
        let entry = parse_line("just some raw output");
        assert_eq!(entry.level, LogLevel::Unknown);
        assert_eq!(entry.message, "just some raw output");
    }

    #[test]
    fn filter_by_level_and_search() {
        let mut filter = LogFilter {
            min_level: LogLevel::Warn,
            search: Some("crash".into()),
        };
        let info = parse_line("[00:00:00] [main/INFO]: crash");
        let warn = parse_line("[00:00:00] [main/WARN]: a crash occurred");
        let error_other = parse_line("[00:00:00] [main/ERROR]: something else");
        assert!(!filter.matches(&info));
        assert!(filter.matches(&warn));
        assert!(!filter.matches(&error_other));
        filter.search = None;
        assert!(filter.matches(&error_other));
    }

    #[test]
    fn buffer_is_bounded() {
        let mut buffer = LogBuffer::new(3);
        for i in 0..10 {
            buffer.push_line(&format!("line {i}"));
        }
        assert_eq!(buffer.len(), 3);
    }

    #[test]
    fn class_file_mapping() {
        assert_eq!(class_file_to_java(52), Some(8));
        assert_eq!(class_file_to_java(61), Some(17));
        assert_eq!(class_file_to_java(65), Some(21));
    }

    #[test]
    fn detects_java_mismatch() {
        let text = "java.lang.UnsupportedClassVersionError: foo has been compiled by a more recent version of the Java Runtime (class file version 61.0), this version of the Java Runtime only recognizes class file versions up to 52.0";
        let analysis = analyze_crash(text);
        let mismatch = analysis.java_mismatch.unwrap();
        assert_eq!(mismatch.required_java, Some(17));
        assert_eq!(mismatch.running_java, Some(8));
    }

    #[test]
    fn detects_oom_and_missing_deps() {
        let text = "java.lang.OutOfMemoryError: Java heap space\nMissing or unsupported mandatory dependencies:\n\tMod ID: 'sodium', Requested by: 'iris'";
        let analysis = analyze_crash(text);
        assert!(analysis.out_of_memory);
        assert!(analysis.missing_dependencies.iter().any(|d| d == "sodium"));
    }
}
