use serde_json::Value;

use crate::search::{self, Video};
use crate::source::{self, Sources};
use crate::xh;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Site {
    YouTube,
    XHamster,
}

impl Site {
    pub fn parse(text: &str) -> Option<Site> {
        match text.trim().to_ascii_lowercase().as_str() {
            "youtube" | "yt" => Some(Site::YouTube),
            "xhamster" | "xh" => Some(Site::XHamster),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Site::YouTube => "youtube",
            Site::XHamster => "xhamster",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Site::YouTube => "YouTube",
            Site::XHamster => "xHamster",
        }
    }

    pub fn next(self) -> Site {
        match self {
            Site::YouTube => Site::XHamster,
            Site::XHamster => Site::YouTube,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Cursor {
    Tube(search::Cursor),
    Xh(xh::Next),
}

#[derive(Default)]
pub struct Page {
    pub videos: Vec<Video>,
    pub cursor: Option<Cursor>,
}

impl Page {
    pub fn extend(&self, into: &mut Vec<Video>) {
        let mut seen: std::collections::HashSet<String> =
            into.iter().map(|video| video.id.clone()).collect();
        for video in &self.videos {
            if seen.insert(video.id.clone()) {
                into.push(video.clone());
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum Entry {
    Search(String),
    Video(Video),
    Profile(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub text: String,
    pub query: String,
    pub profile: Option<String>,
    pub tag: &'static str,
    pub count: Option<u64>,
}

pub fn classify(input: &str) -> Entry {
    let input = input.trim();
    match xh::classify(input) {
        Some(xh::Target::Video(url)) => {
            let id = url.rsplit('/').next().unwrap_or(&url).to_string();
            return Entry::Video(Video {
                id,
                url: Some(url),
                ..Video::default()
            });
        }
        Some(xh::Target::Profile(url)) => return Entry::Profile(url),
        Some(xh::Target::Search(query)) => return Entry::Search(query),
        None => {}
    }
    match search::video_id(input) {
        Some(id) => Entry::Video(Video::stub(&id)),
        None => Entry::Search(input.to_string()),
    }
}

pub fn search(site: Site, query: &str) -> Result<Page, String> {
    match site {
        Site::YouTube => {
            let page = search::search(query)?;
            Ok(Page {
                videos: page.videos,
                cursor: page.cursor.map(Cursor::Tube),
            })
        }
        Site::XHamster => {
            let listing = xh::live().search(query)?;
            Ok(Page {
                videos: listing.videos,
                cursor: listing.next.map(Cursor::Xh),
            })
        }
    }
}

pub fn more(cursor: &Cursor) -> Result<Page, String> {
    match cursor {
        Cursor::Tube(cursor) => {
            let page = search::more(cursor)?;
            Ok(Page {
                videos: page.videos,
                cursor: page.cursor.map(Cursor::Tube),
            })
        }
        Cursor::Xh(next) => {
            let listing = xh::live().more(next)?;
            Ok(Page {
                videos: listing.videos,
                cursor: listing.next.map(Cursor::Xh),
            })
        }
    }
}

pub fn resolve(video: &Video) -> Result<Sources, String> {
    match &video.url {
        Some(url) => xh::live().resolve(url),
        None => source::resolve(&video.id),
    }
}

pub fn profile(url: &str, sort: &str, page: u32) -> Result<xh::Profile, String> {
    xh::live().profile(url, sort, page)
}

pub fn suggest(site: Site, query: &str) -> Vec<Suggestion> {
    match site {
        Site::XHamster => xh::live()
            .suggest(query)
            .into_iter()
            .map(|row| Suggestion {
                profile: row.kind.is_person().then(|| row.url.clone()),
                tag: row.kind.label(),
                text: row.text,
                query: row.query,
                count: row.count,
            })
            .collect(),
        Site::YouTube => youtube_suggest(query),
    }
}

fn youtube_suggest(query: &str) -> Vec<Suggestion> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let url = format!(
        "https://suggestqueries.google.com/complete/search?client=firefox&ds=yt&q={}",
        xh::encode(query)
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
    shape_youtube(&body)
}

fn shape_youtube(body: &str) -> Vec<Suggestion> {
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
    fn input_is_routed_by_what_it_looks_like() {
        match classify("https://xhamster.com/videos/some-title-xhAbC12") {
            Entry::Video(video) => {
                assert_eq!(video.id, "some-title-xhAbC12");
                assert_eq!(
                    video.url.as_deref(),
                    Some("https://xhamster.com/videos/some-title-xhAbC12")
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            classify("https://xhamster.com/creators/amy"),
            Entry::Profile(url) if url.ends_with("/creators/amy")
        ));
        assert!(matches!(
            classify("https://xhamster.com/search/lofi-girl"),
            Entry::Search(query) if query == "lofi girl"
        ));
        assert!(matches!(
            classify("https://youtu.be/dQw4w9WgXcQ"),
            Entry::Video(video) if video.id == "dQw4w9WgXcQ" && video.url.is_none()
        ));
        assert!(matches!(classify("lofi beats"), Entry::Search(query) if query == "lofi beats"));
    }

    #[test]
    fn youtube_suggestions_are_read_from_the_second_element() {
        let rows = shape_youtube(r#"["lofi",["lofi girl","lofi hip hop",""],[],{}]"#);
        let texts: Vec<&str> = rows.iter().map(|row| row.text.as_str()).collect();
        assert_eq!(texts, vec!["lofi girl", "lofi hip hop"]);
        assert!(shape_youtube("nope").is_empty());
        assert!(shape_youtube("[1]").is_empty());
    }

    #[test]
    fn sites_cycle_and_parse() {
        assert_eq!(Site::YouTube.next(), Site::XHamster);
        assert_eq!(Site::XHamster.next(), Site::YouTube);
        assert_eq!(Site::parse(" XH "), Some(Site::XHamster));
        assert_eq!(Site::parse("yt"), Some(Site::YouTube));
        assert_eq!(Site::parse("vimeo"), None);
    }
}
