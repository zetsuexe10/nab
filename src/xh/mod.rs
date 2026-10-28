mod cipher;
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

use crate::hls;
use crate::search::Video;
use crate::source::{Kind, Sources, Stream};

pub use net::encode;
#[cfg(test)]
pub use profile::sample as sample_profile;
pub use profile::{Profile, SORT_SEGMENTS};
pub use suggest::Suggestion;

pub const SITE: &str = "https://xhamster.com";
pub const REFERER: &str = "https://xhamster.com/";
pub const INITIALS: &str = "initials";
pub const PAGE: &str = "page source";

#[derive(Clone, Debug)]
pub struct Next {
    base: String,
    template: Option<String>,
    page: u32,
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
    Profile(String),
    Search(String),
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
        ["pornstars" | "creators" | "channels" | "celebrities", slug, ..] => Some(Target::Profile(
            canonical(&format!("/{}/{slug}", segments[0])),
        )),
        ["search", query, ..] => Some(Target::Search(query.replace('-', " "))),
        _ => None,
    }
}

pub struct Client {
    base: String,
}

pub fn live() -> Client {
    Client::at(SITE)
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
            return Err("could not read the page data, the site may have changed".to_string());
        };
        if initials::blocked(&data) {
            return Err("the site answered with an interstitial, try again shortly".to_string());
        }
        Ok((data, html))
    }

    fn listing(&self, url: String) -> Result<Listing, String> {
        let (data, _) = self.load(&url)?;
        let paging = initials::paging(&data);
        let videos = initials::video_list(&data);
        let next = (paging.page < paging.last && !videos.is_empty()).then(|| Next {
            base: url,
            template: paging.template.clone(),
            page: paging.page,
        });
        Ok(Listing { videos, next })
    }

    pub fn search(&self, query: &str) -> Result<Listing, String> {
        let slug = query.split_whitespace().collect::<Vec<_>>().join("-");
        self.listing(format!("{}/search/{}", self.base, net::encode(&slug)))
    }

    pub fn more(&self, next: &Next) -> Result<Listing, String> {
        let page = next.page + 1;
        let (data, _) = self.load(&next.url(page))?;
        let paging = initials::paging(&data);
        let videos = initials::video_list(&data);
        let following = (page < paging.last && !videos.is_empty()).then(|| Next {
            base: next.base.clone(),
            template: next.template.clone().or(paging.template),
            page,
        });
        Ok(Listing {
            videos,
            next: following,
        })
    }

    pub fn resolve(&self, url: &str) -> Result<Sources, String> {
        let html = net::get(url, REFERER, false)?;
        let data = initials::parse(&html);
        if let Some(data) = &data {
            if initials::blocked(data) {
                return Err("the site answered with an interstitial, try again shortly".to_string());
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
        let detail = data.as_ref().map(|data| initials::detail(data, url));
        let mut resolved = Sources {
            title: detail
                .as_ref()
                .map(|video| video.title.clone())
                .filter(|title| !title.is_empty()),
            streams: Vec::new(),
            detail,
        };
        resolved
            .streams
            .extend(streams(&from_initials, INITIALS, seconds));
        resolved.streams.extend(streams(&from_page, PAGE, seconds));
        resolved.sort();
        Ok(resolved)
    }

    pub fn profile(&self, url: &str, sort: &str, page: u32) -> Result<Profile, String> {
        let base = profile::canonical(url);
        let target = profile::url_for(&base, sort, page);
        let (data, html) = self.load(&target)?;
        let heading = initials::title_of(&html);
        let mut found = profile::extract(&data, &target, heading.as_deref());
        found.url = base;
        if found.paging.page < page && page > 1 && found.videos.is_empty() {
            return Err("that page does not exist".to_string());
        }
        found.paging.page = found.paging.page.max(1);
        if sort != "featured" || found.sort.is_empty() {
            found.sort = sort.to_string();
        }
        Ok(found)
    }

    pub fn suggest(&self, query: &str) -> Vec<Suggestion> {
        let query = query.trim();
        if query.is_empty() {
            return Vec::new();
        }
        let url = format!(
            "{}/api/front/search/suggest?searchValue={}&searchScope=video&orientation=straight",
            self.base,
            net::encode(query)
        );
        net::get(&url, REFERER, true)
            .ok()
            .and_then(|body| serde_json::from_str::<Value>(&body).ok())
            .map(|data| suggest::shape(&data))
            .unwrap_or_default()
    }
}

fn streams(found: &[sources::Source], provider: &'static str, seconds: u64) -> Vec<Stream> {
    found
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
                size: source
                    .bitrate
                    .map(|bitrate| bitrate.saturating_mul(seconds) / 8)
                    .unwrap_or(0),
                url: source.url.clone(),
                height: source.height,
            }
        })
        .collect()
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
            Some(Target::Search("lofi girl".into()))
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
        let first = client.search("lofi  girl").unwrap();
        assert_eq!(first.videos.len(), 2);
        assert_eq!(first.videos[0].title, "Video 1");
        assert_eq!(
            first.videos[0].url.as_deref(),
            Some(format!("{base}/videos/video-1").as_str())
        );
        let second = client.more(first.next.as_ref().unwrap()).unwrap();
        assert_eq!(second.videos.len(), 2);
        assert!(second.next.is_none());
        let rows = client.suggest("lo fi");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].text, "lofi");
        assert_eq!(rows[0].count, Some(9));
        assert!(rows[1].kind.is_person());
        assert!(client.suggest("   ").is_empty());
        assert!(client.suggest("nothing served").is_empty());
    }

    #[test]
    fn a_missing_payload_or_wall_is_reported() {
        let base = crate::testserver::serve(vec![
            file("/search/plain", b"<html>no data</html>".to_vec()),
            file("/search/wall", page(&serde_json::json!({"nothing": true}))),
        ]);
        let client = Client::at(&base);
        assert!(client
            .search("plain")
            .err()
            .unwrap()
            .contains("could not read"));
        assert!(client
            .search("wall")
            .err()
            .unwrap()
            .contains("interstitial"));
        assert!(client.search("absent").is_err());
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
            .filter(|stream| stream.provider == INITIALS)
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
            .filter(|stream| stream.provider == PAGE)
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
            .all(|stream| stream.kind == Kind::Muxed));
        assert!(sources.streams[0].remote().referer.is_some());
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
