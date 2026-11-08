use super::webm::{element, uint_element};
use crate::model::Meta;

const TAGS: u32 = 0x1254_C367;
const TAG: u32 = 0x7373;
const TARGETS: u32 = 0x63C0;
const SIMPLE_TAG: u32 = 0x67C8;
const TAG_NAME: u32 = 0x45A3;
const TAG_STRING: u32 = 0x4487;
const ATTACHMENTS: u32 = 0x1941_A469;
const ATTACHED_FILE: u32 = 0x61A7;
const FILE_DESCRIPTION: u32 = 0x467E;
const FILE_NAME: u32 = 0x466E;
const FILE_MEDIA_TYPE: u32 = 0x4660;
const FILE_DATA: u32 = 0x465C;
const FILE_UID: u32 = 0x46AE;
const COVER_UID: u64 = 0x6E61_6263_6F76_6572;

fn simple(name: &str, value: &str) -> Vec<u8> {
    let mut body = element(TAG_NAME, name.as_bytes());
    body.extend_from_slice(&element(TAG_STRING, value.as_bytes()));
    element(SIMPLE_TAG, &body)
}

pub fn tags(meta: &Meta) -> Vec<u8> {
    let mut items = Vec::new();
    let mut put = |name: &str, value: Option<&str>| {
        if let Some(value) = value {
            items.extend_from_slice(&simple(name, value));
        }
    };
    put("TITLE", meta.title.as_deref());
    put("ARTIST", meta.artist.as_deref());
    put("ALBUM", meta.album.as_deref());
    put("ALBUM_ARTIST", meta.album_artist.as_deref());
    put("COMPOSER", meta.composer.as_deref());
    put("DATE_RELEASED", meta.date.as_deref());
    put("COMMENT", meta.comment.as_deref());
    put("URL", meta.url.as_deref());
    let number = meta.track.map(|value| value.to_string());
    let total = meta.track_total.map(|value| value.to_string());
    put("PART_NUMBER", number.as_deref());
    put("TOTAL_PARTS", total.as_deref());
    if items.is_empty() {
        return Vec::new();
    }
    let mut tag = element(TARGETS, &[]);
    tag.extend_from_slice(&items);
    element(TAGS, &element(TAG, &tag))
}

pub fn attachments(meta: &Meta) -> Vec<u8> {
    let Some(cover) = &meta.cover else {
        return Vec::new();
    };
    let name = format!("cover.{}", cover.extension());
    let mut file = element(FILE_DESCRIPTION, b"cover");
    file.extend_from_slice(&element(FILE_NAME, name.as_bytes()));
    file.extend_from_slice(&element(FILE_MEDIA_TYPE, cover.mime.as_bytes()));
    file.extend_from_slice(&uint_element(FILE_UID, COVER_UID));
    file.extend_from_slice(&element(FILE_DATA, &cover.data));
    element(ATTACHMENTS, &element(ATTACHED_FILE, &file))
}
