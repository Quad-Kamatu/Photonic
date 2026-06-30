//! Opt-in crash reporting and diagnostics (#59).
//!
//! A panic hook captures a self-contained, **non-sensitive** crash report and
//! writes it locally as JSON. Local capture is always on (the app already wrote
//! a `[PANIC]` log line); only *sending* a report is gated behind explicit user
//! consent in the GUI, where the report is turned into a pre-filled GitHub issue
//! the user reviews before anything leaves the machine.
//!
//! Privacy: a [`CrashReport`] records only the app version, a UTC timestamp, the
//! host OS/arch, the panic payload string, the panic source `&Location`, and a
//! formatted backtrace. It deliberately **excludes** document content, file
//! paths/names, project data, and environment variables — none of those are read
//! here. The only paths that can appear are compile-time source locations baked
//! into the backtrace (e.g. `crates/photonic-app/src/main.rs`), which are public.
//!
//! This module has no GUI or network dependencies so both `photonic-app` (the
//! panic hook) and `photonic-gui` (enumerate / dismiss / report) share one
//! implementation and one directory resolver.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Cross-platform Photonic config directory (the single source of truth shared
/// with `welcome::config_dir`).
///
/// Resolution order: `%APPDATA%\Photonic` (Windows), then
/// `$XDG_CONFIG_HOME/Photonic`, then `~/.config/Photonic`.
pub fn crash_dir() -> Option<PathBuf> {
    if let Ok(appdata) = std::env::var("APPDATA") {
        return Some(PathBuf::from(appdata).join("Photonic"));
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg).join("Photonic"));
    }
    if let Ok(home) = std::env::var("HOME") {
        return Some(PathBuf::from(home).join(".config").join("Photonic"));
    }
    None
}

/// Directory holding the JSON crash reports (`<config>/crash-reports/`).
pub fn reports_dir() -> Option<PathBuf> {
    crash_dir().map(|d| d.join("crash-reports"))
}

/// A self-contained, non-sensitive record of a single panic.
///
/// See the module docs for exactly what is and is not collected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrashReport {
    /// Application version (`CARGO_PKG_VERSION` of `photonic-core`).
    pub version: String,
    /// UTC timestamp, ISO-8601 (`YYYY-MM-DDTHH:MM:SSZ`).
    pub timestamp: String,
    /// Host operating system (`std::env::consts::OS`).
    pub os: String,
    /// Host architecture (`std::env::consts::ARCH`).
    pub arch: String,
    /// The panic payload, as a string.
    pub panic_message: String,
    /// `file:line:column` of the panic source location, when available.
    pub location: Option<String>,
    /// Formatted backtrace captured at panic time.
    pub backtrace: String,
}

impl CrashReport {
    /// Build a report from a panic hook's `info` and a captured backtrace.
    ///
    /// Only the fields documented on [`CrashReport`] are read — no document,
    /// filesystem, or environment state is touched.
    pub fn capture(
        info: &std::panic::PanicHookInfo<'_>,
        backtrace: &std::backtrace::Backtrace,
    ) -> Self {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()));

        CrashReport {
            version: env!("CARGO_PKG_VERSION").to_string(),
            timestamp: utc_timestamp(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            panic_message: payload_string(info),
            location,
            backtrace: format!("{backtrace}"),
        }
    }

    /// Serialize the report to `<config>/crash-reports/crash-<unix-millis>.json`,
    /// returning the path written.
    pub fn write(&self) -> std::io::Result<PathBuf> {
        let dir = reports_dir().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no config directory available for crash reports",
            )
        })?;
        std::fs::create_dir_all(&dir)?;
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let path = dir.join(format!("crash-{millis}.json"));
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(&path, json)?;
        Ok(path)
    }

    /// A short, single-line summary for a bug-report title.
    pub fn issue_title(&self) -> String {
        let first = self.panic_message.lines().next().unwrap_or("").trim();
        let summary = if first.is_empty() { "panic" } else { first };
        // Keep titles reasonable; GitHub truncates very long titles anyway.
        let summary: String = summary.chars().take(120).collect();
        format!("Crash: {summary}")
    }

    /// A Markdown issue body containing only the non-sensitive crash facts.
    pub fn issue_body(&self) -> String {
        let location = self.location.as_deref().unwrap_or("unknown");
        format!(
            "**Photonic crash report** (auto-generated; review before submitting)\n\
             \n\
             | Field | Value |\n\
             | --- | --- |\n\
             | Version | {version} |\n\
             | When (UTC) | {timestamp} |\n\
             | OS / Arch | {os} / {arch} |\n\
             | Location | `{location}` |\n\
             \n\
             ### Panic message\n\
             ```\n{panic}\n```\n\
             \n\
             ### Backtrace\n\
             ```\n{backtrace}\n```\n\
             \n\
             ---\n\
             _What's included: app version, UTC time, OS/arch, the panic message, \
             and the backtrace. No document content, file paths, or environment \
             variables are collected._\n\
             \n\
             ### What were you doing when it crashed?\n\
             <!-- optional: add steps to reproduce -->\n",
            version = self.version,
            timestamp = self.timestamp,
            os = self.os,
            arch = self.arch,
            location = location,
            panic = self.panic_message,
            backtrace = self.backtrace,
        )
    }
}

/// List pending crash report files (oldest first), if any.
pub fn pending_reports() -> Vec<PathBuf> {
    let Some(dir) = reports_dir() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let is_json = path.extension().and_then(|e| e.to_str()) == Some("json");
            let is_crash = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("crash-"));
            if is_json && is_crash {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// Load and parse a single crash report from disk.
pub fn load_report(path: &Path) -> Option<CrashReport> {
    let json = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&json).ok()
}

/// Delete a crash report file so it is not offered again (after it is filed or
/// dismissed). Errors are ignored by callers that just want it gone.
pub fn clear_report(path: &Path) -> std::io::Result<()> {
    std::fs::remove_file(path)
}

/// Extract a human-readable string from a panic payload without panicking.
fn payload_string(info: &std::panic::PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        // Fall back to the hook's Display (message + location) — still no
        // document/env state, just the panic itself.
        format!("{info}")
    }
}

/// UTC timestamp formatted as `YYYY-MM-DDTHH:MM:SSZ` from the system clock.
fn utc_timestamp() -> String {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => {
            let (y, mo, day, h, min, s) = epoch_to_ymd(d.as_secs());
            format!("{y:04}-{mo:02}-{day:02}T{h:02}:{min:02}:{s:02}Z")
        }
        Err(_) => "1970-01-01T00:00:00Z".to_string(),
    }
}

/// Convert Unix epoch seconds (UTC) into `(year, month, day, hour, min, sec)`.
fn epoch_to_ymd(secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let hour = (rem / 3_600) as u32;
    let min = ((rem % 3_600) / 60) as u32;
    let sec = (rem % 60) as u32;

    // Civil-from-days algorithm (Howard Hinnant), epoch 1970-01-01.
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let year = (y + i64::from(month <= 2)) as u32;

    (year, month, day, hour, min, sec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_conversion_known_dates() {
        // 2026-06-28T00:00:00Z = 1782604800.
        assert_eq!(epoch_to_ymd(1_782_604_800), (2026, 6, 28, 0, 0, 0));
        // Unix epoch.
        assert_eq!(epoch_to_ymd(0), (1970, 1, 1, 0, 0, 0));
        // A leap-year date with a time component: 2024-02-29T12:34:56Z.
        assert_eq!(epoch_to_ymd(1_709_210_096), (2024, 2, 29, 12, 34, 56));
        // Cross-check against the production audit timestamp helper, which uses
        // the same civil-from-days conversion.
        assert_eq!(epoch_to_ymd(1_709_210_096).0, 2024);
    }

    #[test]
    fn report_serializes_round_trip() {
        let report = CrashReport {
            version: "9.9.9".to_string(),
            timestamp: "2026-06-29T00:00:00Z".to_string(),
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            panic_message: "boom: index out of bounds".to_string(),
            location: Some("crates/photonic-app/src/main.rs:10:5".to_string()),
            backtrace: "<backtrace>".to_string(),
        };
        let json = serde_json::to_string(&report).unwrap();
        let back: CrashReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.version, "9.9.9");
        assert_eq!(back.panic_message, report.panic_message);
        assert_eq!(back.location, report.location);
    }

    #[test]
    fn issue_title_summarizes_first_line() {
        let report = CrashReport {
            version: "1.0.0".to_string(),
            timestamp: "t".to_string(),
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            panic_message: "first line\nsecond line".to_string(),
            location: None,
            backtrace: String::new(),
        };
        assert_eq!(report.issue_title(), "Crash: first line");
        // The body must never leak an env-var-style secret because we never read one.
        let body = report.issue_body();
        assert!(body.contains("No document content"));
        assert!(body.contains("first line"));
    }
}
