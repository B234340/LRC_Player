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

                    let actual_path = fs::canonicalize(&candidate).unwrap_or_else(|_| candidate.clone());
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn parse_timestamp_formats() {
        assert_eq!(parse_timestamp("00:00.00"), Some(0));
        assert_eq!(parse_timestamp("01:02.345"), Some(62345));
        assert_eq!(parse_timestamp("10:20.5"), Some(620500));
    }

    #[test]
    fn loads_lrc_contents() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("song.lrc");
        let mut file = File::create(&path).unwrap();
        writeln!(file, "[00:00.00]Intro").unwrap();
        writeln!(file, "[00:10.50]Verse 1").unwrap();

        let lyrics = Lyrics::from_lrc(&path).unwrap();
        assert_eq!(lyrics.lines.len(), 2);
        assert_eq!(lyrics.lines[0].text, "Intro");
        assert_eq!(lyrics.lines[1].time_ms, 10_500);
    }

    #[test]
    fn discovers_lrc_in_same_folder() {
        let dir = tempdir().unwrap();
        let track = dir.path().join("track.mp3");
        File::create(&track).unwrap();
        let lyric = dir.path().join("track.lrc");
        File::create(&lyric).unwrap();

        let found = Lyrics::discover_for_track(&track);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].file_name().unwrap(), "track.lrc");
    }
}
