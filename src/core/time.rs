pub const DEFAULT_SEEK_STEP_MS: u64 = 10_000;
pub const POSITION_SAVE_THRESHOLD_MS: u64 = 3_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeekAmount {
    pub milliseconds: u64,
}

impl Default for SeekAmount {
    fn default() -> Self {
        Self {
            milliseconds: DEFAULT_SEEK_STEP_MS,
        }
    }
}