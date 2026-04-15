// Placeholder for a buffered output implementation that wraps low-level backends

pub struct BufferedOutput;

impl BufferedOutput {
    pub fn new() -> Self {
        BufferedOutput
    }
}

impl Default for BufferedOutput {
    fn default() -> Self {
        Self::new()
    }
}
