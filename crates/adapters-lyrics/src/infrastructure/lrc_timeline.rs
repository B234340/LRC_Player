//! LRC tokenizer shared by [`crate::infrastructure::lyrics::Lyrics`] and the
//! frame-cheap [`LyricsTimeline`] used by the GUI.
//!
//! `LyricsTimeline` is parsed once (when a lyrics file is loaded) and then
//! queried every frame with O(log n), allocation-free lookups:
//!
//! ```ignore
//! let timeline = LyricsTimeline::from_lrc(path)?;        // once, off the UI thread ideally
//! timeline.set_user_offset_ms(app.lyrics_offset_ms);       // whenever the user changes it
//! if let Some(pos) = timeline.position_at(play_position_ms) {   // each frame
//!     let line = &timeline.lines()[pos.line_index];
//!     // highlight `pos.highlighted_chars` chars of `line.text` (karaoke) …
//! }
//! ```
//!
//! Offset convention (same as the LRC `[offset:]` tag and the GUI's existing
//! `lyrics_offset_ms`): a **positive** offset makes lyrics appear **earlier**.
//! The lyric clock is `position_ms + file_offset_ms + user_offset_ms`.

use crate::infrastructure::lyrics::LyricTagKind;
use anyhow::{Context, Result};
use std::ops::Range;
use std::path::Path;

/// End time assumed for the last line when the file gives no end marker.
pub const DEFAULT_LAST_LINE_MS: u64 = 5_000;

// ---------------------------------------------------------------------------
// Tokenizer (shared)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawSegment {
    pub time_ms: u64,
    pub text: String,
    pub kind: LyricTagKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RawEntry {
    /// One timed lyric line (segments are non-empty; segment 0 starts the line).
    Line(Vec<RawSegment>),
    /// `[key:value]` ID tag, key lower-cased.
    Meta { key: String, value: String },
}

/// Tokenize LRC text into timed lines and ID tags, in file order.
pub(crate) fn tokenize_lrc(contents: &str) -> Vec<RawEntry> {
    let mut entries = Vec::new();
    for raw_line in contents.lines() {
        let line = raw_line.trim().trim_start_matches('\u{feff}');
        if line.is_empty() {
            continue;
        }

        if let Some((key, value)) = parse_id_tag(line) {
            entries.push(RawEntry::Meta { key, value });
            continue;
        }

        // `[t1][t2]text` = the same line repeated at t1 and t2 (standard LRC).
        let mut times = Vec::new();
        let mut rest = line;
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(end) = stripped.find(']') else { break };
            let Some(ms) = parse_timestamp(&stripped[..end]) else { break };
            times.push(ms);
            rest = &stripped[end + 1..];
        }

        if times.len() >= 2 {
            let base = times[0];
            let segments = parse_segments(rest, Some((base, LyricTagKind::Bracket)));
            for &start in &times {
                let shifted = segments
                    .iter()
                    .map(|seg| RawSegment {
                        time_ms: (seg.time_ms + start).saturating_sub(base),
                        text: seg.text.clone(),
                        kind: seg.kind.clone(),
                    })
                    .collect();
                entries.push(RawEntry::Line(shifted));
            }
        } else {
            let segments = parse_segments(line, None);
            if !segments.is_empty() {
                entries.push(RawEntry::Line(segments));
            }
        }
    }
    entries
}

/// Split one line into timed segments. Text before the first timestamp is
/// ignored unless `initial` provides a starting time.
fn parse_segments(line: &str, initial: Option<(u64, LyricTagKind)>) -> Vec<RawSegment> {
    let mut segments = Vec::new();
    let (mut current_time, mut current_kind) = match initial {
        Some((time, kind)) => (Some(time), kind),
        None => (None, LyricTagKind::Bracket),
    };
    let mut current_text = String::new();
    let mut i = 0;

    while i < line.len() {
        let ch = line[i..].chars().next().unwrap();
        if ch == '[' || ch == '<' {
            let closing = if ch == '[' { ']' } else { '>' };
            if let Some(end_rel) = line[i + ch.len_utf8()..].find(closing) {
                let tag = &line[i + ch.len_utf8()..i + ch.len_utf8() + end_rel];
                if let Some(ms) = parse_timestamp(tag) {
                    if let Some(time_ms) = current_time {
                        segments.push(RawSegment {
                            time_ms,
                            text: std::mem::take(&mut current_text),
                            kind: current_kind.clone(),
                        });
                    }
                    current_time = Some(ms);
                    current_kind = if ch == '[' {
                        LyricTagKind::Bracket
                    } else {
                        LyricTagKind::Angle
                    };
                    current_text.clear();
                    i += ch.len_utf8() + end_rel + closing.len_utf8();
                    continue;
                }
            }
        }

        if current_time.is_some() {
            current_text.push(ch);
        }
        i += ch.len_utf8();
    }

    if let Some(time_ms) = current_time {
        segments.push(RawSegment {
            time_ms,
            text: current_text,
            kind: current_kind,
        });
    }
    segments
}

/// `[ti:Title]`, `[offset:+250]`, … (whole line). Timestamps are not ID tags.
fn parse_id_tag(line: &str) -> Option<(String, String)> {
    let inner = line.strip_prefix('[')?;
    let end = inner.find(']')?;
    if !inner[end + 1..].trim().is_empty() {
        return None;
    }
    let tag = &inner[..end];
    if parse_timestamp(tag).is_some() {
        return None;
    }
    let (key, value) = tag.split_once(':')?;
    let key = key.trim();
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphabetic() || c == '#') {
        return None;
    }
    Some((key.to_ascii_lowercase(), value.trim().to_string()))
}

/// Parse an `[offset:...]` value: `+250`, `-120`, `300` (milliseconds).
pub(crate) fn parse_offset_value(value: &str) -> Option<i64> {
    let v = value.trim();
    let v = v.strip_prefix('+').unwrap_or(v);
    v.parse::<i64>().ok()
}

/// `mm:ss`, `mm:ss.x`, `mm:ss.xx`, `mm:ss.xxx` (extra digits are truncated).
pub(crate) fn parse_timestamp(tag: &str) -> Option<u64> {
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

fn non_blank_count(segments: &[RawSegment]) -> usize {
    segments.iter().filter(|seg| !seg.text.trim().is_empty()).count()
}

/// Rank used to choose between several lyrics files for one track
/// (3 = word/char-level enhanced `<>` timing, 2 = other multi-segment
/// timing, 1 = line-based).
pub(crate) fn rank_of_lines<'a>(lines: impl Iterator<Item = &'a [RawSegment]> + Clone) -> u8 {
    let enhanced = lines.clone().any(|segs| {
        segs.iter()
            .filter(|seg| !seg.text.trim().is_empty() && seg.kind == LyricTagKind::Angle)
            .count()
            > 1
    });
    if enhanced {
        3
    } else if lines.clone().any(|segs| non_blank_count(segs) > 1) {
        2
    } else {
        1
    }
}

/// A line written in enhanced syntax that nevertheless carries only one piece
/// of text (e.g. `<00:01.00>text` plus an empty end tag).
pub(crate) fn is_line_only_enhanced_segments(segs: &[RawSegment]) -> bool {
    segs.len() > 1
        && non_blank_count(segs) == 1
        && segs.iter().any(|seg| seg.kind == LyricTagKind::Angle)
}

// ---------------------------------------------------------------------------
// LyricsTimeline
// ---------------------------------------------------------------------------

/// One karaoke segment (word / character run) of a [`TimedLine`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimedSegment {
    /// Start in lyric time (file timestamps, before offsets).
    pub start_ms: u64,
    /// End in lyric time (next segment start, end marker, or line end).
    pub end_ms: u64,
    /// Byte range of this segment inside [`TimedLine::text`].
    pub byte_range: Range<usize>,
    /// Char range of this segment inside [`TimedLine::text`] (for galley cursors).
    pub char_range: Range<usize>,
}

/// One lyric line with its timing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimedLine {
    /// Start in lyric time (file timestamps, before offsets).
    pub start_ms: u64,
    /// End in lyric time: explicit trailing end tag if present (and sane),
    /// otherwise the next line's start, otherwise start + [`DEFAULT_LAST_LINE_MS`].
    pub end_ms: u64,
    /// Display text (timestamps removed, not trimmed).
    pub text: String,
    /// Number of chars in `text`.
    pub char_count: usize,
    /// Karaoke segments; empty for plain line-based lines (fewer than two
    /// non-blank timed pieces). Empty-text timing segments are not listed but
    /// still bound their neighbours' times.
    pub segments: Vec<TimedSegment>,
}

impl TimedLine {
    pub fn is_karaoke(&self) -> bool {
        !self.segments.is_empty()
    }

    pub fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }
}

/// `[ti:]`, `[ar:]`, `[al:]`, `[by:]`, `[length:]`, `[offset:]` ID tags.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LrcMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub author: Option<String>,
    pub length: Option<String>,
    /// Value of the `[offset:]` tag in ms (0 when absent).
    pub offset_ms: i64,
}

/// Where the playback position currently is inside the lyrics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LyricPosition {
    /// Index into [`LyricsTimeline::lines`].
    pub line_index: usize,
    /// 0..=1 progress through the current line (`start_ms..end_ms`).
    pub line_progress: f32,
    /// Current karaoke segment (None for line-based lines, or before the
    /// first segment starts).
    pub segment_index: Option<usize>,
    /// 0..=1 progress through the current segment (0 when no segment).
    pub segment_progress: f32,
    /// Number of chars of the line text to draw highlighted (fractional for
    /// smooth wipes). Line-based lines are fully highlighted.
    pub highlighted_chars: f32,
}

/// Parsed lyrics optimized for per-frame queries. Cheap to clone? No — wrap in
/// `Arc` if it must be shared; queries never allocate.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LyricsTimeline {
    lines: Vec<TimedLine>,
    metadata: LrcMetadata,
    user_offset_ms: i64,
    rank: u8,
    line_only_enhanced: bool,
}

impl LyricsTimeline {
    /// Read and parse an `.lrc` file.
    pub fn from_lrc<P: AsRef<Path>>(path: P) -> Result<Self> {
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("reading lyrics file {}", path.as_ref().display()))?;
        Ok(Self::parse(&contents))
    }

    /// Parse LRC text.
    pub fn parse(contents: &str) -> Self {
        Self::from_entries(tokenize_lrc(contents))
    }

    pub(crate) fn from_entries(entries: Vec<RawEntry>) -> Self {
        let mut metadata = LrcMetadata::default();
        let mut raw_lines: Vec<Vec<RawSegment>> = Vec::new();
        for entry in entries {
            match entry {
                RawEntry::Line(segments) => raw_lines.push(segments),
                RawEntry::Meta { key, value } => {
                    let slot = match key.as_str() {
                        "ti" => &mut metadata.title,
                        "ar" => &mut metadata.artist,
                        "al" => &mut metadata.album,
                        "by" => &mut metadata.author,
                        "length" => &mut metadata.length,
                        "offset" => {
                            if let Some(ms) = parse_offset_value(&value) {
                                metadata.offset_ms = ms;
                            }
                            continue;
                        }
                        _ => continue,
                    };
                    if !value.is_empty() && slot.is_none() {
                        *slot = Some(value);
                    }
                }
            }
        }

        let rank = rank_of_lines(raw_lines.iter().map(|segs| segs.as_slice()));
        let line_only_enhanced = raw_lines.iter().any(|segs| is_line_only_enhanced_segments(segs));

        raw_lines.sort_by_key(|segs| segs[0].time_ms); // stable: keeps file order for equal times
        let starts: Vec<u64> = raw_lines.iter().map(|segs| segs[0].time_ms).collect();

        let mut lines = Vec::with_capacity(raw_lines.len());
        for (index, segs) in raw_lines.into_iter().enumerate() {
            let start_ms = starts[index];
            // First later line with a strictly greater start (equal starts =
            // e.g. original + translation shown together).
            let next_start = starts[index + 1..].iter().copied().find(|&s| s > start_ms);

            // A trailing empty-text tag is an end marker, not a segment.
            let mut content_len = segs.len();
            while content_len > 1 && segs[content_len - 1].text.is_empty() {
                content_len -= 1;
            }
            let explicit_end = (content_len < segs.len())
                .then(|| segs[segs.len() - 1].time_ms)
                .filter(|&end| end > start_ms);

            let end_ms = match (explicit_end, next_start) {
                (Some(end), Some(next)) => end.min(next),
                (Some(end), None) => end,
                (None, Some(next)) => next,
                (None, None) => start_ms + DEFAULT_LAST_LINE_MS,
            };

            let mut text = String::new();
            let mut char_count = 0usize;
            let mut segments = Vec::new();
            for (seg_index, seg) in segs[..content_len].iter().enumerate() {
                let byte_start = text.len();
                let char_start = char_count;
                text.push_str(&seg.text);
                char_count += seg.text.chars().count();
                if seg.text.is_empty() {
                    continue;
                }
                let seg_start = seg.time_ms;
                let seg_end = segs
                    .get(seg_index + 1)
                    .map(|next| next.time_ms)
                    .unwrap_or(end_ms)
                    .min(end_ms.max(seg_start))
                    .max(seg_start);
                segments.push(TimedSegment {
                    start_ms: seg_start,
                    end_ms: seg_end,
                    byte_range: byte_start..text.len(),
                    char_range: char_start..char_count,
                });
            }
            if non_blank_count(&segs[..content_len]) < 2 {
                segments.clear();
            }

            lines.push(TimedLine {
                start_ms,
                end_ms,
                text,
                char_count,
                segments,
            });
        }

        Self {
            lines,
            metadata,
            user_offset_ms: 0,
            rank,
            line_only_enhanced,
        }
    }

    pub fn lines(&self) -> &[TimedLine] {
        &self.lines
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn metadata(&self) -> &LrcMetadata {
        &self.metadata
    }

    /// `[offset:]` value from the file (ms, positive = lyrics earlier).
    pub fn file_offset_ms(&self) -> i64 {
        self.metadata.offset_ms
    }

    /// User adjustment on top of the file offset (positive = lyrics earlier).
    pub fn user_offset_ms(&self) -> i64 {
        self.user_offset_ms
    }

    pub fn set_user_offset_ms(&mut self, offset_ms: i64) {
        self.user_offset_ms = offset_ms;
    }

    /// File offset + user offset.
    pub fn total_offset_ms(&self) -> i64 {
        self.metadata.offset_ms.saturating_add(self.user_offset_ms)
    }

    /// Lyric-clock time for a playback position (may be negative).
    pub fn lyric_time_ms(&self, position_ms: u64) -> i64 {
        (position_ms.min(i64::MAX as u64) as i64).saturating_add(self.total_offset_ms())
    }

    /// Playback position at which line `index` starts (for click-to-seek).
    pub fn seek_position_for_line(&self, index: usize) -> Option<u64> {
        let line = self.lines.get(index)?;
        let pos = (line.start_ms.min(i64::MAX as u64) as i64).saturating_sub(self.total_offset_ms());
        Some(pos.max(0) as u64)
    }

    /// Index of the line active at `position_ms` (the last line whose start is
    /// <= the lyric time), or None before the first line. O(log n).
    pub fn line_index_at(&self, position_ms: u64) -> Option<usize> {
        let t = self.lyric_time_ms(position_ms);
        if t < 0 {
            return None;
        }
        let t = t as u64;
        self.lines.partition_point(|line| line.start_ms <= t).checked_sub(1)
    }

    /// Current line plus progress within it and its karaoke segment. O(log n),
    /// no allocation.
    pub fn position_at(&self, position_ms: u64) -> Option<LyricPosition> {
        let line_index = self.line_index_at(position_ms)?;
        let t = self.lyric_time_ms(position_ms).max(0) as u64;
        let line = &self.lines[line_index];
        let line_progress = progress(t, line.start_ms, line.end_ms);

        if line.segments.is_empty() {
            return Some(LyricPosition {
                line_index,
                line_progress,
                segment_index: None,
                segment_progress: 0.0,
                highlighted_chars: line.char_count as f32,
            });
        }

        let segment_index = line
            .segments
            .partition_point(|seg| seg.start_ms <= t)
            .checked_sub(1);
        let (segment_progress, highlighted_chars) = match segment_index {
            Some(i) => {
                let seg = &line.segments[i];
                let p = progress(t, seg.start_ms, seg.end_ms);
                let len = (seg.char_range.end - seg.char_range.start) as f32;
                (p, seg.char_range.start as f32 + len * p)
            }
            None => (0.0, 0.0),
        };
        Some(LyricPosition {
            line_index,
            line_progress,
            segment_index,
            segment_progress,
            highlighted_chars,
        })
    }

    /// Same ranking as `Lyrics::display_priority_rank` (3 = enhanced word
    /// timing, 2 = multi-segment, 1 = line-based).
    pub fn display_priority_rank(&self) -> u8 {
        self.rank
    }

    pub fn is_line_only_enhanced(&self) -> bool {
        self.line_only_enhanced
    }

    /// True when at least one line has karaoke segments.
    pub fn has_karaoke(&self) -> bool {
        self.lines.iter().any(TimedLine::is_karaoke)
    }
}

fn progress(t: u64, start: u64, end: u64) -> f32 {
    if end <= start {
        return if t >= start { 1.0 } else { 0.0 };
    }
    ((t.saturating_sub(start)) as f64 / (end - start) as f64).clamp(0.0, 1.0) as f32
}

#[path = "lrc_shift.rs"]
mod lrc_shift;

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\u{feff}[ti:Song]\n[ar:Someone]\n[offset:+250]\n\n[00:01.00]First line\n[00:05.00]<00:05.00>Hel<00:05.50>lo <00:06.00>world<00:07.00>\n[00:10.00][00:20.00]Chorus\n[00:15.00]\n";

    #[test]
    fn parses_metadata_offset_and_repeated_tags() {
        let t = LyricsTimeline::parse(SAMPLE);
        assert_eq!(t.metadata().title.as_deref(), Some("Song"));
        assert_eq!(t.metadata().artist.as_deref(), Some("Someone"));
        assert_eq!(t.file_offset_ms(), 250);
        let starts: Vec<u64> = t.lines().iter().map(|l| l.start_ms).collect();
        assert_eq!(starts, vec![1_000, 5_000, 10_000, 15_000, 20_000]);
        assert_eq!(t.lines()[2].text, "Chorus");
        assert_eq!(t.lines()[4].text, "Chorus");
        assert!(t.lines()[3].is_blank());
        // No synthetic 0 ms line in the timeline.
        assert_eq!(t.lines()[0].text, "First line");
    }

    #[test]
    fn line_ends_use_end_marker_next_line_or_default() {
        let t = LyricsTimeline::parse(SAMPLE);
        assert_eq!(t.lines()[0].end_ms, 5_000); // next line
        assert_eq!(t.lines()[1].end_ms, 7_000); // trailing <00:07.00> marker
        assert_eq!(t.lines()[4].end_ms, 20_000 + DEFAULT_LAST_LINE_MS);
    }

    #[test]
    fn karaoke_segments_have_ranges_and_end_times() {
        let t = LyricsTimeline::parse(SAMPLE);
        let line = &t.lines()[1];
        assert_eq!(line.text, "Hello world");
        assert!(line.is_karaoke());
        let segs: Vec<(u64, u64, &str)> = line
            .segments
            .iter()
            .map(|s| (s.start_ms, s.end_ms, &line.text[s.byte_range.clone()]))
            .collect();
        assert_eq!(segs, vec![(5_000, 5_500, "Hel"), (5_500, 6_000, "lo "), (6_000, 7_000, "world")]);
        assert_eq!(line.segments[2].char_range, 6..11);
        assert!(!t.lines()[0].is_karaoke());
    }

    #[test]
    fn line_index_and_progress_apply_offsets() {
        let mut t = LyricsTimeline::parse(SAMPLE);
        // file offset +250: lyric time = position + 250.
        assert_eq!(t.line_index_at(0), None);
        assert_eq!(t.line_index_at(749), None);
        assert_eq!(t.line_index_at(750), Some(0));
        assert_eq!(t.line_index_at(4_750), Some(1));
        assert_eq!(t.line_index_at(1_000_000), Some(4));

        // Karaoke: lyric 5_750 = halfway through "lo ".
        let pos = t.position_at(5_500).unwrap();
        assert_eq!(pos.line_index, 1);
        assert_eq!(pos.segment_index, Some(1));
        assert!((pos.segment_progress - 0.5).abs() < 1e-6);
        assert!((pos.highlighted_chars - 4.5).abs() < 1e-6);
        assert!((pos.line_progress - 0.375).abs() < 1e-6);

        // In the gap after the end marker the whole line stays highlighted.
        let pos = t.position_at(8_000).unwrap();
        assert_eq!(pos.line_index, 1);
        assert_eq!(pos.line_progress, 1.0);
        assert_eq!(pos.highlighted_chars, 11.0);

        // User offset stacks on the file offset.
        t.set_user_offset_ms(-250);
        assert_eq!(t.total_offset_ms(), 0);
        assert_eq!(t.line_index_at(999), None);
        assert_eq!(t.line_index_at(1_000), Some(0));
        assert_eq!(t.seek_position_for_line(2), Some(10_000));
        t.set_user_offset_ms(-1_000);
        assert_eq!(t.seek_position_for_line(0), Some(1_750));
    }

    #[test]
    fn line_based_lines_are_fully_highlighted_and_ranked_1() {
        let t = LyricsTimeline::parse("[00:01.00]abc\n[00:14.189] I feel it<00:14.189>\n");
        assert_eq!(t.display_priority_rank(), 1);
        assert!(!t.has_karaoke());
        assert!(t.is_line_only_enhanced());
        let pos = t.position_at(2_000).unwrap();
        assert_eq!(pos.segment_index, None);
        assert_eq!(pos.highlighted_chars, 3.0);
        assert!((pos.line_progress - (1_000.0 / 13_189.0)).abs() < 1e-4);
        // End marker equal to the start is ignored.
        assert_eq!(t.lines()[1].end_ms, 14_189 + DEFAULT_LAST_LINE_MS);
    }

    #[test]
    fn equal_timestamps_share_end_and_last_wins_lookup() {
        let t = LyricsTimeline::parse("[00:01.00]original\n[00:01.00]translation\n[00:03.00]next\n");
        assert_eq!(t.lines()[0].end_ms, 3_000);
        assert_eq!(t.lines()[1].end_ms, 3_000);
        assert_eq!(t.line_index_at(1_500), Some(1));
    }

    #[test]
    fn negative_offset_tag_and_empty_input() {
        let t = LyricsTimeline::parse("[offset:-500]\n[00:00.00]a\n");
        assert_eq!(t.file_offset_ms(), -500);
        assert_eq!(t.line_index_at(499), None);
        assert_eq!(t.line_index_at(500), Some(0));
        let empty = LyricsTimeline::parse("");
        assert!(empty.is_empty());
        assert_eq!(empty.position_at(1_000), None);
    }

    #[test]
    fn karaoke_lookup_is_cheap_for_large_files() {
        let mut text = String::new();
        for i in 0..5_000u64 {
            let ms = i * 1_000;
            text.push_str(&format!(
                "[{:02}:{:02}.{:02}]<{:02}:{:02}.{:02}>w{}<{:02}:{:02}.{:02}>x\n",
                ms / 60_000, (ms / 1_000) % 60, 0, ms / 60_000, (ms / 1_000) % 60, 0, i,
                ms / 60_000, (ms / 1_000) % 60, 50
            ));
        }
        let t = LyricsTimeline::parse(&text);
        assert_eq!(t.len(), 5_000);
        assert_eq!(t.display_priority_rank(), 3);
        let start = std::time::Instant::now();
        let mut acc = 0usize;
        for p in (0..5_000_000u64).step_by(997) {
            acc += t.position_at(p).map(|pos| pos.line_index).unwrap_or(0);
        }
        assert!(acc > 0);
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
    }
}
