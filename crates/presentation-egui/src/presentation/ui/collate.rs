//! Collation for names and titles: artist lists, album lists and the text columns of the
//! track table all sort with [`SortKey`] / [`collate`].
//!
//! # Rules
//!
//! - Case-insensitive; full-width and half-width forms are equal (`Ｂ` = `B`, `ｶ` = `カ`).
//! - Katakana compares as hiragana (`カ` = `か`). At the primary level small kana equal their
//!   large forms and voiced / semi-voiced kana equal their base (`っ` = `つ`, `が` = `か`,
//!   `ぱ` = `は`); the prolonged sound mark `ー` is ignored. Those differences only break ties.
//! - A leading `The ` is ignored for Latin names (`The Weeknd` sorts under W).
//! - Digit runs compare as numbers ("Track 2" < "Track 10"); leading zeros only break ties.
//! - Common accented Latin letters compare as their base letter (`é` = `e`).
//!
//! # Group order
//!
//! Characters are ordered by group first, then within the group:
//!
//! 1. whitespace, punctuation and symbols (by code point)
//! 2. digits (numeric value of the whole run)
//! 3. Latin letters (alphabetical)
//! 4. other alphabetic scripts: Greek, Cyrillic, Hangul, ... (by code point)
//! 5. kana (gojūon order, which is the hiragana code point order)
//! 6. kanji (by code point)
//!
//! Kanji are ordered by code point (roughly by radical / stroke count), not by reading: the
//! library has no reading (yomi) tags, and a kanji's reading cannot be derived without a
//! dictionary, so `米津玄師` cannot be placed under "よ". If reading tags (e.g. `ARTISTSORT`)
//! are added later, build the key from the reading instead of the display name.
//!
//! # Ties
//!
//! Keys that are equal at the primary level are ordered by the normalized text (so `が`
//! follows `か`, `02` precedes `2`, `The Weeknd` precedes `Weeknd`), and [`collate`] finally
//! falls back to the raw strings, so the order is total and deterministic. Callers sorting
//! rows should add their own last tie-break (row index / path) for stable results.

use std::cmp::Ordering;

use super::views::normalize;

const G_SYMBOL: u32 = 0;
const G_DIGIT: u32 = 1;
const G_LATIN: u32 = 2;
const G_OTHER: u32 = 3;
const G_KANA: u32 = 4;
const G_KANJI: u32 = 5;

#[inline]
fn elem(group: u32, value: u32) -> u32 {
    (group << 24) | (value & 0x00FF_FFFF)
}

/// Precomputed collation key. Compare keys with `Ord` (cheap slice comparison).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct SortKey {
    primary: Box<[u32]>,
    secondary: Box<str>,
}

impl SortKey {
    pub fn new(s: &str) -> Self {
        sort_key(s)
    }

    /// True for an empty (or whitespace-only) source string.
    pub fn is_empty(&self) -> bool {
        self.primary.is_empty() && self.secondary.is_empty()
    }
}

impl Ord for SortKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.primary.cmp(&other.primary).then_with(|| self.secondary.cmp(&other.secondary))
    }
}

impl PartialOrd for SortKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Compare two display strings with the rules of this module (total order).
pub fn collate(a: &str, b: &str) -> Ordering {
    sort_key(a).cmp(&sort_key(b)).then_with(|| a.cmp(b))
}

/// Build the collation key of `s`.
pub fn sort_key(s: &str) -> SortKey {
    let norm = normalize(s);
    // Collapse whitespace runs and trim, so "藤井  風 " == "藤井 風".
    let mut text = String::with_capacity(norm.len());
    for word in norm.split_whitespace() {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(word);
    }
    let body = strip_leading_article(&text);

    let mut primary = Vec::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            let mut digits = String::new();
            digits.push(c);
            while let Some(&d) = chars.peek() {
                if !d.is_ascii_digit() {
                    break;
                }
                digits.push(d);
                chars.next();
            }
            let trimmed = digits.trim_start_matches('0');
            let trimmed = if trimmed.is_empty() { "0" } else { trimmed };
            // Length first, then the digits: compares numbers of any size by value.
            primary.push(elem(G_DIGIT, trimmed.len() as u32));
            primary.extend(trimmed.bytes().map(|b| elem(G_DIGIT, u32::from(b - b'0'))));
            continue;
        }
        if let Some(e) = char_elem(c) {
            primary.push(e);
        }
    }
    SortKey { primary: primary.into_boxed_slice(), secondary: text.into_boxed_str() }
}

/// `"the weeknd"` -> `"weeknd"` (only when a Latin letter or digit follows).
fn strip_leading_article(s: &str) -> &str {
    match s.strip_prefix("the ") {
        Some(rest) if rest.chars().next().is_some_and(|c| c.is_ascii_alphanumeric() || is_latin(c)) => rest,
        _ => s,
    }
}

fn is_latin(c: char) -> bool {
    c.is_ascii_alphabetic() || (('\u{00C0}'..='\u{024F}').contains(&c) && c.is_alphabetic()) || ('\u{1E00}'..='\u{1EFF}').contains(&c)
}

fn is_kana(c: char) -> bool {
    matches!(c, '\u{3041}'..='\u{309F}' | '\u{30A0}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}')
}

fn is_kanji(c: char) -> bool {
    matches!(
        c,
        '\u{3005}'..='\u{3007}'
            | '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{20000}'..='\u{3FFFF}'
    )
}

/// Primary element of one (already normalized, non-digit) char; `None` = ignorable.
fn char_elem(c: char) -> Option<u32> {
    if is_kana(c) {
        return match c {
            // prolonged sound mark: ignorable at the primary level
            'ー' => None,
            '\u{30FD}' => Some(elem(G_KANA, 'ゝ' as u32)),
            '\u{30FE}' => Some(elem(G_KANA, 'ゞ' as u32)),
            // katakana middle dot (・) and other punctuation in the block
            '\u{30A0}' | '\u{30FB}' => Some(elem(G_SYMBOL, c as u32)),
            _ => Some(elem(G_KANA, kana_base(c) as u32)),
        };
    }
    if is_kanji(c) {
        return Some(elem(G_KANJI, c as u32));
    }
    if is_latin(c) {
        return Some(elem(G_LATIN, fold_latin(c) as u32));
    }
    if c.is_alphanumeric() {
        return Some(elem(G_OTHER, c as u32));
    }
    Some(elem(G_SYMBOL, c as u32))
}

/// Large, unvoiced form of a hiragana (`ぁ` -> `あ`, `が` -> `か`, `ぽ` -> `ほ`).
fn kana_base(c: char) -> char {
    let u = c as u32;
    let base = match u {
        // small vowels ぁぃぅぇぉ
        0x3041 | 0x3043 | 0x3045 | 0x3047 | 0x3049 => u + 1,
        // small っ -> つ
        0x3063 => 0x3064,
        // small ゃゅょ
        0x3083 | 0x3085 | 0x3087 => u + 1,
        // small ゎ -> わ
        0x308E => 0x308F,
        // small ゕ ゖ
        0x3095 => 0x304B,
        0x3096 => 0x3051,
        // ゔ -> う
        0x3094 => 0x3046,
        // が..ぢ: voiced forms sit right after their base (か=304B, が=304C, ...)
        0x304C..=0x3062 if u.is_multiple_of(2) => u - 1,
        // づ, で, ど (つ=3064, づ=3065, ...)
        0x3065..=0x3069 if u % 2 == 1 => u - 1,
        // は行: base, voiced, semi-voiced in triples starting at は=306F
        0x306F..=0x307D => u - (u - 0x306F) % 3,
        // ゞ -> ゝ
        0x309E => 0x309D,
        // katakana-only letters left by `normalize` (ヷヸヹヺ, small ㇰ..ㇿ): keep
        _ => u,
    };
    char::from_u32(base).unwrap_or(c)
}

/// Base letter of common accented Latin letters (input is lowercase).
fn fold_latin(c: char) -> char {
    const LATIN1: &str = "aaaaaaæceeeeiiiiðnoooooøuuuuyþy";
    match c {
        // à..ÿ except the division sign (U+00F7)
        '\u{00E0}'..='\u{00F6}' => LATIN1.chars().nth(c as usize - 0xE0).unwrap_or(c),
        '\u{00F8}'..='\u{00FF}' => LATIN1.chars().nth(c as usize - 0xE1).unwrap_or(c),
        'ā' | 'ă' | 'ą' => 'a',
        'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'ď' | 'đ' => 'd',
        'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ī' | 'ĭ' | 'į' | 'ı' => 'i',
        'ł' | 'ľ' | 'ĺ' | 'ļ' => 'l',
        'ń' | 'ň' | 'ņ' => 'n',
        'ō' | 'ŏ' | 'ő' => 'o',
        'ř' | 'ŕ' => 'r',
        'ś' | 'ŝ' | 'ş' | 'š' => 's',
        'ť' | 'ţ' => 't',
        'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'ź' | 'ż' | 'ž' => 'z',
        _ => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(items: &[&str]) -> Vec<String> {
        let mut v: Vec<String> = items.iter().map(|s| s.to_string()).collect();
        v.sort_by(|a, b| collate(a, b));
        v
    }

    #[test]
    fn case_and_width_insensitive() {
        assert_eq!(sort_key("abc").primary, sort_key("ABC").primary);
        assert_eq!(sort_key("ＡＢＣ").primary, sort_key("abc").primary);
        assert_eq!(sort_key("ｶﾀｶﾅ").primary, sort_key("カタカナ").primary);
        assert_eq!(sorted(&["b", "A", "Ｃ", "a"]), vec!["A", "a", "b", "Ｃ"]);
    }

    #[test]
    fn katakana_compares_as_hiragana() {
        assert_eq!(sort_key("ヨアソビ").primary, sort_key("よあそび").primary);
        // あ < か < さ regardless of script
        assert_eq!(sorted(&["サカナ", "かもめ", "アイドル"]), vec!["アイドル", "かもめ", "サカナ"]);
        // voiced / small kana are equal at the primary level, ordered after the base form
        assert_eq!(sort_key("が").primary, sort_key("か").primary);
        assert_eq!(sort_key("ぱ").primary, sort_key("は").primary);
        assert_eq!(sort_key("っ").primary, sort_key("つ").primary);
        assert_eq!(sort_key("ゔ").primary, sort_key("う").primary);
        assert_eq!(sorted(&["が", "か", "き"]), vec!["か", "が", "き"]);
        assert_eq!(sorted(&["ばら", "はる", "ぱん"]), vec!["ばら", "はる", "ぱん"]);
        // the prolonged sound mark is ignored
        assert_eq!(sort_key("ラーメン").primary, sort_key("らめん").primary);
    }

    #[test]
    fn leading_the_is_ignored_for_latin_names() {
        assert_eq!(sorted(&["Vaundy", "The Weeknd", "YOASOBI", "Taylor Swift"]), vec!["Taylor Swift", "Vaundy", "The Weeknd", "YOASOBI"]);
        assert_eq!(sort_key("The Beatles").primary, sort_key("beatles").primary);
        // "The" alone or followed by non-Latin text stays
        assert_eq!(sorted(&["The", "Thea"]), vec!["The", "Thea"]);
        assert_ne!(sort_key("the 東京").primary, sort_key("東京").primary);
        assert_eq!(sorted(&["Weeknd", "The Weeknd"]), vec!["The Weeknd", "Weeknd"]);
    }

    #[test]
    fn natural_number_order() {
        assert_eq!(
            sorted(&["Track 10", "Track 2", "Track 1", "Track 02", "Track"]),
            vec!["Track", "Track 1", "Track 02", "Track 2", "Track 10"]
        );
        assert_eq!(sorted(&["THE BOOK 3", "THE BOOK", "THE BOOK 2"]), vec!["THE BOOK", "THE BOOK 2", "THE BOOK 3"]);
        // full-width digits are digits
        assert_eq!(sort_key("第１０話").primary, sort_key("第10話").primary);
        // arbitrarily long numbers compare by value
        assert_eq!(sorted(&["99999999999999999999999", "100000000000000000000000"]), vec!["99999999999999999999999", "100000000000000000000000"]);
    }

    #[test]
    fn group_order_symbols_digits_latin_other_kana_kanji() {
        let v = sorted(&["米津玄師", "あいみょん", "Ado", "123", "(K)NoW_NAME", "Кино", "ヨルシカ", "藤井 風", "#1", "優里", "ずっと真夜中でいいのに。"]);
        assert_eq!(
            v,
            vec![
                "#1",
                "(K)NoW_NAME",
                "123",
                "Ado",
                "Кино",
                "あいみょん",
                "ずっと真夜中でいいのに。",
                "ヨルシカ",
                // kanji by code point (no reading data): 優 U+512A, 米 U+7C73, 藤 U+85E4
                "優里",
                "米津玄師",
                "藤井 風",
            ]
        );
    }

    #[test]
    fn accents_and_whitespace() {
        assert_eq!(sort_key("Beyoncé").primary, sort_key("beyonce").primary);
        assert_eq!(sorted(&["Bjork", "Björk", "Bjarne"]), vec!["Bjarne", "Bjork", "Björk"]);
        assert_eq!(sort_key("藤井  風 ").primary, sort_key("藤井 風").primary);
    }

    #[test]
    fn total_and_deterministic() {
        // equal keys fall back to the raw strings
        assert_eq!(collate("ABC", "abc"), "ABC".cmp("abc"));
        assert_eq!(collate("x", "x"), Ordering::Equal);
        assert!(SortKey::new("").is_empty());
        assert!(SortKey::new("").cmp(&SortKey::new("a")) == Ordering::Less);
        let items = ["b", "B", "ｂ", "Ｂ", "b "];
        let mut a: Vec<&str> = items.to_vec();
        let mut b: Vec<&str> = items.iter().rev().copied().collect();
        a.sort_by(|x, y| collate(x, y));
        b.sort_by(|x, y| collate(x, y));
        assert_eq!(a, b);
    }
}
