//! Search normalization, filtering and sorting on index vectors.
//!
//! Matching is case-insensitive, full-width/half-width insensitive (ASCII, katakana) and
//! hiragana/katakana insensitive. Queries are split on whitespace; every term must match
//! title, artist or album.

use std::cmp::Ordering;

use crate::presentation::ui::actions::{SortColumn, SortSpec};
use crate::presentation::ui::collate::SortKey;

use super::track_table::TrackRow;

/// Normalize text for matching/sorting.
pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let c = match c {
            // full-width ASCII → ASCII
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            '\u{3000}' => ' ',
            // half-width katakana → hiragana (with voiced-mark composition)
            '\u{FF61}'..='\u{FF9F}' => {
                if let Some(composed) = compose_halfwidth_mark(&mut out, c) {
                    composed
                } else {
                    match halfwidth_kana_to_hiragana(c) {
                        Some(h) => h,
                        None => continue,
                    }
                }
            }
            // katakana → hiragana
            '\u{30A1}'..='\u{30F6}' => char::from_u32(c as u32 - 0x60).unwrap_or(c),
            _ => c,
        };
        if c.is_ascii() {
            out.push(c.to_ascii_lowercase());
        } else {
            out.extend(c.to_lowercase());
        }
    }
    out
}

/// Handle ﾞ/ﾟ after a kana already in `out`: replace the last char with its voiced form.
fn compose_halfwidth_mark(out: &mut String, c: char) -> Option<char> {
    if c != '\u{FF9E}' && c != '\u{FF9F}' {
        return None;
    }
    let last = out.chars().last()?;
    let composed = if c == '\u{FF9E}' {
        match last {
            'う' => Some('ゔ'),
            'か'..='ぢ' | 'つ'..='ど' | 'は'..='ぽ' => {
                // voiceable kana are the ones directly followed by their voiced form
                let next = char::from_u32(last as u32 + 1)?;
                is_voiced_pair(last, next).then_some(next)
            }
            _ => None,
        }
    } else {
        match last {
            'は' | 'ひ' | 'ふ' | 'へ' | 'ほ' => char::from_u32(last as u32 + 2),
            _ => None,
        }
    }?;
    out.pop();
    Some(composed)
}

fn is_voiced_pair(base: char, voiced: char) -> bool {
    const PAIRS: &str = "かがきぎくぐけげこごさざしじすずせぜそぞただちぢつづてでとどはばひびふぶへべほぼ";
    let v: Vec<char> = PAIRS.chars().collect();
    v.chunks(2).any(|p| p[0] == base && p[1] == voiced)
}

fn halfwidth_kana_to_hiragana(c: char) -> Option<char> {
    const TABLE: &str = "。「」、・をぁぃぅぇぉゃゅょっーあいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほまみむめもやゆよらりるれろわん";
    let idx = (c as u32).checked_sub(0xFF61)? as usize;
    TABLE.chars().nth(idx)
}

/// Pre-normalized keys of one row: normalized text for matching, collation keys
/// ([`crate::presentation::ui::collate`]) for sorting.
#[derive(Clone, Debug, Default)]
pub struct RowKeys {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub title_sort: SortKey,
    pub artist_sort: SortKey,
    pub album_sort: SortKey,
    pub genre_sort: SortKey,
}

impl RowKeys {
    pub fn new(row: &TrackRow) -> Self {
        Self {
            title: normalize(&row.title),
            artist: normalize(&row.artist),
            album: normalize(&row.album),
            title_sort: SortKey::new(&row.title),
            artist_sort: SortKey::new(&row.artist),
            album_sort: SortKey::new(&row.album),
            genre_sort: SortKey::new(&row.genre),
        }
    }

    fn matches(&self, term: &str) -> bool {
        self.title.contains(term) || self.artist.contains(term) || self.album.contains(term)
    }
}

pub fn build_keys(rows: &[TrackRow]) -> Vec<RowKeys> {
    rows.iter().map(RowKeys::new).collect()
}

/// Filter + sort without a prebuilt index (normalizes on the fly).
pub fn filter_and_sort(rows: &[TrackRow], query: &str, sort: SortSpec) -> Vec<usize> {
    let keys = build_keys(rows);
    filter_and_sort_with_keys(rows, &keys, query, sort)
}

/// Filter + sort using precomputed keys (`keys.len() == rows.len()`).
pub fn filter_and_sort_with_keys(rows: &[TrackRow], keys: &[RowKeys], query: &str, sort: SortSpec) -> Vec<usize> {
    debug_assert_eq!(rows.len(), keys.len());
    let q = normalize(query);
    let terms: Vec<&str> = q.split_whitespace().collect();
    let mut view: Vec<usize> = if terms.is_empty() {
        (0..rows.len()).collect()
    } else {
        (0..rows.len())
            .filter(|&i| terms.iter().all(|t| keys[i].matches(t)))
            .collect()
    };
    sort_view(rows, keys, &mut view, sort);
    view
}

fn cmp_opt<T: Ord>(a: &Option<T>, b: &Option<T>) -> Ordering {
    // None sorts last
    match (a, b) {
        (Some(a), Some(b)) => a.cmp(b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// Text columns use the shared collation (see [`crate::presentation::ui::collate`]); empty
/// strings sort first in ascending order like most players. `Index` keeps the source order
/// (the library index stores its rows in the default artist → album → disc → track order).
fn sort_view(rows: &[TrackRow], keys: &[RowKeys], view: &mut [usize], sort: SortSpec) {
    let by_album_order = |a: usize, b: usize| {
        keys[a]
            .album_sort
            .cmp(&keys[b].album_sort)
            .then_with(|| cmp_opt(&rows[a].disc_no, &rows[b].disc_no))
            .then_with(|| cmp_opt(&rows[a].track_no, &rows[b].track_no))
            .then_with(|| keys[a].title_sort.cmp(&keys[b].title_sort))
    };
    let cmp = |&a: &usize, &b: &usize| -> Ordering {
        let primary = match sort.column {
            SortColumn::Index => a.cmp(&b),
            SortColumn::Title => keys[a]
                .title_sort
                .cmp(&keys[b].title_sort)
                .then_with(|| keys[a].artist_sort.cmp(&keys[b].artist_sort)),
            SortColumn::Artist => keys[a]
                .artist_sort
                .cmp(&keys[b].artist_sort)
                .then_with(|| by_album_order(a, b)),
            SortColumn::Album => by_album_order(a, b),
            SortColumn::Duration => cmp_opt(&rows[a].duration_ms, &rows[b].duration_ms),
            SortColumn::Year => cmp_opt(&rows[a].year, &rows[b].year).then_with(|| by_album_order(a, b)),
            SortColumn::Genre => keys[a].genre_sort.cmp(&keys[b].genre_sort).then_with(|| by_album_order(a, b)),
        };
        let primary = primary.then_with(|| a.cmp(&b));
        if sort.descending {
            primary.reverse()
        } else {
            primary
        }
    };
    view.sort_by(cmp);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn row(title: &str, artist: &str, album: &str, dur: Option<u64>, track: Option<u32>) -> TrackRow {
        TrackRow {
            path: PathBuf::from(format!("/m/{title}.flac")),
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            duration_ms: dur,
            track_no: track,
            disc_no: None,
            year: None,
            genre: String::new(),
        }
    }

    fn rows() -> Vec<TrackRow> {
        vec![
            row("Blue Bird", "いきものがかり", "Bird Album", Some(200_000), Some(2)),
            row("アイドル", "YOASOBI", "THE BOOK 3", Some(213_000), Some(1)),
            row("夜に駆ける", "YOASOBI", "THE BOOK", Some(261_000), Some(1)),
            row("Lemon", "米津玄師", "STRAY SHEEP", None, Some(3)),
            row("ｶﾞﾗｽ", "Ｔｅｓｔ　Artist", "Glass", Some(100_000), Some(1)),
        ]
    }

    const ASC: fn(SortColumn) -> SortSpec = |column| SortSpec { column, descending: false };

    #[test]
    fn normalize_width_case_and_kana() {
        assert_eq!(normalize("ＡＢＣ１２３"), "abc123");
        assert_eq!(normalize("カタカナ"), "かたかな");
        assert_eq!(normalize("ｶﾀｶﾅ"), "かたかな");
        assert_eq!(normalize("ｶﾞﾗｽ"), "がらす");
        assert_eq!(normalize("ﾊﾟﾋﾟﾌﾟ"), "ぱぴぷ");
        assert_eq!(normalize("ｳﾞ"), "ゔ");
        assert_eq!(normalize("Ｔｅｓｔ　Artist"), "test artist");
        assert_eq!(normalize("ÄÖ"), "äö");
    }

    #[test]
    fn empty_query_keeps_all_in_order() {
        let r = rows();
        assert_eq!(filter_and_sort(&r, "", ASC(SortColumn::Index)), vec![0, 1, 2, 3, 4]);
        assert_eq!(filter_and_sort(&r, "   ", ASC(SortColumn::Index)), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn filter_is_case_width_kana_insensitive() {
        let r = rows();
        assert_eq!(filter_and_sort(&r, "yoasobi", ASC(SortColumn::Index)), vec![1, 2]);
        assert_eq!(filter_and_sort(&r, "ＹＯＡＳＯＢＩ", ASC(SortColumn::Index)), vec![1, 2]);
        assert_eq!(filter_and_sort(&r, "あいどる", ASC(SortColumn::Index)), vec![1]);
        assert_eq!(filter_and_sort(&r, "ガラス", ASC(SortColumn::Index)), vec![4]);
        assert_eq!(filter_and_sort(&r, "test artist", ASC(SortColumn::Index)), vec![4]);
        // all terms must match
        assert_eq!(filter_and_sort(&r, "yoasobi book 3", ASC(SortColumn::Index)), vec![1]);
        assert!(filter_and_sort(&r, "nomatch", ASC(SortColumn::Index)).is_empty());
    }

    #[test]
    fn sorting() {
        let r = rows();
        let titles = |v: Vec<usize>| v.into_iter().map(|i| r[i].title.clone()).collect::<Vec<_>>();
        assert_eq!(
            titles(filter_and_sort(&r, "", ASC(SortColumn::Title))),
            vec!["Blue Bird", "Lemon", "アイドル", "ｶﾞﾗｽ", "夜に駆ける"]
        );
        // natural numbers and a leading "The" in text columns
        let r2 = vec![
            row("Track 10", "The Weeknd", "A", None, None),
            row("Track 2", "Vaundy", "A", None, None),
            row("track 1", "優里", "A", None, None),
        ];
        let t2 = filter_and_sort(&r2, "", ASC(SortColumn::Title));
        assert_eq!(t2, vec![2, 1, 0]);
        let a2 = filter_and_sort(&r2, "", ASC(SortColumn::Artist));
        assert_eq!(a2, vec![1, 0, 2], "Vaundy < (The) Weeknd < 優里");
        let d = filter_and_sort(&r, "", SortSpec { column: SortColumn::Duration, descending: false });
        assert_eq!(d, vec![4, 0, 1, 2, 3]); // None last
        let d = filter_and_sort(&r, "", SortSpec { column: SortColumn::Duration, descending: true });
        assert_eq!(d[0], 3);
        assert_eq!(d[1], 2);
        // artist sort groups by album then track
        let a = filter_and_sort(&r, "yoasobi", ASC(SortColumn::Artist));
        assert_eq!(a, vec![2, 1]); // "the book" < "the book 3"
    }

    #[test]
    fn large_input_is_fast_enough() {
        let mut big = Vec::new();
        for i in 0..20_000 {
            big.push(row(&format!("Song {i}"), &format!("Artist {}", i % 300), &format!("Album {}", i % 1500), Some(i as u64 * 7), Some((i % 12) as u32)));
        }
        let keys = build_keys(&big);
        let t = std::time::Instant::now();
        let v = filter_and_sort_with_keys(&big, &keys, "artist 1", SortSpec { column: SortColumn::Album, descending: false });
        assert!(!v.is_empty());
        // generous bound for debug builds
        assert!(t.elapsed().as_millis() < 2_000);
    }
}
