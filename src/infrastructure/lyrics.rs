use crate::infrastructure::persistence::MetaCache;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LyricTagKind {
    Bracket,
    Angle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricSegment {
    pub time_ms: u64,
    pub text: String,
    pub tag_kind: LyricTagKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricLine {
    pub time_ms: u64,
    pub text: String,
    pub segments: Vec<LyricSegment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lyrics {
    pub lines: Vec<LyricLine>,
}

impl Lyrics {
    pub fn display_priority_rank(&self) -> u8 {
        if self.has_enhanced_segments() {
            3
        } else if self.has_char_level_segments() {
            2
        } else {
            1
        }
    }

    fn has_enhanced_segments(&self) -> bool {
        self.lines
            .iter()
            .any(|line| line.segments.iter().any(|seg| seg.tag_kind == LyricTagKind::Angle))
    }

    fn has_char_level_segments(&self) -> bool {
        self.lines.iter().any(|line| line.segments.len() > 1)
    }

    pub fn from_lrc<P: AsRef<Path>>(path: P) -> Result<Self> {
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("reading lyrics file {}", path.as_ref().display()))?;
        let mut lines = Vec::new();

        for raw_line in contents.lines() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }

            if let Some((ms, text, segments)) = Self::parse_timestamped_line(line) {
                lines.push(LyricLine { time_ms: ms, text, segments });
            }
        }

        lines.sort_by_key(|line| line.time_ms);
        if let Some(first_line) = lines.first() {
            if first_line.time_ms > 0 {
                lines.insert(
                    0,
                    LyricLine {
                        time_ms: 0,
                        text: String::new(),
                        segments: Vec::new(),
                    },
                );
            }
        }

        Ok(Self { lines })
    }

    fn parse_timestamped_line(line: &str) -> Option<(u64, String, Vec<LyricSegment>)> {
        let mut segments = Vec::new();
        let mut i = 0;

        while i < line.len() {
            let remaining = &line[i..];
            let (closing, tag_kind) = if remaining.starts_with('[') {
                (']', LyricTagKind::Bracket)
            } else if remaining.starts_with('<') {
                ('>', LyricTagKind::Angle)
            } else {
                break;
            };

            if let Some(end_rel) = remaining[1..].find(closing) {
                let tag = &remaining[1..1 + end_rel];
                if let Some(ms) = parse_timestamp(tag) {
                    let content_start = i + 1 + end_rel + closing.len_utf8();
                    let next_tag_offset = line[content_start..]
                        .find('[')
                        .or_else(|| line[content_start..].find('<'))
                        .map(|offset| content_start + offset)
                        .unwrap_or(line.len());
                    let segment_text = line[content_start..next_tag_offset].to_string();
                    segments.push(LyricSegment {
                        time_ms: ms,
                        text: segment_text,
                        tag_kind,
                    });
                    i = next_tag_offset;
                    continue;
                }
            }

            break;
        }

        if segments.is_empty() {
            return None;
        }

        let text = segments.iter().map(|segment| segment.text.clone()).collect();
        Some((segments[0].time_ms, text, segments))
    }

    pub fn discover_for_track<P: AsRef<Path>>(track_path: P, lrc_paths: &[String]) -> Vec<PathBuf> {
        fn push_unique(
            results: &mut Vec<PathBuf>,
            seen: &mut std::collections::HashSet<PathBuf>,
            path: PathBuf,
        ) {
            if seen.insert(path.clone()) {
                results.push(path);
            }
        }

        let track_path = track_path.as_ref();
        let mut local_sidecar: Option<PathBuf> = None;
        let mut results = Vec::new();
        let mut seen = std::collections::HashSet::new();

        if let Some(parent) = track_path.parent() {
            if let Some(stem) = track_path.file_stem().and_then(|s| s.to_str()) {
                for ext in ["lrc", "LRC"] {
                    let candidate = parent.join(format!("{}.{}", stem, ext));
                    if !candidate.exists() {
                        continue;
                    }

                    let actual_path = fs::canonicalize(&candidate).unwrap_or_else(|_| candidate.clone());
                    local_sidecar = Some(actual_path);
                    break;
                }
            }
        }

        let lrc_path_roots: Vec<PathBuf> = lrc_paths
            .iter()
            .filter_map(|path| {
                let path_buf = PathBuf::from(path);
                fs::canonicalize(&path_buf).ok().or(Some(path_buf))
            })
            .collect();

        let mut prioritized_meta_paths = Vec::new();
        let mut other_meta_paths = Vec::new();

        if let Ok(cache) = MetaCache::open_default() {
            if let Ok(db_paths) = cache.lyrics_for_track_path(track_path) {
                for path in db_paths.into_iter() {
                    let path_buf = PathBuf::from(path);
                    if !path_buf.exists() {
                        continue;
                    }

                    let path_buf = fs::canonicalize(&path_buf).unwrap_or_else(|_| path_buf.clone());
                    if !lrc_path_roots.is_empty() && lrc_path_roots.iter().any(|root| path_buf.starts_with(root)) {
                        prioritized_meta_paths.push(path_buf);
                    } else {
                        other_meta_paths.push(path_buf);
                    }
                }
            }
        }

        if !lrc_path_roots.is_empty() {
            for path in prioritized_meta_paths {
                push_unique(&mut results, &mut seen, path);
            }
            if results.is_empty() {
                if let Some(local) = &local_sidecar {
                    push_unique(&mut results, &mut seen, local.clone());
                }
            }
            for path in other_meta_paths {
                push_unique(&mut results, &mut seen, path);
            }
        } else {
            if let Some(local) = &local_sidecar {
                push_unique(&mut results, &mut seen, local.clone());
            }
            for path in prioritized_meta_paths {
                push_unique(&mut results, &mut seen, path);
            }
            for path in other_meta_paths {
                push_unique(&mut results, &mut seen, path);
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


