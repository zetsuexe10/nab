use crate::download::Remote;
use crate::model::{Photo, Video};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Muxed,
    Video,
    Audio,
    Gallery,
}

#[derive(Clone, Debug)]
pub struct Stream {
    pub provider: &'static str,
    pub kind: Kind,
    pub quality: String,
    pub ext: String,
    pub size: u64,
    pub url: String,
    pub height: u32,
    pub referer: Option<&'static str>,
    pub extract: bool,
}

impl Stream {
    pub fn is_audio(&self) -> bool {
        self.kind == Kind::Audio
    }

    pub fn is_gallery(&self) -> bool {
        self.kind == Kind::Gallery
    }

    pub fn remote(&self) -> Remote {
        Remote {
            url: self.url.clone(),
            ext: self.ext.clone(),
            referer: self.referer.map(str::to_string),
            extract: self.extract,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Sources {
    pub title: Option<String>,
    pub streams: Vec<Stream>,
    pub detail: Option<Video>,
    pub photos: Vec<Photo>,
    pub preferred: Option<&'static str>,
}

impl Sources {
    pub fn merge(&mut self, other: Sources) {
        if self.title.is_none() {
            self.title = other.title;
        }
        if self.detail.is_none() {
            self.detail = other.detail;
        }
        if self.preferred.is_none() {
            self.preferred = other.preferred;
        }
        if self.photos.is_empty() {
            self.photos = other.photos;
        }
        for stream in other.streams {
            if stream.url.is_empty() {
                continue;
            }
            if self
                .streams
                .iter()
                .any(|existing| existing.url == stream.url && existing.extract == stream.extract)
            {
                continue;
            }
            self.streams.push(stream);
        }
        self.sort();
    }

    pub fn sort(&mut self) {
        let preferred = self.preferred;
        self.streams.sort_by(|left, right| {
            (!left.is_audio())
                .cmp(&(!right.is_audio()))
                .then_with(|| {
                    let first = (Some(left.provider) != preferred)
                        .cmp(&(Some(right.provider) != preferred));
                    if left.is_audio() {
                        first.then(right.size.cmp(&left.size))
                    } else {
                        right
                            .height
                            .cmp(&left.height)
                            .then(first)
                            .then(right.size.cmp(&left.size))
                    }
                })
                .then(left.provider.cmp(right.provider))
        });
    }

    pub fn providers(&self) -> Vec<&'static str> {
        let mut seen: Vec<&'static str> = Vec::new();
        for stream in &self.streams {
            if !seen.contains(&stream.provider) {
                seen.push(stream.provider);
            }
        }
        seen
    }

    pub fn default_provider(&self) -> Option<&'static str> {
        let providers = self.providers();
        self.preferred
            .filter(|wanted| providers.contains(wanted))
            .or(providers.first().copied())
    }

    pub fn audio(&self) -> Option<&Stream> {
        self.streams.iter().find(|stream| stream.is_audio())
    }

    pub fn audio_for(&self, video: &Stream) -> Option<&Stream> {
        if video.kind != Kind::Video {
            return None;
        }
        let wanted = if video.ext == "webm" { "weba" } else { "m4a" };
        self.streams
            .iter()
            .find(|stream| stream.is_audio() && !stream.extract && stream.ext == wanted)
    }

    pub fn videos(&self) -> Vec<&Stream> {
        self.streams
            .iter()
            .filter(|stream| matches!(stream.kind, Kind::Video | Kind::Muxed))
            .collect()
    }

    pub fn best_video(&self, max_height: Option<u32>) -> Option<&Stream> {
        self.videos()
            .into_iter()
            .find(|stream| max_height.is_none_or(|max| stream.height <= max))
    }

    pub fn gallery(&self) -> Option<&Stream> {
        self.streams.iter().find(|stream| stream.is_gallery())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALPHA: &str = "alpha";
    const BETA: &str = "beta";

    fn stream(provider: &'static str, kind: Kind, quality: &str, ext: &str, size: u64) -> Stream {
        Stream {
            provider,
            kind,
            quality: quality.to_string(),
            ext: ext.to_string(),
            size,
            url: format!("https://example.test/{provider}/{quality}/{ext}"),
            height: quality
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .unwrap_or(0),
            referer: None,
            extract: false,
        }
    }

    fn sample() -> Sources {
        let mut sources = Sources::default();
        sources.merge(Sources {
            title: Some("clip".into()),
            streams: vec![
                stream(BETA, Kind::Video, "480p", "mp4", 2_000_000),
                stream(BETA, Kind::Video, "720p", "webm", 3_000_000),
                stream(BETA, Kind::Audio, "128k", "m4a", 5_000_000),
                stream(BETA, Kind::Audio, "128k", "weba", 5_500_000),
            ],
            ..Sources::default()
        });
        sources
    }

    #[test]
    fn keeps_audio_first_and_video_by_height() {
        let sources = sample();
        let order: Vec<_> = sources
            .streams
            .iter()
            .map(|stream| stream.quality.as_str())
            .collect();
        assert_eq!(order, vec!["128k", "128k", "720p", "480p"]);
    }

    #[test]
    fn picks_best_streams_per_kind() {
        let sources = sample();
        assert_eq!(sources.best_video(None).unwrap().quality, "720p");
        assert_eq!(sources.best_video(Some(600)).unwrap().quality, "480p");
        assert_eq!(sources.audio().unwrap().ext, "weba");
        let webm = sources.best_video(None).unwrap();
        assert_eq!(sources.audio_for(webm).unwrap().ext, "weba");
        let mp4 = sources
            .videos()
            .into_iter()
            .find(|stream| stream.height == 480)
            .unwrap();
        assert_eq!(sources.audio_for(mp4).unwrap().ext, "m4a");
    }

    #[test]
    fn muxed_streams_need_no_audio() {
        let mut sources = Sources::default();
        sources.merge(Sources {
            streams: vec![
                stream(ALPHA, Kind::Muxed, "360p", "mp4", 4_000_000),
                stream(ALPHA, Kind::Audio, "128k", "m4a", 1_000_000),
            ],
            ..Sources::default()
        });
        let muxed = sources.best_video(None).unwrap();
        assert!(sources.audio_for(muxed).is_none());
    }

    #[test]
    fn extracted_audio_is_never_paired_with_video() {
        let mut extracted = stream(ALPHA, Kind::Audio, "audio", "m4a", 1_000_000);
        extracted.extract = true;
        let mut sources = Sources::default();
        sources.merge(Sources {
            streams: vec![
                stream(ALPHA, Kind::Video, "720p", "mp4", 4_000_000),
                extracted,
            ],
            ..Sources::default()
        });
        let video = sources.best_video(None).unwrap();
        assert!(sources.audio_for(video).is_none());
        assert!(sources.audio().unwrap().extract);
    }

    #[test]
    fn merges_providers_without_duplicates() {
        let mut sources = sample();
        sources.merge(Sources {
            preferred: Some(ALPHA),
            streams: vec![
                stream(ALPHA, Kind::Muxed, "720p", "mp4", 9_000_000),
                stream(ALPHA, Kind::Audio, "128k", "m4a", 1_000_000),
            ],
            ..Sources::default()
        });
        assert_eq!(sources.streams.len(), 6);
        assert_eq!(sources.providers(), vec![ALPHA, BETA]);
        assert_eq!(sources.best_video(None).unwrap().provider, ALPHA);
        assert_eq!(sources.audio().unwrap().provider, ALPHA);
        assert_eq!(sources.default_provider(), Some(ALPHA));
    }

    #[test]
    fn the_preferred_provider_leads_when_both_answer() {
        let mut sources = Sources {
            streams: vec![stream(BETA, Kind::Video, "720p", "mp4", 8_000_000)],
            ..Sources::default()
        };
        sources.merge(Sources {
            preferred: Some(ALPHA),
            streams: vec![stream(ALPHA, Kind::Video, "720p", "mp4", 5_000_000)],
            ..Sources::default()
        });
        assert_eq!(sources.providers(), vec![ALPHA, BETA]);
        let best = sources.best_video(None).unwrap();
        assert_eq!(best.provider, ALPHA);
        assert!(sources.best_video(Some(480)).is_none());
    }

    #[test]
    fn galleries_are_neither_video_nor_audio() {
        let mut sources = Sources::default();
        sources.merge(Sources {
            streams: vec![stream(ALPHA, Kind::Gallery, "12 photos", "folder", 0)],
            ..Sources::default()
        });
        assert!(sources.best_video(None).is_none());
        assert!(sources.audio().is_none());
        assert!(sources.gallery().is_some());
    }
}
