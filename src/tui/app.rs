use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use super::clip;
use super::store::Store;
use super::theme::ThemeMode;
use crate::download::{self, Job, Msg, Remote, Stage};
use crate::layout::{Folder, Layout};
use crate::model::{Profile, Scope, Video};
use crate::play::{self, Player};
use crate::sites::{self, Cursor, Entry, Page, Site, Suggestion};
use crate::source::{Kind, Sources, Stream};

const NOTICE_FOR: Duration = Duration::from_secs(5);
const PAGE: isize = 8;
const SUGGEST_AFTER: Duration = Duration::from_millis(180);
pub const SUGGEST_MAX: usize = 6;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Query,
    Results,
    Formats,
    Profile,
    Busy,
    Download,
    Done,
    Error,
}

pub enum Flow {
    Continue,
    Quit,
    Play {
        video: Option<Remote>,
        audio: Option<Remote>,
        title: String,
        player: Player,
    },
    PlayFile {
        path: PathBuf,
        video: bool,
        player: Player,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hit {
    Key(KeyCode, KeyModifiers),
    Result(usize),
    Choice(usize),
    Suggest(usize),
    Logo,
}

#[derive(Clone, Copy, Debug)]
pub struct Zone {
    pub rect: Rect,
    pub hit: Hit,
}

enum Work {
    First(Result<Page, String>),
    More(Result<Page, String>),
    Resolved(Video, Result<Sources, String>),
    Profiled(Result<Profile, String>),
    Suggested(Site, Scope, String, Vec<Suggestion>),
    Clipboard(Option<String>),
}

pub struct App {
    pub site: Site,
    pub scope: Scope,
    pub query: String,
    pub caret: usize,
    pub suggestions: Vec<Suggestion>,
    pub suggest_sel: Option<usize>,
    suggest_for: String,
    typed_at: Option<Instant>,
    pub profile: Option<Profile>,
    profile_site: Site,
    pub psel: usize,
    pub pscroll: usize,
    profile_from: Mode,
    formats_from: Mode,
    pub results: Vec<Video>,
    pub sel: usize,
    pub rscroll: usize,
    pub mode: Mode,
    pub back: Mode,
    pub busy: String,
    pub current: Option<Video>,
    pub sources: Option<Sources>,
    pub provider: Option<&'static str>,
    pub choice: usize,
    pub cscroll: usize,
    pub players: Vec<Player>,
    pub player: Option<Player>,
    pub loading_more: bool,
    pub cursor: Option<Cursor>,
    pub steps: Vec<Stage>,
    pub stage: Stage,
    pub progress: u64,
    pub total: u64,
    pub phase_started: Instant,
    pub started: Instant,
    pub clock: Instant,
    pub born: Instant,
    pub cancelling: bool,
    pub picked: String,
    pub saved: Option<PathBuf>,
    pub saved_video: bool,
    pub saved_gallery: bool,
    pub error: String,
    pub notice: Option<(String, Instant)>,
    pub layout: Layout,
    pub theme: ThemeMode,
    pub history: Vec<String>,
    pub recall: Option<usize>,
    pub draft: String,
    pub clip: Option<String>,
    pub clip_taken: bool,
    pub animate: bool,
    pub zones: Vec<Zone>,
    store: Option<Store>,
    ticket: u64,
    work_tx: Sender<(u64, Work)>,
    work_rx: Receiver<(u64, Work)>,
    dl_rx: Option<Receiver<Msg>>,
    cancel: Option<Arc<AtomicBool>>,
}

impl App {
    pub fn new(query: String, layout: Layout, player: Option<Player>) -> Self {
        let (work_tx, work_rx) = mpsc::channel();
        let players: Vec<Player> = [Player::Mpv, Player::Vlc]
            .into_iter()
            .filter(|candidate| play::locate(*candidate).is_some())
            .collect();
        let caret = query.chars().count();
        App {
            site: Site::first(),
            scope: Scope::Videos,
            query,
            caret,
            suggestions: Vec::new(),
            suggest_sel: None,
            suggest_for: String::new(),
            typed_at: None,
            profile: None,
            profile_site: Site::first(),
            psel: 0,
            pscroll: 0,
            profile_from: Mode::Query,
            formats_from: Mode::Results,
            results: Vec::new(),
            sel: 0,
            rscroll: 0,
            mode: Mode::Query,
            back: Mode::Query,
            busy: String::new(),
            current: None,
            sources: None,
            provider: None,
            choice: 0,
            cscroll: 0,
            players,
            player,
            loading_more: false,
            cursor: None,
            steps: Vec::new(),
            stage: Stage::Fetching,
            progress: 0,
            total: 0,
            phase_started: Instant::now(),
            started: Instant::now(),
            clock: Instant::now(),
            born: Instant::now(),
            cancelling: false,
            picked: String::new(),
            saved: None,
            saved_video: false,
            saved_gallery: false,
            error: String::new(),
            notice: None,
            layout,
            theme: ThemeMode::Auto,
            history: Vec::new(),
            recall: None,
            draft: String::new(),
            clip: None,
            clip_taken: false,
            animate: true,
            zones: Vec::new(),
            store: None,
            ticket: 0,
            work_tx,
            work_rx,
            dl_rx: None,
            cancel: None,
        }
    }

    pub fn with_store(mut self, store: Option<Store>) -> Self {
        if let Some(store) = &store {
            self.history = store.history();
            if let Some(theme) = store.theme() {
                self.theme = theme;
            }
            if let Some(site) = store.site() {
                self.site = site;
            }
        }
        self.store = store;
        self
    }

    pub fn with_theme(mut self, theme: Option<ThemeMode>) -> Self {
        if let Some(theme) = theme {
            self.theme = theme;
        }
        self
    }

    pub fn with_site(mut self, site: Option<Site>) -> Self {
        if let Some(site) = site {
            self.site = site;
        }
        self
    }

    pub fn watch_clipboard(&mut self) {
        let tx = self.work_tx.clone();
        std::thread::spawn(move || {
            let found = clip::read().and_then(|text| clip::link(&text));
            let _ = tx.send((0, Work::Clipboard(found)));
        });
    }

    pub fn offered(&self) -> Option<&str> {
        if self.clip_taken || !self.query.is_empty() {
            return None;
        }
        self.clip.as_deref()
    }

    pub fn title(&self) -> String {
        if let Some(video) = &self.current {
            if !video.title.is_empty() {
                return video.title.clone();
            }
        }
        if let Some(title) = self
            .sources
            .as_ref()
            .and_then(|sources| sources.title.clone())
            .filter(|title| !title.is_empty())
        {
            return title;
        }
        self.current
            .as_ref()
            .map(|video| video.id.clone())
            .unwrap_or_else(|| "video".to_string())
    }

    pub fn tell(&mut self, text: impl Into<String>) {
        self.notice = Some((text.into(), Instant::now()));
    }

    pub fn fail(&mut self, error: impl Into<String>, back: Mode) {
        self.error = error.into();
        self.back = back;
        self.mode = Mode::Error;
    }

    pub fn tick(&mut self) {
        if let Some((_, at)) = &self.notice {
            if at.elapsed() > NOTICE_FOR {
                self.notice = None;
            }
        }
        self.poll_work();
        self.poll_download();
        self.maybe_load_more();
        self.maybe_suggest();
    }

    fn edited(&mut self) {
        self.suggest_sel = None;
        self.recall = None;
        let query = self.query.trim();
        if query.is_empty()
            || query.contains("://")
            || sites::link_site(query).is_some()
            || !matches!(
                sites::classify(query, self.site, self.scope),
                Entry::Search(..)
            )
        {
            self.suggestions.clear();
            self.suggest_for.clear();
            self.typed_at = None;
            return;
        }
        if query != self.suggest_for {
            self.typed_at = Some(Instant::now());
        }
    }

    fn maybe_suggest(&mut self) {
        let Some(at) = self.typed_at else {
            return;
        };
        if self.mode != Mode::Query || at.elapsed() < SUGGEST_AFTER {
            return;
        }
        self.typed_at = None;
        let query = self.query.trim().to_string();
        if query.is_empty() || query == self.suggest_for {
            return;
        }
        self.suggest_for = query.clone();
        let tx = self.work_tx.clone();
        let (site, scope) = (self.site, self.scope);
        std::thread::spawn(move || {
            let rows = site.suggest(&query, scope);
            let _ = tx.send((0, Work::Suggested(site, scope, query, rows)));
        });
    }

    pub fn dropdown(&self) -> bool {
        self.mode == Mode::Query && !self.suggestions.is_empty() && !self.query.trim().is_empty()
    }

    fn switch_site(&mut self) {
        self.site = self.site.next();
        if let Some(store) = &self.store {
            store.save_site(self.site);
        }
        if !self.site.supports(self.scope) {
            self.scope = Scope::Videos;
        }
        self.reset_suggestions();
        self.results.clear();
        self.cursor = None;
        self.tell(format!("searching {}", self.site.title()));
    }

    fn switch_scope(&mut self) {
        let next = self.site.next_scope(self.scope);
        if next == self.scope {
            self.tell(format!(
                "{} only has {}",
                self.site.title(),
                self.scope.label()
            ));
            return;
        }
        self.scope = next;
        self.reset_suggestions();
        self.results.clear();
        self.cursor = None;
        self.tell(format!(
            "searching {} {}",
            self.site.title(),
            self.scope.label()
        ));
    }

    fn reset_suggestions(&mut self) {
        self.suggestions.clear();
        self.suggest_for.clear();
        self.suggest_sel = None;
        self.typed_at = None;
        self.edited();
    }

    fn move_suggestion(&mut self, delta: isize) {
        let last = self.suggestions.len() as isize - 1;
        if last < 0 {
            return;
        }
        self.suggest_sel = match (self.suggest_sel, delta) {
            (None, 1) => Some(0),
            (None, _) => Some(last as usize),
            (Some(0), -1) => None,
            (Some(at), step) => Some((at as isize + step).clamp(0, last) as usize),
        };
    }

    fn take_suggestion(&mut self, index: usize) {
        let Some(row) = self.suggestions.get(index).cloned() else {
            return;
        };
        self.query = row.query.clone();
        self.caret = self.len();
        self.suggestions.clear();
        self.suggest_sel = None;
        self.suggest_for = row.query.clone();
        self.typed_at = None;
        match row.profile {
            Some(url) => {
                self.remember(&row.text);
                self.open_profile(self.site, url, "featured", Scope::Videos, 1, Mode::Query);
            }
            None => self.submit(),
        }
    }

    fn busy(&mut self, label: &str) {
        self.mode = Mode::Busy;
        self.busy = label.to_string();
        self.started = Instant::now();
    }

    fn spawn(&mut self, task: impl FnOnce() -> Work + Send + 'static) {
        let tx = self.work_tx.clone();
        let ticket = self.ticket;
        std::thread::spawn(move || {
            let _ = tx.send((ticket, task()));
        });
    }

    fn remember(&mut self, entry: &str) {
        match &self.store {
            Some(store) => store.remember(&mut self.history, entry),
            None => {
                let entry = entry.trim().to_string();
                self.history.retain(|existing| *existing != entry);
                self.history.insert(0, entry);
            }
        }
        self.recall = None;
    }

    pub fn submit(&mut self) {
        let query = self.query.trim().to_string();
        if query.is_empty() {
            return;
        }
        self.remember(&query);
        self.suggestions.clear();
        self.suggest_sel = None;
        self.typed_at = None;
        if let Some(site) = sites::link_site(&query) {
            self.site = site;
        }
        match sites::classify(&query, self.site, self.scope) {
            Entry::Video(video) => {
                if let Some(site) = Site::named(video.site) {
                    self.site = site;
                }
                self.start_resolve(video, Mode::Query)
            }
            Entry::Profile(site, url) => {
                self.site = site;
                self.open_profile(site, url, "featured", Scope::Videos, 1, Mode::Query)
            }
            Entry::Search(text, scope) => {
                self.scope = scope;
                self.start_search(text)
            }
        }
    }

    pub fn start_search(&mut self, query: String) {
        self.ticket += 1;
        self.query = query.clone();
        self.caret = self.query.chars().count();
        self.results.clear();
        self.cursor = None;
        self.sel = 0;
        self.rscroll = 0;
        self.loading_more = false;
        self.back = Mode::Query;
        self.busy("searching");
        let (site, scope) = (self.site, self.scope);
        self.spawn(move || Work::First(site.search(&query, scope)));
    }

    pub fn start_resolve(&mut self, video: Video, back: Mode) {
        self.ticket += 1;
        self.loading_more = false;
        self.back = back;
        self.formats_from = back;
        self.busy("resolving streams");
        self.spawn(move || {
            let result = sites::resolve(&video);
            Work::Resolved(video, result)
        });
    }

    pub fn open_profile(
        &mut self,
        site: Site,
        url: String,
        sort: &str,
        scope: Scope,
        page: u32,
        back: Mode,
    ) {
        self.ticket += 1;
        self.profile_site = site;
        self.loading_more = false;
        self.back = back;
        if back != Mode::Profile {
            self.profile_from = back;
        }
        self.busy("loading profile");
        let sort = sort.to_string();
        self.spawn(move || Work::Profiled(site.profile(&url, &sort, scope, page)));
    }

    fn open_author(&mut self, video: Option<Video>, back: Mode) {
        let Some(video) = video else {
            return;
        };
        match (video.author_url.clone(), Site::named(video.site)) {
            (Some(url), Some(site)) => {
                self.open_profile(site, url, "featured", Scope::Videos, 1, back)
            }
            _ => self.tell("no profile for this one"),
        }
    }

    fn poll_work(&mut self) {
        while let Ok((ticket, work)) = self.work_rx.try_recv() {
            match work {
                Work::Clipboard(found) => {
                    self.clip = found;
                    continue;
                }
                Work::Suggested(site, scope, query, rows) => {
                    if site == self.site
                        && scope == self.scope
                        && query == self.query.trim()
                        && self.mode == Mode::Query
                    {
                        self.suggestions = rows.into_iter().take(SUGGEST_MAX).collect();
                        self.suggest_sel = None;
                    }
                    continue;
                }
                other => {
                    if ticket != self.ticket {
                        continue;
                    }
                    self.apply(other);
                }
            }
        }
    }

    fn apply(&mut self, work: Work) {
        match work {
            Work::First(result) => self.apply_search(result),
            Work::More(result) => self.apply_more(result),
            Work::Resolved(video, result) => self.apply_resolve(video, result),
            Work::Profiled(result) => self.apply_profile(result),
            Work::Suggested(..) | Work::Clipboard(_) => {}
        }
    }

    fn apply_search(&mut self, result: Result<Page, String>) {
        match result {
            Ok(page) if page.videos.is_empty() => self.fail("no results", Mode::Query),
            Ok(page) => {
                self.results = page.videos;
                self.cursor = page.cursor;
                self.sel = 0;
                self.rscroll = 0;
                self.mode = Mode::Results;
            }
            Err(error) => self.fail(error, Mode::Query),
        }
    }

    fn apply_more(&mut self, result: Result<Page, String>) {
        self.loading_more = false;
        match result {
            Ok(page) => {
                page.extend(&mut self.results);
                self.cursor = page.cursor;
            }
            Err(error) => {
                self.cursor = None;
                self.tell(format!("could not load more: {error}"));
            }
        }
    }

    fn apply_resolve(&mut self, video: Video, result: Result<Sources, String>) {
        let back = self.back;
        match result {
            Ok(sources) if sources.streams.is_empty() => self.fail("no downloadable streams", back),
            Ok(sources) => {
                self.provider = sources.default_provider();
                let mut video = video;
                if let Some(detail) = &sources.detail {
                    enrich(&mut video, detail);
                }
                self.current = Some(video);
                self.sources = Some(sources);
                self.choice = 0;
                self.cscroll = 0;
                self.mode = Mode::Formats;
            }
            Err(error) => self.fail(error, back),
        }
    }

    fn apply_profile(&mut self, result: Result<Profile, String>) {
        let back = self.back;
        match result {
            Ok(profile) => {
                self.profile = Some(profile);
                self.psel = 0;
                self.pscroll = 0;
                self.mode = Mode::Profile;
            }
            Err(error) => self.fail(error, back),
        }
    }

    fn maybe_load_more(&mut self) {
        if self.mode != Mode::Results || self.loading_more {
            return;
        }
        let Some(cursor) = self.cursor.clone() else {
            return;
        };
        if self.sel + 3 < self.results.len() {
            return;
        }
        self.loading_more = true;
        self.spawn(move || Work::More(sites::more(&cursor)));
    }

    pub fn providers(&self) -> Vec<&'static str> {
        self.sources
            .as_ref()
            .map(Sources::providers)
            .unwrap_or_default()
    }

    pub fn visible(&self) -> Vec<&Stream> {
        let Some(sources) = &self.sources else {
            return Vec::new();
        };
        let mut streams: Vec<&Stream> = sources
            .streams
            .iter()
            .filter(|stream| self.provider.is_none_or(|name| stream.provider == name))
            .collect();
        streams.sort_by_key(|stream| stream.is_audio());
        streams
    }

    fn cycle_provider(&mut self) {
        let providers = self.providers();
        if providers.len() < 2 {
            return;
        }
        let at = self
            .provider
            .and_then(|active| providers.iter().position(|name| *name == active))
            .unwrap_or(0);
        self.provider = Some(providers[(at + 1) % providers.len()]);
        self.choice = 0;
        self.cscroll = 0;
    }

    fn pick(&mut self, wanted: Option<Player>) -> Option<Player> {
        match wanted {
            Some(player) if self.players.contains(&player) => {
                self.player = Some(player);
                Some(player)
            }
            Some(player) => {
                self.tell(format!("{} is not installed", player.name()));
                None
            }
            None => {
                if self.player.is_none() {
                    self.tell("install mpv or vlc to play");
                }
                self.player
            }
        }
    }

    pub fn installed(&self, player: Player) -> bool {
        self.players.contains(&player)
    }

    fn cycle_theme(&mut self) {
        self.theme = self.theme.next();
        if let Some(store) = &self.store {
            store.save_theme(self.theme);
        }
    }

    fn reset(&mut self) {
        self.query.clear();
        self.caret = 0;
        self.recall = None;
        self.clip_taken = false;
        self.mode = Mode::Query;
    }

    fn home(&mut self) -> Flow {
        match self.mode {
            Mode::Busy | Mode::Download => self.on_key(plain(KeyCode::Esc)),
            Mode::Query => Flow::Continue,
            _ => {
                self.mode = Mode::Query;
                self.caret = self.len();
                Flow::Continue
            }
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Flow {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        if control && key.code == KeyCode::Char('c') {
            return Flow::Quit;
        }
        if control && key.code == KeyCode::Char('t') {
            self.cycle_theme();
            return Flow::Continue;
        }
        self.notice = None;
        match self.mode {
            Mode::Query => self.key_query(key),
            Mode::Results => self.key_results(key),
            Mode::Formats => self.key_formats(key),
            Mode::Profile => self.key_profile(key),
            Mode::Busy => {
                if key.code == KeyCode::Esc {
                    self.ticket += 1;
                    self.loading_more = false;
                    self.mode = if self.back == Mode::Formats && self.sources.is_some() {
                        Mode::Formats
                    } else if self.back == Mode::Profile && self.profile.is_some() {
                        Mode::Profile
                    } else if self.back == Mode::Results && !self.results.is_empty() {
                        Mode::Results
                    } else {
                        Mode::Query
                    };
                    self.tell("cancelled");
                }
                Flow::Continue
            }
            Mode::Download => {
                if key.code == KeyCode::Esc
                    && !matches!(self.stage, Stage::Merging | Stage::Extracting)
                {
                    if let Some(cancel) = &self.cancel {
                        cancel.store(true, Ordering::Relaxed);
                    }
                    self.cancelling = true;
                }
                Flow::Continue
            }
            Mode::Done => self.key_done(key),
            Mode::Error => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q')) {
                    self.mode = match self.back {
                        Mode::Results if self.results.is_empty() => Mode::Query,
                        Mode::Profile if self.profile.is_none() => Mode::Query,
                        Mode::Formats if self.sources.is_none() => Mode::Results,
                        other => other,
                    };
                }
                Flow::Continue
            }
        }
    }

    pub fn on_paste(&mut self, text: String) -> Flow {
        if self.mode != Mode::Query {
            return Flow::Continue;
        }
        let cleaned: String = text
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .filter(|character| !character.is_control())
            .collect();
        if cleaned.is_empty() {
            return Flow::Continue;
        }
        let at = self.byte_at(self.caret);
        self.query.insert_str(at, &cleaned);
        self.caret += cleaned.chars().count();
        self.clip_taken = true;
        if clip::link(&self.query).is_some() {
            self.submit();
        } else {
            self.edited();
        }
        Flow::Continue
    }

    pub fn on_mouse(&mut self, event: MouseEvent) -> Flow {
        match event.kind {
            MouseEventKind::ScrollUp => self.scroll(-1),
            MouseEventKind::ScrollDown => self.scroll(1),
            MouseEventKind::Down(MouseButton::Left) => {
                let hit = self
                    .zones
                    .iter()
                    .find(|zone| {
                        event.column >= zone.rect.x
                            && event.column < zone.rect.x + zone.rect.width
                            && event.row >= zone.rect.y
                            && event.row < zone.rect.y + zone.rect.height
                    })
                    .map(|zone| zone.hit);
                match hit {
                    Some(Hit::Key(code, modifiers)) => {
                        return self.on_key(KeyEvent::new(code, modifiers));
                    }
                    Some(Hit::Result(index)) if self.mode == Mode::Results => {
                        self.sel = index.min(self.results.len().saturating_sub(1));
                        return self.on_key(plain(KeyCode::Enter));
                    }
                    Some(Hit::Result(index)) if self.mode == Mode::Profile => {
                        self.psel = index.min(self.profile_videos().saturating_sub(1));
                        return self.on_key(plain(KeyCode::Enter));
                    }
                    Some(Hit::Suggest(index)) if self.mode == Mode::Query => {
                        self.take_suggestion(index);
                    }
                    Some(Hit::Choice(index)) if self.mode == Mode::Formats => {
                        self.choice = index.min(self.visible().len().saturating_sub(1));
                        return self.on_key(plain(KeyCode::Enter));
                    }
                    Some(Hit::Logo) => return self.home(),
                    _ => {}
                }
            }
            _ => {}
        }
        Flow::Continue
    }

    fn scroll(&mut self, delta: isize) {
        match self.mode {
            Mode::Results => self.move_result(delta),
            Mode::Formats => self.move_choice(delta),
            Mode::Profile => self.move_profile(delta),
            Mode::Query if self.dropdown() => self.move_suggestion(delta),
            _ => {}
        }
    }

    fn key_query(&mut self, key: KeyEvent) -> Flow {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Enter => match self.suggest_sel {
                Some(index) if self.dropdown() => self.take_suggestion(index),
                _ => self.submit(),
            },
            KeyCode::Esc => {
                if self.dropdown() {
                    self.suggestions.clear();
                    self.suggest_sel = None;
                } else if self.results.is_empty() {
                    return Flow::Quit;
                } else {
                    self.mode = Mode::Results;
                }
            }
            KeyCode::Tab => {
                if let Some(row) = self
                    .suggest_sel
                    .filter(|_| self.dropdown())
                    .and_then(|index| self.suggestions.get(index))
                    .map(|row| row.query.clone())
                {
                    self.query = row;
                    self.caret = self.len();
                    self.suggest_for.clear();
                    self.edited();
                } else if let Some(link) = self.offered().map(str::to_string) {
                    self.query = link;
                    self.caret = self.len();
                    self.clip_taken = true;
                    self.edited();
                } else if self.dropdown() {
                    self.move_suggestion(1);
                } else if !self.results.is_empty() {
                    self.mode = Mode::Results;
                }
            }
            KeyCode::Up => {
                if self.suggest_sel.is_some() {
                    self.move_suggestion(-1);
                } else {
                    self.recall_older();
                }
            }
            KeyCode::Down => {
                if self.dropdown() && self.recall.is_none() {
                    self.move_suggestion(1);
                } else if self.recall.is_some() {
                    self.recall_newer();
                } else if !self.results.is_empty() {
                    self.mode = Mode::Results;
                }
            }
            KeyCode::Left => self.caret = self.caret.saturating_sub(1),
            KeyCode::Right => self.caret = (self.caret + 1).min(self.len()),
            KeyCode::Home => self.caret = 0,
            KeyCode::End => self.caret = self.len(),
            KeyCode::Backspace => {
                if self.caret > 0 {
                    self.remove(self.caret - 1, self.caret);
                    self.caret -= 1;
                    self.edited();
                }
            }
            KeyCode::Delete => {
                self.remove(self.caret, self.caret + 1);
                self.edited();
            }
            KeyCode::Char('a') if control => self.caret = 0,
            KeyCode::Char('e') if control => self.caret = self.len(),
            KeyCode::Char('s') if control => self.switch_site(),
            KeyCode::Char('g') if control => self.switch_scope(),
            KeyCode::Char('u') if control => {
                self.remove(0, self.caret);
                self.caret = 0;
                self.edited();
            }
            KeyCode::Char('k') if control => {
                self.remove(self.caret, self.len());
                self.edited();
            }
            KeyCode::Char('w') if control => {
                let chars: Vec<char> = self.query.chars().collect();
                let mut start = self.caret;
                while start > 0 && chars[start - 1].is_whitespace() {
                    start -= 1;
                }
                while start > 0 && !chars[start - 1].is_whitespace() {
                    start -= 1;
                }
                self.remove(start, self.caret);
                self.caret = start;
                self.edited();
            }
            KeyCode::Char(character) if !control => {
                let at = self.byte_at(self.caret);
                self.query.insert(at, character);
                self.caret += 1;
                self.edited();
            }
            _ => {}
        }
        Flow::Continue
    }

    fn recall_older(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let next = match self.recall {
            None => {
                self.draft = self.query.clone();
                0
            }
            Some(at) => (at + 1).min(self.history.len() - 1),
        };
        self.recall = Some(next);
        self.query = self.history[next].clone();
        self.caret = self.len();
        self.suggestions.clear();
    }

    fn recall_newer(&mut self) {
        match self.recall {
            Some(0) => {
                self.recall = None;
                self.query = std::mem::take(&mut self.draft);
            }
            Some(at) => {
                self.recall = Some(at - 1);
                self.query = self.history[at - 1].clone();
            }
            None => return,
        }
        self.caret = self.len();
    }

    fn len(&self) -> usize {
        self.query.chars().count()
    }

    fn byte_at(&self, chars: usize) -> usize {
        self.query
            .char_indices()
            .nth(chars)
            .map(|(index, _)| index)
            .unwrap_or(self.query.len())
    }

    fn remove(&mut self, from: usize, to: usize) {
        let (a, b) = (self.byte_at(from), self.byte_at(to.min(self.len())));
        if a < b {
            self.query.replace_range(a..b, "");
        }
    }

    fn key_results(&mut self, key: KeyEvent) -> Flow {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.move_result(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_result(1),
            KeyCode::PageUp => self.move_result(-PAGE),
            KeyCode::PageDown => self.move_result(PAGE),
            KeyCode::Home | KeyCode::Char('g') => self.sel = 0,
            KeyCode::End | KeyCode::Char('G') => self.sel = self.results.len().saturating_sub(1),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                if let Some(video) = self.results.get(self.sel).cloned() {
                    self.start_resolve(video, Mode::Results);
                }
            }
            KeyCode::Char('/') | KeyCode::Esc | KeyCode::Tab => {
                self.caret = self.len();
                self.mode = Mode::Query;
            }
            KeyCode::Char('o') => {
                let video = self.results.get(self.sel).cloned();
                self.open_author(video, Mode::Results);
            }
            KeyCode::Char('q') => return Flow::Quit,
            _ => {}
        }
        Flow::Continue
    }

    fn move_result(&mut self, delta: isize) {
        if self.results.is_empty() {
            return;
        }
        let last = self.results.len() as isize - 1;
        self.sel = (self.sel as isize + delta).clamp(0, last) as usize;
    }

    fn move_choice(&mut self, delta: isize) {
        let last = self.visible().len() as isize - 1;
        if last < 0 {
            return;
        }
        self.choice = (self.choice as isize + delta).clamp(0, last) as usize;
    }

    fn key_formats(&mut self, key: KeyEvent) -> Flow {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.move_choice(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_choice(1),
            KeyCode::PageUp => self.move_choice(-PAGE),
            KeyCode::PageDown => self.move_choice(PAGE),
            KeyCode::Home | KeyCode::Char('g') => self.choice = 0,
            KeyCode::End | KeyCode::Char('G') => {
                self.choice = self.visible().len().saturating_sub(1)
            }
            KeyCode::Enter | KeyCode::Char('s') | KeyCode::Char('d') => {
                return self.run_stream(true, None)
            }
            KeyCode::Char('p') => return self.run_stream(false, None),
            KeyCode::Char('m') => return self.run_stream(false, Some(Player::Mpv)),
            KeyCode::Char('v') => return self.run_stream(false, Some(Player::Vlc)),
            KeyCode::Tab => self.cycle_provider(),
            KeyCode::Char('o') => {
                let video = self.current.clone();
                self.open_author(video, Mode::Formats);
            }
            KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') => {
                self.mode = match self.formats_from {
                    Mode::Profile if self.profile.is_some() => Mode::Profile,
                    _ if self.results.is_empty() => Mode::Query,
                    _ => Mode::Results,
                };
            }
            KeyCode::Char('/') => {
                self.caret = self.len();
                self.mode = Mode::Query;
            }
            KeyCode::Char('q') => return Flow::Quit,
            _ => {}
        }
        Flow::Continue
    }

    pub fn profile_scopes(&self) -> usize {
        self.profile_site.scopes().len()
    }

    pub fn profile_videos(&self) -> usize {
        self.profile
            .as_ref()
            .map(|profile| profile.videos.len())
            .unwrap_or(0)
    }

    fn move_profile(&mut self, delta: isize) {
        let count = self.profile_videos();
        if count == 0 {
            return;
        }
        self.psel = (self.psel as isize + delta).clamp(0, count as isize - 1) as usize;
    }

    fn sort_ids(&self) -> Vec<String> {
        let Some(profile) = &self.profile else {
            return Vec::new();
        };
        if profile.sorts.is_empty() {
            ["featured", "added", "rated", "viewed", "commented"]
                .iter()
                .map(|id| (*id).to_string())
                .collect()
        } else {
            profile.sorts.iter().map(|sort| sort.id.clone()).collect()
        }
    }

    pub fn sort_label(&self) -> String {
        let Some(profile) = &self.profile else {
            return String::new();
        };
        profile
            .sorts
            .iter()
            .find(|sort| sort.id == profile.sort)
            .map(|sort| sort.label.clone())
            .unwrap_or_else(|| profile.sort.clone())
    }

    fn turn_page(&mut self, forward: bool) {
        let Some(profile) = &self.profile else {
            return;
        };
        let (page, last) = (profile.paging.page, profile.paging.last);
        let (url, sort, scope) = (profile.url.clone(), profile.sort.clone(), profile.scope);
        let site = self.profile_site;
        if forward {
            if last != 0 && page >= last || profile.videos.is_empty() {
                self.tell("last page");
                return;
            }
            self.open_profile(site, url, &sort, scope, page + 1, Mode::Profile);
        } else {
            if page <= 1 {
                self.tell("first page");
                return;
            }
            self.open_profile(site, url, &sort, scope, page - 1, Mode::Profile);
        }
    }

    fn cycle_sort(&mut self) {
        let ids = self.sort_ids();
        let Some(profile) = &self.profile else {
            return;
        };
        if ids.len() < 2 || profile.scope == Scope::Galleries {
            return;
        }
        let at = ids.iter().position(|id| *id == profile.sort).unwrap_or(0);
        let next = ids[(at + 1) % ids.len()].clone();
        let url = profile.url.clone();
        let site = self.profile_site;
        self.open_profile(site, url, &next, Scope::Videos, 1, Mode::Profile);
    }

    fn cycle_profile_scope(&mut self) {
        let Some(profile) = &self.profile else {
            return;
        };
        let next = self.profile_site.next_scope(profile.scope);
        if next == profile.scope {
            return;
        }
        let url = profile.url.clone();
        let site = self.profile_site;
        self.open_profile(site, url, "featured", next, 1, Mode::Profile);
    }

    fn leave_profile(&mut self) {
        self.mode = match self.profile_from {
            Mode::Results if !self.results.is_empty() => Mode::Results,
            Mode::Formats if self.sources.is_some() => Mode::Formats,
            _ => {
                self.caret = self.len();
                Mode::Query
            }
        };
    }

    fn key_profile(&mut self, key: KeyEvent) -> Flow {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.move_profile(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_profile(1),
            KeyCode::PageUp => self.move_profile(-PAGE),
            KeyCode::PageDown => self.move_profile(PAGE),
            KeyCode::Home | KeyCode::Char('g') => self.psel = 0,
            KeyCode::End | KeyCode::Char('G') => {
                self.psel = self.profile_videos().saturating_sub(1)
            }
            KeyCode::Enter => {
                let video = self
                    .profile
                    .as_ref()
                    .and_then(|profile| profile.videos.get(self.psel))
                    .cloned();
                if let Some(video) = video {
                    self.start_resolve(video, Mode::Profile);
                }
            }
            KeyCode::Right | KeyCode::Char('n') | KeyCode::Char(']') => self.turn_page(true),
            KeyCode::Left | KeyCode::Char('p') | KeyCode::Char('[') => self.turn_page(false),
            KeyCode::Char('s') => self.cycle_sort(),
            KeyCode::Char('t') => self.cycle_profile_scope(),
            KeyCode::Char('/') => {
                self.caret = self.len();
                self.mode = Mode::Query;
            }
            KeyCode::Esc | KeyCode::Char('h') => self.leave_profile(),
            KeyCode::Char('q') => return Flow::Quit,
            _ => {}
        }
        Flow::Continue
    }

    fn key_done(&mut self, key: KeyEvent) -> Flow {
        match key.code {
            KeyCode::Enter => self.reset(),
            KeyCode::Char('p') => return self.play_saved(None),
            KeyCode::Char('m') => return self.play_saved(Some(Player::Mpv)),
            KeyCode::Char('v') => return self.play_saved(Some(Player::Vlc)),
            KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') => {
                self.mode = Mode::Formats;
            }
            KeyCode::Char('q') => return Flow::Quit,
            _ => {}
        }
        Flow::Continue
    }

    fn play_saved(&mut self, wanted: Option<Player>) -> Flow {
        let Some(path) = self.saved.clone() else {
            return Flow::Continue;
        };
        if self.saved_gallery {
            self.tell("galleries are folders of photos, open them in your file manager");
            return Flow::Continue;
        }
        match self.pick(wanted) {
            Some(player) => Flow::PlayFile {
                path,
                video: self.saved_video,
                player,
            },
            None => Flow::Continue,
        }
    }

    fn run_stream(&mut self, save: bool, wanted: Option<Player>) -> Flow {
        let Some(sources) = self.sources.clone() else {
            return Flow::Continue;
        };
        let Some(stream) = self
            .visible()
            .get(self.choice)
            .map(|stream| (*stream).clone())
        else {
            return Flow::Continue;
        };
        let title = self.title();
        let id = self
            .current
            .as_ref()
            .map(|video| video.id.clone())
            .unwrap_or_default();
        let pair = sources.audio_for(&stream).map(Stream::remote);
        if stream.is_gallery() {
            if !save {
                self.tell("galleries are saved, not played");
                return Flow::Continue;
            }
            let job = Job::Gallery {
                photos: sources.photos.clone(),
                referer: stream.referer.map(str::to_string),
                title,
                id,
            };
            self.picked = stream.quality.clone();
            self.start_job(job, false);
            return Flow::Continue;
        }
        if save {
            let job = match stream.kind {
                Kind::Audio => Job::Audio {
                    audio: stream.remote(),
                    title,
                    id,
                },
                Kind::Video | Kind::Muxed => Job::Video {
                    video: stream.remote(),
                    audio: pair,
                    title,
                    id,
                },
                Kind::Gallery => return Flow::Continue,
            };
            self.picked = if stream.is_audio() {
                format!("audio {} {}", stream.quality, stream.ext)
            } else {
                format!("{} {}", stream.quality, stream.ext)
            };
            self.start_job(job, !stream.is_audio());
            return Flow::Continue;
        }
        let Some(player) = self.pick(wanted) else {
            return Flow::Continue;
        };
        match stream.kind {
            Kind::Audio => Flow::Play {
                video: None,
                audio: Some(stream.remote()),
                title,
                player,
            },
            Kind::Muxed | Kind::Video | Kind::Gallery => Flow::Play {
                video: Some(stream.remote()),
                audio: pair,
                title,
                player,
            },
        }
    }

    fn site_folder(&self) -> &'static str {
        self.current
            .as_ref()
            .and_then(|video| Site::named(video.site))
            .unwrap_or(self.site)
            .folder()
    }

    fn start_job(&mut self, job: Job, video: bool) {
        let job_folder = job.folder();
        self.steps = match &job {
            Job::Video {
                video: clip,
                audio: Some(audio),
                ..
            } if crate::mux::pairs(&clip.ext, &audio.ext) => {
                vec![Stage::Fetching, Stage::FetchingAudio, Stage::Merging]
            }
            Job::Audio { audio, .. } if audio.extract => {
                vec![Stage::Fetching, Stage::Extracting]
            }
            _ => vec![Stage::Fetching],
        };
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.dl_rx = Some(rx);
        self.cancel = Some(cancel.clone());
        self.progress = 0;
        self.total = 0;
        self.stage = Stage::Fetching;
        self.cancelling = false;
        self.started = Instant::now();
        self.phase_started = Instant::now();
        self.saved_video = video;
        self.saved_gallery = matches!(job_folder, Folder::Galleries);
        self.mode = Mode::Download;
        let dir = self.layout.dir(self.site_folder(), job_folder);
        std::thread::spawn(move || download::run(job, dir, tx, cancel));
    }

    fn poll_download(&mut self) {
        let Some(rx) = self.dl_rx.take() else {
            return;
        };
        let mut keep = true;
        loop {
            match rx.try_recv() {
                Ok(Msg::Total(total)) => self.total = total,
                Ok(Msg::Progress(done)) => self.progress = self.progress.max(done),
                Ok(Msg::Stage(stage)) => {
                    self.stage = stage;
                    self.progress = 0;
                    self.total = 0;
                    self.phase_started = Instant::now();
                }
                Ok(Msg::Note(note)) => self.tell(note),
                Ok(Msg::Done(path)) => {
                    self.saved = Some(path);
                    self.mode = Mode::Done;
                    keep = false;
                    break;
                }
                Ok(Msg::Failed(error)) => {
                    self.fail(error, Mode::Formats);
                    keep = false;
                    break;
                }
                Ok(Msg::Cancelled) => {
                    self.mode = Mode::Formats;
                    self.tell("download cancelled");
                    keep = false;
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if self.mode == Mode::Download {
                        self.fail("download stopped unexpectedly", Mode::Formats);
                    }
                    keep = false;
                    break;
                }
            }
        }
        if keep {
            self.dl_rx = Some(rx);
        }
    }
}

fn enrich(video: &mut Video, detail: &Video) {
    let fill = |slot: &mut String, value: &str| {
        if slot.is_empty() {
            *slot = value.to_string();
        }
    };
    fill(&mut video.title, &detail.title);
    fill(&mut video.channel, &detail.channel);
    fill(&mut video.duration, &detail.duration);
    fill(&mut video.views, &detail.views);
    fill(&mut video.published, &detail.published);
    if video.author_url.is_none() {
        video.author_url = detail.author_url.clone();
    }
    if video.url.is_none() {
        video.url = detail.url.clone();
    }
}

fn plain(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[cfg(test)]
mod tests {
    use super::super::fixture::{app as sample, profiled, suggested};
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState};

    fn press(app: &mut App, code: KeyCode) -> Flow {
        app.on_key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    fn control(app: &mut App, letter: char) -> Flow {
        app.on_key(KeyEvent {
            code: KeyCode::Char(letter),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    fn type_text(app: &mut App, text: &str) {
        for character in text.chars() {
            press(app, KeyCode::Char(character));
        }
    }

    fn click(app: &mut App, column: u16, row: u16) -> Flow {
        app.on_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn empty_query(app: &mut App) {
        app.mode = Mode::Query;
        app.query.clear();
        app.caret = 0;
    }

    #[test]
    fn suggestions_move_fill_and_submit() {
        let mut app = suggested();
        assert!(app.dropdown());
        press(&mut app, KeyCode::Down);
        assert_eq!(app.suggest_sel, Some(0));
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.suggest_sel, Some(1));
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Up);
        assert_eq!(app.suggest_sel, None);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.query, "sam smith");
        assert_eq!(app.caret, 9);
        assert_eq!(app.mode, Mode::Query);
    }

    #[test]
    fn escape_closes_the_dropdown_before_leaving() {
        let mut app = suggested();
        app.results.clear();
        assert!(matches!(press(&mut app, KeyCode::Esc), Flow::Continue));
        assert!(!app.dropdown());
        assert!(matches!(press(&mut app, KeyCode::Esc), Flow::Quit));
    }

    #[test]
    fn choosing_a_person_opens_their_profile_and_a_search_runs() {
        let mut app = suggested();
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.mode, Mode::Busy);
        assert_eq!(app.busy, "loading profile");
        assert!(app.suggestions.is_empty());

        let mut app = suggested();
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.mode, Mode::Busy);
        assert_eq!(app.busy, "searching");
        assert_eq!(app.query, "sam smith");
    }

    #[test]
    fn editing_clears_the_selection_and_links_never_suggest() {
        let mut app = suggested();
        press(&mut app, KeyCode::Down);
        type_text(&mut app, "x");
        assert_eq!(app.suggest_sel, None);
        empty_query(&mut app);
        app.suggestions = suggested().suggestions;
        type_text(&mut app, "https://xhamster.com/search/a");
        assert!(app.suggestions.is_empty());
    }

    #[test]
    fn control_s_switches_site_and_drops_old_results() {
        let mut app = sample();
        app.mode = Mode::Query;
        assert_eq!(app.site, Site::first());
        control(&mut app, 's');
        assert_eq!(app.site, Site::named("xhamster").unwrap());
        assert!(app.results.is_empty());
        control(&mut app, 's');
        assert_eq!(app.site, Site::first());
    }

    #[test]
    fn pasted_site_links_pick_the_site_and_route() {
        let mut app = sample();
        empty_query(&mut app);
        app.on_paste("https://xhamster.com/creators/sample".to_string());
        assert_eq!(app.site, Site::named("xhamster").unwrap());
        assert_eq!(app.mode, Mode::Busy);
        assert_eq!(app.busy, "loading profile");
    }

    #[test]
    fn profile_navigation_pages_sorts_and_goes_back() {
        let mut app = profiled();
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.psel, 2);
        press(&mut app, KeyCode::End);
        assert_eq!(app.psel, 5);
        press(&mut app, KeyCode::Left);
        assert_eq!(app.mode, Mode::Profile);
        assert_eq!(app.notice.as_ref().unwrap().0, "first page");
        press(&mut app, KeyCode::Char('s'));
        assert_eq!(app.mode, Mode::Busy);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Profile);

        let mut app = profiled();
        app.profile.as_mut().unwrap().paging.page = 3;
        press(&mut app, KeyCode::Right);
        assert_eq!(app.notice.as_ref().unwrap().0, "last page");
        app.profile.as_mut().unwrap().paging.page = 1;
        press(&mut app, KeyCode::Right);
        assert_eq!(app.mode, Mode::Busy);
        assert_eq!(app.busy, "loading profile");
    }

    #[test]
    fn opening_a_profile_video_returns_to_the_profile() {
        let mut app = profiled();
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.mode, Mode::Busy);
        assert_eq!(app.back, Mode::Profile);
        app.apply_resolve(
            app.profile.as_ref().unwrap().videos[0].clone(),
            Ok(sample().sources.clone().unwrap()),
        );
        assert_eq!(app.mode, Mode::Formats);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Profile);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Query);
    }

    #[test]
    fn profile_failures_fall_back_without_a_profile() {
        let mut app = sample();
        app.profile = None;
        app.back = Mode::Query;
        app.apply_profile(Err("boom".to_string()));
        assert_eq!(app.mode, Mode::Error);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.mode, Mode::Query);
    }

    #[test]
    fn detail_fills_gaps_but_never_overwrites() {
        let mut video = Video::stub("a");
        video.title = "mine".to_string();
        let mut detail = Video::stub("a");
        detail.title = "theirs".to_string();
        detail.channel = "chan".to_string();
        detail.author_url = Some("https://x/c".to_string());
        enrich(&mut video, &detail);
        assert_eq!(video.title, "mine");
        assert_eq!(video.channel, "chan");
        assert_eq!(video.author_url.as_deref(), Some("https://x/c"));
    }

    #[test]
    fn edits_the_query_at_the_caret() {
        let mut app = sample();
        empty_query(&mut app);
        type_text(&mut app, "héllo wörld");
        assert_eq!(app.query, "héllo wörld");
        press(&mut app, KeyCode::Left);
        press(&mut app, KeyCode::Left);
        press(&mut app, KeyCode::Backspace);
        assert_eq!(app.query, "héllo wöld");
        press(&mut app, KeyCode::Delete);
        assert_eq!(app.query, "héllo wöd");
        press(&mut app, KeyCode::Home);
        press(&mut app, KeyCode::Char('X'));
        assert_eq!(app.query, "Xhéllo wöd");
        press(&mut app, KeyCode::End);
        control(&mut app, 'w');
        assert_eq!(app.query, "Xhéllo ");
        control(&mut app, 'u');
        assert_eq!(app.query, "");
        assert_eq!(app.caret, 0);
        press(&mut app, KeyCode::Backspace);
        press(&mut app, KeyCode::Delete);
        assert_eq!(app.query, "");
    }

    #[test]
    fn escape_returns_to_results_instead_of_quitting() {
        let mut app = sample();
        app.mode = Mode::Query;
        assert!(matches!(press(&mut app, KeyCode::Esc), Flow::Continue));
        assert_eq!(app.mode, Mode::Results);
        app.results.clear();
        app.mode = Mode::Query;
        assert!(matches!(press(&mut app, KeyCode::Esc), Flow::Quit));
    }

    #[test]
    fn results_navigation_stays_in_bounds() {
        let mut app = sample();
        app.mode = Mode::Results;
        press(&mut app, KeyCode::Up);
        assert_eq!(app.sel, 0);
        press(&mut app, KeyCode::End);
        assert_eq!(app.sel, 8);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.sel, 8);
        press(&mut app, KeyCode::PageUp);
        assert_eq!(app.sel, 0);
        press(&mut app, KeyCode::Char('/'));
        assert_eq!(app.mode, Mode::Query);
    }

    #[test]
    fn question_mark_is_just_a_character() {
        let mut app = sample();
        empty_query(&mut app);
        press(&mut app, KeyCode::Char('?'));
        assert_eq!(app.query, "?");
    }

    #[test]
    fn errors_go_back_to_where_they_came_from() {
        let mut app = sample();
        app.fail("nope", Mode::Formats);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Formats);
        app.sources = None;
        app.fail("nope", Mode::Formats);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Results);
        app.results.clear();
        app.fail("nope", Mode::Results);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Query);
    }

    #[test]
    fn videos_come_first_then_audio_and_tab_switches_provider() {
        let mut app = sample();
        app.mode = Mode::Formats;
        let order: Vec<_> = app
            .visible()
            .iter()
            .map(|stream| stream.quality.clone())
            .collect();
        assert_eq!(order, vec!["720p", "360p", "128k"]);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.provider, Some(crate::sites::youtube::YTULTRA));
        assert_eq!(app.visible().len(), 1);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.visible().len(), 3);
    }

    #[test]
    fn playing_pairs_video_with_audio_but_not_muxed() {
        let mut app = sample();
        app.mode = Mode::Formats;
        app.choice = 0;
        match press(&mut app, KeyCode::Char('p')) {
            Flow::Play { video, audio, .. } => {
                assert_eq!(video.unwrap().ext, "mp4");
                assert_eq!(audio.unwrap().ext, "m4a");
            }
            _ => panic!("expected playback"),
        }
        app.choice = 1;
        match press(&mut app, KeyCode::Char('p')) {
            Flow::Play { video, audio, .. } => {
                assert!(video.is_some());
                assert!(audio.is_none());
            }
            _ => panic!("expected playback"),
        }
        app.choice = 2;
        match press(&mut app, KeyCode::Char('p')) {
            Flow::Play { video, audio, .. } => {
                assert!(video.is_none());
                assert!(audio.is_some());
            }
            _ => panic!("expected playback"),
        }
        app.player = None;
        assert!(matches!(
            press(&mut app, KeyCode::Char('p')),
            Flow::Continue
        ));
        assert!(app.notice.is_some());
    }

    #[test]
    fn mpv_and_vlc_can_each_be_chosen_directly() {
        let mut app = sample();
        app.players = vec![Player::Mpv, Player::Vlc];
        app.mode = Mode::Formats;
        app.choice = 0;
        match press(&mut app, KeyCode::Char('v')) {
            Flow::Play { player, .. } => assert_eq!(player, Player::Vlc),
            _ => panic!("expected playback"),
        }
        assert_eq!(app.player, Some(Player::Vlc));
        match press(&mut app, KeyCode::Char('m')) {
            Flow::Play { player, .. } => assert_eq!(player, Player::Mpv),
            _ => panic!("expected playback"),
        }
        match press(&mut app, KeyCode::Char('p')) {
            Flow::Play { player, .. } => assert_eq!(player, Player::Mpv),
            _ => panic!("expected playback"),
        }
        app.players = vec![Player::Mpv];
        assert!(matches!(
            press(&mut app, KeyCode::Char('v')),
            Flow::Continue
        ));
        assert_eq!(app.notice.as_ref().unwrap().0, "vlc is not installed");
        app.mode = Mode::Done;
        app.saved = Some(PathBuf::from("/tmp/x.mp4"));
        app.players = vec![Player::Mpv, Player::Vlc];
        match press(&mut app, KeyCode::Char('v')) {
            Flow::PlayFile { player, .. } => assert_eq!(player, Player::Vlc),
            _ => panic!("expected playback"),
        }
    }

    #[test]
    fn enter_downloads_and_plans_the_merge_steps() {
        let dir = std::env::temp_dir().join(format!("nab-ui-{}", std::process::id()));
        let mut app = sample();
        app.layout = Layout::new(dir.clone());
        app.mode = Mode::Formats;
        app.choice = 0;
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.mode, Mode::Download);
        assert_eq!(app.picked, "720p mp4");
        assert_eq!(
            app.steps,
            vec![Stage::Fetching, Stage::FetchingAudio, Stage::Merging]
        );
        press(&mut app, KeyCode::Esc);
        assert!(app.cancelling);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn done_nabs_another_or_plays_the_file() {
        let mut app = sample();
        app.mode = Mode::Done;
        app.saved = Some(PathBuf::from("/tmp/x.mp4"));
        app.saved_video = true;
        match press(&mut app, KeyCode::Char('p')) {
            Flow::PlayFile { path, video, .. } => {
                assert_eq!(path, PathBuf::from("/tmp/x.mp4"));
                assert!(video);
            }
            _ => panic!("expected playback"),
        }
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Formats);
        app.mode = Mode::Done;
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.mode, Mode::Query);
        assert_eq!(app.query, "");
    }

    #[test]
    fn ctrl_c_quits_from_anywhere_and_ctrl_t_cycles_the_theme() {
        let mut app = sample();
        for mode in [
            Mode::Query,
            Mode::Results,
            Mode::Formats,
            Mode::Busy,
            Mode::Download,
            Mode::Done,
            Mode::Error,
        ] {
            app.mode = mode;
            assert!(matches!(control(&mut app, 'c'), Flow::Quit));
        }
        assert_eq!(app.theme, ThemeMode::Auto);
        control(&mut app, 't');
        assert_eq!(app.theme, ThemeMode::Light);
        control(&mut app, 't');
        control(&mut app, 't');
        assert_eq!(app.theme, ThemeMode::Auto);
    }

    #[test]
    fn history_recall_walks_back_and_restores_the_draft() {
        let mut app = sample();
        app.history = vec!["newest".into(), "older".into()];
        empty_query(&mut app);
        type_text(&mut app, "draft");
        press(&mut app, KeyCode::Up);
        assert_eq!(app.query, "newest");
        press(&mut app, KeyCode::Up);
        assert_eq!(app.query, "older");
        press(&mut app, KeyCode::Up);
        assert_eq!(app.query, "older");
        press(&mut app, KeyCode::Down);
        assert_eq!(app.query, "newest");
        press(&mut app, KeyCode::Down);
        assert_eq!(app.query, "draft");
        assert_eq!(app.caret, 5);
    }

    #[test]
    fn submitting_remembers_the_query() {
        let mut app = sample();
        empty_query(&mut app);
        type_text(&mut app, "lofi");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.mode, Mode::Busy);
        assert_eq!(app.history, vec!["lofi"]);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Query);
    }

    #[test]
    fn clipboard_link_is_offered_on_an_empty_field_and_tab_takes_it() {
        let mut app = sample();
        empty_query(&mut app);
        app.clip = Some("https://youtu.be/dQw4w9WgXcQ".to_string());
        assert!(app.offered().is_some());
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.query, "https://youtu.be/dQw4w9WgXcQ");
        assert!(app.offered().is_none());
    }

    #[test]
    fn pasting_a_video_link_starts_resolving_and_text_just_lands() {
        let mut app = sample();
        empty_query(&mut app);
        app.on_paste("lofi\nsecond line".to_string());
        assert_eq!(app.query, "lofi");
        assert_eq!(app.mode, Mode::Query);
        empty_query(&mut app);
        app.on_paste("https://youtu.be/dQw4w9WgXcQ".to_string());
        assert_eq!(app.mode, Mode::Busy);
        assert_eq!(app.busy, "resolving streams");
    }

    #[test]
    fn clicks_follow_the_recorded_zones() {
        let mut app = sample();
        app.mode = Mode::Results;
        app.zones = vec![
            Zone {
                rect: Rect::new(2, 5, 30, 2),
                hit: Hit::Result(3),
            },
            Zone {
                rect: Rect::new(2, 8, 10, 1),
                hit: Hit::Key(KeyCode::Char('/'), KeyModifiers::NONE),
            },
            Zone {
                rect: Rect::new(0, 0, 10, 3),
                hit: Hit::Logo,
            },
        ];
        click(&mut app, 40, 5);
        assert_eq!(app.mode, Mode::Results);
        click(&mut app, 3, 8);
        assert_eq!(app.mode, Mode::Query);
        app.mode = Mode::Results;
        click(&mut app, 3, 6);
        assert_eq!(app.sel, 3);
        assert_eq!(app.mode, Mode::Busy);
        app.mode = Mode::Formats;
        click(&mut app, 1, 1);
        assert_eq!(app.mode, Mode::Query);
    }

    #[test]
    fn scroll_wheel_moves_the_selection() {
        let mut app = sample();
        app.mode = Mode::Results;
        let wheel = |app: &mut App, kind| {
            app.on_mouse(MouseEvent {
                kind,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            })
        };
        wheel(&mut app, MouseEventKind::ScrollDown);
        wheel(&mut app, MouseEventKind::ScrollDown);
        assert_eq!(app.sel, 2);
        wheel(&mut app, MouseEventKind::ScrollUp);
        assert_eq!(app.sel, 1);
        app.mode = Mode::Formats;
        wheel(&mut app, MouseEventKind::ScrollDown);
        assert_eq!(app.choice, 1);
    }

    #[test]
    fn ctrl_g_switches_scope_only_where_a_site_has_galleries() {
        let mut app = sample();
        app.mode = Mode::Query;
        control(&mut app, 'g');
        assert_eq!(app.scope, Scope::Videos);
        control(&mut app, 's');
        assert_eq!(app.site, Site::named("xhamster").unwrap());
        control(&mut app, 'g');
        assert_eq!(app.scope, Scope::Galleries);
        control(&mut app, 'g');
        assert_eq!(app.scope, Scope::Videos);
        control(&mut app, 'g');
        control(&mut app, 's');
        assert_eq!(app.site, Site::first());
        assert_eq!(app.scope, Scope::Videos);
    }
}
