//! Online lyrics from LRCLIB (<https://lrclib.net>, free, no API key).
//!
//! Lookup: `/api/get` (exact title + artist + album + duration ±2 s) when everything is known,
//! otherwise or on a miss `/api/search`, picking the closest result that has synced lyrics.
//! Some entries carry word-level timing in their `lyricsfile` (YAML); when most lines really
//! have several timed words it is turned into an enhanced LRC (`<mm:ss.xx>` word tags),
//! otherwise the line-synced `syncedLyrics` is used.
//!
//! Only the song's title, artist, album and duration are sent.

use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::Value;
use yaml_rust2::{Yaml, YamlLoader};

const API: &str = "https://lrclib.net/api";
/// LRCLIB asks clients to identify themselves.
const USER_AGENT: &str = concat!("LRC Player/", env!("CARGO_PKG_VERSION"), " (https://github.com/B234340/LRC_Player)");
const TIMEOUT: Duration = Duration::from_secs(10);
/// Search results whose duration differs more than this are not the same recording.
const MAX_DURATION_DIFF_S: f64 = 4.0;

/// What to look for.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LrclibQuery {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: Option<u64>,
}

/// Lookup result.
#[derive(Debug, Clone, PartialEq)]
pub enum LrclibResult {
    /// Synced lyrics as LRC text; `word_sync` = with per-word timing.
    Synced { lrc: String, word_sync: bool },
    /// Found, but only unsynced text.
    PlainOnly,
    Instrumental,
    NotFound,
}

impl LrclibQuery {
    fn duration_s(&self) -> Option<f64> {
        self.duration_ms.map(|ms| ms as f64 / 1000.0)
    }

    /// `/api/get` parameters, when every field is known.
    pub(crate) fn get_params(&self) -> Option<Vec<(&'static str, String)>> {
        let d = self.duration_ms?;
        if self.title.trim().is_empty() || self.artist.trim().is_empty() || self.album.trim().is_empty() {
            return None;
        }
        Some(vec![
            ("track_name", self.title.trim().to_string()),
            ("artist_name", self.artist.trim().to_string()),
            ("album_name", self.album.trim().to_string()),
            ("duration", ((d + 500) / 1000).to_string()),
        ])
    }

    /// `/api/search` parameters (title + artist, or a free-text query without an artist).
    pub(crate) fn search_params(&self) -> Vec<(&'static str, String)> {
        if self.artist.trim().is_empty() {
            vec![("q", self.title.trim().to_string())]
        } else {
            vec![("track_name", self.title.trim().to_string()), ("artist_name", self.artist.trim().to_string())]
        }
    }
}

/// Looks the song up on LRCLIB (blocking; call from a worker thread).
pub fn fetch(query: &LrclibQuery) -> Result<LrclibResult> {
    if query.title.trim().is_empty() {
        return Ok(LrclibResult::NotFound);
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .user_agent(USER_AGENT)
        .build()
        .into();
    if let Some(params) = query.get_params() {
        if let Some(entry) = get_json(&agent, "get", &params)? {
            let found = from_entry(&entry);
            if found != LrclibResult::NotFound && found != LrclibResult::PlainOnly {
                return Ok(found);
            }
        }
    }
    let results = get_json(&agent, "search", &query.search_params())?;
    let results = results.as_ref().and_then(Value::as_array).cloned().unwrap_or_default();
    Ok(match pick(&results, query) {
        Some(entry) => from_entry(entry),
        None if results.iter().any(|e| plain(e).is_some()) => LrclibResult::PlainOnly,
        None => LrclibResult::NotFound,
    })
}

/// GET `/api/{endpoint}`; `None` on 404.
fn get_json(agent: &ureq::Agent, endpoint: &str, params: &[(&'static str, String)]) -> Result<Option<Value>> {
    let url = format!("{API}/{endpoint}");
    let mut req = agent.get(&url);
    for (k, v) in params {
        req = req.query(k, v);
    }
    let mut resp = req.call().with_context(|| format!("LRCLIB {endpoint}"))?;
    let status = resp.status().as_u16();
    if status == 404 {
        return Ok(None);
    }
    if !(200..300).contains(&status) {
        anyhow::bail!("LRCLIB {endpoint}: HTTP {status}");
    }
    let body = resp.body_mut().read_to_string().with_context(|| format!("LRCLIB {endpoint}: reading the response"))?;
    Ok(Some(serde_json::from_str(&body).with_context(|| format!("LRCLIB {endpoint}: invalid JSON"))?))
}

fn synced(e: &Value) -> Option<&str> {
    e.get("syncedLyrics").and_then(Value::as_str).filter(|s| !s.trim().is_empty())
}

fn plain(e: &Value) -> Option<&str> {
    e.get("plainLyrics").and_then(Value::as_str).filter(|s| !s.trim().is_empty())
}

fn has_word_sync(e: &Value) -> bool {
    e.get("hasWordSync").and_then(Value::as_bool).unwrap_or(false)
}

/// Best search result: has synced lyrics, duration within a few seconds (when known), then
/// real word timing, then the closest duration.
pub(crate) fn pick<'a>(results: &'a [Value], query: &LrclibQuery) -> Option<&'a Value> {
    let want = query.duration_s();
    let diff = |e: &Value| match (want, e.get("duration").and_then(Value::as_f64)) {
        (Some(w), Some(d)) => (w - d).abs(),
        (Some(_), None) => f64::MAX,
        (None, _) => 0.0,
    };
    results
        .iter()
        .filter(|e| synced(e).is_some() || has_word_sync(e))
        .filter(|e| diff(e) <= MAX_DURATION_DIFF_S)
        .min_by(|a, b| {
            let words = |e: &Value| u8::from(!(has_word_sync(e) && word_lrc(e).is_some()));
            words(a).cmp(&words(b)).then(diff(a).total_cmp(&diff(b)))
        })
}

/// Turns one LRCLIB entry into a result.
pub(crate) fn from_entry(e: &Value) -> LrclibResult {
    if e.get("instrumental").and_then(Value::as_bool).unwrap_or(false) {
        return LrclibResult::Instrumental;
    }
    if let Some(lrc) = word_lrc(e) {
        return LrclibResult::Synced { lrc, word_sync: true };
    }
    match synced(e) {
        Some(s) => LrclibResult::Synced { lrc: with_header(e, s.trim_end()), word_sync: false },
        None if plain(e).is_some() => LrclibResult::PlainOnly,
        None => LrclibResult::NotFound,
    }
}

fn header(e: &Value) -> String {
    let mut out = String::new();
    for (tag, key) in [("ti", "trackName"), ("ar", "artistName"), ("al", "albumName")] {
        if let Some(v) = e.get(key).and_then(Value::as_str).filter(|v| !v.trim().is_empty()) {
            out.push_str(&format!("[{tag}:{}]\n", v.trim()));
        }
    }
    out.push_str("[re:lrclib.net]\n");
    out
}

fn with_header(e: &Value, body: &str) -> String {
    format!("{}{}\n", header(e), body)
}

fn fmt_time(ms: u64) -> String {
    let cs = (ms + 5) / 10;
    format!("{:02}:{:02}.{:02}", cs / 6000, (cs / 100) % 60, cs % 100)
}

/// Enhanced LRC from the entry's `lyricsfile` when it has real word timing (most lyric lines
/// split into several timed words).
pub(crate) fn word_lrc(e: &Value) -> Option<String> {
    if !has_word_sync(e) {
        return None;
    }
    let doc = YamlLoader::load_from_str(e.get("lyricsfile")?.as_str()?).ok()?;
    let lines = doc.first()?["lines"].as_vec()?;
    let ms = |y: &Yaml| y.as_i64().filter(|v| *v >= 0).map(|v| v as u64);
    let mut out = header(e);
    let (mut lyric_lines, mut word_lines) = (0usize, 0usize);
    for line in lines {
        let text = line["text"].as_str().unwrap_or("");
        let start = ms(&line["start_ms"])?;
        let end = ms(&line["end_ms"]).filter(|&e| e >= start);
        let words: Vec<(&str, u64)> = line["words"]
            .as_vec()
            .map(|ws| ws.iter().filter_map(|w| Some((w["text"].as_str()?, ms(&w["start_ms"])?))).collect())
            .unwrap_or_default();
        if !text.trim().is_empty() {
            lyric_lines += 1;
        }
        if words.len() >= 2 {
            word_lines += 1;
        }
        out.push_str(&format!("[{}]", fmt_time(start)));
        if words.is_empty() {
            out.push_str(text);
        } else {
            let mut last = start;
            for (w, t) in &words {
                let t = (*t).max(last);
                out.push_str(&format!("<{}>{}", fmt_time(t), w));
                last = t;
            }
            out.push_str(&format!("<{}>", fmt_time(end.unwrap_or(last).max(last))));
        }
        out.push('\n');
    }
    // One "word" per line is line-level timing in disguise (common for Japanese entries).
    (lyric_lines > 0 && word_lines * 2 >= lyric_lines).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::lrc_timeline::LyricsTimeline;
    use serde_json::json;

    const WORDS_YAML: &str = "version: '1.0'\nmetadata:\n  title: Song\nlines:\n- text: I've been tryna call\n  start_ms: 27100\n  end_ms: 28198\n  words:\n  - text: 'I''ve '\n    start_ms: 27139\n  - text: 'been '\n    start_ms: 27528\n  - text: 'tryna '\n    start_ms: 27819\n  - text: call\n    start_ms: 28130\n- text: ''\n  start_ms: 28198\n  end_ms: 29000\n- text: On my own\n  start_ms: 29960\n  end_ms: 31779\n  words:\n  - text: 'On '\n    start_ms: 29999\n  - text: 'my '\n    start_ms: 30160\n  - text: own\n    start_ms: 30380\n";
    const LINE_YAML: &str = "version: '1.0'\nlines:\n- text: 夢ならば\n  start_ms: 750\n  end_ms: 5513\n  words:\n  - text: 夢ならば\n    start_ms: 750\n- text: 未だに\n  start_ms: 6550\n  end_ms: 10933\n  words:\n  - text: 未だに\n    start_ms: 6550\n";

    fn entry(duration: f64, synced: Option<&str>, words: Option<&str>) -> Value {
        json!({
            "trackName": "Song", "artistName": "Artist", "albumName": "Album",
            "duration": duration, "instrumental": false,
            "hasWordSync": words.is_some(), "lyricsfile": words,
            "plainLyrics": "a\nb", "syncedLyrics": synced,
        })
    }

    #[test]
    fn word_timing_becomes_enhanced_lrc() {
        let lrc = word_lrc(&entry(200.0, Some("[00:27.10]x"), Some(WORDS_YAML))).unwrap();
        assert!(lrc.starts_with("[ti:Song]\n[ar:Artist]\n[al:Album]\n[re:lrclib.net]\n"));
        assert!(lrc.contains("[00:27.10]<00:27.14>I've <00:27.53>been <00:27.82>tryna <00:28.13>call<00:28.20>\n"), "{lrc}");
        let t = LyricsTimeline::parse(&lrc);
        let line = &t.lines()[0];
        assert_eq!(line.text, "I've been tryna call");
        assert!(line.is_karaoke() && !line.estimated);
        assert_eq!(line.segments.len(), 4);
    }

    #[test]
    fn one_word_per_line_is_not_word_timing() {
        let e = entry(256.0, Some("[00:00.75] 夢ならば\n[00:06.55] 未だに"), Some(LINE_YAML));
        assert!(word_lrc(&e).is_none());
        match from_entry(&e) {
            LrclibResult::Synced { lrc, word_sync } => {
                assert!(!word_sync);
                assert!(lrc.ends_with("[00:00.75] 夢ならば\n[00:06.55] 未だに\n"));
                assert_eq!(LyricsTimeline::parse(&lrc).lines().len(), 2);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn pick_prefers_matching_duration_and_word_timing() {
        let q = LrclibQuery { title: "Song".into(), artist: "Artist".into(), album: String::new(), duration_ms: Some(200_000) };
        let results = vec![
            entry(260.0, Some("[00:01.00]other edit"), None),
            entry(201.0, Some("[00:01.00]line synced"), None),
            entry(199.0, Some("[00:01.00]x"), Some(WORDS_YAML)),
            entry(200.0, None, None),
        ];
        let best = pick(&results, &q).unwrap();
        assert_eq!(best["duration"], 199.0, "word timing wins among close durations");
        let no_words: Vec<Value> = results[..2].to_vec();
        assert_eq!(pick(&no_words, &q).unwrap()["duration"], 201.0, "far duration rejected");
        let unknown = LrclibQuery { duration_ms: None, ..q.clone() };
        assert!(pick(&results[..1], &unknown).is_some(), "any duration when ours is unknown");
        assert!(pick(&results[3..], &q).is_none(), "no synced lyrics");
    }

    #[test]
    fn instrumental_and_plain_only_entries() {
        let mut e = entry(200.0, None, None);
        assert_eq!(from_entry(&e), LrclibResult::PlainOnly);
        e["instrumental"] = json!(true);
        assert_eq!(from_entry(&e), LrclibResult::Instrumental);
        e["instrumental"] = json!(false);
        e["plainLyrics"] = Value::Null;
        assert_eq!(from_entry(&e), LrclibResult::NotFound);
    }

    #[test]
    fn query_parameters() {
        let q = LrclibQuery { title: " 夜に駆ける ".into(), artist: "YOASOBI".into(), album: "THE BOOK".into(), duration_ms: Some(258_600) };
        let get = q.get_params().unwrap();
        assert_eq!(get[0], ("track_name", "夜に駆ける".to_string()));
        assert_eq!(get[3], ("duration", "259".to_string()));
        assert!(LrclibQuery { album: String::new(), ..q.clone() }.get_params().is_none());
        assert_eq!(q.search_params().len(), 2);
        let free = LrclibQuery { artist: String::new(), ..q };
        assert_eq!(free.search_params(), vec![("q", "夜に駆ける".to_string())]);
    }

    /// Talks to the real service.
    #[test]
    #[ignore = "network"]
    fn live_lookup() {
        let q = LrclibQuery { title: "Blinding Lights".into(), artist: "The Weeknd".into(), album: String::new(), duration_ms: Some(200_000) };
        match fetch(&q).unwrap() {
            LrclibResult::Synced { lrc, .. } => assert!(LyricsTimeline::parse(&lrc).lines().len() > 10),
            other => panic!("{other:?}"),
        }
    }
}
