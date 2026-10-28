use std::collections::HashSet;

use serde_json::{json, Value};

const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";
const CLIENT: &str = "2.20261023.01.00";
const SEARCH_API: &str = "https://www.youtube.com/youtubei/v1/search?prettyPrint=false";

#[derive(Clone, Debug, Default)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub duration: String,
    pub views: String,
    pub published: String,
}

impl Video {
    pub fn stub(id: &str) -> Self {
        Video {
            id: id.to_string(),
            ..Video::default()
        }
    }
}

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

impl Page {
    pub fn extend(&self, into: &mut Vec<Video>) {
        let mut seen: HashSet<String> = into.iter().map(|video| video.id.clone()).collect();
        for video in &self.videos {
            if seen.insert(video.id.clone()) {
                into.push(video.clone());
            }
        }
    }
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
        }
    })
}

fn call(body: &Value) -> Result<Value, String> {
    let mut response = ureq::post(SEARCH_API)
        .header("user-agent", USER_AGENT)
        .header("accept-language", "en-US,en;q=0.9")
        .header("content-type", "application/json")
        .header("origin", "https://www.youtube.com")
        .header("referer", "https://www.youtube.com/")
        .send(body.to_string())
        .map_err(|error| format!("search failed: {error}"))?;
    let text = response
        .body_mut()
        .read_to_string()
        .map_err(|error| format!("search failed: {error}"))?;
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
