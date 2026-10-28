use crate::innertube;
use crate::ytultra;

pub const INNERTUBE: &str = "innertube";
pub const YTULTRA: &str = "ytultra";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Muxed,
    Video,
    Audio,
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
}

impl Stream {
    pub fn is_audio(&self) -> bool {
        self.kind == Kind::Audio
    }
}

#[derive(Clone, Debug, Default)]
pub struct Sources {
    pub title: Option<String>,
    pub streams: Vec<Stream>,
}

impl Sources {
    pub fn merge(&mut self, other: Sources) {
        if self.title.is_none() {
            self.title = other.title;
        }
        for stream in other.streams {
            if stream.url.is_empty() {
                continue;
            }
            if self
                .streams
                .iter()
                .any(|existing| existing.url == stream.url)
            {
                continue;
            }
            self.streams.push(stream);
        }
        self.sort();
    }

    fn sort(&mut self) {
        self.streams.sort_by(|left, right| {
            (!left.is_audio())
                .cmp(&(!right.is_audio()))
                .then_with(|| {
                    let first = (left.provider != INNERTUBE).cmp(&(right.provider != INNERTUBE));
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

    pub fn audio(&self) -> Option<&Stream> {
        self.streams.iter().find(|stream| stream.is_audio())
    }

    pub fn audio_for(&self, video: &Stream) -> Option<&Stream> {
        if video.kind == Kind::Muxed {
            return None;
        }
        let wanted = if video.ext == "webm" { "weba" } else { "m4a" };
        self.streams
            .iter()
            .find(|stream| stream.is_audio() && stream.ext == wanted)
    }

    pub fn videos(&self) -> Vec<&Stream> {
        self.streams
            .iter()
            .filter(|stream| !stream.is_audio())
            .collect()
    }

    pub fn best_video(&self, max_height: Option<u32>) -> Option<&Stream> {
        self.videos()
            .into_iter()
            .find(|stream| max_height.is_none_or(|max| stream.height <= max))
    }
}

pub fn resolve(id: &str) -> Result<Sources, String> {
    let mut sources = Sources::default();
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
        });
    }
    Sources {
        title: resolved.title,
        streams,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        }
    }

    fn sample() -> Sources {
        let mut sources = Sources::default();
        sources.merge(Sources {
            title: Some("clip".into()),
            streams: vec![
                stream(YTULTRA, Kind::Video, "480p", "mp4", 2_000_000),
                stream(YTULTRA, Kind::Video, "720p", "webm", 3_000_000),
                stream(YTULTRA, Kind::Audio, "128k", "m4a", 5_000_000),
                stream(YTULTRA, Kind::Audio, "128k", "weba", 5_500_000),
            ],
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
            title: None,
            streams: vec![
                stream(INNERTUBE, Kind::Muxed, "360p", "mp4", 4_000_000),
                stream(INNERTUBE, Kind::Audio, "128k", "m4a", 1_000_000),
            ],
        });
        let muxed = sources.best_video(None).unwrap();
        assert!(sources.audio_for(muxed).is_none());
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

    #[test]
    fn merges_providers_without_duplicates() {
        let mut sources = sample();
        sources.merge(Sources {
            title: None,
            streams: vec![
                stream(INNERTUBE, Kind::Muxed, "720p", "mp4", 9_000_000),
                stream(INNERTUBE, Kind::Audio, "128k", "m4a", 1_000_000),
            ],
        });
        assert_eq!(sources.streams.len(), 6);
        assert_eq!(sources.providers(), vec![INNERTUBE, YTULTRA]);
        assert_eq!(sources.best_video(None).unwrap().provider, INNERTUBE);
        assert_eq!(sources.audio().unwrap().provider, INNERTUBE);
    }

    #[test]
    fn innertube_leads_when_both_providers_answer() {
        let mut sources = Sources {
            title: None,
            streams: vec![stream(YTULTRA, Kind::Video, "720p", "mp4", 8_000_000)],
        };
        sources.merge(Sources {
            title: None,
            streams: vec![stream(INNERTUBE, Kind::Video, "720p", "mp4", 5_000_000)],
        });
        assert_eq!(sources.providers(), vec![INNERTUBE, YTULTRA]);
        let best = sources.best_video(None).unwrap();
        assert_eq!(best.provider, INNERTUBE);
        assert!(sources.best_video(Some(480)).is_none());
    }
}
