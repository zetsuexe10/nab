mod bytes;
mod ilst;
mod mkv;
mod mp4;
mod mtags;
mod webm;

#[cfg(test)]
pub mod synth;
#[cfg(test)]
mod tests;

use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::model::Meta;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Container {
    Mp4,
    WebM,
    Mkv,
}

impl Container {
    pub fn for_video(ext: &str) -> Container {
        Container::parse(ext).unwrap_or(Container::Mp4)
    }

    pub fn parse(name: &str) -> Option<Container> {
        match name.to_ascii_lowercase().as_str() {
            "mp4" | "m4v" | "m4a" => Some(Container::Mp4),
            "webm" | "weba" => Some(Container::WebM),
            "mkv" | "mka" | "matroska" => Some(Container::Mkv),
            _ => None,
        }
    }

    pub fn names() -> &'static str {
        "mp4, webm or mkv"
    }

    pub fn audio_extension(self) -> &'static str {
        match self {
            Container::Mp4 => "m4a",
            Container::WebM => "weba",
            Container::Mkv => "mka",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Container::Mp4 => "mp4",
            Container::WebM => "webm",
            Container::Mkv => "mkv",
        }
    }

    fn doc(self) -> Option<webm::Doc> {
        match self {
            Container::Mp4 => None,
            Container::WebM => Some(webm::Doc::WebM),
            Container::Mkv => Some(webm::Doc::Matroska),
        }
    }
}

pub fn pairs(video_ext: &str, audio_ext: &str) -> bool {
    matches!(
        (
            video_ext.to_ascii_lowercase().as_str(),
            audio_ext.to_ascii_lowercase().as_str()
        ),
        ("mp4", "m4a") | ("webm", "weba") | ("mkv", "mka")
    )
}

pub fn sniff(path: &Path) -> Result<Container, String> {
    let mut head = [0u8; 64];
    let mut file =
        File::open(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let read = file
        .read(&mut head)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let head = &head[..read];
    if head.len() >= 4 && head[..4] == [0x1a, 0x45, 0xdf, 0xa3] {
        let matroska = head.windows(8).any(|window| window == b"matroska");
        return Ok(if matroska {
            Container::Mkv
        } else {
            Container::WebM
        });
    }
    let known: [&[u8; 4]; 8] = [
        b"ftyp", b"styp", b"moov", b"free", b"skip", b"wide", b"mdat", b"sidx",
    ];
    if head.len() >= 8 && known.iter().any(|kind| &head[4..8] == kind.as_slice()) {
        return Ok(Container::Mp4);
    }
    Err(format!(
        "{} is not an mp4, webm or mkv file",
        path.display()
    ))
}

pub fn merge(video: &Path, audio: &Path, out: &Path, meta: Option<&Meta>) -> Result<(), String> {
    let first = sniff(video)?;
    if first != sniff(audio)? {
        return Err("video and audio are in different containers".to_string());
    }
    merge_into(video, audio, out, first, meta)
}

pub fn merge_into(
    video: &Path,
    audio: &Path,
    out: &Path,
    target: Container,
    meta: Option<&Meta>,
) -> Result<(), String> {
    let first = sniff(video)?;
    let second = sniff(audio)?;
    if first != second {
        return Err("video and audio are in different containers".to_string());
    }
    match (first, target) {
        (Container::Mp4, Container::Mp4) => mp4::merge(video, audio, out, meta),
        (Container::Mp4, Container::Mkv) => mkv::merge(video, audio, out, meta),
        (Container::WebM | Container::Mkv, Container::WebM) if first == target => {
            webm::merge(video, audio, out, webm::Doc::WebM, meta)
        }
        (Container::WebM | Container::Mkv, Container::Mkv) => {
            webm::merge(video, audio, out, webm::Doc::Matroska, meta)
        }
        _ => Err(format!(
            "{} streams cannot be stored as {}",
            first.extension(),
            target.extension()
        )),
    }
}

pub fn extract_audio(source: &Path, out: &Path, meta: Option<&Meta>) -> Result<(), String> {
    extract_audio_into(source, out, sniff(source)?, meta)
}

pub fn retag(source: &Path, out: &Path, meta: &Meta) -> Result<(), String> {
    match sniff(source)? {
        Container::Mp4 => mp4::retag(source, out, meta),
        Container::WebM => webm::retag(source, out, webm::Doc::WebM, meta),
        Container::Mkv => webm::retag(source, out, webm::Doc::Matroska, meta),
    }
}

pub fn extract_audio_into(
    source: &Path,
    out: &Path,
    target: Container,
    meta: Option<&Meta>,
) -> Result<(), String> {
    let from = sniff(source)?;
    match (from, target) {
        (Container::Mp4, Container::Mp4) => mp4::extract_audio(source, out, meta),
        (Container::Mp4, Container::Mkv) => mkv::extract_audio(source, out, meta),
        (Container::WebM | Container::Mkv, _) => match target.doc() {
            Some(doc) if from == target || target == Container::Mkv => {
                webm::extract_audio(source, out, doc, meta)
            }
            _ => Err(format!(
                "audio cannot be moved from {} to {}",
                from.extension(),
                target.extension()
            )),
        },
        (Container::Mp4, Container::WebM) => {
            Err("audio cannot be moved from mp4 to webm".to_string())
        }
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
        Container::WebM | Container::Mkv => {
            let video = webm::demux_for_test(path, 1)?;
            let audio = webm::demux_for_test(path, 2)?;
            Ok((video.count(), audio.count()))
        }
    }
}
