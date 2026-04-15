// Application-level port for audio output

pub trait AudioOutputPort {
    fn start(&mut self) -> anyhow::Result<()>;
}
