//! Text formatting helpers (times, offsets).

/// `m:ss`, or `h:mm:ss` from one hour on.
pub fn format_time(ms: u64) -> String {
    let total = ms / 1000;
    let (h, m, s) = (total / 3600, (total / 60) % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// `-m:ss` remaining time.
pub fn format_remaining(position_ms: u64, duration_ms: u64) -> String {
    format!("-{}", format_time(duration_ms.saturating_sub(position_ms)))
}

/// Optional duration; `--:--` when unknown.
pub fn format_duration_opt(ms: Option<u64>) -> String {
    match ms {
        Some(ms) => format_time(ms),
        None => "--:--".to_owned(),
    }
}

/// Lyric offset like `+0.30 s` / `−1.50 s` / `±0.00 s`.
pub fn format_offset(ms: i64) -> String {
    let sign = match ms.signum() {
        1 => "+",
        -1 => "−",
        _ => "±",
    };
    let abs = ms.unsigned_abs();
    format!("{sign}{}.{:02} s", abs / 1000, (abs % 1000) / 10)
}

/// Long total like `1:12:05` (sum of a playlist / queue).
pub fn format_total(ms: u64) -> String {
    format_time(ms)
}

/// Integer with thousands separators: `20000` → `20,000`.
pub fn format_count(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(format_time(0), "0:00");
        assert_eq!(format_time(999), "0:00");
        assert_eq!(format_time(61_000), "1:01");
        assert_eq!(format_time(10 * 60_000 + 5_000), "10:05");
        assert_eq!(format_time(3_600_000 + 2 * 60_000 + 3_000), "1:02:03");
        assert_eq!(format_remaining(30_000, 90_000), "-1:00");
        assert_eq!(format_remaining(100_000, 90_000), "-0:00");
        assert_eq!(format_duration_opt(None), "--:--");
    }

    #[test]
    fn counts() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1000), "1,000");
        assert_eq!(format_count(1234567), "1,234,567");
    }

    #[test]
    fn offsets() {
        assert_eq!(format_offset(0), "±0.00 s");
        assert_eq!(format_offset(300), "+0.30 s");
        assert_eq!(format_offset(-1500), "−1.50 s");
        assert_eq!(format_offset(12_345), "+12.34 s");
    }
}
