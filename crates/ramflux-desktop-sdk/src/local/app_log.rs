// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The application log: one redacted line per event in
//! `<log dir>/desktop.log`, rotated by size.
//!
//! The desktop logs only its own events (start, source, daemon launch and
//! exit, failures), never message bodies, keys or secrets. Every line still
//! goes through [`redact`] as a second line of defence: values of secret
//! looking fields, long opaque tokens and PEM blocks are replaced by
//! `[redacted]`.
//!
//! [`init`] opens the process-wide log; [`write`] (and the `info`/`warn`
//! helpers) append to it and also echo to standard error. Before `init`,
//! or without a log directory, lines go to standard error only.

use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// File name of the application log.
pub const LOG_FILE: &str = "desktop.log";

/// File name of the output of a daemon the desktop started.
pub const DAEMON_LOG_FILE: &str = "rfd.log";

/// A log file is rotated once it grows past this size.
pub const MAX_LOG_BYTES: u64 = 1024 * 1024;

/// Rotated files kept (`desktop.log.1` is the newest).
pub const KEEP_ROTATED: usize = 3;

/// Field names whose values are never logged.
const SECRET_FIELDS: [&str; 10] = [
    "secret",
    "token",
    "nonce",
    "passphrase",
    "password",
    "private",
    "key",
    "signature",
    "code",
    "body",
];

/// Shortest run of opaque token characters that is redacted.
const TOKEN_MIN_LEN: usize = 24;

struct Log {
    path: PathBuf,
    file: Mutex<Option<File>>,
}

static LOG: OnceLock<Log> = OnceLock::new();

/// Opens the process-wide log in `dir` (created if needed), rotating an
/// oversized file first. Later calls do nothing. Returns the log file.
pub fn init(dir: Option<PathBuf>) -> Option<PathBuf> {
    let dir = dir?;
    let log = LOG.get_or_init(|| {
        let path = dir.join(LOG_FILE);
        let file = std::fs::create_dir_all(&dir).ok().and_then(|()| open(&path));
        Log { path, file: Mutex::new(file) }
    });
    Some(log.path.clone())
}

/// The log file once [`init`] ran.
pub fn path() -> Option<PathBuf> {
    LOG.get().map(|log| log.path.clone())
}

fn open(path: &Path) -> Option<File> {
    rotate(path, MAX_LOG_BYTES, KEEP_ROTATED);
    File::options().create(true).append(true).open(path).ok()
}

/// Shifts `path` to `path.1` (and older files one further, keeping
/// `keep`) when it is larger than `max_bytes`.
pub fn rotate(path: &Path, max_bytes: u64, keep: usize) {
    let big = std::fs::metadata(path).is_ok_and(|m| m.len() > max_bytes);
    if !big || keep == 0 {
        return;
    }
    let numbered = |n: usize| {
        let mut name = path.as_os_str().to_owned();
        name.push(format!(".{n}"));
        PathBuf::from(name)
    };
    let _ = std::fs::remove_file(numbered(keep));
    for n in (1..keep).rev() {
        let _ = std::fs::rename(numbered(n), numbered(n + 1));
    }
    let _ = std::fs::rename(path, numbered(1));
}

/// Appends one redacted line at `level`, rotating when the file grew too
/// large; echoes it to standard error.
pub fn write(level: &str, message: &str) {
    let line = format!("{} {level} {}", timestamp(), redact(message));
    eprintln!("{line}");
    let Some(log) = LOG.get() else { return };
    let Ok(mut file) = log.file.lock() else { return };
    let too_big =
        file.as_ref().and_then(|f| f.metadata().ok()).is_some_and(|m| m.len() > MAX_LOG_BYTES);
    if too_big {
        *file = open(&log.path);
    }
    if let Some(file) = file.as_mut() {
        let _ = writeln!(file, "{line}");
    }
}

/// Logs at info level.
pub fn info(message: &str) {
    write("INFO", message);
}

/// Logs at warning level.
pub fn warn(message: &str) {
    write("WARN", message);
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '_' | '-')
}

fn looks_like_token(word: &str) -> bool {
    word.len() >= TOKEN_MIN_LEN
        && !word.starts_with('/')
        && word.chars().all(is_token_char)
        && word.chars().any(|c| c.is_ascii_digit())
        && word.chars().any(|c| c.is_ascii_alphabetic())
}

fn is_secret_field(name: &str) -> bool {
    let name = name.trim_matches(|c: char| !c.is_ascii_alphanumeric()).to_ascii_lowercase();
    !name.is_empty() && SECRET_FIELDS.iter().any(|field| name.contains(field))
}

fn redact_word(word: &str) -> String {
    for sep in ['=', ':'] {
        if let Some((name, value)) = word.split_once(sep)
            && !value.is_empty()
            && is_secret_field(name)
        {
            return format!("{name}{sep}[redacted]");
        }
    }
    let core = word.trim_matches(|c: char| matches!(c, '"' | '\'' | ',' | ';' | '(' | ')'));
    if looks_like_token(core) { word.replace(core, "[redacted]") } else { word.to_owned() }
}

/// `text` with secret field values, opaque tokens and PEM blocks replaced
/// by `[redacted]`.
pub fn redact(text: &str) -> String {
    let (text, pem) = match text.find("-----BEGIN") {
        Some(at) => (&text[..at], true),
        None => (text, false),
    };
    let mut out = String::with_capacity(text.len());
    let mut pending_field = false;
    for (i, word) in text.split(' ').enumerate() {
        if i > 0 {
            out.push(' ');
        }
        if pending_field && !word.is_empty() {
            out.push_str("[redacted]");
            pending_field = false;
            continue;
        }
        // `"key": "value"` splits into two words: redact the second.
        if (word.ends_with(':') || word.ends_with("\":")) && is_secret_field(word) {
            pending_field = true;
            out.push_str(word);
            continue;
        }
        out.push_str(&redact_word(word));
    }
    if pem {
        out.push_str("[redacted]");
    }
    out
}

/// UTC time as `YYYY-MM-DDTHH:MM:SSZ`.
fn timestamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rem = secs % 86_400;
    // Civil date from days since the epoch (proleptic Gregorian calendar).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_tokens_and_pem_are_redacted() {
        let line = redact(
            "unlock passphrase=hunter2 token: abc nonce=9f8e7d \
             id=4f9a2c7e1b3d5a6f8e0c2b4d6a8f1e3c path=/tmp/rf-1/rfd.sock \
             cert -----BEGIN PRIVATE KEY----- MIIE",
        );
        for secret in ["hunter2", "abc", "9f8e7d", "4f9a2c7e1b3d5a6f8e0c2b4d6a8f1e3c", "MIIE"] {
            assert!(!line.contains(secret), "{secret} leaked: {line}");
        }
        assert!(line.contains("passphrase=[redacted]"));
        assert!(line.contains("path=/tmp/rf-1/rfd.sock"), "paths stay readable: {line}");
        assert_eq!(redact("daemon started pid=4242"), "daemon started pid=4242");
        let json = redact(r#"{"key": "c2VjcmV0IGtleSBtYXRlcmlhbA", "node": "n-1"}"#);
        assert!(!json.contains("c2VjcmV0"), "{json}");
        assert!(json.contains("\"n-1\""), "{json}");
    }

    #[test]
    fn rotation_keeps_a_bounded_number_of_files() {
        let dir = std::env::temp_dir().join(format!("rf-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join(LOG_FILE);
        for round in 0..5 {
            std::fs::write(&path, format!("round {round} {}", "x".repeat(64))).expect("write");
            rotate(&path, 16, 2);
        }
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .expect("dir")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["desktop.log.1", "desktop.log.2"]);
        let newest = std::fs::read_to_string(dir.join("desktop.log.1")).expect("newest");
        assert!(newest.starts_with("round 4"));
        rotate(&dir.join("missing.log"), 16, 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn timestamps_are_utc_iso_dates() {
        let stamp = timestamp();
        assert_eq!(stamp.len(), 20, "{stamp}");
        assert!(stamp.starts_with("20") && stamp.ends_with('Z'), "{stamp}");
    }
}
