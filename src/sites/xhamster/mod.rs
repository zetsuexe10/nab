mod cipher;
mod gallery;
mod initials;
mod json;
mod net;
mod page;
mod profile;
mod sources;
mod suggest;

use std::cell::RefCell;
use std::collections::HashMap;

use serde_json::Value;

use super::{Cursor, Entry, Page, Provider, Suggestion};
use crate::hls;
use crate::model::{Media, Profile, Scope, Video};
use crate::source::{Kind, Sources, Stream};

pub use net::encode;
#[cfg(test)]
pub use profile::sample as sample_profile;

pub const ID: &str = "xhamster";
pub const SITE: &str = "https://xhamster.com";
pub const REFERER: &str = "https://xhamster.com/";
pub const INITIALS: &str = "initials";
pub const PAGE: &str = "page source";
const MAX_GALLERY_PAGES: u32 = 60;

pub struct XHamster;

impl Provider for XHamster {
    fn id(&self) -> &'static str {
        ID
    }

    fn title(&self) -> &'static str {
        "xHamster"
    }

    fn folder(&self) -> &'static str {
        "Xhamster"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["xh"]
    }

    fn scopes(&self) -> &'static [Scope] {
        &[Scope::Videos, Scope::Galleries]
    }

    fn classify(&self, input: &str) -> Option<Entry> {
        Some(match classify(input)? {
            Target::Video(url) => Entry::Video(stub(&url, Media::Video)),
            Target::Gallery(url) => Entry::Video(stub(&url, Media::Gallery)),
            Target::Profile(url) => Entry::Profile(crate::sites::Site::named(ID)?, url),
            Target::Search(query, scope) => Entry::Search(query, scope),
        })
    }

    fn search(&self, query: &str, scope: Scope) -> Result<Page, String> {
        Ok(page_of(live().search(query, scope)?))
    }

    fn more(&self, cursor: &Cursor) -> Result<Page, String> {
        let next = cursor
            .state::<Next>()
            .ok_or_else(|| "unknown cursor".to_string())?;
        Ok(page_of(live().more(next)?))
    }

    fn resolve(&self, item: &Video) -> Result<Sources, String> {
        let url = item
            .url
            .as_deref()
            .ok_or_else(|| "this item has no page address".to_string())?;
        match item.media {
            Media::Gallery => live().gallery(url),
            Media::Video => live().resolve(url),
        }
    }

    fn profile(&self, url: &str, sort: &str, scope: Scope, page: u32) -> Result<Profile, String> {
        live().profile(url, sort, scope, page)
    }

    fn suggest(&self, query: &str, scope: Scope) -> Vec<Suggestion> {
        live().suggest(query, scope)
    }
}

fn stub(url: &str, media: Media) -> Video {
    let id = url.rsplit('/').next().unwrap_or(url).to_string();
    Video {
        id,
        url: Some(url.to_string()),
        media,
        site: ID,
        ..Video::default()
    }
}

fn page_of(listing: Listing) -> Page {
    Page {
        videos: listing.videos,
        cursor: listing.next.map(|next| Cursor::new(ID, next)),
    }
}

#[derive(Clone, Debug)]
pub struct Next {
    base: String,
    template: Option<String>,
    page: u32,
    scope: Scope,
}

impl Next {
    fn url(&self, page: u32) -> String {
        if page <= 1 {
            return self.base.clone();
        }
        if let Some(template) = self.template.as_ref().filter(|t| t.contains("{#}")) {
            return template.replace("{#}", &page.to_string());
        }
        let (path, query) = match self.base.split_once('?') {
            Some((path, query)) => (path, Some(query)),
            None => (self.base.as_str(), None),
        };
        match query {
            Some(query) if query.split('&').any(|pair| pair.starts_with("page=")) => {
                let rewritten: Vec<String> = query
                    .split('&')
                    .map(|pair| {
                        if pair.starts_with("page=") {
                            format!("page={page}")
                        } else {
                            pair.to_string()
                        }
                    })
                    .collect();
                format!("{path}?{}", rewritten.join("&"))
            }
            Some(query) => format!("{}/{page}?{query}", path.trim_end_matches('/')),
            None => format!("{}/{page}", path.trim_end_matches('/')),
        }
    }
}

pub struct Listing {
    pub videos: Vec<Video>,
    pub next: Option<Next>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Video(String),
    Gallery(String),
    Profile(String),
    Search(String, Scope),
}

pub fn classify(input: &str) -> Option<Target> {
    let input = input.trim();
    let lower = input.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))?;
    let host = rest.split('/').next().unwrap_or("");
    if !host.contains("xhamster") || input.contains(char::is_whitespace) {
        return None;
    }
    let skip = input.len() - rest.len();
    let path = &input[skip + host.len()..];
    let path = path.split(['?', '#']).next().unwrap_or("");
    let segments: Vec<&str> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let canonical = |tail: &str| format!("{SITE}{tail}");
    match segments.as_slice() {
        ["videos" | "shorts", _, ..] => Some(Target::Video(canonical(&format!(
            "/{}",
            segments[..2].join("/")
        )))),
        ["photos", "gallery", _, ..] => Some(Target::Gallery(canonical(&format!(
            "/{}",
            segments[..3].join("/")
        )))),
        ["photos", "search", query, ..] => {
            Some(Target::Search(query.replace('-', " "), Scope::Galleries))
        }
        ["pornstars" | "creators" | "channels" | "celebrities", slug, ..] => Some(Target::Profile(
            canonical(&format!("/{}/{slug}", segments[0])),
        )),
        ["search", query, ..] => Some(Target::Search(query.replace('-', " "), Scope::Videos)),
        _ => None,
    }
}

pub struct Client {
    base: String,
}

pub fn live() -> Client {
    Client::at(SITE)
}

fn unreadable() -> String {
    "could not read the page data, the site may have changed".to_string()
}

fn interstitial() -> String {
    "the site answered with an interstitial, try again shortly".to_string()
}

impl Client {
    pub fn at(base: &str) -> Client {
        Client {
            base: base.trim_end_matches('/').to_string(),
        }
    }

    fn load(&self, url: &str) -> Result<(Value, String), String> {
        let html = net::get(url, REFERER, false)?;
        let Some(data) = initials::parse(&html) else {
            return Err(unreadable());
        };
        if initials::blocked(&data) {
            return Err(interstitial());
        }
        Ok((data, html))
    }

    fn items(data: &Value, scope: Scope) -> (Vec<Video>, crate::model::Paging) {
        match scope {
            Scope::Videos => {
                let mut videos = initials::video_list(data);
                videos.iter_mut().for_each(|video| video.site = ID);
                (videos, initials::paging(data))
            }
            Scope::Galleries => {
                let mut galleries = gallery::listing(data);
                galleries.iter_mut().for_each(|item| item.site = ID);
                (galleries, gallery::directory_paging(data))
            }
        }
    }

    fn listing(&self, url: String, scope: Scope) -> Result<Listing, String> {
        let (data, _) = self.load(&url)?;
        let (videos, paging) = Self::items(&data, scope);
        let next = (paging.page < paging.last && !videos.is_empty()).then(|| Next {
            base: url,
            template: paging.template.clone(),
            page: paging.page,
            scope,
        });
        Ok(Listing { videos, next })
    }

    pub fn search(&self, query: &str, scope: Scope) -> Result<Listing, String> {
        let slug = query.split_whitespace().collect::<Vec<_>>().join("-");
        let root = match scope {
            Scope::Videos => "search",
            Scope::Galleries => "photos/search",
        };
        self.listing(
            format!("{}/{root}/{}", self.base, net::encode(&slug)),
            scope,
        )
    }

    pub fn more(&self, next: &Next) -> Result<Listing, String> {
        let page = next.page + 1;
        let (data, _) = self.load(&next.url(page))?;
        let (videos, paging) = Self::items(&data, next.scope);
        let following = (page < paging.last && !videos.is_empty()).then(|| Next {
            base: next.base.clone(),
            template: next.template.clone().or(paging.template),
            page,
            scope: next.scope,
        });
        Ok(Listing {
            videos,
            next: following,
        })
    }

    pub fn gallery(&self, url: &str) -> Result<Sources, String> {
        let (data, _) = self.load(url)?;
        if json::flag(json::dig(&data, "galleryPage.galleryModel.isGeoBlocked"))
            || json::flag(json::dig(&data, "galleryPage.isGeoBlocked"))
        {
            return Err("this gallery is not available in your region".to_string());
        }
        let mut found = gallery::page(&data, url);
        let wanted = found.claimed.unwrap_or(0) as usize;
        let mut page = found.paging.page.max(1);
        let last = found.paging.last.min(MAX_GALLERY_PAGES);
        let next = Next {
            base: url.to_string(),
            template: found.paging.template.clone(),
            page,
            scope: Scope::Galleries,
        };
        while found.photos.len() < wanted && page < last {
            page += 1;
            let Ok((more, _)) = self.load(&next.url(page)) else {
                break;
            };
            let extra = gallery::photos(&more);
            if extra.is_empty() {
                break;
            }
            for photo in extra {
                if !found.photos.iter().any(|held| held.url == photo.url) {
                    found.photos.push(photo);
                }
            }
        }
        if found.photos.is_empty() {
            return Err("no photos found in this gallery".to_string());
        }
        found.item.site = ID;
        let total = found.photos.len();
        let label = if total == 1 {
            "1 photo".to_string()
        } else {
            format!("{total} photos")
        };
        Ok(Sources {
            title: Some(found.item.title.clone()).filter(|title| !title.is_empty()),
            streams: vec![Stream {
                provider: INITIALS,
                kind: Kind::Gallery,
                quality: label,
                ext: "folder".to_string(),
                size: 0,
                url: url.to_string(),
                height: 0,
                referer: Some(REFERER),
                extract: false,
            }],
            detail: Some(found.item),
            photos: found.photos,
            preferred: Some(INITIALS),
        })
    }

    pub fn resolve(&self, url: &str) -> Result<Sources, String> {
        let html = net::get(url, REFERER, false)?;
        let data = initials::parse(&html);
        if let Some(data) = &data {
            if initials::blocked(data) {
                return Err(interstitial());
            }
            if json::flag(json::dig(data, "videoEntity.isGeoBlocked")) {
                return Err("this video is not available in your region".to_string());
            }
        }
        let seconds = data.as_ref().map(initials::duration_seconds).unwrap_or(0);
        let cache: RefCell<HashMap<String, Vec<hls::Variant>>> = RefCell::new(HashMap::new());
        let master = |playlist: &str| -> Vec<hls::Variant> {
            if let Some(held) = cache.borrow().get(playlist) {
                return held.clone();
            }
            let variants = hls::fetch(playlist, Some(REFERER))
                .map(|text| hls::parse_master(&text, playlist))
                .unwrap_or_default();
            cache
                .borrow_mut()
                .insert(playlist.to_string(), variants.clone());
            variants
        };
        let mut found = data
            .as_ref()
            .map(sources::from_initials)
            .unwrap_or_default();
        if found.is_empty() {
            if let Some(direct) = data
                .as_ref()
                .and_then(|data| json::text(json::dig(data, "videoModel.downloadFile")))
            {
                found.push(sources::Source {
                    url: direct,
                    height: 0,
                    hls: false,
                    codec: "h264".to_string(),
                    bitrate: None,
                    vr: false,
                });
            }
        }
        let from_initials = sources::expand(found, seconds, &master);
        let from_page = sources::expand(page::sources(&html), seconds, &master);
        if from_initials.is_empty() && from_page.is_empty() {
            return Err(
                if data.as_ref().is_some_and(|data| {
                    json::dig(data, "videoEntity.isDownloadable").and_then(Value::as_bool)
                        == Some(false)
                }) {
                    "the site does not offer this video for download".to_string()
                } else {
                    "no downloadable streams".to_string()
                },
            );
        }
        let detail = data.as_ref().map(|data| {
            let mut detail = initials::detail(data, url);
            detail.site = ID;
            detail
        });
        let mut resolved = Sources {
            title: detail
                .as_ref()
                .map(|video| video.title.clone())
                .filter(|title| !title.is_empty()),
            streams: Vec::new(),
            detail,
            photos: Vec::new(),
            preferred: Some(INITIALS),
        };
        resolved
            .streams
            .extend(streams(&from_initials, INITIALS, seconds));
        resolved.streams.extend(streams(&from_page, PAGE, seconds));
        resolved.sort();
        Ok(resolved)
    }

    pub fn profile(
        &self,
        url: &str,
        sort: &str,
        scope: Scope,
        page: u32,
    ) -> Result<Profile, String> {
        let base = profile::canonical(url);
        if scope == Scope::Galleries {
            return self.profile_galleries(&base, page);
        }
        let target = profile::url_for(&base, sort, page);
        let (data, html) = self.load(&target)?;
        let heading = initials::title_of(&html);
        let mut found = profile::extract(&data, &target, heading.as_deref());
        found.url = base;
        found.videos.iter_mut().for_each(|video| video.site = ID);
        if found.paging.page < page && page > 1 && found.videos.is_empty() {
            return Err("that page does not exist".to_string());
        }
        found.paging.page = found.paging.page.max(1);
        if sort != "featured" || found.sort.is_empty() {
            found.sort = sort.to_string();
        }
        Ok(found)
    }

    fn profile_galleries(&self, base: &str, page: u32) -> Result<Profile, String> {
        let (data, html) = self.load(base)?;
        let heading = initials::title_of(&html);
        let mut found = profile::extract(&data, base, heading.as_deref());
        found.url = base.to_string();
        let listing = gallery::person_url(&data).unwrap_or_else(|| format!("{base}/photos"));
        let target = profile::url_for(&listing, "featured", page);
        let (list, _) = self.load(&target)?;
        let (mut items, paging) = gallery::person(&list);
        items.iter_mut().for_each(|item| item.site = ID);
        if items.is_empty() {
            return Err(if page > 1 {
                "that page does not exist".to_string()
            } else {
                "this profile has no galleries".to_string()
            });
        }
        found.videos = items;
        found.scope = Scope::Galleries;
        found.paging = paging;
        found.paging.page = found.paging.page.max(page);
        found.sorts = Vec::new();
        found.sort = String::new();
        Ok(found)
    }

    pub fn suggest(&self, query: &str, scope: Scope) -> Vec<Suggestion> {
        let query = query.trim();
        if query.is_empty() {
            return Vec::new();
        }
        let area = match scope {
            Scope::Videos => "video",
            Scope::Galleries => "photo",
        };
        let url = format!(
            "{}/api/front/search/suggest?searchValue={}&searchScope={area}&orientation=straight",
            self.base,
            net::encode(query)
        );
        net::get(&url, REFERER, true)
            .ok()
            .and_then(|body| serde_json::from_str::<Value>(&body).ok())
            .map(|data| {
                suggest::shape(&data)
                    .into_iter()
                    .map(|row| Suggestion {
                        profile: row.kind.is_person().then(|| row.url.clone()),
                        tag: row.kind.label(),
                        text: row.text,
                        query: row.query,
                        count: row.count,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn smallest(found: &[sources::Source], hls: bool) -> Option<&sources::Source> {
    found
        .iter()
        .filter(|source| source.hls == hls)
        .min_by_key(|source| {
            if source.height == 0 {
                u32::MAX
            } else {
                source.height
            }
        })
}

fn streams(found: &[sources::Source], provider: &'static str, seconds: u64) -> Vec<Stream> {
    let size_of = |source: &sources::Source| {
        source
            .bitrate
            .map(|bitrate| bitrate.saturating_mul(seconds) / 8)
            .unwrap_or(0)
    };
    let mut listed: Vec<Stream> = found
        .iter()
        .map(|source| {
            let mut quality = if source.height > 0 {
                format!("{}p", source.height)
            } else if source.hls {
                "stream".to_string()
            } else {
                "video".to_string()
            };
            if source.vr {
                quality.push_str(" vr");
            }
            Stream {
                provider,
                kind: Kind::Muxed,
                quality,
                ext: if source.hls { "m3u8" } else { "mp4" }.to_string(),
                size: size_of(source),
                url: source.url.clone(),
                height: source.height,
                referer: Some(REFERER),
                extract: false,
            }
        })
        .collect();
    if let Some(source) = smallest(found, false).or_else(|| smallest(found, true)) {
        listed.push(Stream {
            provider,
            kind: Kind::Audio,
            quality: "audio".to_string(),
            ext: "m4a".to_string(),
            size: size_of(source),
            url: source.url.clone(),
            height: 0,
            referer: Some(REFERER),
            extract: true,
        });
    }
    listed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasted_links_are_sorted_into_videos_profiles_and_searches() {
        assert_eq!(
            classify("https://xhamster.com/videos/some-title-xhAbC12?x=1"),
            Some(Target::Video(
                "https://xhamster.com/videos/some-title-xhAbC12".into()
            ))
        );
        assert_eq!(
            classify("https://m.xhamster2.com/pornstars/jane-doe/newest/2"),
            Some(Target::Profile(
                "https://xhamster.com/pornstars/jane-doe".into()
            ))
        );
        assert_eq!(
            classify("http://xhamster.desi/creators/amy"),
            Some(Target::Profile("https://xhamster.com/creators/amy".into()))
        );
        assert_eq!(
            classify("https://xhamster.com/search/lofi-girl?sort=newest"),
            Some(Target::Search("lofi girl".into(), Scope::Videos))
        );
        assert_eq!(classify("https://xhamster.com/"), None);
        assert_eq!(classify("https://example.com/videos/a-b"), None);
        assert_eq!(classify("lofi beats"), None);
        assert_eq!(classify("https://xhamster.com/videos/a b"), None);
    }

    #[test]
    fn later_pages_follow_the_template_or_the_path() {
        let next = |base: &str, template: Option<&str>| Next {
            base: base.to_string(),
            template: template.map(str::to_string),
            page: 1,
            scope: Scope::Videos,
        };
        assert_eq!(
            next(
                "https://x.test/search/a",
                Some("https://x.test/search/a/{#}?z=1")
            )
            .url(3),
            "https://x.test/search/a/3?z=1"
        );
        assert_eq!(
            next("https://x.test/search/a", None).url(2),
            "https://x.test/search/a/2"
        );
        assert_eq!(
            next("https://x.test/search/a?sort=new", None).url(2),
            "https://x.test/search/a/2?sort=new"
        );
        assert_eq!(
            next("https://x.test/a?page=1&q=2", None).url(5),
            "https://x.test/a?page=5&q=2"
        );
        assert_eq!(next("https://x.test/a", None).url(1), "https://x.test/a");
    }

    fn page(data: &Value) -> Vec<u8> {
        format!("<html><script>window.initials={data};</script></html>").into_bytes()
    }

    fn row(id: u32, base: &str) -> Value {
        serde_json::json!({
            "id": id,
            "title": format!("Video {id}"),
            "pageURL": format!("{base}/videos/video-{id}"),
            "duration": 90,
            "views": 1000
        })
    }

    fn file(path: &str, body: Vec<u8>) -> (String, Vec<u8>) {
        (path.to_string(), body)
    }

    #[test]
    fn search_pages_through_the_template_and_suggestions_are_shaped() {
        let base = crate::testserver::serve_with(|base| {
            vec![
                file(
                    "/search/lofi-girl",
                    page(&serde_json::json!({
                        "searchResult": {"videoThumbProps": [row(1, base), row(2, base)]},
                        "pagination": {"active": 1, "maxPages": 2, "pageLinkTemplate": format!("{base}/search/lofi-girl/{{#}}")}
                    })),
                ),
                file(
                    "/search/lofi-girl/2",
                    page(&serde_json::json!({
                        "searchResult": {"videoThumbProps": [row(2, base), row(3, base)]},
                        "pagination": {"active": 2, "maxPages": 2}
                    })),
                ),
                file(
                    "/api/front/search/suggest?searchValue=lo%20fi&searchScope=video&orientation=straight",
                    br#"[{"link":"https://xhamster.com/search/lofi","text":"<b>lo</b>fi","type2":"search","count":9},{"link":"https://xhamster.com/creators/lo-fi","text":"Lo Fi","type2":"creator"},{"link":"https://xhamster.com/users/u","text":"U","type2":"user"}]"#.to_vec(),
                ),
            ]
        });
        let client = Client::at(&base);
        let first = client.search("lofi  girl", Scope::Videos).unwrap();
        assert_eq!(first.videos.len(), 2);
        assert_eq!(first.videos[0].title, "Video 1");
        assert_eq!(
            first.videos[0].url.as_deref(),
            Some(format!("{base}/videos/video-1").as_str())
        );
        let second = client.more(first.next.as_ref().unwrap()).unwrap();
        assert_eq!(second.videos.len(), 2);
        assert!(second.next.is_none());
        let rows = client.suggest("lo fi", Scope::Videos);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].text, "lofi");
        assert_eq!(rows[0].count, Some(9));
        assert!(rows[1].profile.is_some());
        assert_eq!(rows[1].tag, "creator");
        assert!(client.suggest("   ", Scope::Videos).is_empty());
        assert!(client.suggest("nothing served", Scope::Videos).is_empty());
    }

    #[test]
    fn a_missing_payload_or_wall_is_reported() {
        let base = crate::testserver::serve(vec![
            file("/search/plain", b"<html>no data</html>".to_vec()),
            file("/search/wall", page(&serde_json::json!({"nothing": true}))),
        ]);
        let client = Client::at(&base);
        assert!(client
            .search("plain", Scope::Videos)
            .err()
            .unwrap()
            .contains("could not read"));
        assert!(client
            .search("wall", Scope::Videos)
            .err()
            .unwrap()
            .contains("interstitial"));
        assert!(client.search("absent", Scope::Videos).is_err());
    }

    #[test]
    fn a_video_page_yields_initials_and_page_source_streams() {
        let base = crate::testserver::serve_with(|base| {
            let direct = cipher::encrypt(3, 99, &format!("{base}/media/720p.h264.mp4"));
            let data = serde_json::json!({
                "videoEntity": {"id": 5, "title": "A &amp; B", "duration": 100},
                "videoModel": {"author": {"name": "Studio", "pageURL": format!("{base}/channels/studio")}},
                "xplayerSettings": {"sources": {
                    "hls": {"h264": {"url": format!("{base}/media/master.m3u8")}},
                    "standard": {"h264": [
                        {"quality": "720p", "url": direct},
                        {"quality": "240p", "url": "0a0b0c0d0e0f0a0b0c0d0e0f"}
                    ]}
                }}
            });
            let mut html = String::from("<html><script>window.initials=");
            html.push_str(&data.to_string());
            html.push_str(&format!(
                ";</script><source src=\"{base}/media/480p.h264.mp4\"><link href=\"{base}/media/preview.t.mp4\"></html>"
            ));
            vec![
                file("/videos/clip", html.into_bytes()),
                file(
                    "/media/master.m3u8",
                    b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=4000000,RESOLUTION=1920x1080\n1080.h264.mp4.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=2000000,RESOLUTION=1280x720\n720.h264.mp4.m3u8\n".to_vec(),
                ),
            ]
        });
        let client = Client::at(&base);
        let sources = client.resolve(&format!("{base}/videos/clip")).unwrap();
        assert_eq!(sources.title.as_deref(), Some("A & B"));
        let detail = sources.detail.as_ref().unwrap();
        assert_eq!(detail.channel, "Studio");
        assert_eq!(detail.duration, "1:40");
        assert_eq!(
            detail.author_url.as_deref(),
            Some(format!("{base}/channels/studio").as_str())
        );
        assert_eq!(sources.providers(), vec![INITIALS, PAGE]);
        let initial: Vec<(String, String, u64)> = sources
            .streams
            .iter()
            .filter(|stream| stream.provider == INITIALS && stream.kind == Kind::Muxed)
            .map(|stream| (stream.quality.clone(), stream.ext.clone(), stream.size))
            .collect();
        assert_eq!(
            initial,
            vec![
                ("1080p".to_string(), "m3u8".to_string(), 50_000_000),
                ("720p".to_string(), "mp4".to_string(), 25_000_000),
                ("720p".to_string(), "m3u8".to_string(), 25_000_000),
            ]
        );
        let scraped: Vec<(String, String)> = sources
            .streams
            .iter()
            .filter(|stream| stream.provider == PAGE && stream.kind == Kind::Muxed)
            .map(|stream| (stream.quality.clone(), stream.ext.clone()))
            .collect();
        assert_eq!(
            scraped,
            vec![
                ("1080p".to_string(), "m3u8".to_string()),
                ("720p".to_string(), "m3u8".to_string()),
                ("480p".to_string(), "mp4".to_string()),
            ]
        );
        assert!(sources
            .streams
            .iter()
            .all(|stream| matches!(stream.kind, Kind::Muxed | Kind::Audio)));
        assert!(sources
            .streams
            .iter()
            .all(|stream| stream.referer == Some(REFERER)));
        let audio: Vec<(&str, bool, bool)> = sources
            .streams
            .iter()
            .filter(|stream| stream.is_audio())
            .map(|stream| {
                (
                    stream.provider,
                    stream.extract,
                    stream.url.ends_with(".mp4"),
                )
            })
            .collect();
        assert_eq!(audio, vec![(INITIALS, true, true), (PAGE, true, true)]);
        let from_page = sources
            .streams
            .iter()
            .find(|stream| stream.is_audio() && stream.provider == PAGE)
            .unwrap();
        assert!(from_page.url.contains("480p"));
        assert_eq!(from_page.ext, "m4a");
        assert!(from_page.remote().extract);
        assert!(sources
            .audio_for(sources.best_video(None).unwrap())
            .is_none());
    }

    #[test]
    fn a_video_with_nothing_to_download_says_so() {
        let base = crate::testserver::serve(vec![
            file(
                "/videos/locked",
                page(&serde_json::json!({"videoEntity": {"id": 1, "isDownloadable": false}})),
            ),
            file(
                "/videos/empty",
                page(&serde_json::json!({"videoEntity": {"id": 2}})),
            ),
            file(
                "/videos/geo",
                page(&serde_json::json!({"videoEntity": {"id": 3, "isGeoBlocked": true}})),
            ),
        ]);
        let client = Client::at(&base);
        assert!(client
            .resolve(&format!("{base}/videos/locked"))
            .err()
            .unwrap()
            .contains("not offer"));
        assert!(client
            .resolve(&format!("{base}/videos/empty"))
            .err()
            .unwrap()
            .contains("no downloadable"));
        assert!(client
            .resolve(&format!("{base}/videos/geo"))
            .err()
            .unwrap()
            .contains("region"));
    }

    #[test]
    fn profiles_load_with_the_chosen_sort_and_page() {
        let base = crate::testserver::serve_with(|base| {
            let data = serde_json::json!({
                "infoExp4040Component": {"pageTitle": "Jane Doe Porn Videos"},
                "layoutPage": {"videoListProps": {"videoThumbProps": [row(1, base)]}},
                "paginationComponent": {"currentPageNumber": 2, "lastPageNumber": 5}
            });
            let mut html = b"<h1 data-role=\"page-title\">Jane Doe Porn Videos</h1>".to_vec();
            html.extend(page(&data));
            vec![file("/pornstars/jane-doe/newest/2", html)]
        });
        let client = Client::at(&base);
        let found = client
            .profile(
                &format!("{base}/pornstars/jane-doe/featured?x=1"),
                "added",
                Scope::Videos,
                2,
            )
            .unwrap();
        assert_eq!(found.name, "Jane Doe");
        assert_eq!(found.videos.len(), 1);
        assert_eq!((found.paging.page, found.paging.last), (2, 5));
        assert_eq!(found.sort, "added");
        assert!(found.url.ends_with("/pornstars/jane-doe"));
    }
}
