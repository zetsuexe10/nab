use serde_json::Value;

use super::json::{array, decode_entities, dig, find_array, first_text, flag, number, text};
use crate::model::{Paging, Video};
use crate::util;

pub fn parse(html: &str) -> Option<Value> {
    for marker in [
        "window.initials=",
        "window.initials =",
        "window[\"initials\"]=",
    ] {
        let mut from = 0;
        while let Some(found) = html[from..].find(marker) {
            let after = from + found + marker.len();
            from = after;
            let Some(open) = html[after - 1..].find('{').map(|at| at + after - 1) else {
                continue;
            };
            let Some(raw) = slice_object(html, open) else {
                continue;
            };
            if let Some(value) = parse_relaxed(raw) {
                return Some(value);
            }
        }
    }
    None
}

fn slice_object(text: &str, start: usize) -> Option<&str> {
    let mut depth = 0usize;
    let mut inside = false;
    let mut escaped = false;
    for (offset, byte) in text.as_bytes()[start..].iter().enumerate() {
        if inside {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                inside = false;
            }
            continue;
        }
        match byte {
            b'"' => inside = true,
            b'{' => depth += 1,
            b'}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(&text[start..start + offset + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_relaxed(raw: &str) -> Option<Value> {
    let object = |value: Value| value.is_object().then_some(value);
    if let Some(value) = serde_json::from_str(raw).ok().and_then(object) {
        return Some(value);
    }
    serde_json::from_str(&strip_trailing_commas(raw))
        .ok()
        .and_then(object)
}

fn strip_trailing_commas(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len());
    let mut inside = false;
    let mut escaped = false;
    for (index, character) in chars.iter().enumerate() {
        if inside {
            out.push(*character);
            if escaped {
                escaped = false;
            } else if *character == '\\' {
                escaped = true;
            } else if *character == '"' {
                inside = false;
            }
            continue;
        }
        if *character == '"' {
            inside = true;
        }
        if *character == ',' {
            let next = chars[index + 1..].iter().find(|c| !c.is_whitespace());
            if matches!(next, Some('}') | Some(']')) {
                continue;
            }
        }
        out.push(*character);
    }
    out
}

pub fn title_of(html: &str) -> Option<String> {
    let marker = html.find("data-role=\"page-title\"")?;
    let tag_end = html[marker..].find('>')? + marker + 1;
    let close = html[tag_end..].find("</h1>")? + tag_end;
    let mut inner = html[tag_end..close.min(tag_end + 400)].to_string();
    while let Some(start) = inner.find("<!--") {
        let end = inner[start..]
            .find("-->")
            .map(|at| start + at + 3)
            .unwrap_or(inner.len());
        inner.replace_range(start..end, "");
    }
    let cleaned = super::json::strip_tags(&inner);
    (!cleaned.is_empty()).then_some(cleaned)
}

pub fn video(raw: &Value) -> Option<Video> {
    let url = text(raw.get("pageURL"))?;
    let id = match raw.get("id")? {
        Value::Number(number) => number.to_string(),
        Value::String(text) if !text.is_empty() => text.clone(),
        _ => return None,
    };
    let landing = raw.get("landing").filter(|landing| landing.is_object());
    let title = decode_entities(&text(raw.get("title")).unwrap_or_default())
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let seconds = number(raw.get("duration")).unwrap_or(0.0).max(0.0) as u64;
    let views = number(raw.get("views")).filter(|views| *views >= 0.0);
    let created = number(raw.get("created")).filter(|created| *created > 0.0);
    let short = url.contains("/shorts/")
        || matches!(
            text(raw.get("videoType"))
                .unwrap_or_default()
                .to_lowercase()
                .as_str(),
            "short" | "moment" | "clip"
        );
    let mut label = Vec::new();
    if flag(raw.get("isVR")) || flag(raw.get("isVr")) {
        label.push("VR");
    }
    if flag(raw.get("isUHD")) || flag(raw.get("is4k")) {
        label.push("4K");
    } else if flag(raw.get("isFHD")) {
        label.push("1080");
    } else if flag(raw.get("isHD")) {
        label.push("HD");
    }
    let mut views_text = views
        .map(|views| format!("{} views", util::short_count(views as u64)))
        .unwrap_or_default();
    if !label.is_empty() {
        let badges = label.join(" ");
        views_text = if views_text.is_empty() {
            badges
        } else {
            format!("{views_text} · {badges}")
        };
    }
    Some(Video {
        id,
        title: if title.is_empty() {
            "Untitled".to_string()
        } else {
            title
        },
        channel: landing
            .and_then(|landing| text(landing.get("name")))
            .unwrap_or_default(),
        duration: if short {
            "short".to_string()
        } else if seconds > 0 {
            util::duration(seconds)
        } else {
            String::new()
        },
        views: views_text,
        published: created
            .map(|created| util::ago(created as u64, util::now()))
            .unwrap_or_default(),
        url: Some(url),
        author_url: landing.and_then(|landing| text(landing.get("link"))),
        media: crate::model::Media::Video,
        site: super::ID,
    })
}

fn videos_from(candidate: Option<&Value>) -> Vec<Video> {
    candidate
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(video).collect())
        .unwrap_or_default()
}

pub fn video_list(initials: &Value) -> Vec<Video> {
    let paths = [
        "searchResult.videoThumbProps",
        "pagesCategoryComponent.trendingVideoListProps.videoThumbProps",
        "layoutPage.videoListProps.videoThumbProps",
        "videoListComponent.videoThumbProps",
        "relatedVideosComponent.videoTabInitialData.videoListProps.videoThumbProps",
        "pornstarPageComponent.videoListProps.videoThumbProps",
        "channelPageComponent.videoListProps.videoThumbProps",
        "newestVideoSectionComponent.videoListProps.videoThumbProps",
        "layoutPage.list",
    ];
    for path in paths {
        let found = videos_from(dig(initials, path));
        if !found.is_empty() {
            return found;
        }
    }
    let deep = find_array(initials, "videoThumbProps", &[])
        .map(|items| items.iter().filter_map(video).collect::<Vec<Video>>());
    deep.unwrap_or_default()
}

fn positive(value: Option<&Value>) -> Option<u32> {
    number(value)
        .filter(|found| found.is_finite() && *found > 0.0)
        .map(|found| found as u32)
}

pub fn paging(initials: &Value) -> Paging {
    let current = [
        "pagination.active",
        "pagesCategoryComponent.paginationProps.currentPageNumber",
        "paginationComponent.currentPageNumber",
        "layoutPage.pagination.currentPageNumber",
        "page",
        "activePage",
    ]
    .iter()
    .find_map(|path| positive(dig(initials, path)))
    .unwrap_or(1);
    let mut last = [
        "pagination.maxPages",
        "pagination.maxPage",
        "pagesCategoryComponent.paginationProps.lastPageNumber",
        "paginationComponent.lastPageNumber",
        "layoutPage.pagination.lastPageNumber",
        "maxVideoPages",
        "maxMomentPages",
        "maxPages",
    ]
    .iter()
    .find_map(|path| positive(dig(initials, path)))
    .unwrap_or(1);
    let mut template = None;
    for path in [
        "paginationComponent",
        "layoutPage.paginationProps",
        "pagination",
        "pagesCategoryComponent.paginationProps",
        "layoutPage.pornstarListProps.paginationProps",
        "layoutPage.pagination",
    ] {
        let Some(node) = dig(initials, path).filter(|node| node.is_object()) else {
            continue;
        };
        if let Some(found) = ["lastPageNumber", "maxPages", "maxPage"]
            .iter()
            .find_map(|key| positive(node.get(*key)))
        {
            last = last.max(found);
        }
        if let Some(link) = text(node.get("pageLinkTemplate")).filter(|link| link.contains("{#}")) {
            template.get_or_insert(link);
        }
    }
    Paging {
        page: current,
        last: last.max(1),
        template,
    }
}

pub fn duration_seconds(initials: &Value) -> u64 {
    number(dig(initials, "videoEntity.duration"))
        .or_else(|| number(dig(initials, "videoModel.duration")))
        .unwrap_or(0.0)
        .max(0.0) as u64
}

pub fn page_title(initials: &Value) -> Option<String> {
    first_text(&[
        dig(initials, "videoEntity.title"),
        dig(initials, "videoModel.title"),
    ])
    .map(|title| super::json::tidy(&title))
}

pub fn detail(initials: &Value, url: &str) -> Video {
    let entity = dig(initials, "videoEntity");
    let model = dig(initials, "videoModel");
    let field = |name: &str| -> Option<&Value> {
        entity
            .and_then(|node| node.get(name))
            .or_else(|| model.and_then(|node| node.get(name)))
    };
    let author = dig(initials, "videoModel.author");
    let id = match field("id") {
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::String(text)) => text.clone(),
        _ => url.rsplit('-').next().unwrap_or(url).to_string(),
    };
    let seconds = duration_seconds(initials);
    let views =
        number(field("views")).map(|views| format!("{} views", util::short_count(views as u64)));
    Video {
        id,
        title: page_title(initials).unwrap_or_default(),
        channel: author
            .and_then(|author| text(author.get("name")))
            .unwrap_or_default(),
        duration: if seconds > 0 {
            util::duration(seconds)
        } else {
            String::new()
        },
        views: views.unwrap_or_default(),
        published: number(model.and_then(|node| node.get("created")))
            .filter(|created| *created > 0.0)
            .map(|created| util::ago(created as u64, util::now()))
            .unwrap_or_default(),
        url: Some(text(field("pageURL")).unwrap_or_else(|| url.to_string())),
        author_url: author.and_then(|author| text(author.get("pageURL"))),
        media: crate::model::Media::Video,
        site: super::ID,
    }
}

pub fn blocked(initials: &Value) -> bool {
    if !video_list(initials).is_empty() {
        return false;
    }
    for path in [
        "channelInfo",
        "aboutMeComponent",
        "displayUserModel",
        "xplayerSettings",
        "videoEntity",
        "infoExp4040Component",
        "pornstarSectionComponent",
    ] {
        if dig(initials, path).is_some() {
            return false;
        }
    }
    !(paging(initials).last > 1
        || !array(initials, "layoutPage.pornstarListProps.pornstars").is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn finds_the_payload_past_braces_in_strings() {
        let html = r#"<script>var a=1;</script><script>window.initials={"title":"a } b { \" c","n":{"x":[1,2,]},};</script>"#;
        let value = parse(html).unwrap();
        assert_eq!(value["title"], "a } b { \" c");
        assert_eq!(value["n"]["x"], json!([1, 2]));
        assert!(parse("<html>none</html>").is_none());
        assert!(parse("window.initials={\"a\":").is_none());
    }

    #[test]
    fn video_rows_carry_the_listing_fields() {
        let raw = json!({
            "id": 77, "title": "Hello &amp; welcome", "pageURL": "https://x.test/videos/hello-xh1",
            "duration": 3725, "views": 1260000, "created": 1, "isFHD": true, "isHD": true,
            "landing": {"name": "Some Studio", "link": "https://x.test/channels/studio"}
        });
        let found = video(&raw).unwrap();
        assert_eq!(found.id, "77");
        assert_eq!(found.title, "Hello & welcome");
        assert_eq!(found.duration, "1:02:05");
        assert_eq!(found.views, "1.3M views · 1080");
        assert_eq!(found.channel, "Some Studio");
        assert_eq!(
            found.author_url.as_deref(),
            Some("https://x.test/channels/studio")
        );
        assert!(found.published.ends_with("years ago"));
        assert!(video(&json!({"id": 1})).is_none());
        let clip = video(&json!({"id": "9", "pageURL": "https://x.test/shorts/a"})).unwrap();
        assert_eq!(clip.duration, "short");
        assert_eq!(clip.title, "Untitled");
    }

    #[test]
    fn lists_come_from_known_places_and_deep_scans() {
        let row = json!({"id": 1, "pageURL": "https://x.test/videos/a-1", "title": "A"});
        let known = json!({"searchResult": {"videoThumbProps": [row.clone()]}});
        assert_eq!(video_list(&known).len(), 1);
        let hidden = json!({"somewhere": {"deep": {"videoThumbProps": [row.clone(), row]}}});
        assert_eq!(video_list(&hidden).len(), 2);
        assert!(video_list(&json!({})).is_empty());
    }

    #[test]
    fn paging_reads_counters_and_templates() {
        let initials = json!({
            "pagination": {"active": 2, "maxPages": 9, "pageLinkTemplate": "https://x.test/search/q/{#}"},
        });
        let found = paging(&initials);
        assert_eq!((found.page, found.last), (2, 9));
        assert_eq!(
            found.template.as_deref(),
            Some("https://x.test/search/q/{#}")
        );
        assert_eq!(paging(&json!({})).last, 1);
    }

    #[test]
    fn page_heading_is_read_without_comments_or_tags() {
        let html = r#"<h1 class="a" data-role="page-title"><!-- x --><span>Jane  Doe</span> Porn Videos</h1>"#;
        assert_eq!(title_of(html).as_deref(), Some("Jane Doe Porn Videos"));
        assert_eq!(title_of("<h1>nothing</h1>"), None);
    }

    #[test]
    fn interstitials_are_recognised() {
        assert!(blocked(&json!({"nothing": 1})));
        assert!(!blocked(&json!({"xplayerSettings": {}})));
        assert!(!blocked(&json!({"pagination": {"maxPages": 4}})));
    }
}
