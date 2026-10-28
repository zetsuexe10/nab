use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

const ENDPOINT: &str = "https://api.ytultra.com/ikool/youtube/download";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/156.0.0.0 Safari/537.36";
const SIGN: &str = "e585b970f48cfc7c98cc2987cd9fa5493d14b74cef1de257b3bc33fc86ab5231";
static NONCE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub struct Media {
    pub url: Option<String>,
    pub quality: String,
    pub format: String,
    pub size: u64,
    pub locked: bool,
}

impl Media {
    pub fn is_audio(&self) -> bool {
        self.format.contains(".m4a") || self.format.contains(".weba")
    }

    pub fn available(&self) -> bool {
        !self.locked && self.url.as_deref().is_some_and(|url| !url.is_empty())
    }

    pub fn height(&self) -> u32 {
        self.quality
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .unwrap_or(0)
    }

    pub fn extension(&self) -> String {
        if let Some(start) = self.format.find("[.") {
            let rest = &self.format[start + 2..];
            let end = rest.find(']').unwrap_or(rest.len());
            let extension = rest[..end].trim();
            if !extension.is_empty() {
                return extension.to_string();
            }
        }
        for extension in ["m4a", "weba", "mp4", "webm"] {
            if self.format.contains(extension) {
                return extension.to_string();
            }
        }
        "mp4".to_string()
    }
}

#[derive(Clone, Debug, Default)]
pub struct Resolved {
    pub title: Option<String>,
    pub medias: Vec<Media>,
}

pub fn resolve(video_id: &str) -> Result<Resolved, String> {
    let payload = serde_json::json!({
        "url": format!("https://www.youtube.com/watch?v={video_id}")
    })
    .to_string();
    let mut response = ureq::post(ENDPOINT)
        .header("accept", "*/*")
        .header("content-type", "application/json")
        .header("lang", "en")
        .header("nonce", nonce())
        .header("origin", "https://www.ytultra.com")
        .header("referer", "https://www.ytultra.com/")
        .header("sign", SIGN)
        .header("timestamp", millis().to_string())
        .header("x-app-id", "2")
        .header("user-agent", USER_AGENT)
        .send(payload.as_str())
        .map_err(|error| format!("stream lookup failed: {error}"))?;
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|error| format!("stream lookup failed: {error}"))?;
    let json: Value = serde_json::from_str(&body)
        .map_err(|_| "unexpected response from stream service".to_string())?;
    if json.get("code").and_then(Value::as_str) != Some("0000") {
        let message = json
            .get("msg")
            .and_then(Value::as_str)
            .unwrap_or("stream service error");
        let code = json.get("code").and_then(Value::as_str).unwrap_or("?");
        return Err(format!("stream lookup failed: {message} ({code})"));
    }
    let data = json
        .get("data")
        .ok_or_else(|| "stream service returned no data".to_string())?;
    let title = data
        .get("title")
        .and_then(Value::as_str)
        .filter(|title| !title.is_empty())
        .map(str::to_string);
    let medias = data
        .get("medias")
        .and_then(Value::as_array)
        .ok_or_else(|| "no streams available".to_string())?
        .iter()
        .map(parse_media)
        .collect();
    Ok(Resolved { title, medias })
}

fn millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0)
}

fn nonce() -> String {
    let step = NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut state = millis() as u64
        ^ ((std::process::id() as u64) << 32)
        ^ step.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    let mut out = String::with_capacity(32);
    for _ in 0..4 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.push_str(&format!("{:08x}", state as u32));
    }
    out
}

fn parse_media(value: &Value) -> Media {
    Media {
        url: value
            .get("url")
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
            .map(str::to_string),
        quality: value
            .get("quality")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        format: value
            .get("format")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        size: value.get("fileSize").and_then(Value::as_u64).unwrap_or(0),
        locked: value
            .get("locked")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn makes_a_request_nonce() {
        let first = nonce();
        assert_eq!(first.len(), 32);
        assert!(first.chars().all(|character| character.is_ascii_hexdigit()));
        assert_ne!(first, nonce());
    }

    #[test]
    fn parses_extension_and_height() {
        let media = Media {
            url: Some("x".into()),
            quality: "144p".into(),
            format: "144p (1.96 MB) [.mp4]".into(),
            size: 1,
            locked: false,
        };
        assert_eq!(media.extension(), "mp4");
        assert_eq!(media.height(), 144);
        assert!(media.available());
    }
}
