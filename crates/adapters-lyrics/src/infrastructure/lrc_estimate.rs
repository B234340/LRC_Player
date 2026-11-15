//! Estimated per-character timing for line-based lyrics ("pseudo karaoke").
//!
//! A line-based LRC only knows when each line starts. For a karaoke-style wipe the line's
//! characters are spread over its time, weighted by how long they take to sing: roughly one
//! unit per mora (kana), two for a kanji in Japanese text (average reading length), one per
//! syllable elsewhere, and almost nothing for small kana, spaces and punctuation.
//!
//! The wipe never runs slower than [`MAX_MS_PER_UNIT`], so a line followed by an instrumental
//! gap finishes when the singing would, not at the next line. Estimated lines are marked
//! (`TimedLine::estimated`): they are only for display and are never written back to a file
//! or moved by a character shift.

use super::{LyricsTimeline, TimedSegment};

/// Slowest wipe: ms per weight unit (~2.5 mora / s, a slow ballad).
pub const MAX_MS_PER_UNIT: u64 = 400;
/// Part of the line's span the wipe may use at most (a short breath before the next line).
const SPAN_USE: f64 = 0.95;

fn is_kana(c: char) -> bool {
    matches!(c, '\u{3041}'..='\u{309F}' | '\u{30A0}'..='\u{30FF}' | '\u{FF66}'..='\u{FF9F}')
}

fn is_han(c: char) -> bool {
    matches!(c, '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' | '\u{F900}'..='\u{FAFF}' | '々' | '〆')
}

/// Singing-time weight of one character.
pub(crate) fn char_weight(c: char, japanese: bool) -> f64 {
    match c {
        // Small kana fused with the previous mora (っ/ッ is a mora of its own).
        'ぁ' | 'ぃ' | 'ぅ' | 'ぇ' | 'ぉ' | 'ゃ' | 'ゅ' | 'ょ' | 'ゎ' | 'ァ' | 'ィ' | 'ゥ' | 'ェ' | 'ォ' | 'ャ'
        | 'ュ' | 'ョ' | 'ヮ' | 'ｧ' | 'ｨ' | 'ｩ' | 'ｪ' | 'ｫ' | 'ｬ' | 'ｭ' | 'ｮ' => 0.25,
        'ﾞ' | 'ﾟ' => 0.0,
        'ー' | 'ｰ' | '〜' | '～' => 1.0,
        c if is_kana(c) => 1.0,
        c if is_han(c) => {
            if japanese {
                2.0
            } else {
                1.0
            }
        }
        '\u{AC00}'..='\u{D7AF}' => 1.0, // Hangul syllable
        c if c.is_whitespace() => 0.15,
        c if c.is_ascii_alphabetic() || c.is_alphabetic() => 0.35, // ~3 letters per syllable
        c if c.is_ascii_digit() => 0.8,
        _ => 0.05, // punctuation, symbols
    }
}

impl LyricsTimeline {
    /// A copy where every non-blank line without karaoke timing gets estimated per-character
    /// segments (marked `estimated`). Lines with real timing are unchanged.
    pub fn with_estimated_karaoke(&self) -> Self {
        let mut out = self.clone();
        // Han-only lines in a Japanese song are Japanese too.
        let japanese_song = self.lines.iter().any(|l| l.text.chars().any(is_kana));
        for line in &mut out.lines {
            if line.is_karaoke() || line.is_blank() {
                continue;
            }
            let japanese = japanese_song || line.text.chars().any(is_kana);
            let weights: Vec<f64> = line.text.chars().map(|c| char_weight(c, japanese)).collect();
            let total: f64 = weights.iter().sum();
            let span = line.end_ms.saturating_sub(line.start_ms);
            if total <= 0.0 || span == 0 {
                continue;
            }
            let wipe = (span as f64 * SPAN_USE).min(total * MAX_MS_PER_UNIT as f64);
            let mut segments = Vec::with_capacity(weights.len());
            let mut acc = 0.0;
            let mut byte = 0usize;
            for (i, (c, w)) in line.text.chars().zip(&weights).enumerate() {
                let start = line.start_ms + (wipe * acc / total).round() as u64;
                acc += w;
                let end = line.start_ms + (wipe * acc / total).round() as u64;
                let len = c.len_utf8();
                segments.push(TimedSegment { start_ms: start, end_ms: end, byte_range: byte..byte + len, char_range: i..i + 1 });
                byte += len;
            }
            line.segments = segments;
            line.estimated = true;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::super::LyricsTimeline;
    use super::MAX_MS_PER_UNIT;

    #[test]
    fn line_based_lines_get_a_weighted_character_wipe() {
        let t = LyricsTimeline::parse("[00:10.00]きょうは晴れ\n[00:20.00]next\n").with_estimated_karaoke();
        let line = &t.lines()[0];
        assert!(line.estimated && line.is_karaoke());
        assert_eq!(line.segments.len(), 6);
        // き(1) ょ(0.25) う(1) は(1) 晴(2) れ(1) = 6.25 units, capped at 400 ms per unit.
        let wipe_end = line.segments.last().unwrap().end_ms;
        assert_eq!(wipe_end, 10_000 + (6.25 * MAX_MS_PER_UNIT as f64) as u64);
        let dur = |i: usize| line.segments[i].end_ms - line.segments[i].start_ms;
        assert!(dur(1) < dur(0), "small ょ is short");
        assert_eq!(dur(4), 2 * dur(3), "a kanji takes two kana in Japanese");
        // Segments are contiguous and ordered.
        assert!(line.segments.windows(2).all(|w| w[0].end_ms == w[1].start_ms));
    }

    #[test]
    fn short_lines_fit_inside_their_span() {
        let t = LyricsTimeline::parse("[00:00.00]あいうえおかきくけこ\n[00:01.00]x\n").with_estimated_karaoke();
        let end = t.lines()[0].segments.last().unwrap().end_ms;
        assert_eq!(end, 950, "95 % of the 1 s span");
    }

    #[test]
    fn position_wipes_through_the_line() {
        let t = LyricsTimeline::parse("[00:00.00]あいう\n[00:10.00]x\n").with_estimated_karaoke();
        let at = |ms| t.position_at(ms).unwrap().highlighted_chars;
        assert!(at(0) < 0.1);
        assert!((at(600) - 1.5).abs() < 0.01, "{}", at(600));
        assert!((at(5_000) - 3.0).abs() < 1e-6);
    }

    #[test]
    fn real_karaoke_and_blank_lines_are_untouched() {
        let src = "[00:00.00]<00:00.00>あ<00:01.00>い<00:02.00>\n[00:03.00]\n[00:05.00]plain words here\n";
        let t = LyricsTimeline::parse(src);
        let e = t.with_estimated_karaoke();
        assert_eq!(e.lines()[0], t.lines()[0]);
        assert_eq!(e.lines()[1], t.lines()[1]);
        assert!(e.lines()[2].estimated);
        // Estimated timing is never saved nor counted for character shifts.
        assert_eq!(e.karaoke_char_count(), t.karaoke_char_count());
        assert_eq!(LyricsTimeline::parse(&e.to_lrc_string()).lines()[2].segments.len(), 0);
        assert_eq!(e.with_text_shift(0, 1).lines()[2], e.lines()[2]);
    }

    #[test]
    fn chinese_han_counts_one_syllable() {
        let t = LyricsTimeline::parse("[00:00.00]我爱你\n[00:10.00]x\n").with_estimated_karaoke();
        let end = t.lines()[0].segments.last().unwrap().end_ms;
        assert_eq!(end, 3 * MAX_MS_PER_UNIT);
    }
}
