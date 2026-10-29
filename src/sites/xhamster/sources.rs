use std::collections::BTreeMap;

use serde_json::Value;

use super::cipher::decipher_url;
use super::json::{dig, text};
use crate::hls;

pub const CODECS: [&str; 4] = ["h264", "h265", "av1", "vp9"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub url: String,
    pub height: u32,
    pub hls: bool,
    pub codec: String,
    pub bitrate: Option<u64>,
    pub vr: bool,
}

fn height_of(label: &str) -> u32 {
    let digits: String = label
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    if (3..=4).contains(&digits.len()) {
        digits.parse().unwrap_or(0)
    } else {
        0
    }
}

pub fn from_initials(initials: &Value) -> Vec<Source> {
    let Some(root) = dig(initials, "xplayerSettings.sources").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut out: Vec<Source> = Vec::new();
    let mut push = |raw: &Value, label: &str, codec: &str| {
        if !raw.is_object() {
            return;
        }
        let url = raw
            .get("url")
            .and_then(Value::as_str)
            .and_then(decipher_url)
            .or_else(|| {
                raw.get("fallback")
                    .and_then(Value::as_str)
                    .and_then(decipher_url)
            });
        let Some(url) = url else {
            return;
        };
        if out.iter().any(|source| source.url == url) {
            return;
        }
        let hls = url.to_ascii_lowercase().contains(".m3u8");
        let vr = text(raw.get("framePack")).is_some()
            || text(raw.get("type")).is_some_and(|kind| kind.to_lowercase().contains("vr"));
        out.push(Source {
            url,
            height: height_of(label),
            hls,
            codec: codec.to_string(),
            bitrate: None,
            vr,
        });
    };
    if let Some(entries) = root.get("hls").and_then(Value::as_object) {
        for (codec, entry) in entries {
            push(entry, "hls-master", codec);
        }
    }
    if let Some(groups) = root.get("standard").and_then(Value::as_object) {
        for (codec, list) in groups {
            let Some(items) = list.as_array() else {
                continue;
            };
            for item in items {
                let label = text(item.get("quality"))
                    .or_else(|| text(item.get("label")))
                    .unwrap_or_else(|| "auto".to_string());
                push(item, &label, codec);
            }
        }
    }
    out.sort_by(|a, b| {
        a.hls
            .cmp(&b.hls)
            .then(b.height.cmp(&a.height))
            .then(a.codec.cmp(&b.codec))
    });
    out
}

pub fn session_locked(url: &str) -> bool {
    let mut rest = url;
    while let Some(at) = rest.find("data=") {
        let before = if at == 0 {
            b'/'
        } else {
            rest.as_bytes()[at - 1]
        };
        let tail = &rest[at + 5..];
        let address: String = tail
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let after = tail[address.len()..].chars().next();
        if matches!(before, b'/' | b',')
            && address.split('.').count() == 4
            && address
                .split('.')
                .all(|part| !part.is_empty() && part.len() <= 3)
            && matches!(after, None | Some('-') | Some('/') | Some(','))
        {
            return true;
        }
        rest = tail;
    }
    false
}

fn rank(codec: &str, preferred: &str) -> usize {
    let mut order = vec![preferred];
    order.extend(CODECS.iter().copied().filter(|name| *name != preferred));
    order
        .iter()
        .position(|name| name.eq_ignore_ascii_case(codec))
        .unwrap_or(order.len())
}

pub type Master<'a> = dyn Fn(&str) -> Vec<hls::Variant> + 'a;

#[derive(Default)]
struct Slot {
    file: Option<Source>,
    list: Option<Source>,
}

pub fn expand(sources: Vec<Source>, seconds: u64, master: &Master<'_>) -> Vec<Source> {
    let estimate = |bitrate: Option<u64>| bitrate.filter(|_| seconds > 0);
    let mut out: Vec<Source> = Vec::new();
    let mut ladder: BTreeMap<u32, BTreeMap<String, Slot>> = BTreeMap::new();
    let push = |out: &mut Vec<Source>, source: Source| {
        if !out.iter().any(|existing| existing.url == source.url) {
            out.push(source);
        }
    };
    for source in sources {
        if !source.hls {
            if source.height > 0 {
                let slot = ladder
                    .entry(source.height)
                    .or_default()
                    .entry(source.codec.to_lowercase())
                    .or_default();
                slot.file = Some(source);
            } else {
                push(&mut out, source);
            }
            continue;
        }
        let variants = master(&source.url);
        if variants.is_empty() {
            push(
                &mut out,
                Source {
                    height: 0,
                    ..source
                },
            );
            continue;
        }
        for variant in variants {
            let candidate = Source {
                url: variant.url,
                height: variant.height,
                hls: true,
                codec: source.codec.clone(),
                bitrate: (variant.bandwidth > 0).then_some(variant.bandwidth),
                vr: source.vr,
            };
            if variant.height == 0 {
                push(&mut out, candidate);
                continue;
            }
            let slot = ladder
                .entry(variant.height)
                .or_default()
                .entry(source.codec.to_lowercase())
                .or_default();
            let better = slot
                .list
                .as_ref()
                .is_none_or(|held| candidate.bitrate.unwrap_or(0) > held.bitrate.unwrap_or(0));
            if better {
                slot.list = Some(candidate);
            }
        }
    }
    let preferred = CODECS[0];
    for by_codec in ladder.into_values() {
        let mut slots: Vec<(String, Slot)> = by_codec.into_iter().collect();
        slots.sort_by_key(|slot| rank(&slot.0, preferred));
        let file = slots.iter().find_map(|(_, slot)| slot.file.clone());
        let list = slots.iter().find_map(|(_, slot)| slot.list.clone());
        let borrowed = file
            .as_ref()
            .and_then(|source| source.bitrate)
            .or_else(|| list.as_ref().and_then(|source| source.bitrate));
        if let Some(mut file) = file {
            file.bitrate = file.bitrate.or(estimate(borrowed).and(borrowed));
            push(&mut out, file);
        }
        if let Some(list) = list {
            push(&mut out, list);
        }
    }
    out.sort_by(|a, b| {
        b.height
            .cmp(&a.height)
            .then(session_locked(&a.url).cmp(&session_locked(&b.url)))
            .then(a.hls.cmp(&b.hls))
            .then(b.bitrate.unwrap_or(0).cmp(&a.bitrate.unwrap_or(0)))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn blob(id: u8) -> &'static str {
        match id {
            3 => "037856341249f5a20ae9ea44b7dc94012a93fc0a4f4d165d304ae78172ac27ca",
            _ => "0400000000e59b0923e935569b8aa466b92c7f74fc4f991af145e3c7bbda55fa",
        }
    }

    #[test]
    fn reads_both_packagings_and_skips_decoys() {
        let initials = json!({"xplayerSettings": {"sources": {
            "hls": {"h264": {"url": "https://cdn.test/master.m3u8"}},
            "standard": {"h264": [
                {"quality": "720p", "url": blob(3)},
                {"quality": "480p", "url": "https://cdn.test/480p.h264.mp4"},
                {"quality": "360p", "url": "0a0b0c0d0e0f0a0b0c0d0e0f"},
                {"quality": "auto", "url": "https://cdn.test/auto.m3u8"}
            ]}
        }}});
        let found = from_initials(&initials);
        let described: Vec<(u32, bool)> = found.iter().map(|s| (s.height, s.hls)).collect();
        assert_eq!(
            described,
            vec![(720, false), (480, false), (0, true), (0, true)]
        );
        assert_eq!(found[0].url, "https://c.x/3/720p.h264.mp4");
    }

    #[test]
    fn fallback_is_used_when_the_primary_does_not_decipher() {
        let initials = json!({"xplayerSettings": {"sources": {"standard": {"h264": [
            {"quality": "720p", "url": "0a0b0c0d0e0f0a0b0c0d0e0f", "fallback": "https://cdn.test/720p.mp4"}
        ]}}}});
        assert_eq!(from_initials(&initials)[0].url, "https://cdn.test/720p.mp4");
        assert!(from_initials(&json!({})).is_empty());
    }

    #[test]
    fn locked_urls_are_spotted() {
        assert!(session_locked("https://v.test/a.mp4?x=1,data=10.1.2.3-dvp"));
        assert!(session_locked("https://v.test/data=1.2.3.4/a.mp4"));
        assert!(!session_locked("https://v.test/a.mp4?data=abc"));
        assert!(!session_locked("https://v.test/a.mp4"));
    }

    #[test]
    fn ladder_lists_each_height_once_per_packaging_with_codec_preference() {
        let file = |height, codec: &str| Source {
            url: format!("https://cdn.test/{height}.{codec}.mp4"),
            height,
            hls: false,
            codec: codec.to_string(),
            bitrate: None,
            vr: false,
        };
        let master = Source {
            url: "https://cdn.test/master.m3u8".to_string(),
            height: 0,
            hls: true,
            codec: "h264".to_string(),
            bitrate: None,
            vr: false,
        };
        let sources = vec![
            file(720, "av1"),
            file(720, "h264"),
            file(480, "h264"),
            master,
        ];
        let lookup = |_: &str| {
            vec![
                hls::Variant {
                    url: "https://cdn.test/1080.h264.mp4.m3u8".into(),
                    height: 1080,
                    bandwidth: 4_000_000,
                },
                hls::Variant {
                    url: "https://cdn.test/720.h264.mp4.m3u8".into(),
                    height: 720,
                    bandwidth: 2_000_000,
                },
            ]
        };
        let out = expand(sources, 100, &lookup);
        let described: Vec<(u32, bool, String)> = out
            .iter()
            .map(|s| {
                (
                    s.height,
                    s.hls,
                    s.url.rsplit('/').next().unwrap().to_string(),
                )
            })
            .collect();
        assert_eq!(
            described,
            vec![
                (1080, true, "1080.h264.mp4.m3u8".to_string()),
                (720, false, "720.h264.mp4".to_string()),
                (720, true, "720.h264.mp4.m3u8".to_string()),
                (480, false, "480.h264.mp4".to_string()),
            ]
        );
        assert_eq!(out[1].bitrate, Some(2_000_000));
    }

    #[test]
    fn dead_masters_stay_listed() {
        let locked = Source {
            url: "https://v.test/a.mp4?data=1.2.3.4-dvp".into(),
            height: 720,
            hls: false,
            codec: "h264".into(),
            bitrate: None,
            vr: false,
        };
        let free = Source {
            url: "https://v.test/b.mp4".into(),
            height: 720,
            hls: false,
            codec: "av1".into(),
            bitrate: None,
            vr: false,
        };
        let master = Source {
            url: "https://v.test/m.m3u8".into(),
            height: 0,
            hls: true,
            codec: "h264".into(),
            bitrate: None,
            vr: false,
        };
        let out = expand(vec![locked, free, master], 0, &|_| Vec::new());
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].height, 720);
        assert!(out[1].hls && out[1].height == 0);
    }
}
