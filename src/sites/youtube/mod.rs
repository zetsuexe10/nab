pub mod innertube;
pub mod search;
pub mod ytultra;

use serde_json::Value;

use super::{Cursor, Entry, Page, Provider, Suggestion};
use crate::model::{Scope, Video};
use crate::source::{Kind, Sources, Stream};

pub const ID: &str = "youtube";
pub const INNERTUBE: &str = "innertube";
pub const YTULTRA: &str = "ytultra";

pub struct YouTube;

impl Provider for YouTube {
    fn id(&self) -> &'static str {
        ID
    }

    fn title(&self) -> &'static str {
        "YouTube"
    }

    fn folder(&self) -> &'static str {
        "Youtube"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["yt"]
    }

    fn classify(&self, input: &str) -> Option<Entry> {
        let id = search::video_id(input)?;
        let mut video = Video::stub(&id);
        video.site = ID;
        Some(Entry::Video(video))
    }

    fn search(&self, query: &str, _scope: Scope) -> Result<Page, String> {
        Ok(tag(search::search(query)?))
    }

    fn more(&self, cursor: &Cursor) -> Result<Page, String> {
        let state = cursor
            .state::<search::Cursor>()
            .ok_or_else(|| "unknown cursor".to_string())?;
        Ok(tag(search::more(state)?))
    }

    fn resolve(&self, item: &Video) -> Result<Sources, String> {
        resolve(&item.id)
    }

    fn suggest(&self, query: &str, _scope: Scope) -> Vec<Suggestion> {
        suggest(query)
    }
}

fn tag(page: search::Page) -> Page {
    Page {
        videos: page
            .videos
            .into_iter()
            .map(|mut video| {
                video.site = ID;
                video
            })
            .collect(),
        cursor: page.cursor.map(|cursor| Cursor::new(ID, cursor)),
    }
}

pub fn resolve(id: &str) -> Result<Sources, String> {
    let mut sources = Sources {
        preferred: Some(INNERTUBE),
        ..Sources::default()
    };
    let mut errors: Vec<String> = Vec::new();
    match innertube::resolve(id) {
        Ok(direct) => sources.merge(direct),
        Err(error) => errors.push(format!("{INNERTUBE}: {error}")),
    }
    match ytultra::resolve(id) {
        Ok(resolved) => sources.merge(from_ytultra(resolved)),
        Err(error) => errors.push(format!("{YTULTRA}: {error}")),
    }
    if sources.streams.is_empty() {
        return Err(if errors.is_empty() {
            "no downloadable streams".to_string()
        } else {
            errors.join("; ")
        });
    }
    Ok(sources)
}

fn from_ytultra(resolved: ytultra::Resolved) -> Sources {
    let mut streams = Vec::new();
    for media in resolved.medias.iter().filter(|media| media.available()) {
        let Some(url) = media.url.clone() else {
            continue;
        };
        let kind = if media.is_audio() {
            Kind::Audio
        } else {
            Kind::Video
        };
        streams.push(Stream {
            provider: YTULTRA,
            kind,
            quality: media.quality.clone(),
            ext: media.extension(),
            size: media.size,
            url,
            height: media.height(),
            referer: None,
            extract: false,
        });
    }
    Sources {
        title: resolved.title,
        streams,
        ..Sources::default()
    }
}

fn suggest(query: &str) -> Vec<Suggestion> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let url = format!(
        "https://suggestqueries.google.com/complete/search?client=firefox&ds=yt&q={}",
        super::xhamster::encode(query)
    );
    let Ok(mut response) = ureq::get(&url)
        .header("user-agent", crate::download::USER_AGENT)
        .call()
    else {
        return Vec::new();
    };
    let Ok(body) = response.body_mut().read_to_string() else {
        return Vec::new();
    };
    shape_suggestions(&body)
}

fn shape_suggestions(body: &str) -> Vec<Suggestion> {
    let Ok(data) = serde_json::from_str::<Value>(body) else {
        return Vec::new();
    };
    data.get(1)
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(Value::as_str)
                .filter(|row| !row.trim().is_empty())
                .map(|row| Suggestion {
                    text: row.to_string(),
                    query: row.to_string(),
                    profile: None,
                    tag: "",
                    count: None,
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggestions_are_read_from_the_second_element() {
        let rows = shape_suggestions(r#"["lofi",["lofi girl","lofi hip hop",""],[],{}]"#);
        let texts: Vec<&str> = rows.iter().map(|row| row.text.as_str()).collect();
        assert_eq!(texts, vec!["lofi girl", "lofi hip hop"]);
        assert!(shape_suggestions("nope").is_empty());
        assert!(shape_suggestions("[1]").is_empty());
    }

    #[test]
    fn hides_locked_ytultra_streams() {
        let resolved = ytultra::Resolved {
            title: Some("Khola Janala".into()),
            medias: vec![
                ytultra::Media {
                    url: None,
                    quality: "1080p".into(),
                    format: "1080p (77.16 MB) [.mp4]".into(),
                    size: 80_911_999,
                    locked: true,
                },
                ytultra::Media {
                    url: Some("https://v/720".into()),
                    quality: "720p".into(),
                    format: "720p (25.23 MB) [.mp4]".into(),
                    size: 26_455_880,
                    locked: false,
                },
                ytultra::Media {
                    url: Some("https://a/m4a".into()),
                    quality: "720p".into(),
                    format: "720p (3.29 MB) [.m4a]".into(),
                    size: 3_449_447,
                    locked: false,
                },
            ],
        };
        let sources = from_ytultra(resolved);
        assert_eq!(sources.title.as_deref(), Some("Khola Janala"));
        assert_eq!(sources.streams.len(), 2);
        assert!(sources.streams.iter().all(|stream| stream.size > 0));
        assert_eq!(sources.best_video(None).unwrap().quality, "720p");
        assert_eq!(sources.audio().unwrap().ext, "m4a");
    }
}
