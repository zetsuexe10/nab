use serde_json::Value;

use super::json::{array, count, decode_entities, dig, first_text, flag, text, tidy};
use super::{gallery, initials};
use crate::model::{Choice, Profile, Scope, Video};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Pornstar,
    Creator,
    Channel,
    Celebrity,
    User,
}

impl Kind {
    pub fn from_url(url: &str) -> Kind {
        if url.contains("/creators/") {
            Kind::Creator
        } else if url.contains("/channels/") {
            Kind::Channel
        } else if url.contains("/celebrities/") {
            Kind::Celebrity
        } else if url.contains("/users/") {
            Kind::User
        } else {
            Kind::Pornstar
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Pornstar => "pornstar",
            Kind::Creator => "creator",
            Kind::Channel => "channel",
            Kind::Celebrity => "celebrity",
            Kind::User => "user",
        }
    }
}

pub const SORT_SEGMENTS: [(&str, &str); 5] = [
    ("featured", ""),
    ("added", "newest"),
    ("rated", "best"),
    ("viewed", "most-viewed"),
    ("commented", "most-commented"),
];

pub fn url_for(base: &str, sort: &str, page: u32) -> String {
    let clean = base
        .split(['?', '#'])
        .next()
        .unwrap_or(base)
        .trim_end_matches('/');
    let mut segments: Vec<String> = Vec::new();
    if let Some((_, segment)) = SORT_SEGMENTS.iter().find(|(id, _)| *id == sort) {
        if !segment.is_empty() {
            segments.push((*segment).to_string());
        }
    }
    if page > 1 {
        segments.push(page.to_string());
    }
    if segments.is_empty() {
        clean.to_string()
    } else {
        format!("{clean}/{}", segments.join("/"))
    }
}

pub fn canonical(url: &str) -> String {
    let path_start = url.find("://").map(|at| at + 3).unwrap_or(0);
    let host_end = url[path_start..]
        .find('/')
        .map(|at| path_start + at)
        .unwrap_or(url.len());
    let path = &url[host_end..];
    let segments: Vec<&str> = path
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments.len() >= 2 {
        format!("{}/{}/{}", &url[..host_end], segments[0], segments[1])
    } else {
        url.split(['?', '#']).next().unwrap_or(url).to_string()
    }
}

fn slug_of(url: &str) -> String {
    for marker in [
        "/users/",
        "/pornstars/",
        "/channels/",
        "/creators/",
        "/celebrities/",
    ] {
        if let Some(at) = url.find(marker) {
            let rest = &url[at + marker.len()..];
            let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
            return rest[..end].to_string();
        }
    }
    url.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_string()
}

fn clean_title(raw: Option<String>) -> Option<String> {
    let mut title = tidy(&raw?);
    for suffix in [
        "Porn Videos",
        "Short Videos",
        "HD Videos",
        "VR Videos",
        "Amateur Videos",
        "Sex Videos",
        "Videos",
        "Video",
    ] {
        if let Some(at) = title.find(&format!(" {suffix}")) {
            title.truncate(at);
        }
    }
    loop {
        let trimmed = title.trim_end();
        let word = trimmed.rsplit(' ').next().unwrap_or("");
        let drop = matches!(
            word.to_ascii_uppercase().as_str(),
            "UHD" | "4K" | "HD" | "VR" | "SD"
        ) || (word.ends_with('p')
            && word.len() >= 4
            && word[..word.len() - 1].bytes().all(|b| b.is_ascii_digit()));
        if drop && trimmed.contains(' ') {
            title = trimmed[..trimmed.len() - word.len()].trim_end().to_string();
        } else {
            break;
        }
    }
    (!title.is_empty() && title.chars().count() <= 64 && !title.contains(['|', '<', '>']))
        .then_some(title)
}

struct Person {
    name: String,
    handle: Option<String>,
    verified: bool,
    country: Option<String>,
    subscribers: Option<u64>,
    views: Option<u64>,
    video_count: Option<u64>,
    gender: Option<String>,
}

fn person_ref(raw: Option<&Value>) -> Option<Person> {
    let raw = raw.filter(|raw| raw.is_object())?;
    first_text(&[raw.get("pageURL"), raw.get("channelURL"), raw.get("url")])?;
    let handle = first_text(&[
        raw.get("nickname"),
        raw.get("nickName"),
        raw.get("username"),
        raw.get("userName"),
        raw.get("handle"),
        raw.get("screenName"),
    ]);
    let name = first_text(&[
        raw.get("displayName"),
        raw.get("channelName"),
        raw.get("name"),
    ])
    .or_else(|| handle.clone())?;
    let subscription = raw.get("subscriptionModel");
    Some(Person {
        name: decode_entities(&name),
        handle: handle.map(|handle| handle.trim_start_matches('@').to_string()),
        verified: flag(raw.get("isVerified")) || flag(raw.get("verified")),
        country: first_text(&[raw.get("translatedCountryName"), raw.get("country")]),
        subscribers: count(&[
            raw.get("subscribersCount"),
            raw.get("subscribers"),
            subscription.and_then(|node| node.get("subscribersCount")),
            subscription.and_then(|node| node.get("subscribers")),
        ]),
        views: count(&[
            raw.get("viewsCount"),
            raw.get("views"),
            raw.get("viewCount"),
            raw.get("totalViews"),
        ]),
        video_count: count(&[raw.get("videoCount")]),
        gender: None,
    })
}

fn hero(initials: &Value) -> Option<Person> {
    let hero = dig(initials, "infoExp4040Component.pornstarTop").filter(|hero| hero.is_object())?;
    let name = clean_title(text(dig(initials, "infoExp4040Component.pageTitle")))
        .or_else(|| text(hero.get("name")).map(|name| decode_entities(&name)))?;
    let personal = dig(
        initials,
        "infoExp4040Component.displayUserModel.personalInfo",
    );
    let handle = text(personal.and_then(|node| node.get("username")))
        .or_else(|| text(dig(initials, "aboutMeComponent.pornstarUserName")))
        .map(|handle| tidy(&handle).trim_start_matches('@').to_string())
        .filter(|handle| !handle.is_empty());
    let button = dig(
        initials,
        "infoExp4040Component.subscribeButtonsProps.subscribeButtonProps",
    );
    let verified = flag(dig(
        initials,
        "infoExp4040Component.displayUserModel.verified",
    )) || dig(initials, "channelInfo.personType").and_then(Value::as_str)
        == Some("pornstar");
    Some(Person {
        name,
        handle,
        verified,
        country: first_text(&[hero.get("translatedCountryName"), hero.get("country")]),
        subscribers: count(&[button.and_then(|node| node.get("subscribers"))]),
        views: count(&[hero.get("viewsCount")]),
        video_count: count(&[hero.get("videoCount")]),
        gender: text(
            personal
                .and_then(|node| node.get("gender"))
                .and_then(|node| node.get("name")),
        ),
    })
}

fn about_gender(initials: &Value) -> Option<String> {
    array(initials, "aboutMeComponent.personalInfoList")
        .iter()
        .find(|item| text(item.get("label")).as_deref() == Some("I am"))
        .and_then(|item| text(item.get("value")))
        .map(|value| tidy(&value))
}

fn readable(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(decode_entities(text)),
        Value::Array(items) => {
            let names: Vec<String> = items
                .iter()
                .filter_map(|item| match item {
                    Value::String(text) => Some(text.clone()),
                    other => first_text(&[other.get("name"), other.get("label")]),
                })
                .collect();
            (!names.is_empty()).then(|| decode_entities(&names.join(", ")))
        }
        _ => None,
    }
}

fn info(initials: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for key in ["personalInfoList", "appearanceInfoList"] {
        for item in array(initials, &format!("aboutMeComponent.{key}")) {
            let label = text(item.get("label"));
            let value = readable(item.get("value")).filter(|value| !value.trim().is_empty());
            if let (Some(label), Some(value)) = (label, value) {
                out.push((decode_entities(&label), value));
            }
        }
    }
    out
}

fn aliases(initials: &Value) -> Vec<String> {
    text(dig(initials, "infoExp4040Component.aliases"))
        .map(|raw| {
            raw.split(',')
                .map(tidy)
                .filter(|alias| !alias.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn socials(initials: &Value) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for item in array(initials, "infoExp4040Component.socials") {
        let name = first_text(&[item.get("name"), item.get("hint")]);
        let link = text(item.get("url"));
        let (Some(name), Some(link)) = (name, link) else {
            continue;
        };
        let lowered = name.to_lowercase();
        if link.starts_with("http") && !out.iter().any(|(held, _)| held.to_lowercase() == lowered) {
            out.push((name, link));
        }
    }
    out
}

fn choices(list: Option<&Value>) -> Vec<Choice> {
    list.and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let id = text(item.get("value"))?;
                    let label = first_text(&[item.get("label"), item.get("text")])?;
                    Some(Choice {
                        id,
                        label: decode_entities(&label),
                        count: count(&[item.get("count")]).filter(|found| *found > 0),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn selected(list: Option<&Value>) -> Option<String> {
    list?
        .as_array()?
        .iter()
        .find(|item| flag(item.get("selected")))
        .and_then(|item| text(item.get("value")))
}

fn filter_total(filters: Option<&Value>) -> Option<u64> {
    let filters = filters?;
    for key in ["durationList", "formatList", "qualityList"] {
        let Some(list) = filters
            .get(key)
            .and_then(Value::as_array)
            .filter(|l| !l.is_empty())
        else {
            continue;
        };
        let entry = list
            .iter()
            .find(|item| text(item.get("value")).as_deref() == Some("any"))
            .unwrap_or(&list[0]);
        if let Some(total) = count(&[entry.get("count")]).filter(|total| *total > 0) {
            return Some(total);
        }
    }
    None
}

const SECTIONS: [&str; 6] = [
    "layoutPage.videoListProps.videoThumbProps",
    "trendingVideoSectionComponent.videoListProps.videoThumbProps",
    "newestVideoSectionComponent.videoListProps.videoThumbProps",
    "publisherVideoSectionComponent.videoListProps.videoThumbProps",
    "videoListComponent.videoThumbProps",
    "userRecentVideosCollection",
];

fn videos_of(initials: &Value, total: Option<u64>) -> Vec<Video> {
    let mut best: Vec<Video> = Vec::new();
    for path in SECTIONS {
        let shaped: Vec<Video> = array(initials, path)
            .iter()
            .filter_map(initials::video)
            .collect();
        if total.is_some_and(|total| shaped.len() as u64 > total) {
            continue;
        }
        if shaped.len() > best.len() {
            best = shaped;
        }
    }
    if !best.is_empty() {
        return best;
    }
    super::json::find_array(
        initials,
        "videoThumbProps",
        &[
            "extendedVideoSectionComponent",
            "recommendedVideoSectionComponent",
        ],
    )
    .map(|items| items.iter().filter_map(initials::video).collect())
    .unwrap_or_default()
}

pub fn extract(initials: &Value, url: &str, heading: Option<&str>) -> Profile {
    let kind = Kind::from_url(url).label();
    let slug = slug_of(url);
    let filters = dig(
        initials,
        "tagsListWithVideoFiltersComponent.collapsableFiltersProps",
    );
    let base = hero(initials)
        .or_else(|| person_ref(dig(initials, "pornstarSectionComponent.pornstar")))
        .or_else(|| {
            person_ref(dig(
                initials,
                "layoutPage.channelLandingInfoProps.sponsorChannel",
            ))
        })
        .or_else(|| person_ref(dig(initials, "displayUserModel")));
    let fallback_name = slug
        .replace('-', " ")
        .split(' ')
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ");
    let heading =
        clean_title(heading.map(str::to_string)).or_else(|| text(initials.get("username")));
    let person = base.unwrap_or(Person {
        name: if fallback_name.is_empty() {
            "Unknown".to_string()
        } else {
            fallback_name
        },
        handle: None,
        verified: false,
        country: None,
        subscribers: None,
        views: None,
        video_count: None,
        gender: None,
    });
    let on_format_tab = text(initials.get("format")).is_some_and(|format| format != "any");
    let video_count = person.video_count.or_else(|| {
        if on_format_tab {
            None
        } else {
            filter_total(filters)
        }
    });
    let about = dig(initials, "aboutMeComponent");
    let paging = initials::paging(initials);
    let sort = text(initials.get("sort")).unwrap_or_else(|| "featured".to_string());
    let subscribers = person.subscribers.or_else(|| {
        count(&[
            dig(
                initials,
                "pornstarSectionComponent.pornstar.subscriptionModel.subscribers",
            ),
            dig(
                initials,
                "infoExp4040Component.subscribeButtonsProps.subscribeButtonProps.subscribers",
            ),
        ])
    });
    Profile {
        name: heading.unwrap_or(person.name),
        kind,
        gallery_count: gallery::person_total(initials),
        scope: Scope::Videos,
        handle: person.handle.or_else(|| text(initials.get("username"))),
        verified: person.verified,
        country: person.country,
        gender: person.gender.or_else(|| about_gender(initials)),
        subscribers,
        views: person.views,
        video_count,
        aliases: aliases(initials),
        socials: socials(initials),
        bio: about
            .and_then(|about| text(about.get("text")))
            .map(|bio| tidy(&bio))
            .filter(|bio| !bio.is_empty()),
        info: info(initials),
        videos: videos_of(initials, video_count),
        paging,
        url: canonical(url),
        sorts: choices(filters.and_then(|filters| filters.get("orderList"))),
        sort: selected(filters.and_then(|filters| filters.get("orderList"))).unwrap_or(sort),
    }
}

#[cfg(test)]
pub fn sample() -> Profile {
    let videos = (0..6)
        .map(|index| {
            let mut video = Video::stub(&format!("clip-{index}"));
            video.title = format!("Profile clip number {index}");
            video.url = Some(format!("https://xhamster.com/videos/clip-{index}"));
            video.duration = "12:34".to_string();
            video.views = "98K views".to_string();
            video.published = "3 weeks ago".to_string();
            video
        })
        .collect();
    Profile {
        name: "Sample Creator".to_string(),
        kind: "creator",
        gallery_count: Some(4),
        scope: Scope::Videos,
        handle: Some("sample".to_string()),
        verified: true,
        country: Some("Norway".to_string()),
        gender: Some("female".to_string()),
        subscribers: Some(120_000),
        views: Some(4_500_000),
        video_count: Some(88),
        aliases: vec!["Sam".to_string(), "Sammy".to_string()],
        socials: vec![("twitter".to_string(), "https://t/x".to_string())],
        bio: Some(
            "A short biography that should wrap over a few lines in the narrow column.".to_string(),
        ),
        info: vec![("hair".to_string(), "brown".to_string())],
        videos,
        paging: crate::model::Paging {
            page: 1,
            last: 3,
            template: None,
        },
        url: "https://xhamster.com/creators/sample".to_string(),
        sorts: vec![
            Choice {
                id: "featured".to_string(),
                label: "featured".to_string(),
                count: None,
            },
            Choice {
                id: "added".to_string(),
                label: "newest".to_string(),
                count: None,
            },
        ],
        sort: "featured".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub fn fixture() -> Value {
        json!({
            "sort": "featured",
            "infoExp4040Component": {
                "pageTitle": "Jane Doe Porn Videos",
                "aliases": "Janie, J. Doe ,",
                "socials": [
                    {"name": "Twitter", "url": "https://twitter.test/jane"},
                    {"name": "twitter", "url": "https://twitter.test/dupe"},
                    {"name": "Broken", "url": "javascript:x"}
                ],
                "pornstarTop": {"name": "Jane", "videoCount": 3, "viewsCount": "2.5M", "translatedCountryName": "Canada"},
                "displayUserModel": {"verified": true, "personalInfo": {"username": "@janedoe", "gender": {"name": "Female"}}},
                "subscribeButtonsProps": {"subscribeButtonProps": {"subscribers": "12K"}}
            },
            "aboutMeComponent": {
                "text": "  Hello &amp; welcome.\n  Enjoy.  ",
                "personalInfoList": [{"label": "Born", "value": "Toronto"}, {"label": "Empty", "value": ""}],
                "appearanceInfoList": [{"label": "Hair", "value": [{"name": "Brown"}, {"label": "Red"}]}]
            },
            "tagsListWithVideoFiltersComponent": {"collapsableFiltersProps": {
                "orderList": [
                    {"value": "featured", "label": "Featured", "selected": true},
                    {"value": "added", "label": "Newest", "count": 3}
                ],
                "durationList": [{"value": "any", "count": 3}]
            }},
            "layoutPage": {"videoListProps": {"videoThumbProps": [
                {"id": 1, "pageURL": "https://x.test/videos/a-1", "title": "One", "duration": 60},
                {"id": 2, "pageURL": "https://x.test/videos/b-2", "title": "Two", "duration": 120}
            ]}},
            "paginationComponent": {"currentPageNumber": 1, "lastPageNumber": 4}
        })
    }

    #[test]
    fn profile_fields_come_from_the_hero_and_about_blocks() {
        let profile = extract(
            &fixture(),
            "https://x.test/pornstars/jane-doe/newest/2",
            None,
        );
        assert_eq!(profile.name, "Jane Doe");
        assert_eq!(profile.kind, "pornstar");
        assert_eq!(profile.handle.as_deref(), Some("janedoe"));
        assert!(profile.verified);
        assert_eq!(profile.country.as_deref(), Some("Canada"));
        assert_eq!(profile.gender.as_deref(), Some("Female"));
        assert_eq!(profile.subscribers, Some(12_000));
        assert_eq!(profile.views, Some(2_500_000));
        assert_eq!(profile.video_count, Some(3));
        assert_eq!(profile.aliases, vec!["Janie", "J. Doe"]);
        assert_eq!(
            profile.socials,
            vec![(
                "Twitter".to_string(),
                "https://twitter.test/jane".to_string()
            )]
        );
        assert_eq!(profile.bio.as_deref(), Some("Hello & welcome. Enjoy."));
        assert_eq!(
            profile.info,
            vec![
                ("Born".to_string(), "Toronto".to_string()),
                ("Hair".to_string(), "Brown, Red".to_string())
            ]
        );
        assert_eq!(profile.videos.len(), 2);
        assert_eq!((profile.paging.page, profile.paging.last), (1, 4));
        assert_eq!(profile.sort, "featured");
        assert_eq!(profile.sorts.len(), 2);
        assert_eq!(profile.url, "https://x.test/pornstars/jane-doe");
    }

    #[test]
    fn sparse_pages_fall_back_to_the_url_and_heading() {
        let sparse = json!({"username": "Coco-Lovelock", "pagination": {"maxPages": 2}});
        let profile = extract(
            &sparse,
            "https://x.test/creators/coco-lovelock",
            Some("Coco Lovelock 4K Videos"),
        );
        assert_eq!(profile.name, "Coco Lovelock");
        assert_eq!(profile.kind, "creator");
        assert_eq!(profile.handle.as_deref(), Some("Coco-Lovelock"));
        assert!(profile.videos.is_empty());
        let bare = extract(&json!({}), "https://x.test/channels/some-studio/", None);
        assert_eq!(bare.name, "Some Studio");
        assert_eq!(bare.kind, "channel");
    }

    #[test]
    fn borrowed_lists_longer_than_the_total_are_ignored() {
        let mut initials = fixture();
        initials["infoExp4040Component"]["pornstarTop"]["videoCount"] = json!(1);
        initials["layoutPage"]["videoListProps"]["videoThumbProps"] = json!([
            {"id": 1, "pageURL": "https://x.test/videos/a-1", "title": "One"},
            {"id": 2, "pageURL": "https://x.test/videos/b-2", "title": "Two"}
        ]);
        initials["newestVideoSectionComponent"] = json!({"videoListProps": {"videoThumbProps": [
            {"id": 3, "pageURL": "https://x.test/videos/c-3", "title": "Three"}
        ]}});
        let profile = extract(&initials, "https://x.test/pornstars/jane-doe", None);
        assert_eq!(profile.videos.len(), 1);
        assert_eq!(profile.videos[0].title, "Three");
    }

    #[test]
    fn urls_follow_the_site_grammar() {
        let base = "https://x.test/pornstars/jane-doe/?utm=1";
        assert_eq!(
            url_for(base, "featured", 1),
            "https://x.test/pornstars/jane-doe"
        );
        assert_eq!(
            url_for(base, "added", 1),
            "https://x.test/pornstars/jane-doe/newest"
        );
        assert_eq!(
            url_for(base, "viewed", 3),
            "https://x.test/pornstars/jane-doe/most-viewed/3"
        );
        assert_eq!(
            url_for(base, "featured", 2),
            "https://x.test/pornstars/jane-doe/2"
        );
        assert_eq!(
            canonical("https://x.test/pornstars/jane-doe/newest/2?a=1"),
            "https://x.test/pornstars/jane-doe"
        );
    }

    #[test]
    fn titles_lose_their_suffixes() {
        assert_eq!(
            clean_title(Some("Angela White Porn Videos".into())).as_deref(),
            Some("Angela White")
        );
        assert_eq!(
            clean_title(Some("Jane Doe 1080p HD Videos".into())).as_deref(),
            Some("Jane Doe")
        );
        assert_eq!(clean_title(Some("a | b".into())), None);
        assert_eq!(clean_title(None), None);
    }
}
