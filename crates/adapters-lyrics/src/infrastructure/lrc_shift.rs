//! Shifting the lyric *text* against fixed timestamps, and writing a timeline back as LRC.
//!
//! Some `.lrc` files have the right timestamps but the text is off by a line (each line's text
//! sits on the next line's timestamp) or, in karaoke files, off by a few characters. Shifting
//! keeps every timestamp where it is and slides the text across them:
//!
//! * **Line shift** `k`: line `i` shows the text (and karaoke timing, scaled to its span) of
//!   line `i - k`. Positive = text moves to later lines.
//! * **Char shift** `k`: every character of the karaoke lines gets its own time slot (a
//!   segment's span is split evenly over its characters); character `j` of the whole lyrics
//!   moves to slot `j + k`. Positive = text moves to later slots. Line-based lines (no
//!   per-character timing) are not touched.
//!
//! Nothing is lost at the ends: text pushed past the first / last slot is merged into it.
//! Shifts are always applied to the original timeline, so +1 then -1 restores it exactly.

use std::fmt::Write as _;

use super::{LyricsTimeline, TimedLine, TimedSegment};

impl LyricsTimeline {
    /// A copy with the text shifted by `line_shift` lines, then `char_shift` characters
    /// (karaoke lines only). Timestamps and offsets are unchanged.
    pub fn with_text_shift(&self, line_shift: i32, char_shift: i32) -> Self {
        let mut out = self.clone();
        if line_shift != 0 {
            out.lines = shift_lines(&self.lines, line_shift);
        }
        if char_shift != 0 {
            shift_chars(&mut out.lines, char_shift);
        }
        out
    }

    /// Number of characters a char shift can move (all characters with karaoke timing).
    pub fn karaoke_char_count(&self) -> usize {
        self.lines
            .iter()
            .flat_map(|l| l.segments.iter())
            .map(|s| s.char_range.end - s.char_range.start)
            .sum()
    }

    /// The timeline as LRC text with the total offset baked into the timestamps (no
    /// `[offset:]` tag). Karaoke lines use `<mm:ss.xx>` word tags.
    pub fn to_lrc_string(&self) -> String {
        let offset = self.total_offset_ms();
        let ts = |t: u64| fmt_time((t as i64).saturating_sub(offset).max(0) as u64);
        let mut out = String::new();
        let meta = &self.metadata;
        for (key, value) in [
            ("ti", &meta.title),
            ("ar", &meta.artist),
            ("al", &meta.album),
            ("by", &meta.author),
            ("length", &meta.length),
        ] {
            if let Some(v) = value {
                let _ = writeln!(out, "[{key}:{v}]");
            }
        }
        for line in &self.lines {
            let _ = write!(out, "[{}]", ts(line.start_ms));
            if line.segments.is_empty() {
                out.push_str(&line.text);
            } else {
                let mut at = 0usize;
                for seg in &line.segments {
                    out.push_str(&line.text[at..seg.byte_range.start]);
                    let _ = write!(out, "<{}>", ts(seg.start_ms));
                    out.push_str(&line.text[seg.byte_range.clone()]);
                    at = seg.byte_range.end;
                }
                out.push_str(&line.text[at..]);
                if let Some(last) = line.segments.last() {
                    let _ = write!(out, "<{}>", ts(last.end_ms));
                }
            }
            out.push('\n');
        }
        out
    }
}

/// `mm:ss.xx` (centiseconds, rounded).
fn fmt_time(ms: u64) -> String {
    let cs = (ms + 5) / 10;
    format!("{:02}:{:02}.{:02}", cs / 6000, (cs / 100) % 60, cs % 100)
}

/// Maps `t` from `[from_start, from_end]` onto `[to_start, to_end]`.
fn rescale(t: u64, from_start: u64, from_end: u64, to_start: u64, to_end: u64) -> u64 {
    let from_span = from_end.saturating_sub(from_start);
    let to_span = to_end.saturating_sub(to_start);
    if from_span == 0 {
        return to_start;
    }
    let rel = t.saturating_sub(from_start).min(from_span) as u128;
    to_start + (rel * to_span as u128 / from_span as u128) as u64
}

/// Target slot of item `i` shifted by `k` among `n` slots (clamped: nothing falls off).
fn target(i: usize, k: i32, n: usize) -> usize {
    (i as i64 + k as i64).clamp(0, n as i64 - 1) as usize
}

fn shift_lines(lines: &[TimedLine], k: i32) -> Vec<TimedLine> {
    let n = lines.len();
    let mut incoming: Vec<Vec<usize>> = vec![Vec::new(); n];
    for i in 0..n {
        incoming[target(i, k, n)].push(i);
    }
    lines
        .iter()
        .zip(incoming)
        .map(|(slot, mut sources)| {
            // Blank lines landing on a slot together with text do not count (shifting text onto
            // an empty line keeps its karaoke timing).
            if sources.len() > 1 {
                sources.retain(|&i| !lines[i].is_blank());
            }
            (slot, sources)
        })
        .map(|(slot, sources)| match sources.as_slice() {
            [] => TimedLine {
                start_ms: slot.start_ms,
                end_ms: slot.end_ms,
                text: String::new(),
                char_count: 0,
                segments: Vec::new(),
            },
            [one] => {
                let src = &lines[*one];
                let segments = src
                    .segments
                    .iter()
                    .map(|seg| TimedSegment {
                        start_ms: rescale(seg.start_ms, src.start_ms, src.end_ms, slot.start_ms, slot.end_ms),
                        end_ms: rescale(seg.end_ms, src.start_ms, src.end_ms, slot.start_ms, slot.end_ms),
                        byte_range: seg.byte_range.clone(),
                        char_range: seg.char_range.clone(),
                    })
                    .collect();
                TimedLine {
                    start_ms: slot.start_ms,
                    end_ms: slot.end_ms,
                    text: src.text.clone(),
                    char_count: src.char_count,
                    segments,
                }
            }
            many => {
                // Several lines pushed onto the first / last slot: keep all their text.
                let text = many
                    .iter()
                    .map(|&i| lines[i].text.trim())
                    .filter(|t| !t.is_empty())
                    .collect::<Vec<_>>()
                    .join("  ");
                TimedLine {
                    start_ms: slot.start_ms,
                    end_ms: slot.end_ms,
                    char_count: text.chars().count(),
                    text,
                    segments: Vec::new(),
                }
            }
        })
        .collect()
}

/// One per-character time slot of a karaoke line.
struct CharSlot {
    line: usize,
    /// Char index inside the line text.
    char_index: usize,
    start_ms: u64,
    end_ms: u64,
}

fn shift_chars(lines: &mut [TimedLine], k: i32) {
    // Slots and the character stream, in lyric order.
    let mut slots: Vec<CharSlot> = Vec::new();
    let mut stream: Vec<char> = Vec::new();
    for (li, line) in lines.iter().enumerate() {
        let chars: Vec<char> = line.text.chars().collect();
        for seg in &line.segments {
            let len = (seg.char_range.end - seg.char_range.start) as u64;
            for (j, ci) in seg.char_range.clone().enumerate() {
                let j = j as u64;
                let span = seg.end_ms.saturating_sub(seg.start_ms);
                slots.push(CharSlot {
                    line: li,
                    char_index: ci,
                    start_ms: seg.start_ms + span * j / len,
                    end_ms: seg.start_ms + span * (j + 1) / len,
                });
                stream.push(chars[ci]);
            }
        }
    }
    let n = slots.len();
    if n == 0 {
        return;
    }
    let mut assigned: Vec<String> = vec![String::new(); n];
    for (j, c) in stream.into_iter().enumerate() {
        assigned[target(j, k, n)].push(c);
    }

    // Rebuild every karaoke line: slot characters are replaced by their assigned text, other
    // characters (outside segments) stay; one segment per non-empty slot.
    let mut slot_iter = slots.iter().zip(assigned.iter()).peekable();
    for (li, line) in lines.iter_mut().enumerate() {
        if line.segments.is_empty() {
            continue;
        }
        let mut text = String::new();
        let mut char_count = 0usize;
        let mut segments = Vec::new();
        for (ci, ch) in line.text.chars().enumerate() {
            match slot_iter.peek() {
                Some((slot, _)) if slot.line == li && slot.char_index == ci => {
                    let (slot, piece) = slot_iter.next().expect("peeked");
                    if piece.is_empty() {
                        continue;
                    }
                    let (byte_start, char_start) = (text.len(), char_count);
                    text.push_str(piece);
                    char_count += piece.chars().count();
                    segments.push(TimedSegment {
                        start_ms: slot.start_ms,
                        end_ms: slot.end_ms,
                        byte_range: byte_start..text.len(),
                        char_range: char_start..char_count,
                    });
                }
                _ => {
                    text.push(ch);
                    char_count += 1;
                }
            }
        }
        line.text = text;
        line.char_count = char_count;
        line.segments = segments;
    }
}

#[cfg(test)]
mod tests {
    use super::super::LyricsTimeline;

    fn texts(t: &LyricsTimeline) -> Vec<&str> {
        t.lines().iter().map(|l| l.text.as_str()).collect()
    }

    fn starts(t: &LyricsTimeline) -> Vec<u64> {
        t.lines().iter().map(|l| l.start_ms).collect()
    }

    const LINES: &str = "[00:10.00]\n[00:14.00]one\n[00:18.00]two\n[00:22.00]three\n";

    #[test]
    fn line_shift_moves_text_and_keeps_timestamps() {
        let t = LyricsTimeline::parse(LINES);
        // Text is one line late: move it one line earlier.
        let up = t.with_text_shift(-1, 0);
        assert_eq!(starts(&up), starts(&t));
        assert_eq!(texts(&up), vec!["one", "two", "three", ""]);
        // And later.
        let down = t.with_text_shift(1, 0);
        assert_eq!(texts(&down), vec!["", "", "one", "two  three"]);
    }

    #[test]
    fn shifts_are_computed_from_the_original() {
        let t = LyricsTimeline::parse(LINES);
        assert_eq!(t.with_text_shift(0, 0), t);
        assert_eq!(texts(&t.with_text_shift(-3, 0)), vec!["one  two  three", "", "", ""]);
        assert_eq!(texts(&t.with_text_shift(-1, 0).with_text_shift(0, 0)), texts(&t.with_text_shift(-1, 0)));
    }

    #[test]
    fn line_shift_rescales_karaoke_timing_into_the_new_line() {
        let t = LyricsTimeline::parse("[00:00.00]\n[00:10.00]<00:10.00>c<00:14.00>d<00:18.00>\n[00:20.00]x\n");
        let up = t.with_text_shift(-1, 0);
        let first = &up.lines()[0];
        assert_eq!(first.text, "cd");
        // c/d were at 0% / 50% of their 10..18 s line (end tag); line 0 spans 0..10 s.
        let seg_starts: Vec<u64> = first.segments.iter().map(|s| s.start_ms).collect();
        assert_eq!(seg_starts, vec![0, 5_000]);
    }

    #[test]
    fn char_shift_slides_text_over_karaoke_slots_across_lines() {
        let t = LyricsTimeline::parse(
            "[00:01.00]<00:01.00>あ<00:02.00>い<00:03.00>う<00:04.00>\n[00:05.00]<00:05.00>え<00:06.00>お<00:07.00>\n",
        );
        assert_eq!(t.karaoke_char_count(), 5);
        // Text one character early: move it one slot later.
        let s = t.with_text_shift(0, 1);
        assert_eq!(texts(&s), vec!["あい", "うえお"]);
        let l0: Vec<u64> = s.lines()[0].segments.iter().map(|x| x.start_ms).collect();
        assert_eq!(l0, vec![2_000, 3_000], "slot timing stays, first slot is empty");
        let l1: Vec<u64> = s.lines()[1].segments.iter().map(|x| x.start_ms).collect();
        assert_eq!(l1, vec![5_000, 6_000], "う moved onto え's slot");
        assert_eq!(s.lines()[1].segments[1].char_range, 1..3, "the last slot holds お plus the overflow");
        // And the other way.
        // あ/い share the first slot; the rest moves one slot earlier, across the line break.
        assert_eq!(texts(&t.with_text_shift(0, -1)), vec!["あいうえ", "お"]);
    }

    #[test]
    fn char_shift_splits_word_segments_into_character_slots() {
        let t = LyricsTimeline::parse("[00:00.00]<00:00.00>abc<00:03.00>de<00:05.00>\n");
        let s = t.with_text_shift(0, -1);
        // a/b share the first slot (nothing falls off), each later char moves one slot earlier.
        let line = &s.lines()[0];
        assert_eq!(line.text, "abcde");
        let st: Vec<u64> = line.segments.iter().map(|x| x.start_ms).collect();
        assert_eq!(st, vec![0, 1_000, 2_000, 3_000]);
        assert_eq!(line.segments[0].char_range, 0..2);
    }

    #[test]
    fn char_shift_leaves_line_based_lines_alone() {
        let t = LyricsTimeline::parse(LINES);
        assert_eq!(t.karaoke_char_count(), 0);
        assert_eq!(t.with_text_shift(0, 2), t);
    }

    #[test]
    fn to_lrc_string_round_trips_and_bakes_the_offset() {
        let src = "[ti:Song]\n[offset:500]\n[00:01.00]<00:01.00>あ<00:02.00>い<00:03.00>\n[00:05.00]plain line\n";
        let mut t = LyricsTimeline::parse(src);
        t.set_user_offset_ms(250);
        let out = t.to_lrc_string();
        assert!(out.starts_with("[ti:Song]\n"));
        assert!(!out.contains("[offset:"));
        let back = LyricsTimeline::parse(&out);
        assert_eq!(texts(&back), texts(&t));
        // 1000 ms - 750 ms total offset.
        assert_eq!(back.lines()[0].start_ms, 250);
        assert_eq!(back.lines()[1].start_ms, 4_250);
        assert_eq!(back.lines()[0].segments.len(), 2);
        // Shifted timelines round-trip too.
        let shifted = LyricsTimeline::parse(src).with_text_shift(0, 1);
        assert_eq!(texts(&LyricsTimeline::parse(&shifted.to_lrc_string())), texts(&shifted));
    }
}
