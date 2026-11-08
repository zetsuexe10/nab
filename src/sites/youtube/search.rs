use std::collections::HashSet;

use serde_json::{json, Value};

use crate::model::Video;

const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";
const CLIENT: &str = "2.20261023.01.00";
const SEARCH_API: &str = "https://www.youtube.com/youtubei/v1/search?prettyPrint=false";

#[derive(Clone, Debug)]
pub struct Cursor {
    token: String,
    client: String,
}

#[derive(Debug, Default)]
pub struct Page {
    pub videos: Vec<Video>,
    pub cursor: Option<Cursor>,
}

pub fn search(query: &str) -> Result<Page, String> {
    let body = json!({
        "context": context(),
        "query": query,
    });
    let json = call(&body)?;
    Ok(page_from(&json, CLIENT.to_string()))
}

pub fn more(cursor: &Cursor) -> Result<Page, String> {
    let body = json!({
        "context": context(),
        "continuation": cursor.token,
    });
    let json = call(&body)?;
    Ok(page_from(&json, cursor.client.clone()))
}

fn context() -> Value {
    json!({
        "client": {
            "clientName": "WEB",
            "clientVersion": CLIENT,
            "hl": "en",
            "gl": "US",
        },
        "user": {"lockedSafetyMode": false}
    })
}

fn call(body: &Value) -> Result<Value, String> {
    let headers = [
        ("user-agent", USER_AGENT.to_string()),
        ("accept-language", "en-US,en;q=0.9".to_string()),
        (
            "cookie",
            "SOCS=CAI; CONSENT=YES+1; PREF=f6=40000000&hl=en".to_string(),
        ),
        ("origin", "https://www.youtube.com".to_string()),
        ("referer", "https://www.youtube.com/".to_string()),
    ];
    let text = crate::sites::net::post(SEARCH_API, &headers, &body.to_string(), "search")?;
    serde_json::from_str(&text).map_err(|_| "could not read search results".to_string())
}

fn page_from(json: &Value, client: String) -> Page {
    let mut found = Vec::new();
    collect_videos(json, &mut found);
    let mut seen = HashSet::new();
    found.retain(|video| seen.insert(video.id.clone()));
    let cursor = find_token(json).map(|token| Cursor { token, client });
    Page {
        videos: found,
        cursor,
    }
}

pub fn video_id(input: &str) -> Option<String> {
    let input = input.trim();
    if !input.contains("youtube.com") && !input.contains("youtu.be") {
        return None;
    }
    for marker in ["watch?v=", "youtu.be/", "/shorts/", "/embed/", "/live/"] {
        if let Some(position) = input.find(marker) {
            let rest = &input[position + marker.len()..];
            let id: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .take(11)
                .collect();
            if id.len() == 11 {
                return Some(id);
            }
        }
    }
    None
}

fn find_token(value: &Value) -> Option<String> {
    let mut token = None;
    collect_token(value, &mut token);
    token
}

fn collect_token(value: &Value, token: &mut Option<String>) {
    match value {
        Value::Object(map) => {
            if let Some(endpoint) = map.get("continuationEndpoint") {
                let api = endpoint
                    .pointer("/commandMetadata/webCommandMetadata/apiUrl")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if api.contains("/youtubei/v1/search") {
                    if let Some(found) = endpoint
                        .pointer("/continuationCommand/token")
                        .and_then(Value::as_str)
                    {
                        *token = Some(found.to_string());
                    }
                }
            }
            for nested in map.values() {
                collect_token(nested, token);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_token(item, token);
            }
        }
        _ => {}
    }
}

fn collect_videos(value: &Value, out: &mut Vec<Video>) {
    match value {
        Value::Object(map) => {
            if let Some(renderer) = map.get("videoRenderer") {
                if let Some(video) = parse_renderer(renderer) {
                    out.push(video);
                }
            }
            if let Some(model) = map.get("lockupViewModel") {
                if let Some(video) = parse_lockup(model) {
                    out.push(video);
                }
            }
            if let Some(model) = map.get("shortsLockupViewModel") {
                if let Some(video) = parse_short(model) {
                    out.push(video);
                }
            }
            for nested in map.values() {
                collect_videos(nested, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_videos(item, out);
            }
        }
        _ => {}
    }
}

fn parse_renderer(renderer: &Value) -> Option<Video> {
    let id = renderer.get("videoId")?.as_str()?.to_string();
    let title = text_of(renderer.get("title"));
    if title.is_empty() {
        return None;
    }
    let mut channel = text_of(renderer.get("ownerText"));
    if channel.is_empty() {
        channel = text_of(renderer.get("longBylineText"));
    }
    let mut views = text_of(renderer.get("viewCountText"));
    if views.is_empty() {
        views = text_of(renderer.get("shortViewCountText"));
    }
    let duration = renderer
        .pointer("/lengthText/simpleText")
        .and_then(Value::as_str)
        .unwrap_or("live")
        .to_string();
    let published = renderer
        .pointer("/publishedTimeText/simpleText")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    Some(Video {
        id,
        title,
        channel,
        duration,
        views,
        published,
        ..Video::default()
    })
}

fn parse_lockup(model: &Value) -> Option<Video> {
    if model.get("contentType").and_then(Value::as_str) != Some("LOCKUP_CONTENT_TYPE_VIDEO") {
        return None;
    }
    let id = model.get("contentId")?.as_str()?.to_string();
    let meta = model.pointer("/metadata/lockupMetadataViewModel")?;
    let title = text_of(meta.get("title"));
    if title.is_empty() {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    if let Some(rows) = meta
        .pointer("/metadata/contentMetadataViewModel/metadataRows")
        .and_then(Value::as_array)
    {
        for row in rows {
            if let Some(items) = row.get("metadataParts").and_then(Value::as_array) {
                for item in items {
                    let text = text_of(item.get("text"));
                    if !text.is_empty() {
                        parts.push(text);
                    }
                }
            }
        }
    }
    let mut duration = String::new();
    if let Some(overlays) = model
        .pointer("/contentImage/thumbnailViewModel/overlays")
        .and_then(Value::as_array)
    {
        for overlay in overlays {
            if let Some(badges) = overlay
                .pointer("/thumbnailBottomOverlayViewModel/badges")
                .and_then(Value::as_array)
            {
                for badge in badges {
                    let text = badge
                        .pointer("/thumbnailBadgeViewModel/text")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if !text.is_empty() {
                        duration = text.to_string();
                    }
                }
            }
        }
    }
    let views = parts
        .iter()
        .find(|part| part.contains("view") || part.contains("watching"))
        .cloned()
        .unwrap_or_default();
    let published = parts
        .iter()
        .find(|part| {
            part.contains("ago") || part.contains("Streamed") || part.contains("Premiered")
        })
        .cloned()
        .unwrap_or_default();
    let channel = parts
        .iter()
        .find(|part| **part != views && **part != published)
        .cloned()
        .unwrap_or_default();
    Some(Video {
        id,
        title,
        channel,
        duration: if duration.is_empty() {
            "live".to_string()
        } else {
            duration
        },
        views,
        published,
        ..Video::default()
    })
}

fn parse_short(model: &Value) -> Option<Video> {
    let id = model
        .pointer("/onTap/innertubeCommand/reelWatchEndpoint/videoId")
        .and_then(Value::as_str)
        .or_else(|| {
            model
                .get("entityId")
                .and_then(Value::as_str)
                .map(|id| id.trim_start_matches("shorts-shelf-item-"))
        })?
        .to_string();
    let title = text_of(model.pointer("/overlayMetadata/primaryText"));
    if title.is_empty() || id.len() != 11 {
        return None;
    }
    Some(Video {
        id,
        title,
        channel: String::new(),
        duration: "short".to_string(),
        views: text_of(model.pointer("/overlayMetadata/secondaryText")),
        ..Video::default()
    })
}

fn text_of(value: Option<&Value>) -> String {
    let Some(Value::Object(map)) = value else {
        return String::new();
    };
    if let Some(text) = map.get("simpleText").and_then(Value::as_str) {
        return text.to_string();
    }
    if let Some(runs) = map.get("runs").and_then(Value::as_array) {
        let mut out = String::new();
        for run in runs {
            if let Some(text) = run.get("text").and_then(Value::as_str) {
                out.push_str(text);
            }
        }
        return out;
    }
    if let Some(text) = map.get("content").and_then(Value::as_str) {
        return text.to_string();
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_renderers() {
        let json: Value = serde_json::from_str(
            r#"{"contents":{"items":[
            {"videoRenderer":{"videoId":"JZKoY1Lfs4Q","title":{"runs":[{"text":"Khola Janala"}]},"ownerText":{"runs":[{"text":"SWAT"}]},"lengthText":{"simpleText":"5:40"},"viewCountText":{"simpleText":"1,234 views"},"publishedTimeText":{"simpleText":"2 years ago"}}},
            {"videoRenderer":{"videoId":"JZKoY1Lfs4Q","title":{"runs":[{"text":"duplicate"}]}}}
        ]}}"#,
        )
        .unwrap();
        let mut videos = Vec::new();
        collect_videos(&json, &mut videos);
        assert_eq!(videos.len(), 2);
        assert_eq!(videos[0].title, "Khola Janala");
        assert_eq!(videos[0].duration, "5:40");
        assert_eq!(videos[0].channel, "SWAT");
    }

    #[test]
    fn parses_lockup_and_short_view_models() {
        let json: Value = serde_json::from_str(
            r#"{"items":[
            {"lockupViewModel":{"contentId":"abcdefghijk","contentType":"LOCKUP_CONTENT_TYPE_VIDEO",
              "contentImage":{"thumbnailViewModel":{"overlays":[{"thumbnailBottomOverlayViewModel":{"badges":[{"thumbnailBadgeViewModel":{"text":"12:34"}}]}}]}},
              "metadata":{"lockupMetadataViewModel":{"title":{"content":"A mature title"},
                "metadata":{"contentMetadataViewModel":{"metadataRows":[
                  {"metadataParts":[{"text":{"content":"Some Channel"}}]},
                  {"metadataParts":[{"text":{"content":"1.2M views"}},{"text":{"content":"3 years ago"}}]}]}}}}}},
            {"lockupViewModel":{"contentId":"PLxxxxxxxxx","contentType":"LOCKUP_CONTENT_TYPE_PLAYLIST"}},
            {"shortsLockupViewModel":{"onTap":{"innertubeCommand":{"reelWatchEndpoint":{"videoId":"zyxwvutsrqp"}}},
              "overlayMetadata":{"primaryText":{"content":"A short"},"secondaryText":{"content":"4M views"}}}}
        ]}"#,
        )
        .unwrap();
        let mut videos = Vec::new();
        collect_videos(&json, &mut videos);
        assert_eq!(videos.len(), 2);
        assert_eq!(videos[0].id, "abcdefghijk");
        assert_eq!(videos[0].title, "A mature title");
        assert_eq!(videos[0].channel, "Some Channel");
        assert_eq!(videos[0].duration, "12:34");
        assert_eq!(videos[0].views, "1.2M views");
        assert_eq!(videos[0].published, "3 years ago");
        assert_eq!(videos[1].duration, "short");
        assert_eq!(videos[1].views, "4M views");
    }

    #[test]
    fn reads_cursor_and_client() {
        let json: Value = serde_json::from_str(
            r#"{"contents":{"items":[
            {"continuationItemRenderer":{"continuationEndpoint":{
                "commandMetadata":{"webCommandMetadata":{"sendPost":true,"apiUrl":"/youtubei/v1/search"}},
                "continuationCommand":{"token":"abc"}}}}
        ]}}"#,
        )
        .unwrap();
        let page = page_from(&json, "9.9".to_string());
        let cursor = page.cursor.unwrap();
        assert_eq!(cursor.token, "abc");
        assert_eq!(cursor.client, "9.9");
    }

    #[test]
    fn ignores_foreign_continuations() {
        let json: Value = serde_json::from_str(
            r#"{"x":{"continuationEndpoint":{
            "commandMetadata":{"webCommandMetadata":{"apiUrl":"/youtubei/v1/next"}},
            "continuationCommand":{"token":"other"}}}}"#,
        )
        .unwrap();
        assert!(page_from(&json, "1".to_string()).cursor.is_none());
    }

    #[test]
    fn detects_video_ids() {
        assert_eq!(
            video_id("https://www.youtube.com/watch?v=JZKoY1Lfs4Q&t=10"),
            Some("JZKoY1Lfs4Q".to_string())
        );
        assert_eq!(
            video_id("https://youtu.be/dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".to_string())
        );
        assert_eq!(video_id("superman theme"), None);
    }
}
