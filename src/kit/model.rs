#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Media {
    #[default]
    Video,
    Gallery,
    Person,
}

#[derive(Clone, Debug, Default)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub duration: String,
    pub views: String,
    pub published: String,
    pub url: Option<String>,
    pub author_url: Option<String>,
    pub media: Media,
    pub site: &'static str,
}

impl Video {
    pub fn stub(id: &str) -> Self {
        Video {
            id: id.to_string(),
            ..Video::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Photo {
    pub id: String,
    pub url: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Scope {
    #[default]
    Videos,
    Galleries,
    Stars,
}

impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Scope::Videos => "videos",
            Scope::Galleries => "galleries",
            Scope::Stars => "stars",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paging {
    pub page: u32,
    pub last: u32,
    pub template: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub id: String,
    pub label: String,
    pub count: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct Profile {
    pub name: String,
    pub kind: &'static str,
    pub handle: Option<String>,
    pub verified: bool,
    pub country: Option<String>,
    pub gender: Option<String>,
    pub subscribers: Option<u64>,
    pub views: Option<u64>,
    pub video_count: Option<u64>,
    pub gallery_count: Option<u64>,
    pub aliases: Vec<String>,
    pub socials: Vec<(String, String)>,
    pub bio: Option<String>,
    pub info: Vec<(String, String)>,
    pub videos: Vec<Video>,
    pub scope: Scope,
    pub paging: Paging,
    pub url: String,
    pub sorts: Vec<Choice>,
    pub sort: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Comment {
    pub id: String,
    pub author: String,
    pub text: String,
    pub likes: u64,
    pub created: Option<u64>,
    pub reply_to: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct CommentPage {
    pub comments: Vec<Comment>,
    pub total: Option<u64>,
    pub page: u32,
    pub last: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cover {
    pub mime: &'static str,
    pub data: Vec<u8>,
}

impl Cover {
    pub fn sniff(data: Vec<u8>) -> Option<Cover> {
        let mime = if data.starts_with(&[0xff, 0xd8, 0xff]) {
            "image/jpeg"
        } else if data.starts_with(&[0x89, b'P', b'N', b'G']) {
            "image/png"
        } else if data.len() > 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
            "image/webp"
        } else {
            return None;
        };
        Some(Cover { mime, data })
    }

    pub fn extension(&self) -> &'static str {
        match self.mime {
            "image/png" => "png",
            "image/webp" => "webp",
            _ => "jpg",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub composer: Option<String>,
    pub track: Option<u32>,
    pub track_total: Option<u32>,
    pub date: Option<String>,
    pub comment: Option<String>,
    pub url: Option<String>,
    pub cover_urls: Vec<String>,
    pub cover: Option<Cover>,
}

impl Meta {
    pub fn basic(title: &str, artist: &str) -> Meta {
        let clean = |text: &str| Some(text.trim().to_string()).filter(|text| !text.is_empty());
        Meta {
            title: clean(title),
            artist: clean(artist),
            ..Meta::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.artist.is_none()
            && self.album.is_none()
            && self.album_artist.is_none()
            && self.composer.is_none()
            && self.track.is_none()
            && self.date.is_none()
            && self.comment.is_none()
            && self.url.is_none()
            && self.cover.is_none()
    }

    pub fn audio(mut self) -> Meta {
        if self.album.is_some() {
            return self;
        }
        let Some(title) = self.title.clone() else {
            return self;
        };
        if let Some((artist, song)) = title.split_once(" - ") {
            let (artist, song) = (artist.trim(), song.trim());
            if !artist.is_empty() && !song.is_empty() && !song.contains(" - ") {
                self.artist = Some(artist.to_string());
                self.title = Some(song.to_string());
            }
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_are_recognised_by_their_first_bytes() {
        assert_eq!(
            Cover::sniff(vec![0xff, 0xd8, 0xff, 0xe0]).unwrap().mime,
            "image/jpeg"
        );
        assert_eq!(
            Cover::sniff(b"\x89PNG\r\n\x1a\n".to_vec())
                .unwrap()
                .extension(),
            "png"
        );
        let mut webp = b"RIFF\0\0\0\0WEBPVP8 ".to_vec();
        webp.push(0);
        assert_eq!(Cover::sniff(webp).unwrap().mime, "image/webp");
        assert!(Cover::sniff(b"<html>".to_vec()).is_none());
    }

    #[test]
    fn audio_splits_artist_and_song_from_a_dashed_title() {
        let meta = Meta::basic("Rick Astley - Never Gonna Give You Up", "RickAstleyVEVO").audio();
        assert_eq!(meta.artist.as_deref(), Some("Rick Astley"));
        assert_eq!(meta.title.as_deref(), Some("Never Gonna Give You Up"));
        let plain = Meta::basic("Lofi beats", "Lofi Girl").audio();
        assert_eq!(plain.artist.as_deref(), Some("Lofi Girl"));
        let many = Meta::basic("A - B - C", "chan").audio();
        assert_eq!(many.title.as_deref(), Some("A - B - C"));
        let mut album = Meta::basic("A - B", "chan");
        album.album = Some("Record".into());
        assert_eq!(album.audio().title.as_deref(), Some("A - B"));
    }
}
