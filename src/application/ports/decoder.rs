// Application-level decoder port placeholder

pub trait DecoderPort {
    fn decode_to_stream(&self, _path: &std::path::Path) -> anyhow::Result<()>;
}
