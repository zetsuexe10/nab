use std::sync::Mutex;

use serde_json::{json, Value};

use crate::source::{Kind, Sources, Stream, INNERTUBE};

const KEY: &str = "AIzaSyAO_FJ2SlqU8Q4STEHLGCilw_Y9_11qcW8";
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";

static VISITOR: Mutex<Option<String>> = Mutex::new(None);

pub fn resolve(video_id: &str) -> Result<Sources, String> {
    let mut outcome = None;
    for _ in 0..2 {
        match request(video_id, visitor().as_deref()) {
            Ok(sources) => return Ok(sources),
            Err(error) => {
                outcome = Some(error);
                forget_visitor();
            }
        }
    }
    Err(outcome.unwrap_or_else(|| "stream lookup failed".to_string()))
}

fn request(video_id: &str, visitor: Option<&str>) -> Result<Sources, String> {
    let mut client = client();
    if let Some(visitor) = visitor {
        client["visitorData"] = json!(visitor);
    }
    let body = json!({
        "context": {"client": client},
        "videoId": video_id,
        "contentCheckOk": true,
        "racyCheckOk": true,
        "playbackContext": {
            "contentPlaybackContext": {"html5Preference": "HTML5_PREF_WANTS"}
        }
    });
    let text = post("player", &body, visitor)?;
    let json: Value = serde_json::from_str(&text)
        .map_err(|_| "youtube sent an unexpected response".to_string())?;
    parse(&json)
}

fn visitor() -> Option<String> {
    if let Ok(guard) = VISITOR.lock() {
        if let Some(token) = guard.as_ref() {
            return Some(token.clone());
        }
    }
    let token = fetch_visitor();
    if let Ok(mut guard) = VISITOR.lock() {
        *guard = token.clone();
    }
    token
}

fn forget_visitor() {
    if let Ok(mut guard) = VISITOR.lock() {
        *guard = None;
    }
}

fn fetch_visitor() -> Option<String> {
    let body = json!({"context": {"client": client()}});
    let text = post("visitor_id", &body, None).ok()?;
    let json: Value = serde_json::from_str(&text).ok()?;
    visitor_from(&json)
}

fn visitor_from(json: &Value) -> Option<String> {
    json.pointer("/responseContext/visitorData")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
}

fn client() -> Value {
    json!({
        "clientName": "VISIONOS",
        "clientVersion": "1.02",
        "deviceMake": "Apple",
        "deviceModel": "RealityDevice17,1",
        "osName": "visionOS",
        "osVersion": "26.5.23O471",
        "hl": "en",
        "gl": "US",
        "timeZone": "UTC",
        "utcOffsetMinutes": 0
    })
}

fn post(name: &str, body: &Value, visitor: Option<&str>) -> Result<String, String> {
    let endpoint =
        format!("https://www.youtube.com/youtubei/v1/{name}?key={KEY}&prettyPrint=false");
    let mut request = ureq::post(endpoint.as_str())
        .header("content-type", "application/json")
        .header("origin", "https://www.youtube.com")
        .header("referer", "https://www.youtube.com/")
        .header("user-agent", USER_AGENT)
        .header("x-youtube-client-name", "101")
        .header("x-youtube-client-version", "1.02");
    if let Some(visitor) = visitor {
        request = request.header("x-goog-visitor-id", visitor);
    }
    let mut response = request
        .send(body.to_string())
        .map_err(|error| format!("stream lookup failed: {error}"))?;
    response
        .body_mut()
        .read_to_string()
        .map_err(|error| format!("stream lookup failed: {error}"))
}

fn parse(json: &Value) -> Result<Sources, String> {
    let status = json
        .pointer("/playabilityStatus/status")
        .and_then(Value::as_str)
        .unwrap_or("");
    if status != "OK" {
        let reason = json
            .pointer("/playabilityStatus/reason")
            .and_then(Value::as_str)
            .filter(|reason| !reason.is_empty())
            .unwrap_or(if status.is_empty() {
                "no playable streams"
            } else {
                status
            });
        return Err(reason.to_string());
    }
    let title = json
        .pointer("/videoDetails/title")
        .and_then(Value::as_str)
        .filter(|title| !title.is_empty())
        .map(str::to_string);
    let streaming = json
        .get("streamingData")
        .ok_or_else(|| "youtube returned no streams".to_string())?;
    let mut built = Sources {
        title,
        streams: Vec::new(),
        detail: None,
    };
    for (key, muxed) in [("formats", true), ("adaptiveFormats", false)] {
        let Some(items) = streaming.get(key).and_then(Value::as_array) else {
            continue;
        };
        for item in items {
            let Some(stream) = parse_format(item, muxed) else {
                continue;
            };
            keep(&mut built.streams, stream);
        }
    }
    if built.streams.is_empty() {
        return Err("youtube returned no downloadable streams".to_string());
    }
    let mut sources = Sources::default();
    sources.merge(built);
    Ok(sources)
}

fn keep(streams: &mut Vec<Stream>, stream: Stream) {
    let duplicate = streams.iter_mut().find(|existing| {
        existing.kind == stream.kind
            && existing.quality == stream.quality
            && existing.ext == stream.ext
    });
    match duplicate {
        Some(existing) if stream.size > existing.size => *existing = stream,
        Some(_) => {}
        None => streams.push(stream),
    }
}

fn parse_format(value: &Value, muxed: bool) -> Option<Stream> {
    let url = value
        .get("url")
        .and_then(Value::as_str)
        .filter(|url| url.starts_with("https://"))?
        .to_string();
    let mime = value.get("mimeType").and_then(Value::as_str).unwrap_or("");
    let base = mime.split(';').next().unwrap_or("").trim().to_string();
    let subtype = base.split('/').nth(1)?;
    if subtype.is_empty() {
        return None;
    }
    let audio = base.starts_with("audio/");
    let ext = match subtype {
        "webm" if audio => "weba",
        "mp4" if audio => "m4a",
        other => other,
    };
    let kind = if audio {
        Kind::Audio
    } else if muxed {
        Kind::Muxed
    } else {
        Kind::Video
    };
    let (quality, height) = if audio {
        (audio_quality(value), 0)
    } else {
        let label = value
            .get("qualityLabel")
            .and_then(Value::as_str)
            .filter(|label| !label.is_empty())
            .unwrap_or("unknown")
            .to_string();
        let height = label
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .unwrap_or(0);
        (label, height)
    };
    let size = value
        .get("contentLength")
        .and_then(Value::as_str)
        .and_then(|length| length.parse::<u64>().ok())
        .unwrap_or(0);
    Some(Stream {
        provider: INNERTUBE,
        kind,
        quality,
        ext: ext.to_string(),
        size,
        url,
        height,
    })
}

fn audio_quality(value: &Value) -> String {
    let bitrate = value
        .get("averageBitrate")
        .or_else(|| value.get("bitrate"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if bitrate > 0 {
        return format!("{}k", bitrate / 1000);
    }
    value
        .get("audioQuality")
        .and_then(Value::as_str)
        .unwrap_or("audio")
        .trim_start_matches("AUDIO_QUALITY_")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Value {
        json!({
            "playabilityStatus": {"status": "OK"},
            "videoDetails": {"title": "Khola Janala"},
            "streamingData": {
                "formats": [
                    {
                        "url": "https://redirector.example/videoplayback?muxed=1",
                        "mimeType": "video/mp4; codecs=\"avc1.64001F, mp4a.40.2\"",
                        "qualityLabel": "360p",
                        "contentLength": "1000000"
                    }
                ],
                "adaptiveFormats": [
                    {
                        "url": "https://redirector.example/videoplayback?v=720avc",
                        "mimeType": "video/mp4; codecs=\"avc1.64001F\"",
                        "qualityLabel": "720p",
                        "contentLength": "5000000"
                    },
                    {
                        "url": "https://redirector.example/videoplayback?v=720av1",
                        "mimeType": "video/mp4; codecs=\"av01.0.04M.08\"",
                        "qualityLabel": "720p",
                        "contentLength": "3000000"
                    },
                    {
                        "url": "https://redirector.example/videoplayback?v=720vp9",
                        "mimeType": "video/webm; codecs=\"vp9\"",
                        "qualityLabel": "720p",
                        "contentLength": "4000000"
                    },
                    {
                        "url": "https://redirector.example/videoplayback?a=opus",
                        "mimeType": "audio/webm; codecs=\"opus\"",
                        "audioQuality": "AUDIO_QUALITY_MEDIUM",
                        "averageBitrate": 136331,
                        "contentLength": "3000000"
                    },
                    {
                        "url": "https://redirector.example/videoplayback?a=m4a",
                        "mimeType": "audio/mp4; codecs=\"mp4a.40.2\"",
                        "averageBitrate": 129491,
                        "contentLength": "2000000"
                    },
                    {
                        "mimeType": "video/mp4; codecs=\"missing url\"",
                        "qualityLabel": "144p"
                    }
                ]
            }
        })
    }

    #[test]
    fn builds_streams_from_player_response() {
        let sources = parse(&fixture()).unwrap();
        assert_eq!(sources.title.as_deref(), Some("Khola Janala"));
        assert_eq!(sources.streams.len(), 5);
        let muxed = sources
            .streams
            .iter()
            .find(|stream| stream.kind == Kind::Muxed)
            .unwrap();
        assert_eq!(muxed.quality, "360p");
        assert_eq!(muxed.ext, "mp4");
        assert_eq!(muxed.height, 360);
        let heights: Vec<u32> = sources
            .videos()
            .iter()
            .map(|stream| stream.height)
            .collect();
        assert_eq!(heights, vec![720, 720, 360]);
        let videos = sources.videos();
        let mp4_720: Vec<&&Stream> = videos
            .iter()
            .filter(|stream| stream.height == 720 && stream.ext == "mp4")
            .collect();
        assert_eq!(mp4_720.len(), 1);
        assert_eq!(mp4_720[0].size, 5_000_000);
        let audio: Vec<&Stream> = sources
            .streams
            .iter()
            .filter(|stream| stream.is_audio())
            .collect();
        assert_eq!(audio.len(), 2);
        assert_eq!(sources.audio().unwrap().ext, "weba");
        assert_eq!(sources.audio().unwrap().quality, "136k");
        let best = sources.best_video(None).unwrap();
        assert_eq!(best.ext, "mp4");
        assert_eq!(best.height, 720);
        assert_eq!(sources.audio_for(best).unwrap().ext, "m4a");
        let webm = sources
            .videos()
            .into_iter()
            .find(|stream| stream.ext == "webm")
            .unwrap();
        assert_eq!(sources.audio_for(webm).unwrap().ext, "weba");
        assert!(sources.videos().iter().all(|stream| stream.height != 480));
        for stream in &sources.streams {
            assert_eq!(stream.provider, INNERTUBE);
        }
    }

    #[test]
    fn reads_visitor_tokens() {
        let json = json!({"responseContext": {"visitorData": "CgtYMGxwcm5n"}});
        assert_eq!(visitor_from(&json).as_deref(), Some("CgtYMGxwcm5n"));
        assert!(visitor_from(&json!({})).is_none());
        assert!(visitor_from(&json!({"responseContext": {"visitorData": ""}})).is_none());
    }

    #[test]
    fn reports_playability_failures() {
        let json = json!({
            "playabilityStatus": {"status": "LOGIN_REQUIRED", "reason": "Sign in to confirm your age"}
        });
        let error = parse(&json).unwrap_err();
        assert_eq!(error, "Sign in to confirm your age");
        let empty = json!({"playabilityStatus": {"status": "OK"}});
        assert_eq!(parse(&empty).unwrap_err(), "youtube returned no streams");
    }
}
