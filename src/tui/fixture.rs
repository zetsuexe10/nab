use std::path::PathBuf;

use super::app::{App, Mode};
use crate::play::Player;
use crate::search::Video;
use crate::source::{Kind, Sources, Stream, INNERTUBE, YTULTRA};

pub fn video(index: usize) -> Video {
    let mut video = Video::stub(&format!("id{index:09}"));
    video.title = if index == 1 {
        "日本語のとても長いタイトル 🎵 with wide characters that must not break the layout at all"
            .to_string()
    } else {
        format!("Result number {index} with a fairly long title to clip somewhere")
    };
    video.channel = "Some Channel".to_string();
    video.duration = "3:45".to_string();
    video.views = "1.2M views".to_string();
    video.published = "2 years ago".to_string();
    video
}

pub fn stream(provider: &'static str, kind: Kind, quality: &str, ext: &str, size: u64) -> Stream {
    Stream {
        provider,
        kind,
        quality: quality.to_string(),
        ext: ext.to_string(),
        size,
        url: format!("https://x/{provider}/{quality}/{ext}"),
        height: quality.trim_end_matches(['p', 'k']).parse().unwrap_or(0),
    }
}

pub fn app() -> App {
    let mut app = App::new(
        "lofi".to_string(),
        PathBuf::from("/tmp/out"),
        Some(Player::Mpv),
    );
    app.animate = false;
    app.players = vec![Player::Mpv, Player::Vlc];
    app.results = (0..9).map(video).collect();
    app.mode = Mode::Results;
    let mut sources = Sources::default();
    sources.merge(Sources {
        title: Some("A title".to_string()),
        streams: vec![
            stream(INNERTUBE, Kind::Audio, "128k", "m4a", 3_000_000),
            stream(INNERTUBE, Kind::Video, "720p", "mp4", 20_000_000),
            stream(INNERTUBE, Kind::Muxed, "360p", "mp4", 9_000_000),
            stream(YTULTRA, Kind::Video, "1080p", "webm", 40_000_000),
        ],
    });
    app.current = Some(video(0));
    app.provider = Some(INNERTUBE);
    app.sources = Some(sources);
    app
}
