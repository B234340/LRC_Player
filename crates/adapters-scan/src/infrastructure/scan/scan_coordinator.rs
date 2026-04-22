use lrc_player_adapters_persistence::persistence::MetaCache;

pub struct ScanCoordinator2 {
    _meta: MetaCache,
}

impl ScanCoordinator2 {
    pub fn new(meta: MetaCache) -> Self {
        Self { _meta: meta }
    }
}
