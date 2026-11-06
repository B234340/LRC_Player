//! Persisting a lyrics offset back into an `.lrc` file, safely.
//!
//! [`save_lrc_offset`] bakes the effective offset (`[offset:]` tag already in
//! the file + the user's adjustment) into every `[mm:ss.xx]` / `<mm:ss.xx>`
//! timestamp, removes the `[offset:]` line (it is now folded in), keeps a copy
//! of the previous file as `<name>.bak`, and replaces the file atomically
//! (temp file in the same directory + fsync + rename). Readers never see a
//! half-written file, and a failure leaves the original untouched.

use crate::infrastructure::lrc_timeline::parse_offset_value;
use anyhow::{Context, Result};
use lrc_player_adapters_persistence::persistence::state_saver::write_atomic;
use std::fs;
use std::path::{Path, PathBuf};

/// Result of [`save_lrc_offset`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LrcOffsetSave {
    /// Copy of the file as it was before this save (None if nothing was written).
    pub backup_path: Option<PathBuf>,
    /// Number of timestamp tags rewritten.
    pub shifted_tags: usize,
    /// Shift applied to every timestamp (ms; = -(file offset + user offset)).
    pub applied_shift_ms: i64,
    /// Whether an `[offset:]` line was removed.
    pub removed_offset_tag: bool,
}

/// `song.lrc` -> `song.lrc.bak`.
pub fn lrc_backup_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".bak");
    path.with_file_name(name)
}

/// Bake `user_offset_ms` (same convention as `LyricsTimeline::user_offset_ms`:
/// positive = lyrics earlier) plus any `[offset:]` tag into the timestamps of
/// the `.lrc` at `path`. After this, loading the file with a user offset of 0
/// shows lyrics exactly as they were shown before saving.
///
/// Nothing is written when the effective offset is 0 and there is no
/// `[offset:]` tag.
pub fn save_lrc_offset<P: AsRef<Path>>(path: P, user_offset_ms: i64) -> Result<LrcOffsetSave> {
    let path = path.as_ref();
    let bytes = fs::read(path).with_context(|| format!("reading lyrics file {}", path.display()))?;
    let contents = String::from_utf8(bytes)
        .with_context(|| format!("lyrics file {} is not valid UTF-8; not modifying it", path.display()))?;

    let (without_tag, file_offset_ms) = strip_offset_tags(&contents);
    let removed_offset_tag = file_offset_ms.is_some();
    let total = file_offset_ms.unwrap_or(0).saturating_add(user_offset_ms);
    let applied_shift_ms = total.saturating_neg();

    if total == 0 && !removed_offset_tag {
        return Ok(LrcOffsetSave {
            backup_path: None,
            shifted_tags: 0,
            applied_shift_ms: 0,
            removed_offset_tag: false,
        });
    }

    let (shifted, shifted_tags) = shift_lrc_timestamps(&without_tag, applied_shift_ms);
    let backup_path = replace_file_with_backup(path, shifted.as_bytes())?;
    Ok(LrcOffsetSave {
        backup_path: Some(backup_path),
        shifted_tags,
        applied_shift_ms,
        removed_offset_tag,
    })
}

/// Copy `path` to `<path>.bak` (overwriting an older backup), then atomically
/// replace `path` with `contents`. Returns the backup path.
pub fn replace_file_with_backup(path: &Path, contents: &[u8]) -> Result<PathBuf> {
    let backup = lrc_backup_path(path);
    fs::copy(path, &backup)
        .with_context(|| format!("backing up {} to {}", path.display(), backup.display()))?;
    if let Ok(file) = fs::File::open(&backup) {
        let _ = file.sync_all();
    }
    write_atomic(path, contents)?;
    Ok(backup)
}

/// Remove whole-line `[offset:...]` tags; returns the text and the first
/// offset value found.
fn strip_offset_tags(contents: &str) -> (String, Option<i64>) {
    let mut out = String::with_capacity(contents.len());
    let mut offset = None;
    for line in contents.split_inclusive('\n') {
        let trimmed = line.trim().trim_start_matches('\u{feff}');
        let value = trimmed
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .and_then(|s| s.split_once(':'))
            .filter(|(key, _)| key.trim().eq_ignore_ascii_case("offset"))
            .and_then(|(_, value)| parse_offset_value(value));
        match value {
            Some(ms) => {
                offset.get_or_insert(ms);
                // Keep a BOM if the tag line carried it.
                if line.starts_with('\u{feff}') {
                    out.push('\u{feff}');
                }
            }
            None => out.push_str(line),
        }
    }
    (out, offset)
}

/// Shift every `[mm:ss(.f{1,3})]` and `<mm:ss(.f{1,3})>` timestamp by
/// `shift_ms` (clamped at 0), keeping the original precision when possible.
/// Returns the new text and the number of rewritten tags. Non-timestamp tags
/// (`[ti:..]`, `[ar:..]`, …) are untouched.
pub fn shift_lrc_timestamps(contents: &str, shift_ms: i64) -> (String, usize) {
    let mut out = String::with_capacity(contents.len() + 16);
    let mut count = 0usize;
    let mut i = 0usize;
    while i < contents.len() {
        let rest = &contents[i..];
        let close = match rest.as_bytes()[0] {
            b'[' => Some(']'),
            b'<' => Some('>'),
            _ => None,
        };
        if let Some(close) = close {
            if let Some(end) = rest[1..].find(close) {
                let tag = &rest[1..1 + end];
                if let Some((ms, frac_digits)) = parse_strict_timestamp(tag) {
                    let shifted = (ms.min(i64::MAX as u64) as i64).saturating_add(shift_ms).max(0) as u64;
                    out.push_str(&rest[..1]);
                    out.push_str(&format_timestamp(shifted, frac_digits));
                    out.push(close);
                    count += 1;
                    i += 1 + end + close.len_utf8();
                    continue;
                }
            }
        }
        let ch = rest.chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    (out, count)
}

/// Digits-only `mm:ss` / `mm:ss.f{1,3}` with seconds < 60.
fn parse_strict_timestamp(tag: &str) -> Option<(u64, usize)> {
    let (minutes, seconds_part) = tag.split_once(':')?;
    if minutes.is_empty() || !minutes.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (seconds, frac) = match seconds_part.split_once('.') {
        Some((s, f)) => (s, Some(f)),
        None => (seconds_part, None),
    };
    if seconds.is_empty() || seconds.len() > 2 || !seconds.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let seconds: u64 = seconds.parse().ok()?;
    if seconds >= 60 {
        return None;
    }
    let (millis, digits) = match frac {
        None => (0, 0),
        Some(f) if !f.is_empty() && f.len() <= 3 && f.bytes().all(|b| b.is_ascii_digit()) => {
            let value: u64 = f.parse().ok()?;
            (value * 10u64.pow(3 - f.len() as u32), f.len())
        }
        Some(_) => return None,
    };
    Some((minutes.parse::<u64>().ok()? * 60_000 + seconds * 1_000 + millis, digits))
}

/// Format with the original number of fractional digits when that is exact,
/// otherwise with millisecond precision (never lose precision).
fn format_timestamp(ms: u64, frac_digits: usize) -> String {
    let minutes = ms / 60_000;
    let seconds = (ms / 1_000) % 60;
    let millis = ms % 1_000;
    if frac_digits == 0 && millis == 0 {
        format!("{minutes:02}:{seconds:02}")
    } else if frac_digits <= 2 && millis % 10 == 0 {
        format!("{minutes:02}:{seconds:02}.{:02}", millis / 10)
    } else {
        format!("{minutes:02}:{seconds:02}.{millis:03}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::lrc_timeline::LyricsTimeline;
    use tempfile::tempdir;

    #[test]
    fn shift_keeps_precision_and_ignores_id_tags() {
        let src = "[ar:x]\n[00:01.50]a<00:02.123>b\n[01:00]c\n";
        let (out, n) = shift_lrc_timestamps(src, 250);
        assert_eq!(n, 3);
        assert_eq!(out, "[ar:x]\n[00:01.75]a<00:02.373>b\n[01:00.25]c\n");
        let (out, _) = shift_lrc_timestamps("[00:00.10]a\n[00:01.00]b", -255);
        assert_eq!(out, "[00:00.00]a\n[00:00.745]b");
    }

    #[test]
    fn save_offset_bakes_tag_and_user_offset_with_backup() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("song.lrc");
        let original = "[ti:T]\r\n[offset:+200]\r\n[00:10.00]Line\r\n[00:12.00]<00:12.00>Ka<00:12.50>ra\r\n";
        fs::write(&path, original).unwrap();

        let mut before = LyricsTimeline::from_lrc(&path).unwrap();
        before.set_user_offset_ms(300);
        let shown_before: Vec<Option<usize>> = (0..14_000).step_by(37).map(|p| before.line_index_at(p)).collect();

        let report = save_lrc_offset(&path, 300).unwrap();
        assert_eq!(report.applied_shift_ms, -500);
        assert_eq!(report.shifted_tags, 4);
        assert!(report.removed_offset_tag);
        let backup = report.backup_path.unwrap();
        assert_eq!(backup, dir.path().join("song.lrc.bak"));
        assert_eq!(fs::read_to_string(&backup).unwrap(), original);

        let saved = fs::read_to_string(&path).unwrap();
        assert_eq!(saved, "[ti:T]\r\n[00:09.50]Line\r\n[00:11.50]<00:11.50>Ka<00:12.00>ra\r\n");

        // Same display with user offset 0 after saving.
        let after = LyricsTimeline::from_lrc(&path).unwrap();
        assert_eq!(after.total_offset_ms(), 0);
        let shown_after: Vec<Option<usize>> = (0..14_000).step_by(37).map(|p| after.line_index_at(p)).collect();
        assert_eq!(shown_before, shown_after);

        // No stray temp files.
        let names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
    }

    #[test]
    fn zero_offset_without_tag_writes_nothing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.lrc");
        fs::write(&path, "[00:01.00]x\n").unwrap();
        let report = save_lrc_offset(&path, 0).unwrap();
        assert_eq!(report.backup_path, None);
        assert!(!lrc_backup_path(&path).exists());
    }

    #[test]
    fn non_utf8_file_is_left_untouched() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sjis.lrc");
        let bytes = b"[00:01.00]\x82\xa0\n".to_vec();
        fs::write(&path, &bytes).unwrap();
        assert!(save_lrc_offset(&path, 100).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert!(!lrc_backup_path(&path).exists());
    }
}
