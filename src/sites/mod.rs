pub mod xhamster;
pub mod youtube;

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use crate::model::{CommentPage, Profile, Scope, Video};
use crate::source::Sources;

pub trait Provider: Sync {
    fn id(&self) -> &'static str;
    fn title(&self) -> &'static str;
    fn folder(&self) -> &'static str;
    fn aliases(&self) -> &'static [&'static str];

    fn scopes(&self) -> &'static [Scope] {
        &[Scope::Videos]
    }

    fn classify(&self, input: &str) -> Option<Entry>;

    fn search(&self, query: &str, scope: Scope) -> Result<Page, String>;

    fn more(&self, cursor: &Cursor) -> Result<Page, String>;

    fn resolve(&self, item: &Video) -> Result<Sources, String>;

    fn profile(
        &self,
        _url: &str,
        _sort: &str,
        _scope: Scope,
        _page: u32,
    ) -> Result<Profile, String> {
        Err(format!("{} has no profiles", self.title()))
    }

    fn suggest(&self, _query: &str, _scope: Scope) -> Vec<Suggestion> {
        Vec::new()
    }

    fn related(&self, _item: &Video) -> Result<Page, String> {
        Err(format!("{} has no related videos", self.title()))
    }

    fn comments(&self, _item: &Video, _page: u32) -> Result<CommentPage, String> {
        Err(format!("{} has no comments", self.title()))
    }
}

static REGISTRY: [&dyn Provider; 2] = [&youtube::YouTube, &xhamster::XHamster];

#[derive(Clone, Copy)]
pub struct Site(&'static dyn Provider);

impl PartialEq for Site {
    fn eq(&self, other: &Site) -> bool {
        self.id() == other.id()
    }
}

impl Eq for Site {}

impl fmt::Debug for Site {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Site({})", self.id())
    }
}

impl Site {
    pub fn all() -> Vec<Site> {
        REGISTRY.iter().map(|provider| Site(*provider)).collect()
    }

    pub fn first() -> Site {
        Site(REGISTRY[0])
    }

    pub fn parse(text: &str) -> Option<Site> {
        let wanted = text.trim().to_ascii_lowercase();
        Site::all().into_iter().find(|site| {
            site.id() == wanted || site.0.aliases().iter().any(|alias| *alias == wanted)
        })
    }

    pub fn named(id: &str) -> Option<Site> {
        Site::all().into_iter().find(|site| site.id() == id)
    }

    pub fn names() -> String {
        Site::all()
            .iter()
            .map(|site| site.id())
            .collect::<Vec<_>>()
            .join(" or ")
    }

    pub fn id(self) -> &'static str {
        self.0.id()
    }

    pub fn label(self) -> &'static str {
        self.0.id()
    }

    pub fn title(self) -> &'static str {
        self.0.title()
    }

    pub fn folder(self) -> &'static str {
        self.0.folder()
    }

    pub fn scopes(self) -> &'static [Scope] {
        self.0.scopes()
    }

    pub fn next(self) -> Site {
        let all = Site::all();
        let at = all.iter().position(|site| *site == self).unwrap_or(0);
        all[(at + 1) % all.len()]
    }

    pub fn next_scope(self, scope: Scope) -> Scope {
        let scopes = self.scopes();
        let at = scopes.iter().position(|item| *item == scope).unwrap_or(0);
        scopes[(at + 1) % scopes.len()]
    }

    pub fn profile_scopes(self) -> Vec<Scope> {
        self.scopes()
            .iter()
            .copied()
            .filter(|scope| *scope != Scope::Stars)
            .collect()
    }

    pub fn next_profile_scope(self, scope: Scope) -> Scope {
        let scopes = self.profile_scopes();
        let at = scopes.iter().position(|item| *item == scope).unwrap_or(0);
        scopes
            .get((at + 1) % scopes.len().max(1))
            .copied()
            .unwrap_or(scope)
    }

    pub fn has_comments(self) -> bool {
        self.0.id() == xhamster::ID
    }

    pub fn supports(self, scope: Scope) -> bool {
        self.scopes().contains(&scope)
    }

    pub fn search(self, query: &str, scope: Scope) -> Result<Page, String> {
        self.0.search(query, scope)
    }

    pub fn profile(
        self,
        url: &str,
        sort: &str,
        scope: Scope,
        page: u32,
    ) -> Result<Profile, String> {
        self.0.profile(url, sort, scope, page)
    }

    pub fn suggest(self, query: &str, scope: Scope) -> Vec<Suggestion> {
        self.0.suggest(query, scope)
    }
}

#[derive(Clone)]
pub struct Cursor {
    site: &'static str,
    state: Arc<dyn Any + Send + Sync>,
}

impl fmt::Debug for Cursor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Cursor({})", self.site)
    }
}

impl Cursor {
    pub fn new<T: Any + Send + Sync>(site: &'static str, state: T) -> Cursor {
        Cursor {
            site,
            state: Arc::new(state),
        }
    }

    pub fn state<T: Any>(&self) -> Option<&T> {
        self.state.downcast_ref::<T>()
    }
}

#[derive(Default)]
pub struct Page {
    pub videos: Vec<Video>,
    pub cursor: Option<Cursor>,
}

impl Page {
    pub fn extend(&self, into: &mut Vec<Video>) {
        let mut seen: std::collections::HashSet<String> =
            into.iter().map(|video| video.id.clone()).collect();
        for video in &self.videos {
            if seen.insert(video.id.clone()) {
                into.push(video.clone());
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum Entry {
    Search(String, Scope),
    Video(Video),
    Profile(Site, String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub text: String,
    pub query: String,
    pub profile: Option<String>,
    pub tag: &'static str,
    pub count: Option<u64>,
}

pub fn classify(input: &str, current: Site, scope: Scope) -> Entry {
    let input = input.trim();
    for site in Site::all() {
        if let Some(entry) = site.0.classify(input) {
            return entry;
        }
    }
    Entry::Search(input.to_string(), scope_for(current, scope))
}

pub fn link_site(input: &str) -> Option<Site> {
    let input = input.trim();
    Site::all()
        .into_iter()
        .find(|site| site.0.classify(input).is_some())
}

fn scope_for(site: Site, scope: Scope) -> Scope {
    if site.supports(scope) {
        scope
    } else {
        Scope::Videos
    }
}

pub fn more(cursor: &Cursor) -> Result<Page, String> {
    match Site::named(cursor.site) {
        Some(site) => site.0.more(cursor),
        None => Err("unknown site".to_string()),
    }
}

pub fn related(item: &Video) -> Result<Page, String> {
    match Site::named(item.site) {
        Some(site) => site.0.related(item),
        None => Err("unknown site".to_string()),
    }
}

pub fn comments(item: &Video, page: u32) -> Result<CommentPage, String> {
    match Site::named(item.site) {
        Some(site) => site.0.comments(item, page),
        None => Err("unknown site".to_string()),
    }
}

pub fn resolve(item: &Video) -> Result<Sources, String> {
    match Site::named(item.site) {
        Some(site) => site.0.resolve(item),
        None => Err("unknown site".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_is_routed_by_what_it_looks_like() {
        let tube = Site::first();
        match classify(
            "https://xhamster.com/videos/some-title-xhAbC12",
            tube,
            Scope::Videos,
        ) {
            Entry::Video(video) => {
                assert_eq!(video.id, "some-title-xhAbC12");
                assert_eq!(video.site, "xhamster");
                assert_eq!(
                    video.url.as_deref(),
                    Some("https://xhamster.com/videos/some-title-xhAbC12")
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            classify("https://xhamster.com/creators/amy", tube, Scope::Videos),
            Entry::Profile(site, url) if site.id() == "xhamster" && url.ends_with("/creators/amy")
        ));
        assert!(matches!(
            classify("https://xhamster.com/search/lofi-girl", tube, Scope::Videos),
            Entry::Search(query, Scope::Videos) if query == "lofi girl"
        ));
        assert!(matches!(
            classify("https://xhamster.com/photos/search/lofi-girl", tube, Scope::Videos),
            Entry::Search(query, Scope::Galleries) if query == "lofi girl"
        ));
        match classify(
            "https://xhamster.com/photos/gallery/set-55",
            tube,
            Scope::Videos,
        ) {
            Entry::Video(video) => {
                assert_eq!(video.media, crate::model::Media::Gallery);
                assert_eq!(video.id, "set-55");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            classify("https://youtu.be/dQw4w9WgXcQ", Site::named("xhamster").unwrap(), Scope::Videos),
            Entry::Video(video) if video.id == "dQw4w9WgXcQ" && video.site == "youtube"
        ));
        assert!(matches!(
            classify("lofi beats", tube, Scope::Videos),
            Entry::Search(query, Scope::Videos) if query == "lofi beats"
        ));
        assert!(matches!(
            classify("lofi beats", tube, Scope::Galleries),
            Entry::Search(_, Scope::Videos)
        ));
    }

    #[test]
    fn sites_cycle_and_parse() {
        let tube = Site::first();
        assert_eq!(tube.id(), "youtube");
        assert_eq!(tube.next().id(), "xhamster");
        assert_eq!(tube.next().next(), tube);
        assert_eq!(Site::parse(" XH ").unwrap().id(), "xhamster");
        assert_eq!(Site::parse("yt"), Some(tube));
        assert_eq!(Site::parse("vimeo"), None);
        assert_eq!(Site::names(), "youtube or xhamster");
    }

    #[test]
    fn scopes_cycle_within_what_a_site_offers() {
        let tube = Site::first();
        let adult = Site::named("xhamster").unwrap();
        assert_eq!(tube.next_scope(Scope::Videos), Scope::Videos);
        assert_eq!(adult.next_scope(Scope::Videos), Scope::Galleries);
        assert_eq!(adult.next_scope(Scope::Galleries), Scope::Stars);
        assert_eq!(adult.next_scope(Scope::Stars), Scope::Videos);
        assert_eq!(
            adult.profile_scopes(),
            vec![Scope::Videos, Scope::Galleries]
        );
        assert_eq!(adult.next_profile_scope(Scope::Galleries), Scope::Videos);
        assert!(!tube.supports(Scope::Galleries));
    }

    #[test]
    fn cursors_hold_their_site_state() {
        let cursor = Cursor::new("youtube", 7u32);
        assert_eq!(cursor.state::<u32>(), Some(&7));
        assert_eq!(cursor.state::<String>(), None);
        assert!(more(&Cursor::new("nowhere", 0u8)).is_err());
    }
}
