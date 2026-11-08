use serde_json::Value;

use crate::model::Meta;

const COMMENT_LIMIT: usize = 2000;
const AUTO_MARK: &str = "Provided to YouTube by";

#[derive(Debug, Default, PartialEq, Eq)]
struct Auto {
    title: Option<String>,
    artists: Vec<String>,
    album: Option<String>,
    released: Option<String>,
    composer: Option<String>,
}

fn clean(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn parse_auto(description: &str) -> Option<Auto> {
    let text = description.replace("\r\n", "\n");
    let paragraphs: Vec<&str> = text
        .split("\n\n")
        .map(str::trim)
        .filter(|paragraph| !paragraph.is_empty())
        .collect();
    if !paragraphs.first()?.starts_with(AUTO_MARK) {
        return None;
    }
    let mut auto = Auto::default();
    if let Some(line) = paragraphs.get(1) {
        let mut parts = line
            .split(" · ")
            .map(str::trim)
            .filter(|part| !part.is_empty());
        auto.title = parts.next().map(str::to_string);
        auto.artists = parts.map(str::to_string).collect();
    }
    if let Some(line) = paragraphs.get(2) {
        if !line.starts_with('℗') && !line.starts_with("Released on:") && !line.contains(": ") {
            auto.album = clean(line);
        }
    }
    for line in text.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("Released on:") {
            auto.released = clean(value);
        } else if let Some(value) = line.strip_prefix("Composer:") {
            auto.composer = clean(value);
        }
    }
    auto.title.as_ref()?;
    Some(auto)
}

fn topic(author: &str) -> &str {
    author.strip_suffix(" - Topic").unwrap_or(author).trim()
}

fn day(value: &str) -> Option<String> {
    let head: String = value.trim().chars().take(10).collect();
    let bytes = head.as_bytes();
    let shaped = bytes.len() == 10
        && bytes.iter().enumerate().all(|(index, byte)| {
            if index == 4 || index == 7 {
                *byte == b'-'
            } else {
                byte.is_ascii_digit()
            }
        });
    shaped.then_some(head)
}

fn capped(text: &str) -> String {
    if text.chars().count() <= COMMENT_LIMIT {
        return text.to_string();
    }
    let cut: String = text.chars().take(COMMENT_LIMIT).collect();
    format!("{}…", cut.trim_end())
}

fn covers(json: &Value, id: &str) -> Vec<String> {
    let mut found: Vec<(u64, String)> = Vec::new();
    for path in [
        "/videoDetails/thumbnail/thumbnails",
        "/microformat/playerMicroformatRenderer/thumbnail/thumbnails",
    ] {
        let Some(items) = json.pointer(path).and_then(Value::as_array) else {
            continue;
        };
        for item in items {
            let Some(url) = item
                .get("url")
                .and_then(Value::as_str)
                .filter(|url| url.starts_with("https://"))
            else {
                continue;
            };
            let area = item.get("width").and_then(Value::as_u64).unwrap_or(0)
                * item.get("height").and_then(Value::as_u64).unwrap_or(0);
            found.push((area, url.to_string()));
        }
    }
    found.sort_by_key(|item| std::cmp::Reverse(item.0));
    let mut urls: Vec<String> = Vec::new();
    for (_, url) in found {
        if !urls.contains(&url) {
            urls.push(url);
        }
    }
    if !id.is_empty() {
        for name in ["maxresdefault", "hqdefault"] {
            let url = format!("https://i.ytimg.com/vi/{id}/{name}.jpg");
            if !urls.contains(&url) {
                urls.push(url);
            }
        }
    }
    urls
}

pub fn from_player(json: &Value, id: &str) -> Option<Meta> {
    let text = |path: &str| json.pointer(path).and_then(Value::as_str).and_then(clean);
    let title = text("/videoDetails/title");
    let author = text("/videoDetails/author");
    let description = text("/videoDetails/shortDescription");
    let published = text("/microformat/playerMicroformatRenderer/publishDate")
        .or_else(|| text("/microformat/playerMicroformatRenderer/uploadDate"))
        .and_then(|value| day(&value));
    let auto = description.as_deref().and_then(parse_auto);
    let mut meta = Meta {
        title,
        artist: author.as_deref().map(|name| topic(name).to_string()),
        date: published,
        url: (!id.is_empty()).then(|| format!("https://www.youtube.com/watch?v={id}")),
        cover_urls: covers(json, id),
        ..Meta::default()
    };
    match auto {
        Some(auto) => {
            if let Some(song) = auto.title {
                meta.title = Some(song);
            }
            if !auto.artists.is_empty() {
                meta.album_artist = auto.artists.first().cloned();
                meta.artist = Some(auto.artists.join("; "));
            }
            meta.album = auto.album;
            meta.composer = auto.composer;
            if let Some(released) = auto.released.as_deref().and_then(day) {
                meta.date = Some(released);
            }
        }
        None => {
            meta.comment = description.as_deref().map(capped);
        }
    }
    (!meta.is_empty()).then_some(meta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const AUTO_DESCRIPTION: &str = "Provided to YouTube by Example Records\n\nBlue Hour · Ana Lima · Sam Roe\n\nNight Shift\n\n℗ 2021 Example Records\n\nReleased on: 2021-03-05\n\nComposer: Ana Lima\nLyricist: Sam Roe\n\nAuto-generated by YouTube.";

    #[test]
    fn reads_the_auto_generated_music_description() {
        let auto = parse_auto(AUTO_DESCRIPTION).unwrap();
        assert_eq!(auto.title.as_deref(), Some("Blue Hour"));
        assert_eq!(auto.artists, vec!["Ana Lima", "Sam Roe"]);
        assert_eq!(auto.album.as_deref(), Some("Night Shift"));
        assert_eq!(auto.released.as_deref(), Some("2021-03-05"));
        assert_eq!(auto.composer.as_deref(), Some("Ana Lima"));
        assert!(parse_auto("just a regular description\n\nwith paragraphs").is_none());
    }

    #[test]
    fn music_tracks_get_album_artists_and_release_dates() {
        let json = json!({
            "videoDetails": {
                "title": "Blue Hour",
                "author": "Ana Lima - Topic",
                "shortDescription": AUTO_DESCRIPTION,
                "thumbnail": {"thumbnails": [
                    {"url": "https://lh3.example/small=w60", "width": 60, "height": 60},
                    {"url": "https://lh3.example/big=w544", "width": 544, "height": 544}
                ]}
            },
            "microformat": {"playerMicroformatRenderer": {"publishDate": "2021-03-06T00:00:00-08:00"}}
        });
        let meta = from_player(&json, "abcdefghijk").unwrap();
        assert_eq!(meta.title.as_deref(), Some("Blue Hour"));
        assert_eq!(meta.artist.as_deref(), Some("Ana Lima; Sam Roe"));
        assert_eq!(meta.album_artist.as_deref(), Some("Ana Lima"));
        assert_eq!(meta.album.as_deref(), Some("Night Shift"));
        assert_eq!(meta.date.as_deref(), Some("2021-03-05"));
        assert_eq!(meta.composer.as_deref(), Some("Ana Lima"));
        assert!(meta.comment.is_none());
        assert_eq!(meta.cover_urls[0], "https://lh3.example/big=w544");
        assert_eq!(
            meta.url.as_deref(),
            Some("https://www.youtube.com/watch?v=abcdefghijk")
        );
        assert!(meta.cover_urls.last().unwrap().ends_with("hqdefault.jpg"));
    }

    #[test]
    fn plain_videos_keep_the_channel_and_description() {
        let json = json!({
            "videoDetails": {
                "title": "Some talk",
                "author": "A Channel",
                "shortDescription": "about things"
            },
            "microformat": {"playerMicroformatRenderer": {"publishDate": "2019-11-02T10:00:00-07:00"}}
        });
        let meta = from_player(&json, "abcdefghijk").unwrap();
        assert_eq!(meta.artist.as_deref(), Some("A Channel"));
        assert_eq!(meta.comment.as_deref(), Some("about things"));
        assert_eq!(meta.date.as_deref(), Some("2019-11-02"));
        assert!(meta.album.is_none());
    }

    #[test]
    fn long_descriptions_are_capped() {
        let json = json!({"videoDetails": {"title": "t", "shortDescription": "x".repeat(5000)}});
        let meta = from_player(&json, "").unwrap();
        assert_eq!(meta.comment.unwrap().chars().count(), COMMENT_LIMIT + 1);
        assert!(meta.url.is_none());
    }
}
