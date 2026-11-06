//! Display metadata (title / artist / album) for track rows.
//!
//! Track lists used to open SQLite and run one un-cached query per row on every frame.
//! [`TrackMetaCache`] keeps resolved rows (including "not in the library" fallbacks) and
//! resolves only rows it has not seen, with one batched `track_metadata_many` query per call
//! on the shared connection. Lists are virtualized, so a call covers the ~20-40 visible rows;
//! scrolling a cached list costs only hash lookups. Cleared when the library changes.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use lrc_player_application::infrastructure::persistence::SharedMetaCache;

use crate::presentation::components::track_list::TrackDisplay;

/// Upper bound on cached rows before the cache is reset.
const MAX_ENTRIES: usize = 100_000;

#[derive(Default)]
pub(crate) struct TrackMetaCache {
    map: HashMap<String, Arc<TrackDisplay>>,
    /// Rows from the library DB (as opposed to file-name fallbacks).
    from_db: HashMap<String, bool>,
}

pub(crate) fn fallback_display(path: &str) -> TrackDisplay {
    let title = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
        .to_string();
    TrackDisplay {
        path: path.to_string(),
        title,
        artist: "Unknown artist".to_string(),
        album: "Unknown album".to_string(),
    }
}

fn or_default(value: String, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value
    }
}

impl TrackMetaCache {
    pub(crate) fn clear(&mut self) {
        self.map.clear();
        self.from_db.clear();
    }

    /// Resolves every path not cached yet with one batched query. Returns false when the
    /// shared connection was busy (the caller shows fallbacks and retries next frame).
    pub(crate) fn ensure<'a>(
        &mut self,
        meta: Option<&SharedMetaCache>,
        paths: impl IntoIterator<Item = &'a String>,
    ) -> bool {
        let missing: Vec<&str> = paths
            .into_iter()
            .filter(|p| !self.map.contains_key(p.as_str()))
            .map(|p| p.as_str())
            .collect();
        if missing.is_empty() {
            return true;
        }
        if self.map.len() + missing.len() > MAX_ENTRIES {
            self.clear();
        }
        let rows = match meta {
            Some(meta) => match meta.try_with(|cache| cache.track_metadata_many(&missing)) {
                Some(Ok(rows)) => rows,
                Some(Err(err)) => {
                    tracing::debug!("track metadata query failed: {err:#}");
                    HashMap::new()
                }
                None => return false,
            },
            None => HashMap::new(),
        };
        let mut rows = rows;
        for path in missing {
            let display = match rows.remove(path) {
                Some(row) => {
                    let fallback = fallback_display(path);
                    self.from_db.insert(path.to_string(), true);
                    TrackDisplay {
                        path: path.to_string(),
                        title: or_default(row.title, &fallback.title),
                        artist: or_default(row.artist, "Unknown artist"),
                        album: or_default(row.album, "Unknown album"),
                    }
                }
                None => fallback_display(path),
            };
            self.map.insert(path.to_string(), Arc::new(display));
        }
        true
    }

    /// Cached display row, or a file-name fallback (not cached) when unresolved.
    pub(crate) fn display(&self, path: &str) -> Arc<TrackDisplay> {
        match self.map.get(path) {
            Some(d) => Arc::clone(d),
            None => Arc::new(fallback_display(path)),
        }
    }

    /// Library tags for `path` if they are cached (used to show something immediately on a
    /// track change while the probe runs).
    pub(crate) fn lookup_tags(&self, path: &str) -> (Option<String>, Option<String>, Option<String>) {
        if !self.from_db.get(path).copied().unwrap_or(false) {
            return (None, None, None);
        }
        match self.map.get(path) {
            Some(d) => (Some(d.title.clone()), Some(d.artist.clone()), Some(d.album.clone())),
            None => (None, None, None),
        }
    }
}
