#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricLine {
    pub timestamp_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct LyricDocument {
    lines: Vec<LyricLine>,
}

impl LyricDocument {
    pub fn parse(raw: &str) -> Self {
        let mut lines = Vec::new();

        for source_line in raw.lines() {
            if let Some((timestamp_ms, text)) = parse_lrc_line(source_line) {
                lines.push(LyricLine {
                    timestamp_ms,
                    text,
                });
            }
        }

        lines.sort_by_key(|line| line.timestamp_ms);
        Self { lines }
    }

    pub fn current_line(&self, position_ms: u64) -> Option<&LyricLine> {
        self.lines
            .iter()
            .take_while(|line| line.timestamp_ms <= position_ms)
            .last()
    }

    pub fn lines(&self) -> &[LyricLine] {
        &self.lines
    }
}

fn parse_lrc_line(line: &str) -> Option<(u64, String)> {
    let close_bracket = line.find(']')?;
    let timestamp = line.strip_prefix('[')?.get(..close_bracket - 1)?;
    let text = line.get(close_bracket + 1..)?.trim().to_string();
    let timestamp_ms = parse_timestamp_ms(timestamp)?;
    Some((timestamp_ms, text))
}

fn parse_timestamp_ms(value: &str) -> Option<u64> {
    let mut segments = value.split(':');
    let minutes = segments.next()?.parse::<u64>().ok()?;
    let seconds_fraction = segments.next()?;
    if segments.next().is_some() {
        return None;
    }

    let mut seconds_segments = seconds_fraction.split('.');
    let seconds = seconds_segments.next()?.parse::<u64>().ok()?;
    let centiseconds = seconds_segments
        .next()
        .and_then(|segment| segment.parse::<u64>().ok())
        .unwrap_or(0);

    Some(minutes * 60_000 + seconds * 1_000 + centiseconds * 10)
}