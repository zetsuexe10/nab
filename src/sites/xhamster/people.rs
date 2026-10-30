use serde_json::{json, Value};

use super::json::{array, count, decode_entities, find_array, first_text, flag, tidy};
use super::profile::Kind;
use crate::model::{Comment, CommentPage, Media, Video};
use crate::util::short_count;

pub const KINDS: [(&str, &str, &str); 4] = [
    ("pornstars", "pornstar", "/pornstars"),
    ("creators", "creator", "/creators"),
    ("channels", "channel", "/channels"),
    ("celebrities", "celebrity", "/celebrities"),
];

pub fn directory_path(query: &str) -> Option<&'static str> {
    let wanted = query.trim().to_ascii_lowercase();
    if wanted.is_empty() {
        return Some(KINDS[0].2);
    }
    KINDS
        .iter()
        .find(|(plural, single, _)| *plural == wanted || *single == wanted)
        .map(|(_, _, path)| *path)
}

pub fn person(raw: &Value, fallback: Kind) -> Option<Video> {
    if !raw.is_object() {
        return None;
    }
    let url = first_text(&[
        raw.get("pageURL"),
        raw.get("channelURL"),
        raw.get("url"),
        raw.get("link"),
    ])?;
    let handle = first_text(&[
        raw.get("nickname"),
        raw.get("nickName"),
        raw.get("username"),
        raw.get("userName"),
        raw.get("handle"),
    ]);
    let name = first_text(&[
        raw.get("displayName"),
        raw.get("channelName"),
        raw.get("name"),
    ])
    .or(handle)?;
    let subscription = raw.get("subscriptionModel");
    let videos = count(&[raw.get("videoCount")]).filter(|found| *found > 0);
    let subscribers = count(&[
        raw.get("subscribersCount"),
        raw.get("subscribers"),
        subscription.and_then(|node| node.get("subscribersCount")),
        subscription.and_then(|node| node.get("subscribers")),
    ])
    .filter(|found| *found > 0);
    let mut facts = Vec::new();
    if let Some(videos) = videos {
        facts.push(format!("{} videos", short_count(videos)));
    }
    if let Some(subscribers) = subscribers {
        facts.push(format!("{} subscribers", short_count(subscribers)));
    }
    let verified = flag(raw.get("isVerified")) || flag(raw.get("verified"));
    let kind = Kind::from_url(&url);
    let kind = if kind == Kind::Pornstar {
        fallback
    } else {
        kind
    };
    let mut name = decode_entities(&name);
    if verified {
        name.push_str(" ✓");
    }
    let id = url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(&url)
        .to_string();
    Some(Video {
        id,
        title: tidy(&name),
        channel: [
            Some(kind.label().to_string()),
            first_text(&[raw.get("translatedCountryName"), raw.get("country")]),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · "),
        duration: String::new(),
        views: facts.join(" · "),
        published: String::new(),
        url: Some(url.clone()),
        author_url: Some(url),
        media: Media::Person,
        site: super::ID,
    })
}

pub fn directory(initials: &Value, fallback: Kind) -> Vec<Video> {
    let lists: [Option<&[Value]>; 5] = [
        Some(array(initials, "layoutPage.persons")),
        Some(array(initials, "layoutPage.pornstarListProps.pornstars")),
        Some(array(initials, "layoutPage.channels")),
        Some(array(initials, "channels")),
        Some(array(initials, "layoutPage.channelsNamesProps.list")),
    ];
    for list in lists.into_iter().flatten() {
        let found: Vec<Video> = list
            .iter()
            .filter_map(|raw| person(raw, fallback))
            .collect();
        if !found.is_empty() {
            return dedup(found);
        }
    }
    for key in ["pornstars", "persons", "creators", "celebrities"] {
        if let Some(list) = find_array(initials, key, &[]) {
            let found: Vec<Video> = list
                .iter()
                .filter_map(|raw| person(raw, fallback))
                .collect();
            if !found.is_empty() {
                return dedup(found);
            }
        }
    }
    Vec::new()
}

fn dedup(items: Vec<Video>) -> Vec<Video> {
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(item.url.clone()))
        .collect()
}

pub fn kind_of(path: &str) -> Kind {
    Kind::from_url(&format!("{path}/"))
}

pub fn related_params() -> Value {
    json!({
        "tabType": "video",
        "tabId": null,
        "isDesktop": true,
        "channelId": null,
        "creatorId": null,
        "newestMixinUsedCount": 0,
        "extDebug": null,
        "extTestVersion": null,
        "isUniversalShown": true,
        "viewIdForce": null,
        "fhVideoParams": {
            "authorUsername": null,
            "authorId": null,
            "authorHasTrafficPowerUp": false,
            "authorHasSponsorCSWidget": false
        },
        "xhlVideoParams": {
            "isShownForCamsCategory": false,
            "videoCategory": null,
            "stripChatCountry": null
        },
        "fhBaseOffset": 0
    })
}

pub fn related(data: &Value) -> (Vec<Video>, u32) {
    let videos = array(data, "videoThumbProps")
        .iter()
        .filter_map(super::initials::video)
        .collect();
    let last = count(&[data.get("maxPages")])
        .map(|found| found as u32)
        .unwrap_or(0);
    (videos, last)
}

pub fn comment_call(id: u64, page: u32) -> Value {
    json!([{
        "name": "entityCommentCollectionFetch",
        "requestData": {
            "spamFolder": false,
            "page": page,
            "append": true,
            "entity": {"entityModel": "videoModel", "entityID": id}
        }
    }])
}

fn comment(raw: &Value, index: usize) -> Option<Comment> {
    let body = first_text(&[raw.get("text"), raw.get("comment"), raw.get("body")])?;
    let body = tidy(&decode_entities(&strip(&body)));
    if body.is_empty() {
        return None;
    }
    let author = raw.get("author");
    let name = first_text(&[
        author.and_then(|node| node.get("name")),
        raw.get("userName"),
        raw.get("authorName"),
    ])
    .map(|name| tidy(&decode_entities(&name)))
    .unwrap_or_default();
    Some(Comment {
        id: raw
            .get("id")
            .or_else(|| raw.get("uuid"))
            .and_then(|value| match value {
                Value::Number(number) => Some(number.to_string()),
                Value::String(text) if !text.is_empty() => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_else(|| index.to_string()),
        author: name,
        text: body,
        likes: count(&[raw.get("likes"), raw.get("likesCount")]).unwrap_or(0),
        created: count(&[raw.get("created"), raw.get("createdAt"), raw.get("date")]),
        reply_to: first_text(&[raw.get("replyToUserName"), raw.get("replyToName")])
            .map(|name| tidy(&name)),
    })
}

fn strip(html: &str) -> String {
    super::json::strip_tags(html)
}

pub fn comments(data: &Value, page: u32) -> CommentPage {
    let result = data.as_array().and_then(|items| items.first());
    let mut comments: Vec<Comment> = Vec::new();
    if let Some(rows) = result
        .and_then(|node| node.get("responseData"))
        .and_then(Value::as_array)
    {
        for (index, row) in rows.iter().enumerate() {
            if let Some(found) = comment(row, index) {
                if !comments.iter().any(|held| held.id == found.id) {
                    comments.push(found);
                }
            }
        }
    }
    let extras = result.and_then(|node| node.get("extras"));
    let paging = extras.and_then(|node| node.get("paging"));
    CommentPage {
        comments,
        total: count(&[extras.and_then(|node| node.get("total"))]),
        page: count(&[paging.and_then(|node| node.get("active"))])
            .map(|found| found as u32)
            .unwrap_or(page),
        last: count(&[paging.and_then(|node| node.get("maxPages"))])
            .map(|found| found as u32)
            .unwrap_or(page)
            .max(page),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_pick_a_directory() {
        assert_eq!(directory_path(""), Some("/pornstars"));
        assert_eq!(directory_path(" Creators "), Some("/creators"));
        assert_eq!(directory_path("celebrity"), Some("/celebrities"));
        assert_eq!(directory_path("amy"), None);
    }

    #[test]
    fn people_rows_carry_name_facts_and_profile() {
        let raw = json!({
            "pageURL": "https://xhamster.com/pornstars/amy-doe",
            "name": "Amy &amp; Doe",
            "videoCount": 1200,
            "subscribersCount": 54000,
            "isVerified": true,
            "country": "US"
        });
        let row = person(&raw, Kind::Pornstar).unwrap();
        assert_eq!(row.id, "amy-doe");
        assert_eq!(row.title, "Amy & Doe ✓");
        assert_eq!(row.views, "1.2K videos · 54K subscribers");
        assert_eq!(row.channel, "pornstar · US");
        assert_eq!(row.media, Media::Person);
        assert_eq!(row.author_url, row.url);
        assert!(person(&json!({"name": "x"}), Kind::Pornstar).is_none());
    }

    #[test]
    fn directories_read_known_shapes_and_skip_repeats() {
        let page = json!({"layoutPage": {"pornstarListProps": {"pornstars": [
            {"pageURL": "https://xhamster.com/pornstars/a", "name": "A"},
            {"pageURL": "https://xhamster.com/pornstars/a", "name": "A"},
            {"pageURL": "https://xhamster.com/pornstars/b", "name": "B"}
        ]}}});
        let found = directory(&page, Kind::Pornstar);
        assert_eq!(found.len(), 2);
        let deep = json!({"x": {"creators": [{"url": "https://xhamster.com/creators/c", "displayName": "C"}]}});
        assert_eq!(directory(&deep, Kind::Creator)[0].id, "c");
        assert!(directory(&json!({}), Kind::Pornstar).is_empty());
    }

    #[test]
    fn related_and_comment_payloads_are_shaped() {
        let (videos, last) = related(&json!({
            "videoThumbProps": [{"id": 5, "title": "T", "pageURL": "https://xhamster.com/videos/t-5"}],
            "maxPages": 4
        }));
        assert_eq!((videos.len(), last), (1, 4));
        let page = comments(
            &json!([{
                "responseData": [
                    {"id": 9, "text": "<b>nice</b> &amp; fun", "author": {"name": "Zed"}, "likes": 3, "created": 100, "replyToUserName": "Amy"},
                    {"id": 9, "text": "dup"},
                    {"id": 10, "text": "  "}
                ],
                "extras": {"total": 42, "paging": {"maxPages": 5, "active": 2}}
            }]),
            2,
        );
        assert_eq!(page.comments.len(), 1);
        assert_eq!(page.comments[0].text, "nice & fun");
        assert_eq!(page.comments[0].author, "Zed");
        assert_eq!(page.comments[0].reply_to.as_deref(), Some("Amy"));
        assert_eq!((page.total, page.page, page.last), (Some(42), 2, 5));
        assert_eq!(comments(&json!(null), 3).last, 3);
        assert_eq!(
            comment_call(7, 2)[0]["requestData"]["entity"]["entityID"],
            7
        );
    }
}
