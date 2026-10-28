use std::process::{Command, Stdio};

use crate::search;

const READERS: &[&[&str]] = &[
    &["pbpaste"],
    &["wl-paste", "-n"],
    &["xclip", "-o", "-selection", "clipboard"],
    &["xsel", "-ob"],
    &["powershell", "-NoProfile", "-Command", "Get-Clipboard"],
];

pub fn read() -> Option<String> {
    for reader in READERS {
        let Ok(output) = Command::new(reader[0])
            .args(&reader[1..])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
        else {
            continue;
        };
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

pub fn link(text: &str) -> Option<String> {
    let first = text.lines().next()?.trim();
    if first.is_empty() || first.contains(char::is_whitespace) {
        return None;
    }
    search::video_id(first)?;
    Some(first.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_video_links_are_offered() {
        assert_eq!(
            link("https://www.youtube.com/watch?v=dQw4w9WgXcQ\nmore"),
            Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ".to_string())
        );
        assert_eq!(
            link("https://youtu.be/dQw4w9WgXcQ"),
            Some("https://youtu.be/dQw4w9WgXcQ".to_string())
        );
        assert_eq!(link("lofi beats to study to"), None);
        assert_eq!(link("dQw4w9WgXcQ extra words"), None);
        assert_eq!(link(""), None);
    }
}
