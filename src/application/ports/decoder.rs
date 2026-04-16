use crate::infrastructure::decode::{DecodedSource, SymphoniaDecoder};
use anyhow::Result;

/// Application-level decoder port used by higher-level use cases.
pub trait DecoderPort {
    fn decode_file(&self, path: &std::path::Path) -> Result<DecodedSource>;
}

impl DecoderPort for SymphoniaDecoder {
    fn decode_file(&self, path: &std::path::Path) -> Result<DecodedSource> {
        self.decode_file(path)
    }
}
