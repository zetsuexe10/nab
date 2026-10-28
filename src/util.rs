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
}
