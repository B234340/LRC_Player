use std::cell::RefCell;
use std::collections::HashMap;

use crate::core::ids::TrackId;
use crate::app::view_models::{AlbumTabVm, ArtistTabVm, TrackRowStaticVm};

/// VmCache centralizes caches previously kept on `AppFacade`.
#[derive(Debug, Default)]
pub struct VmCache {
    pub track_rows: RefCell<HashMap<TrackId, TrackRowStaticVm>>,
    pub album_vm: Option<AlbumTabVm>,
    pub artist_vm: Option<ArtistTabVm>,
}

impl VmCache {
    pub fn new() -> Self {
        Self { track_rows: RefCell::new(HashMap::new()), album_vm: None, artist_vm: None }
    }

    pub fn invalidate_album(&mut self) {
        self.album_vm = None;
    }

    pub fn invalidate_artist(&mut self) {
        self.artist_vm = None;
    }

    pub fn clear_track_rows(&self) {
        self.track_rows.borrow_mut().clear();
    }
}
