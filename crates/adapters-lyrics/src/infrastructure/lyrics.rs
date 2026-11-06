use crate::infrastructure::lrc_timeline::{
    is_line_only_enhanced_segments, tokenize_lrc, RawEntry, RawSegment,
};
use lrc_player_adapters_persistence::persistence::MetaCache;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub use crate::infrastructure::lrc_file::{
    lrc_backup_path, replace_file_with_backup, save_lrc_offset, shift_lrc_timestamps, LrcOffsetSave,
};
pub use crate::infrastructure::lrc_timeline::{
    LrcMetadata, LyricPosition, LyricsTimeline, TimedLine, TimedSegment, DEFAULT_LAST_LINE_MS,
};

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
    /// Computed from the raw file before trailing end markers were dropped
    /// from `lines` (see `from_entries`), so the discovery tie-break keeps
    /// working for `[t] text<t>`-style files.
    line_only_enhanced: bool,
}

/// A discovered lyrics file, parsed once, with its ranking keys.
/// Produced by [`Lyrics::discover_ranked`] / [`Lyrics::rank_candidates`].
#[derive(Debug, Clone)]
pub struct LyricsCandidate {
    pub path: PathBuf,
    /// `display_priority_rank()`; 0 when the file could not be read.
    pub rank: u8,
    pub line_only_enhanced: bool,
    /// Legacy representation (what the current overlay draws).
    pub lyrics: Option<Lyrics>,
    /// Frame-cheap representation for the new UI (offset tag applied at query time).
    pub timeline: Option<LyricsTimeline>,
    pub error: Option<String>,
}

impl Lyrics {
    /// Build from already-parsed lines (no synthetic line is added).
    pub fn new(lines: Vec<LyricLine>) -> Self {
        Self {
            lines,
            line_only_enhanced: false,
        }
    }

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
        self.lines.iter().any(|line| {
            line.segments
                .iter()
                .filter(|seg| !seg.text.trim().is_empty())
                .filter(|seg| seg.tag_kind == LyricTagKind::Angle)
                .count()
                > 1
        })
    }

    fn has_char_level_segments(&self) -> bool {
        self.lines.iter().any(|line| {
            line.segments
                .iter()
                .filter(|seg| !seg.text.trim().is_empty())
                .count()
                > 1
        })
    }

    pub fn is_line_only_enhanced(&self) -> bool {
        self.line_only_enhanced
            || self.lines.iter().any(|line| {
                line.segments.len() > 1
                    && line.segments.iter().filter(|seg| !seg.text.trim().is_empty()).count() == 1
                    && line.segments.iter().any(|seg| seg.tag_kind == LyricTagKind::Angle)
            })
    }

    pub fn from_lrc<P: AsRef<Path>>(path: P) -> Result<Self> {
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("reading lyrics file {}", path.as_ref().display()))?;
        Ok(Self::parse_str(&contents))
    }

    /// Parse LRC text (same result as `from_lrc` on a file with this content).
    ///
    /// Note: when the first lyric starts after 0 ms an empty line at 0 ms is
    /// inserted (the overlay relies on it to show a blank line before the
    /// first lyric). `LyricsTimeline` does not do this.
    pub fn parse_str(contents: &str) -> Self {
        Self::from_entries(&tokenize_lrc(contents))
    }

    fn from_entries(entries: &[RawEntry]) -> Self {
        let raw_lines: Vec<&[RawSegment]> = entries
            .iter()
            .filter_map(|entry| match entry {
                RawEntry::Line(segments) => Some(segments.as_slice()),
                RawEntry::Meta { .. } => None,
            })
            .collect();
        let line_only_enhanced = raw_lines.iter().any(|segs| is_line_only_enhanced_segments(segs));

        let mut lines: Vec<LyricLine> = raw_lines.iter().map(|segs| Self::legacy_line(segs)).collect();

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

        Self {
            lines,
            line_only_enhanced,
        }
    }

    fn legacy_line(raw: &[RawSegment]) -> LyricLine {
        let mut segments: Vec<LyricSegment> = raw
            .iter()
            .map(|seg| LyricSegment {
                time_ms: seg.time_ms,
                text: seg.text.clone(),
                tag_kind: seg.kind.clone(),
            })
            .collect();

        // A line with a single piece of text followed by empty timestamp tags
        // (`[00:14.189] text<00:14.189>`) is line-based: the trailing tags are
        // end markers, not karaoke segments. Keeping them made the overlay
        // treat the line as karaoke (segments.len() > 1).
        let non_blank = segments.iter().filter(|seg| !seg.text.trim().is_empty()).count();
        if non_blank == 1 {
            while segments.len() > 1 && segments.last().is_some_and(|seg| seg.text.trim().is_empty()) {
                segments.pop();
            }
        }

        let text = segments.iter().map(|segment| segment.text.as_str()).collect();
        LyricLine {
            time_ms: segments[0].time_ms,
            text,
            segments,
        }
    }

    /// Read + parse each path exactly once and sort by (rank desc,
    /// line-only-enhanced last, file name) — the same order the GUI's
    /// discovery comparator produced, without re-parsing inside the sort.
    pub fn rank_candidates<I>(paths: I) -> Vec<LyricsCandidate>
    where
        I: IntoIterator<Item = PathBuf>,
    {
        let mut candidates: Vec<LyricsCandidate> = paths
            .into_iter()
            .map(|path| match fs::read_to_string(&path) {
                Ok(contents) => {
                    let entries = tokenize_lrc(&contents);
                    let lyrics = Lyrics::from_entries(&entries);
                    let timeline = LyricsTimeline::from_entries(entries);
                    LyricsCandidate {
                        rank: lyrics.display_priority_rank(),
                        line_only_enhanced: lyrics.is_line_only_enhanced(),
                        lyrics: Some(lyrics),
                        timeline: Some(timeline),
                        error: None,
                        path,
                    }
                }
                Err(err) => LyricsCandidate {
                    error: Some(format!("reading lyrics file {}: {}", path.display(), err)),
                    path,
                    rank: 0,
                    line_only_enhanced: false,
                    lyrics: None,
                    timeline: None,
                },
            })
            .collect();
        candidates.sort_by(|a, b| {
            b.rank
                .cmp(&a.rank)
                .then_with(|| a.line_only_enhanced.cmp(&b.line_only_enhanced))
                .then_with(|| a.path.file_name().cmp(&b.path.file_name()))
        });
        candidates
    }

    /// `discover_for_track` + `rank_candidates`: every candidate file is read
    /// and parsed once; the first entry is the one to load. Does blocking file
    /// I/O — call it from a worker thread when possible.
    pub fn discover_ranked<P: AsRef<Path>>(track_path: P, lrc_paths: &[String]) -> Vec<LyricsCandidate> {
        Self::rank_candidates(Self::discover_for_track(track_path, lrc_paths))
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

        // Reuse the long-lived shared connection instead of opening SQLite
        // (and running PRAGMAs/DDL) on every track change. If another thread
        // is using it right now, fall back to a private connection.
        let lookup = |cache: &MetaCache| cache.lyrics_for_track_path(track_path).ok();
        let db_paths = match MetaCache::shared() {
            Ok(shared) => shared
                .try_with(lookup)
                .unwrap_or_else(|| MetaCache::open_default().ok().and_then(|c| lookup(&c))),
            Err(_) => None,
        };
        {
            if let Some(db_paths) = db_paths {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn enhanced_lrc_with_only_line_timestamps_is_not_treated_as_enhanced() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("line_only_enhanced.lrc");
        let mut file = File::create(&path).unwrap();
        writeln!(file, "<00:00.033>Intro").unwrap();
        writeln!(file, "<00:05.000>Verse").unwrap();

        let lyrics = Lyrics::from_lrc(&path).unwrap();
        assert_eq!(lyrics.display_priority_rank(), 1);
        // lines[0] is the synthetic blank line at 0 ms that `from_lrc` inserts
        // whenever the first lyric starts after 0 ms (here 33 ms).
        assert_eq!(lyrics.lines.len(), 3);
        assert_eq!((lyrics.lines[0].time_ms, lyrics.lines[0].text.as_str()), (0, ""));
        assert_eq!(lyrics.lines[1].text, "Intro");
        assert_eq!(lyrics.lines[1].time_ms, 33);
        assert_eq!(lyrics.lines[2].text, "Verse");
        assert!(lyrics.lines.iter().all(|line| line.segments.len() <= 1));
    }

    #[test]
    fn enhanced_lrc_with_segmented_timestamps_is_treated_as_enhanced() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("char_level_enhanced.lrc");
        let mut file = File::create(&path).unwrap();
        writeln!(file, "<00:00.000><00:00.500>He<00:01.000>llo").unwrap();

        let lyrics = Lyrics::from_lrc(&path).unwrap();
        assert_eq!(lyrics.display_priority_rank(), 3);
        assert_eq!(lyrics.lines.len(), 1);
        assert_eq!(lyrics.lines[0].segments.len(), 3);
    }

    #[test]
    fn enhanced_lrc_with_trailing_empty_timestamp_is_line_based() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("line_enhanced_trailing_tag.lrc");
        let mut file = File::create(&path).unwrap();
        writeln!(file, "[00:14.189] I feel it risin' within me<00:14.189>").unwrap();

        let lyrics = Lyrics::from_lrc(&path).unwrap();
        assert_eq!(lyrics.display_priority_rank(), 1);
        // Synthetic blank line at 0 ms + the lyric line.
        assert_eq!(lyrics.lines.len(), 2);
        assert_eq!(lyrics.lines[0].time_ms, 0);
        assert_eq!(lyrics.lines[1].time_ms, 14_189);
        // The trailing empty `<00:14.189>` is an end marker, not a segment.
        assert_eq!(lyrics.lines[1].segments.len(), 1);
        assert_eq!(lyrics.lines[1].text, " I feel it risin' within me");
        // ...but the file is still recognised as line-only enhanced, which the
        // discovery tie-break relies on.
        assert!(lyrics.is_line_only_enhanced());
    }

    /// Mirror of presentation-egui/tests/test_infrastructure_lyrics.rs
    /// `parses_enhanced_lrc_without_displaying_timestamps` (kept here so the
    /// contract is checked even when the GUI crate does not compile).
    #[test]
    fn gui_contract_enhanced_lrc_with_synthetic_first_line() {
        let lyrics = Lyrics::parse_str("[00:00.033]<00:00.033>world.<00:00.547>execute (<00:00.576>me) ; <00:00.608>[<00:00.640>Key <00:00.671>Ingredient <00:00.702>ver.] <00:00.733>- <00:00.743><00:00.733>Mili<00:00.779>");
        assert_eq!(lyrics.lines.len(), 2);
        assert_eq!(lyrics.lines[1].text, "world.execute (me) ; [Key Ingredient ver.] - Mili");
        assert!(lyrics.lines[1].segments.len() > 1);
        assert_eq!(lyrics.lines[1].segments[0].time_ms, 33);
        assert_eq!(lyrics.lines[1].segments[1].time_ms, 33);
        assert_eq!(lyrics.lines[1].segments[2].time_ms, 547);
        let plain = Lyrics::parse_str("[00:00.00]Intro\n[01:02.345]Chorus\n[10:20.5]Outro\n");
        let times: Vec<u64> = plain.lines.iter().map(|l| l.time_ms).collect();
        assert_eq!(times, vec![0, 62_345, 620_500]);
    }

    #[test]
    fn karaoke_end_marker_is_kept_when_line_has_several_words() {
        let lyrics = Lyrics::parse_str("[00:00.033]<00:00.033>world.<00:00.547>execute<00:00.779>");
        let line = &lyrics.lines[1];
        assert_eq!(line.text, "world.execute");
        // bracket "" + 2 words + end marker that bounds the last word.
        assert_eq!(line.segments.len(), 4);
        assert_eq!(line.segments[3].time_ms, 779);
        assert_eq!(lyrics.display_priority_rank(), 3);
    }

    #[test]
    fn repeated_line_timestamps_expand_to_separate_lines() {
        let lyrics = Lyrics::parse_str("[00:00.00]a\n[00:12.00][00:45.00]Chorus\n");
        let lines: Vec<(u64, &str, usize)> = lyrics
            .lines
            .iter()
            .map(|l| (l.time_ms, l.text.as_str(), l.segments.len()))
            .collect();
        assert_eq!(lines, vec![(0, "a", 1), (12_000, "Chorus", 1), (45_000, "Chorus", 1)]);
        assert_eq!(lyrics.display_priority_rank(), 1);
    }

    #[test]
    fn id_tags_are_not_lyric_lines() {
        let lyrics = Lyrics::parse_str("[ti:Title]\n[offset:+100]\n[00:00.00]x\n");
        assert_eq!(lyrics.lines.len(), 1);
        assert_eq!(lyrics.lines[0].text, "x");
    }

    #[test]
    fn rank_candidates_parses_once_and_orders_like_gui() {
        let dir = tempdir().unwrap();
        let write = |name: &str, body: &str| {
            let p = dir.path().join(name);
            std::fs::write(&p, body).unwrap();
            p
        };
        let plain = write("b_plain.lrc", "[00:01.00]plain\n");
        let line_only = write("a_line_only.lrc", "[00:01.00]x<00:02.00>\n");
        let karaoke = write("z_karaoke.lrc", "[00:01.00]<00:01.00>a<00:01.50>b\n");
        let missing = dir.path().join("missing.lrc");

        let ranked = Lyrics::rank_candidates(vec![plain.clone(), missing.clone(), line_only.clone(), karaoke.clone()]);
        let order: Vec<&PathBuf> = ranked.iter().map(|c| &c.path).collect();
        assert_eq!(order, vec![&karaoke, &plain, &line_only, &missing]);
        assert_eq!(ranked[0].rank, 3);
        assert!(ranked[0].lyrics.is_some() && ranked[0].timeline.is_some());
        assert!(ranked[3].error.is_some());
        assert_eq!(ranked[3].rank, 0);
        // Candidate lyrics equal a fresh parse of the same file.
        assert_eq!(ranked[1].lyrics.as_ref().unwrap(), &Lyrics::from_lrc(&plain).unwrap());
    }
}


