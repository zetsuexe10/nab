pub fn sanitize_filename(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut spaced = true;
    for ch in input.chars() {
        let ch = if ch.is_control()
            || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
        {
            ' '
        } else {
            ch
        };
        if ch.is_whitespace() {
            if !spaced {
                out.push(' ');
                spaced = true;
            }
        } else {
            out.push(ch);
            spaced = false;
        }
    }
    let trimmed = out.trim_matches(|c: char| c == '.' || c.is_whitespace());
    let mut name: String = trimmed.chars().take(120).collect();
    name = name.trim_end().to_string();
    if name.is_empty() {
        name.push_str("untitled");
    }
    name
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn duration(seconds: u64) -> String {
    let (hours, minutes, secs) = (seconds / 3600, (seconds % 3600) / 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes}:{secs:02}")
    }
}

pub fn short_count(value: u64) -> String {
    let (scaled, suffix) = match value {
        0..=999 => return value.to_string(),
        1_000..=999_999 => (value as f64 / 1_000.0, "K"),
        1_000_000..=999_999_999 => (value as f64 / 1_000_000.0, "M"),
        _ => (value as f64 / 1_000_000_000.0, "B"),
    };
    let text = format!("{scaled:.1}");
    format!("{}{suffix}", text.trim_end_matches(".0"))
}

pub fn ago(from: u64, now: u64) -> String {
    let span = now.saturating_sub(from);
    let (count, unit) = match span {
        0..=59 => return "just now".to_string(),
        60..=3_599 => (span / 60, "minute"),
        3_600..=86_399 => (span / 3_600, "hour"),
        86_400..=2_591_999 => (span / 86_400, "day"),
        2_592_000..=31_535_999 => (span / 2_592_000, "month"),
        _ => (span / 31_536_000, "year"),
    };
    format!("{count} {unit}{} ago", if count == 1 { "" } else { "s" })
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_path_separators() {
        assert_eq!(sanitize_filename("AC/DC: Live?"), "AC DC Live");
        assert_eq!(sanitize_filename("  ...  "), "untitled");
        assert_eq!(sanitize_filename("a\t\tb"), "a b");
    }

    #[test]
    fn caps_length() {
        let name = sanitize_filename(&"x".repeat(300));
        assert_eq!(name.chars().count(), 120);
    }

    #[test]
    fn formats_sizes() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0 MB");
    }

    #[test]
    fn formats_durations_counts_and_ages() {
        assert_eq!(duration(65), "1:05");
        assert_eq!(duration(3_725), "1:02:05");
        assert_eq!(short_count(999), "999");
        assert_eq!(short_count(1_000), "1K");
        assert_eq!(short_count(1_260_000), "1.3M");
        assert_eq!(short_count(2_000_000_000), "2B");
        assert_eq!(ago(100, 130), "just now");
        assert_eq!(ago(0, 7_200), "2 hours ago");
        assert_eq!(ago(0, 86_400), "1 day ago");
        assert_eq!(ago(0, 2 * 31_536_000), "2 years ago");
    }
}
