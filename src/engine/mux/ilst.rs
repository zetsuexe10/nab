use super::bytes::Put;
use super::mp4::{atom, full};
use crate::model::Meta;

const TEXT: u32 = 1;
const JPEG: u32 = 13;
const PNG: u32 = 14;
const BINARY: u32 = 0;

fn item(kind: [u8; 4], flags: u32, payload: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(payload.len() + 4);
    body.put_u32(0);
    body.extend_from_slice(payload);
    atom(&kind, &full(b"data", 0, flags, &body)[..])
}

fn text(kind: [u8; 4], value: &str) -> Vec<u8> {
    item(kind, TEXT, value.as_bytes())
}

fn named(tail: &[u8; 3]) -> [u8; 4] {
    [0xa9, tail[0], tail[1], tail[2]]
}

fn comment(meta: &Meta) -> Option<String> {
    match (meta.comment.as_deref(), meta.url.as_deref()) {
        (Some(note), Some(url)) => Some(format!("{note}\n\n{url}")),
        (Some(note), None) => Some(note.to_string()),
        (None, Some(url)) => Some(url.to_string()),
        (None, None) => None,
    }
}

pub fn list(meta: &Meta) -> Vec<u8> {
    let mut out = Vec::new();
    let mut put = |piece: Vec<u8>| out.extend_from_slice(&piece);
    if let Some(value) = &meta.title {
        put(text(named(b"nam"), value));
    }
    if let Some(value) = &meta.artist {
        put(text(named(b"ART"), value));
    }
    if let Some(value) = &meta.album_artist {
        put(text(*b"aART", value));
    }
    if let Some(value) = &meta.album {
        put(text(named(b"alb"), value));
    }
    if let Some(value) = &meta.date {
        put(text(named(b"day"), value));
    }
    if let Some(value) = &meta.composer {
        put(text(named(b"wrt"), value));
    }
    if let Some(value) = comment(meta) {
        put(text(named(b"cmt"), &value));
    }
    if let Some(number) = meta.track {
        let mut payload = Vec::new();
        payload.put_u16(0);
        payload.put_u16(number.min(u16::MAX as u32) as u16);
        payload.put_u16(meta.track_total.unwrap_or(0).min(u16::MAX as u32) as u16);
        payload.put_u16(0);
        put(item(*b"trkn", BINARY, &payload));
    }
    if let Some(cover) = &meta.cover {
        let flags = match cover.mime {
            "image/jpeg" => Some(JPEG),
            "image/png" => Some(PNG),
            _ => None,
        };
        if let Some(flags) = flags {
            put(item(*b"covr", flags, &cover.data));
        }
    }
    out
}

pub fn udta(meta: &Meta) -> Vec<u8> {
    let items = list(meta);
    if items.is_empty() {
        return Vec::new();
    }
    let mut handler = Vec::new();
    handler.put_u32(0);
    handler.extend_from_slice(b"mdir");
    handler.extend_from_slice(b"appl");
    handler.put_u32(0);
    handler.put_u32(0);
    handler.put_u8(0);
    let mut body = full(b"hdlr", 0, 0, &handler);
    body.extend_from_slice(&atom(b"ilst", &items));
    atom(b"udta", &full(b"meta", 0, 0, &body))
}
