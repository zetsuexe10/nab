mod bytes;
mod mp4;
mod webm;

#[cfg(test)]
pub mod synth;
#[cfg(test)]
mod tests;

use std::fs::File;
use std::io::Read;
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Container {
    Mp4,
    WebM,
}

impl Container {
    pub fn for_video(ext: &str) -> Container {
        if ext.eq_ignore_ascii_case("webm") {
            Container::WebM
        } else {
            Container::Mp4
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Container::Mp4 => "mp4",
            Container::WebM => "webm",
        }
    }
}

pub fn pairs(video_ext: &str, audio_ext: &str) -> bool {
    matches!(
        (
            video_ext.to_ascii_lowercase().as_str(),
            audio_ext.to_ascii_lowercase().as_str()
        ),
        ("mp4", "m4a") | ("webm", "weba")
    )
}

pub fn sniff(path: &Path) -> Result<Container, String> {
    let mut head = [0u8; 12];
    let mut file =
        File::open(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let read = file
        .read(&mut head)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    if read >= 4 && head[..4] == [0x1a, 0x45, 0xdf, 0xa3] {
        return Ok(Container::WebM);
    }
    let known: [&[u8; 4]; 8] = [
        b"ftyp", b"styp", b"moov", b"free", b"skip", b"wide", b"mdat", b"sidx",
    ];
    if read >= 8 && known.iter().any(|kind| &head[4..8] == kind.as_slice()) {
        return Ok(Container::Mp4);
    }
    Err(format!("{} is not an mp4 or webm file", path.display()))
}

pub fn merge(video: &Path, audio: &Path, out: &Path) -> Result<(), String> {
    let first = sniff(video)?;
    let second = sniff(audio)?;
    if first != second {
        return Err("video and audio are in different containers".to_string());
    }
    match first {
        Container::Mp4 => mp4::merge(video, audio, out),
        Container::WebM => webm::merge(video, audio, out),
    }
}

#[cfg(test)]
pub fn track_counts(path: &Path) -> Result<(usize, usize), String> {
    match sniff(path)? {
        Container::Mp4 => {
            let video = mp4::demux(path)?.into_part(b"vide")?;
            let audio = mp4::demux(path)?.into_part(b"soun")?;
            Ok((video.track.samples.len(), audio.track.samples.len()))
        }
        Container::WebM => {
            let video = webm::demux_for_test(path, 1)?;
            let audio = webm::demux_for_test(path, 2)?;
            Ok((video.count(), audio.count()))
        }
    }
}
