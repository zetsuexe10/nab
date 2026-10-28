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
use crate::play::{self, Player};
use crate::search::{self, Cursor, Page, Video};
use crate::source::{self, Kind, Sources, Stream};

const NOTICE_FOR: Duration = Duration::from_secs(5);
const PAGE: isize = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Query,
    Results,
    Formats,
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
    Clipboard(Option<String>),
}

pub struct App {
    pub query: String,
    pub caret: usize,
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
    pub error: String,
    pub notice: Option<(String, Instant)>,
    pub out_dir: PathBuf,
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
    pub fn new(query: String, out_dir: PathBuf, player: Option<Player>) -> Self {
        let (work_tx, work_rx) = mpsc::channel();
        let players: Vec<Player> = [Player::Mpv, Player::Vlc]
            .into_iter()
            .filter(|candidate| play::locate(*candidate).is_some())
            .collect();
        let caret = query.chars().count();
        App {
            query,
            caret,
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
            error: String::new(),
            notice: None,
            out_dir,
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
        if let Some(id) = search::video_id(&query) {
            self.start_resolve(Video::stub(&id), Mode::Query);
        } else {
            self.start_search(query);
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
        self.spawn(move || Work::First(search::search(&query)));
    }

    pub fn start_resolve(&mut self, video: Video, back: Mode) {
        self.ticket += 1;
        self.loading_more = false;
        self.back = back;
        self.busy("resolving streams");
        let id = video.id.clone();
        self.spawn(move || Work::Resolved(video, source::resolve(&id)));
    }

    fn poll_work(&mut self) {
        while let Ok((ticket, work)) = self.work_rx.try_recv() {
            if let Work::Clipboard(found) = work {
                self.clip = found;
                continue;
            }
            if ticket != self.ticket {
                continue;
            }
            match work {
                Work::First(result) => self.apply_search(result),
                Work::More(result) => self.apply_more(result),
                Work::Resolved(video, result) => self.apply_resolve(video, result),
                Work::Clipboard(_) => {}
            }
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
                let providers = sources.providers();
                self.provider = providers
                    .iter()
                    .copied()
                    .find(|name| *name == source::INNERTUBE)
                    .or(providers.first().copied());
                self.current = Some(video);
                self.sources = Some(sources);
                self.choice = 0;
                self.cscroll = 0;
                self.mode = Mode::Formats;
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
        self.spawn(move || Work::More(search::more(&cursor)));
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
            Mode::Busy => {
                if key.code == KeyCode::Esc {
                    self.ticket += 1;
                    self.loading_more = false;
                    self.mode = if self.back == Mode::Formats && self.sources.is_some() {
                        Mode::Formats
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
                if key.code == KeyCode::Esc && self.stage != Stage::Merging {
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
            _ => {}
        }
    }

    fn key_query(&mut self, key: KeyEvent) -> Flow {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Enter => self.submit(),
            KeyCode::Esc => {
                if self.results.is_empty() {
                    return Flow::Quit;
                }
                self.mode = Mode::Results;
            }
            KeyCode::Tab => {
                if let Some(link) = self.offered().map(str::to_string) {
                    self.query = link;
                    self.caret = self.len();
                    self.clip_taken = true;
                } else if !self.results.is_empty() {
                    self.mode = Mode::Results;
                }
            }
            KeyCode::Up => self.recall_older(),
            KeyCode::Down => {
                if self.recall.is_some() {
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
                }
            }
            KeyCode::Delete => self.remove(self.caret, self.caret + 1),
            KeyCode::Char('a') if control => self.caret = 0,
            KeyCode::Char('e') if control => self.caret = self.len(),
            KeyCode::Char('u') if control => {
                self.remove(0, self.caret);
                self.caret = 0;
            }
            KeyCode::Char('k') if control => self.remove(self.caret, self.len()),
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
            }
            KeyCode::Char(character) if !control => {
                let at = self.byte_at(self.caret);
                self.query.insert(at, character);
                self.caret += 1;
                self.recall = None;
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
            KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') => {
                self.mode = if self.results.is_empty() {
                    Mode::Query
                } else {
                    Mode::Results
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
        match self.pick(wanted) {
            Some(player) => Flow::PlayFile {
                path,
                video: self.saved_video,
                player,
            },
            None => Flow::Continue,
        }
    }

    fn remote(stream: &Stream) -> Remote {
        Remote {
            url: stream.url.clone(),
            ext: stream.ext.clone(),
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
        let pair = sources.audio_for(&stream).map(Self::remote);
        if save {
            let job = match stream.kind {
                Kind::Audio => Job::Audio {
                    audio: Self::remote(&stream),
                    title,
                    id,
                },
                Kind::Video | Kind::Muxed => Job::Video {
                    video: Self::remote(&stream),
                    audio: pair,
                    title,
                    id,
                },
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
                audio: Some(Self::remote(&stream)),
                title,
                player,
            },
            Kind::Muxed | Kind::Video => Flow::Play {
                video: Some(Self::remote(&stream)),
                audio: pair,
                title,
                player,
            },
        }
    }

    fn start_job(&mut self, job: Job, video: bool) {
        self.steps = match &job {
            Job::Video {
                video: clip,
                audio: Some(audio),
                ..
            } if crate::mux::pairs(&clip.ext, &audio.ext) => {
                vec![Stage::Fetching, Stage::FetchingAudio, Stage::Merging]
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
        self.mode = Mode::Download;
        let dir = self.out_dir.clone();
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

fn plain(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[cfg(test)]
mod tests {
    use super::super::fixture::app as sample;
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
        assert_eq!(app.provider, Some(source::YTULTRA));
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
        let dir = std::env::temp_dir().join(format!("snag-ui-{}", std::process::id()));
        let mut app = sample();
        app.out_dir = dir.clone();
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
    fn done_snags_another_or_plays_the_file() {
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
}
