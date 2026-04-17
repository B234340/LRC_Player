use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricLine {
    pub time_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lyrics {
    pub lines: Vec<LyricLine>,
}

impl Lyrics {
    pub fn from_lrc<P: AsRef<Path>>(path: P) -> Result<Self> {
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("reading lyrics file {}", path.as_ref().display()))?;
        let mut lines = Vec::new();

        for raw_line in contents.lines() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }

            if let Some(start) = line.find('[') {
                if let Some(end) = line[start..].find(']') {
                    let tag = &line[start + 1..start + end];
                    if let Some(ms) = parse_timestamp(tag) {
                        let text = line[start + end + 1..].trim().to_string();
                        lines.push(LyricLine { time_ms: ms, text });
                    }
                }
            }
        }

        lines.sort_by_key(|line| line.time_ms);
        Ok(Self { lines })
    }

    pub fn discover_for_track<P: AsRef<Path>>(track_path: P) -> Vec<PathBuf> {
        let mut results = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let track_path = track_path.as_ref();
        if let Some(parent) = track_path.parent() {
            if let Some(stem) = track_path.file_stem().and_then(|s| s.to_str()) {
                for ext in ["lrc", "LRC"] {
                    let candidate = parent.join(format!("{}.{}", stem, ext));
                    if !candidate.exists() {
                        continue;
                    }

                    let actual_path =
                        fs::canonicalize(&candidate).unwrap_or_else(|_| candidate.clone());
                    if seen.insert(actual_path.clone()) {
                        results.push(actual_path);
                    }
                }
            }
        }
        results
    }
}

fn parse_timestamp(tag: &str) -> Option<u64> {
    let mut parts = tag.splitn(2, ':');
    let minutes = parts.next()?.parse::<u64>().ok()?;
    let seconds_part = parts.next()?;
    let mut sec_parts = seconds_part.splitn(2, '.');
    let seconds = sec_parts.next()?.parse::<u64>().ok()?;
    let millis = match sec_parts.next() {
        Some(frac) => {
            let normalized = match frac.len() {
                0 => "000".to_string(),
                1 => format!("{}00", frac),
                2 => format!("{}0", frac),
                _ => frac.chars().take(3).collect(),
            };
            normalized.parse::<u64>().ok()?
        }
        None => 0,
    };
    Some(minutes * 60_000 + seconds * 1_000 + millis)
}


