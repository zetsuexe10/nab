use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::hlsdl;
use crate::http;
use crate::layout::Folder;
use crate::model::{Cover, Meta, Photo};
use crate::mux;
use crate::photos;
use crate::util::{human_size, sanitize_filename};

pub use crate::http::USER_AGENT;
const CHUNK: u64 = 4 * 1024 * 1024;
const WORKERS: usize = 6;
const COVER_LIMIT: u64 = 12 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Remote {
    pub url: String,
    pub ext: String,
    pub referer: Option<String>,
    pub extract: bool,
}

pub enum Job {
    Audio {
        audio: Remote,
        title: String,
        id: String,
        meta: Option<Meta>,
    },
    Video {
        video: Remote,
        audio: Option<Remote>,
        wrap: Option<mux::Container>,
        title: String,
        id: String,
        meta: Option<Meta>,
    },
    Gallery {
        photos: Vec<Photo>,
        referer: Option<String>,
        title: String,
        id: String,
    },
}

impl Job {
    pub fn folder(&self) -> Folder {
        match self {
            Job::Audio { .. } => Folder::Audios,
            Job::Video { .. } => Folder::Videos,
            Job::Gallery { .. } => Folder::Galleries,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    Fetching,
    FetchingAudio,
    Extracting,
    Merging,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Stage::Fetching => "downloading",
            Stage::FetchingAudio => "downloading audio",
            Stage::Extracting => "extracting audio",
            Stage::Merging => "merging audio and video",
        }
    }
}

pub enum Msg {
    Total(u64),
    Progress(u64),
    Stage(Stage),
    Note(String),
    Done(PathBuf),
    Failed(String),
    Cancelled,
}

pub fn run(job: Job, dir: PathBuf, tx: Sender<Msg>, cancel: Arc<AtomicBool>) {
    match execute(&job, &dir, &tx, &cancel) {
        Ok(Some(path)) => {
            let _ = tx.send(Msg::Done(path));
        }
        Ok(None) => {
            let _ = tx.send(Msg::Cancelled);
        }
        Err(error) => {
            let _ = tx.send(Msg::Failed(error));
        }
    }
}

pub fn run_blocking(job: Job, dir: PathBuf) -> Result<PathBuf, String> {
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker = std::thread::spawn(move || run(job, dir, tx, cancel));
    let mut total = 0u64;
    let mut best = 0u64;
    let mut last = Instant::now();
    let outcome = loop {
        match rx.recv() {
            Ok(Msg::Total(bytes)) => {
                total = bytes;
                best = 0;
            }
            Ok(Msg::Progress(done)) => {
                best = best.max(done);
                if last.elapsed() >= Duration::from_millis(150) {
                    let percent = best.saturating_mul(100).checked_div(total).unwrap_or(0);
                    eprint!(
                        "\r\x1b[K  {percent:>3}%  {} / {}",
                        human_size(best),
                        human_size(total)
                    );
                    last = Instant::now();
                }
            }
            Ok(Msg::Stage(stage)) => eprint!("\r\x1b[K  {}", stage.label()),
            Ok(Msg::Note(note)) => eprintln!("\r\x1b[K  {note}"),
            Ok(Msg::Done(path)) => {
                eprintln!("\r\x1b[K  saved {}", path.display());
                break Ok(path);
            }
            Ok(Msg::Failed(error)) => break Err(error),
            Ok(Msg::Cancelled) => break Err("cancelled".to_string()),
            Err(_) => break Err("download failed".to_string()),
        }
    };
    let _ = worker.join();
    outcome
}

fn execute(
    job: &Job,
    dir: &Path,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
) -> Result<Option<PathBuf>, String> {
    fs::create_dir_all(dir).map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    let pid = std::process::id();
    match job {
        Job::Audio {
            audio,
            title,
            id,
            meta,
        } => {
            let stem = stem(title, id);
            let meta = with_cover(meta.as_ref(), cancel);
            let part = dir.join(format!(".nab-{id}-{pid}.audio"));
            let cut = dir.join(format!(".nab-{id}-{pid}.cut"));
            let scratch = dir.join(format!(".nab-{id}-{pid}.segments"));
            let _temps = Temps(vec![part.clone(), cut.clone(), scratch.clone()]);
            let _ = tx.send(Msg::Stage(Stage::Fetching));
            let Some(_) = fetch_any(audio, &part, &scratch, tx, cancel)? else {
                return Ok(None);
            };
            if !audio.extract {
                let tagged = tag_in_place(&part, &cut, meta.as_ref(), tx);
                let ext = if tagged {
                    mux::sniff(&cut)
                        .map(mux::Container::audio_extension)
                        .unwrap_or(&audio.ext)
                } else {
                    &audio.ext
                };
                let dest = unique_path(dir, &stem, ext, id);
                save(if tagged { &cut } else { &part }, &dest)?;
                return Ok(Some(dest));
            }
            let _ = tx.send(Msg::Stage(Stage::Extracting));
            mux::extract_audio(&part, &cut, meta.as_ref())
                .map_err(|error| format!("could not extract the audio: {error}"))?;
            let extension = mux::sniff(&cut)
                .map(mux::Container::audio_extension)
                .unwrap_or("m4a");
            let dest = unique_path(dir, &stem, extension, id);
            save(&cut, &dest)?;
            Ok(Some(dest))
        }
        Job::Gallery {
            photos: items,
            referer,
            title,
            id,
        } => {
            let stem = stem(title, id);
            let scratch = dir.join(format!(".nab-{id}-{pid}.gallery"));
            let _temps = Temps(vec![scratch.clone()]);
            let _ = tx.send(Msg::Stage(Stage::Fetching));
            let Some(outcome) = photos::fetch(items, referer.as_deref(), &scratch, tx, cancel)?
            else {
                return Ok(None);
            };
            if outcome.saved == 0 {
                return Err("no photo could be downloaded".to_string());
            }
            if outcome.failed > 0 {
                let _ = tx.send(Msg::Note(format!(
                    "{} of {} photos could not be downloaded",
                    outcome.failed,
                    outcome.saved + outcome.failed
                )));
            }
            let dest = unique_dir(dir, &stem, id);
            save(&scratch, &dest)?;
            Ok(Some(dest))
        }
        Job::Video {
            video,
            audio,
            wrap,
            title,
            id,
            meta,
        } => {
            let stem = stem(title, id);
            let meta = with_cover(meta.as_ref(), cancel);
            let video_part = dir.join(format!(".nab-{id}-{pid}.video"));
            let audio_part = dir.join(format!(".nab-{id}-{pid}.audio"));
            let merged_part = dir.join(format!(".nab-{id}-{pid}.merged"));
            let _temps = Temps(vec![
                video_part.clone(),
                audio_part.clone(),
                merged_part.clone(),
            ]);
            let _ = tx.send(Msg::Stage(Stage::Fetching));
            let scratch = dir.join(format!(".nab-{id}-{pid}.segments"));
            let _scratch = Temps(vec![scratch.clone()]);
            let Some(container) = fetch_any(video, &video_part, &scratch, tx, cancel)? else {
                return Ok(None);
            };
            if video.ext == "m3u8" {
                let dest = unique_path(dir, &stem, container, id);
                save(&video_part, &dest)?;
                return Ok(Some(dest));
            }
            let pair = audio
                .as_ref()
                .filter(|remote| mux::pairs(&video.ext, &remote.ext));
            let Some(remote) = pair else {
                if audio.is_some() {
                    let _ = tx.send(Msg::Note(
                        "no audio track fits this container, saved video only".to_string(),
                    ));
                }
                let tagged = tag_in_place(&video_part, &merged_part, meta.as_ref(), tx);
                let dest = unique_path(dir, &stem, &video.ext, id);
                save(if tagged { &merged_part } else { &video_part }, &dest)?;
                return Ok(Some(dest));
            };
            let _ = tx.send(Msg::Stage(Stage::FetchingAudio));
            if !fetch(remote, &audio_part, tx, cancel)? {
                return Ok(None);
            }
            let _ = tx.send(Msg::Stage(Stage::Merging));
            merge_streams(
                &video_part,
                &audio_part,
                &merged_part,
                *wrap,
                meta.as_ref(),
                tx,
            )?;
            let container = mux::sniff(&merged_part)
                .map(mux::Container::extension)
                .unwrap_or_else(|_| mux::Container::for_video(&video.ext).extension());
            let dest = unique_path(dir, &stem, container, id);
            save(&merged_part, &dest)?;
            Ok(Some(dest))
        }
    }
}

fn merge_streams(
    video: &Path,
    audio: &Path,
    out: &Path,
    wrap: Option<mux::Container>,
    meta: Option<&Meta>,
    tx: &Sender<Msg>,
) -> Result<(), String> {
    if let Some(target) = wrap {
        match mux::merge_into(video, audio, out, target, meta) {
            Ok(()) => return Ok(()),
            Err(error) => {
                let _ = tx.send(Msg::Note(format!(
                    "could not save as {}: {error}",
                    target.extension()
                )));
            }
        }
    }
    mux::merge(video, audio, out, meta)
        .map_err(|error| format!("could not merge audio and video: {error}"))
}

fn tag_in_place(source: &Path, out: &Path, meta: Option<&Meta>, tx: &Sender<Msg>) -> bool {
    let Some(meta) = meta.filter(|meta| !meta.is_empty()) else {
        return false;
    };
    match mux::retag(source, out, meta) {
        Ok(()) => true,
        Err(error) => {
            let _ = fs::remove_file(out);
            let _ = tx.send(Msg::Note(format!("saved without tags: {error}")));
            false
        }
    }
}

fn with_cover(meta: Option<&Meta>, cancel: &AtomicBool) -> Option<Meta> {
    let mut meta = meta.cloned()?;
    if meta.cover.is_none() {
        for url in &meta.cover_urls {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            if let Some(cover) = fetch_cover(url) {
                meta.cover = Some(cover);
                break;
            }
        }
    }
    Some(meta)
}

fn fetch_cover(url: &str) -> Option<Cover> {
    let mut response = http::get(url, None, None).ok()?;
    let data = response
        .body_mut()
        .with_config()
        .limit(COVER_LIMIT)
        .read_to_vec()
        .ok()?;
    Cover::sniff(data)
}

fn fetch_any(
    remote: &Remote,
    part: &Path,
    scratch: &Path,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
) -> Result<Option<&'static str>, String> {
    if remote.ext == "m3u8" {
        return hlsdl::fetch(remote, part, scratch, tx, cancel);
    }
    if !fetch(remote, part, tx, cancel)? {
        return Ok(None);
    }
    Ok(Some(if remote.ext == "mp4" { "mp4" } else { "other" }))
}

fn save(part: &Path, dest: &Path) -> Result<(), String> {
    fs::rename(part, dest).map_err(|error| format!("cannot save {}: {error}", dest.display()))
}

fn fetch(
    remote: &Remote,
    dest: &Path,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
) -> Result<bool, String> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(false);
    }
    let url = remote.url.as_str();
    let referer = remote.referer.as_deref();
    let mut response = http::get(url, referer, Some((0, None)))?;
    let ranged = response.status().as_u16() == 206;
    let total = response
        .headers()
        .get("content-range")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.rsplit('/').next())
        .and_then(|value| value.parse::<u64>().ok())
        .or_else(|| response.body().content_length())
        .unwrap_or(0);
    let _ = tx.send(Msg::Total(total));
    if ranged && total > CHUNK {
        drop(response);
        return fetch_segments(url, referer, dest, total, tx, cancel);
    }
    let mut reader = response.body_mut().as_reader();
    let mut writer = BufWriter::new(create(dest)?);
    let mut running = 0u64;
    let Some(written) = http::pump(&mut reader, &mut writer, cancel, |read| {
        running += read;
        let _ = tx.send(Msg::Progress(running));
    })?
    else {
        return Ok(false);
    };
    writer
        .flush()
        .map_err(|error| format!("cannot write download: {error}"))?;
    if total > 0 && written < total {
        return Err("download ended early".to_string());
    }
    Ok(true)
}

fn create(path: &Path) -> Result<File, String> {
    File::create(path).map_err(|error| format!("cannot write download: {error}"))
}

fn fetch_segments(
    url: &str,
    referer: Option<&str>,
    dest: &Path,
    total: u64,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
) -> Result<bool, String> {
    create(dest)?
        .set_len(total)
        .map_err(|error| format!("cannot write download: {error}"))?;
    let segments = total.div_ceil(CHUNK) as usize;
    let next = AtomicUsize::new(0);
    let done = AtomicU64::new(0);
    let failure: Mutex<Option<String>> = Mutex::new(None);
    let stopped = AtomicBool::new(false);
    std::thread::scope(|scope| {
        for _ in 0..WORKERS.min(segments) {
            let tx = tx.clone();
            let next = &next;
            let done = &done;
            let failure = &failure;
            let stopped = &stopped;
            scope.spawn(move || loop {
                if cancel.load(Ordering::Relaxed) || stopped.load(Ordering::Relaxed) {
                    return;
                }
                let index = next.fetch_add(1, Ordering::Relaxed);
                if index >= segments {
                    return;
                }
                let start = index as u64 * CHUNK;
                let end = (start + CHUNK).min(total) - 1;
                let outcome = http::retry(cancel, || {
                    let mut taken = 0u64;
                    let result = fetch_segment(url, referer, dest, start, end, cancel, |read| {
                        taken += read;
                        let progress = done.fetch_add(read, Ordering::Relaxed) + read;
                        let _ = tx.send(Msg::Progress(progress));
                    });
                    if !matches!(result, Ok(Some(()))) {
                        done.fetch_sub(taken, Ordering::Relaxed);
                    }
                    result
                });
                if let Err(error) = outcome {
                    if failure.lock().map(|mut slot| slot.replace(error)).is_ok() {
                        stopped.store(true, Ordering::Relaxed);
                    }
                    return;
                }
            });
        }
    });
    if let Some(error) = failure.into_inner().map_err(|_| "download failed")? {
        return Err(error);
    }
    if cancel.load(Ordering::Relaxed) {
        return Ok(false);
    }
    let written = fs::metadata(dest).map(|meta| meta.len()).unwrap_or(0);
    if written != total {
        return Err("download ended early".to_string());
    }
    Ok(true)
}

fn fetch_segment(
    url: &str,
    referer: Option<&str>,
    dest: &Path,
    start: u64,
    end: u64,
    cancel: &AtomicBool,
    seen: impl FnMut(u64),
) -> Result<Option<()>, String> {
    let mut response = http::get(url, referer, Some((start, Some(end))))?;
    if response.status().as_u16() != 206 {
        return Err("the server stopped honouring range requests".to_string());
    }
    let file = OpenOptions::new()
        .write(true)
        .open(dest)
        .map_err(|error| format!("cannot write download: {error}"))?;
    let mut writer = BufWriter::new(file);
    writer
        .seek(SeekFrom::Start(start))
        .map_err(|error| format!("cannot write download: {error}"))?;
    let mut reader = response.body_mut().as_reader();
    let Some(written) = http::pump(&mut reader, &mut writer, cancel, seen)? else {
        return Ok(None);
    };
    writer
        .flush()
        .map_err(|error| format!("cannot write download: {error}"))?;
    if written != end - start + 1 {
        return Err("download ended early".to_string());
    }
    Ok(Some(()))
}

fn stem(title: &str, id: &str) -> String {
    let cleaned = sanitize_filename(title);
    if cleaned == "untitled" {
        sanitize_filename(id)
    } else {
        cleaned
    }
}

pub fn unique_path(dir: &Path, stem: &str, ext: &str, id: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.{ext}"));
    if !first.exists() {
        return first;
    }
    let second = dir.join(format!("{stem} [{id}].{ext}"));
    if !second.exists() {
        return second;
    }
    for number in 2..1000 {
        let candidate = dir.join(format!("{stem} [{id}] {number}.{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem} [{id}] {}.{ext}", std::process::id()))
}

pub fn unique_dir(dir: &Path, stem: &str, id: &str) -> PathBuf {
    let first = dir.join(stem);
    if !first.exists() {
        return first;
    }
    let second = dir.join(format!("{stem} [{id}]"));
    if !second.exists() {
        return second;
    }
    for number in 2..1000 {
        let candidate = dir.join(format!("{stem} [{id}] {number}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem} [{id}] {}", std::process::id()))
}

struct Temps(Vec<PathBuf>);

impl Drop for Temps {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = fs::remove_file(path);
            let _ = fs::remove_dir_all(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn names_do_not_collide() {
        let dir = std::env::temp_dir().join(format!("nab-paths-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let first = unique_path(&dir, "clip", "m4a", "abcdefghijk");
        assert_eq!(first.file_name().unwrap(), "clip.m4a");
        fs::write(&first, b"x").unwrap();
        let second = unique_path(&dir, "clip", "m4a", "abcdefghijk");
        assert_eq!(second.file_name().unwrap(), "clip [abcdefghijk].m4a");
        fs::write(&second, b"x").unwrap();
        let third = unique_path(&dir, "clip", "m4a", "abcdefghijk");
        assert_eq!(third.file_name().unwrap(), "clip [abcdefghijk] 2.m4a");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn temp_parts_are_cleaned() {
        let dir = std::env::temp_dir().join(format!("nab-temps-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let part = dir.join("part");
        fs::write(&part, b"x").unwrap();
        {
            let _temps = Temps(vec![part.clone()]);
        }
        assert!(!part.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    fn served() -> (String, Vec<mux::synth::Frame>, Vec<mux::synth::Frame>) {
        let video = mux::synth::frames(true, 8, 1);
        let audio = mux::synth::frames(false, 9, 2);
        let base = crate::testserver::serve(vec![
            (
                "/v.mp4".to_string(),
                mux::synth::fragmented(true, &video, 15360, 512),
            ),
            (
                "/a.m4a".to_string(),
                mux::synth::fragmented(false, &audio, 44100, 1024),
            ),
        ]);
        (base, video, audio)
    }

    fn collect(rx: mpsc::Receiver<Msg>) -> (Vec<Stage>, Vec<String>, Option<PathBuf>) {
        let mut stages = Vec::new();
        let mut notes = Vec::new();
        let mut done = None;
        for message in rx {
            match message {
                Msg::Stage(stage) => stages.push(stage),
                Msg::Note(note) => notes.push(note),
                Msg::Done(path) => done = Some(path),
                Msg::Failed(error) => panic!("{error}"),
                _ => {}
            }
        }
        (stages, notes, done)
    }

    #[test]
    fn downloads_and_merges_in_process() {
        let (base, video, audio) = served();
        let dir = std::env::temp_dir().join(format!("nab-e2e-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let (tx, rx) = mpsc::channel();
        let job = Job::Video {
            video: Remote {
                url: format!("{base}/v.mp4"),
                ext: "mp4".to_string(),
                referer: None,
                extract: false,
            },
            audio: Some(Remote {
                url: format!("{base}/a.m4a"),
                ext: "m4a".to_string(),
                referer: None,
                extract: false,
            }),
            wrap: None,
            title: "My: clip".to_string(),
            id: "abcdefghijk".to_string(),
            meta: None,
        };
        run(job, dir.clone(), tx, Arc::new(AtomicBool::new(false)));
        let (stages, notes, done) = collect(rx);
        let path = done.unwrap();
        assert_eq!(
            stages,
            vec![Stage::Fetching, Stage::FetchingAudio, Stage::Merging]
        );
        assert!(notes.is_empty());
        assert_eq!(path.file_name().unwrap(), "My clip.mp4");
        assert_eq!(
            mux::track_counts(&path).unwrap(),
            (video.len(), audio.len())
        );
        let hidden: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".nab"))
            .collect();
        assert!(hidden.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn saves_the_merged_pair_as_matroska_on_request() {
        let (base, video, audio) = served();
        let dir = std::env::temp_dir().join(format!("nab-e2e-mkv-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let (tx, rx) = mpsc::channel();
        let job = Job::Video {
            video: Remote {
                url: format!("{base}/v.mp4"),
                ext: "mp4".to_string(),
                referer: None,
                extract: false,
            },
            audio: Some(Remote {
                url: format!("{base}/a.m4a"),
                ext: "m4a".to_string(),
                referer: None,
                extract: false,
            }),
            wrap: Some(mux::Container::Mkv),
            title: "wrapped".to_string(),
            id: "abcdefghijk".to_string(),
            meta: None,
        };
        run(job, dir.clone(), tx, Arc::new(AtomicBool::new(false)));
        let (_, notes, done) = collect(rx);
        assert!(notes.is_empty());
        let path = done.unwrap();
        assert_eq!(path.file_name().unwrap(), "wrapped.mkv");
        assert_eq!(
            mux::track_counts(&path).unwrap(),
            (video.len(), audio.len())
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_impossible_format_falls_back_with_a_note() {
        let (base, video, audio) = served();
        let dir = std::env::temp_dir().join(format!("nab-e2e-fallback-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let (tx, rx) = mpsc::channel();
        let job = Job::Video {
            video: Remote {
                url: format!("{base}/v.mp4"),
                ext: "mp4".to_string(),
                referer: None,
                extract: false,
            },
            audio: Some(Remote {
                url: format!("{base}/a.m4a"),
                ext: "m4a".to_string(),
                referer: None,
                extract: false,
            }),
            wrap: Some(mux::Container::WebM),
            title: "fallback".to_string(),
            id: "abcdefghijk".to_string(),
            meta: None,
        };
        run(job, dir.clone(), tx, Arc::new(AtomicBool::new(false)));
        let (_, notes, done) = collect(rx);
        assert_eq!(notes.len(), 1);
        let path = done.unwrap();
        assert_eq!(path.file_name().unwrap(), "fallback.mp4");
        assert_eq!(
            mux::track_counts(&path).unwrap(),
            (video.len(), audio.len())
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unpaired_audio_saves_video_with_a_note() {
        let (base, video, _) = served();
        let dir = std::env::temp_dir().join(format!("nab-e2e-solo-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let (tx, rx) = mpsc::channel();
        let job = Job::Video {
            video: Remote {
                url: format!("{base}/v.mp4"),
                ext: "mp4".to_string(),
                referer: None,
                extract: false,
            },
            audio: Some(Remote {
                url: format!("{base}/a.m4a"),
                ext: "weba".to_string(),
                referer: None,
                extract: false,
            }),
            wrap: None,
            title: "solo".to_string(),
            id: "abcdefghijk".to_string(),
            meta: None,
        };
        run(job, dir.clone(), tx, Arc::new(AtomicBool::new(false)));
        let (_, notes, done) = collect(rx);
        assert_eq!(notes.len(), 1);
        let path = done.unwrap();
        assert_eq!(path.file_name().unwrap(), "solo.mp4");
        assert_eq!(
            fs::read(&path).unwrap(),
            mux::synth::fragmented(true, &video, 15360, 512)
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_pair_fails_cleanly() {
        let base = crate::testserver::serve(vec![
            ("/v.mp4".to_string(), vec![7u8; 5000]),
            ("/a.m4a".to_string(), vec![9u8; 5000]),
        ]);
        let dir = std::env::temp_dir().join(format!("nab-e2e-bad-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let (tx, rx) = mpsc::channel();
        let job = Job::Video {
            video: Remote {
                url: format!("{base}/v.mp4"),
                ext: "mp4".to_string(),
                referer: None,
                extract: false,
            },
            audio: Some(Remote {
                url: format!("{base}/a.m4a"),
                ext: "m4a".to_string(),
                referer: None,
                extract: false,
            }),
            wrap: None,
            title: "bad".to_string(),
            id: "abcdefghijk".to_string(),
            meta: None,
        };
        run(job, dir.clone(), tx, Arc::new(AtomicBool::new(false)));
        let failure = rx.into_iter().find_map(|message| match message {
            Msg::Failed(error) => Some(error),
            _ => None,
        });
        assert!(failure.unwrap().contains("could not merge"));
        let left = fs::read_dir(&dir)
            .map(|entries| entries.count())
            .unwrap_or(0);
        assert_eq!(left, 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reports_cancelled_before_connecting() {
        let dir = std::env::temp_dir().join(format!("nab-cancel-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let (tx, _rx) = mpsc::channel();
        let cancel = AtomicBool::new(true);
        let dest = dir.join("part");
        let outcome = fetch(
            &Remote {
                url: "https://example.invalid/x".to_string(),
                ext: "mp4".to_string(),
                referer: None,
                extract: false,
            },
            &dest,
            &tx,
            &cancel,
        )
        .unwrap();
        assert!(!outcome);
        assert!(!dest.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    fn hls_job(url: String) -> Job {
        Job::Video {
            video: Remote {
                url,
                ext: "m3u8".to_string(),
                referer: Some("https://example.test/".to_string()),
                extract: false,
            },
            audio: None,
            wrap: None,
            title: "Stream: clip".to_string(),
            id: "abc".to_string(),
            meta: None,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nab-hls-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn hls_transport_streams_are_joined_in_order_and_saved_as_ts() {
        let segment = |tag: u8| {
            let mut bytes = vec![0x47u8];
            bytes.extend(std::iter::repeat_n(tag, 3000 + tag as usize));
            bytes
        };
        let segments: Vec<Vec<u8>> = (0..9).map(segment).collect();
        let mut playlist = String::from("#EXTM3U\n");
        for index in 0..9 {
            playlist.push_str(&format!("#EXTINF:2.0,\nseg{index}.ts\n"));
        }
        let mut files = vec![
            (
                "/h/master.m3u8".to_string(),
                b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1,RESOLUTION=640x360\nlow.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=2,RESOLUTION=1280x720\nhigh.m3u8\n".to_vec(),
            ),
            ("/h/high.m3u8".to_string(), playlist.into_bytes()),
            ("/h/low.m3u8".to_string(), b"#EXTM3U\n#EXTINF:2,\nnope.ts\n".to_vec()),
        ];
        for (index, bytes) in segments.iter().enumerate() {
            files.push((format!("/h/seg{index}.ts"), bytes.clone()));
        }
        let base = crate::testserver::serve(files);
        let dir = scratch("ts");
        let saved = run_blocking(hls_job(format!("{base}/h/master.m3u8")), dir.clone()).unwrap();
        assert_eq!(saved.file_name().unwrap(), "Stream clip.ts");
        assert_eq!(fs::read(&saved).unwrap(), segments.concat());
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .collect();
        assert_eq!(leftovers.len(), 1, "{leftovers:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn hls_fragmented_mp4_keeps_the_init_segment_first_and_saves_as_mp4() {
        let init = [&[0u8, 0, 0, 16][..], b"ftypisom", &[0u8; 8][..]].concat();
        let pieces: Vec<Vec<u8>> = (0..4u8)
            .map(|tag| [&[0u8, 0, 0, 12][..], b"moof", &[tag; 4000][..]].concat())
            .collect();
        let mut playlist = String::from("#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n");
        for index in 0..4 {
            playlist.push_str(&format!("#EXTINF:4,\nf{index}.m4s\n"));
        }
        let mut files = vec![
            ("/m/v.m3u8".to_string(), playlist.into_bytes()),
            ("/m/init.mp4".to_string(), init.clone()),
        ];
        for (index, bytes) in pieces.iter().enumerate() {
            files.push((format!("/m/f{index}.m4s"), bytes.clone()));
        }
        let base = crate::testserver::serve(files);
        let dir = scratch("mp4");
        let saved = run_blocking(hls_job(format!("{base}/m/v.m3u8")), dir.clone()).unwrap();
        assert_eq!(saved.extension().unwrap(), "mp4");
        assert_eq!(fs::read(&saved).unwrap(), [init, pieces.concat()].concat());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn hls_failures_are_reported_and_leave_nothing_behind() {
        let base = crate::testserver::serve(vec![
            (
                "/e/enc.m3u8".to_string(),
                b"#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"k\"\n#EXTINF:2,\na.ts\n".to_vec(),
            ),
            ("/e/empty.m3u8".to_string(), b"#EXTM3U\n".to_vec()),
            (
                "/e/gone.m3u8".to_string(),
                b"#EXTM3U\n#EXTINF:2,\nmissing.ts\n".to_vec(),
            ),
        ]);
        let dir = scratch("fail");
        let encrypted = run_blocking(hls_job(format!("{base}/e/enc.m3u8")), dir.clone());
        assert!(encrypted.unwrap_err().contains("encrypted"));
        let empty = run_blocking(hls_job(format!("{base}/e/empty.m3u8")), dir.clone());
        assert!(empty.unwrap_err().contains("no segments"));
        let gone = run_blocking(hls_job(format!("{base}/e/gone.m3u8")), dir.clone());
        assert!(gone.unwrap_err().contains("download failed"));
        let left = fs::read_dir(&dir)
            .map(|entries| entries.count())
            .unwrap_or(0);
        assert_eq!(left, 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn extracted_audio_lands_as_an_m4a_with_only_the_sound_track() {
        let video = mux::synth::frames(true, 6, 1);
        let audio = mux::synth::frames(false, 7, 2);
        let mut both = std::env::temp_dir().join(format!("nab-xa-src-{}", std::process::id()));
        let _ = fs::remove_dir_all(&both);
        fs::create_dir_all(&both).unwrap();
        fs::write(
            both.join("v.mp4"),
            mux::synth::progressive(true, &video, 15360, 512),
        )
        .unwrap();
        fs::write(
            both.join("a.m4a"),
            mux::synth::progressive(false, &audio, 44100, 1024),
        )
        .unwrap();
        mux::merge(
            &both.join("v.mp4"),
            &both.join("a.m4a"),
            &both.join("j.mp4"),
            None,
        )
        .unwrap();
        let joined = fs::read(both.join("j.mp4")).unwrap();
        let _ = fs::remove_dir_all(&both);
        both = std::env::temp_dir().join(format!("nab-xa-{}", std::process::id()));
        let _ = fs::remove_dir_all(&both);
        let base = crate::testserver::serve(vec![("/clip.mp4".to_string(), joined)]);
        let (tx, rx) = mpsc::channel();
        let job = Job::Audio {
            audio: Remote {
                url: format!("{base}/clip.mp4"),
                ext: "mp4".to_string(),
                referer: None,
                extract: true,
            },
            title: "clip".to_string(),
            id: "abcdefghijk".to_string(),
            meta: None,
        };
        run(job, both.clone(), tx, Arc::new(AtomicBool::new(false)));
        let (stages, _, done) = collect(rx);
        let path = done.unwrap();
        assert_eq!(stages, vec![Stage::Fetching, Stage::Extracting]);
        assert_eq!(path.file_name().unwrap(), "clip.m4a");
        let bytes = fs::read(&path).unwrap();
        let has = |tag: &[u8]| bytes.windows(4).any(|window| window == tag);
        assert!(has(b"soun") && !has(b"vide"));
        assert!(mux::track_counts(&path).is_err());
        let _ = fs::remove_dir_all(&both);
    }

    #[test]
    fn galleries_save_every_photo_into_one_numbered_folder() {
        let base = crate::testserver::serve(vec![
            ("/a.jpg".to_string(), vec![1u8; 300]),
            ("/b.png".to_string(), vec![2u8; 400]),
            ("/c.webp".to_string(), vec![3u8; 500]),
        ]);
        let dir = std::env::temp_dir().join(format!("nab-gal-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let (tx, rx) = mpsc::channel();
        let photos = ["a.jpg", "b.png", "c.webp", "missing.jpg"]
            .iter()
            .enumerate()
            .map(|(index, name)| crate::model::Photo {
                id: index.to_string(),
                url: format!("{base}/{name}"),
            })
            .collect();
        let job = Job::Gallery {
            photos,
            referer: None,
            title: "Trip: day 1".to_string(),
            id: "42".to_string(),
        };
        run(job, dir.clone(), tx, Arc::new(AtomicBool::new(false)));
        let (_, notes, done) = collect(rx);
        let folder = done.unwrap();
        assert!(folder.is_dir());
        assert_eq!(folder.file_name().unwrap(), "Trip day 1");
        let mut names: Vec<String> = fs::read_dir(&folder)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names.len(), 3);
        assert!(names[0].starts_with("001"));
        assert!(names[2].starts_with("003"));
        assert_eq!(notes.len(), 1);
        let hidden = fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".nab"))
            .count();
        assert_eq!(hidden, 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_gallery_with_no_reachable_photo_is_an_error() {
        let base = crate::testserver::serve(vec![]);
        let dir = std::env::temp_dir().join(format!("nab-gal-none-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let (tx, rx) = mpsc::channel();
        let job = Job::Gallery {
            photos: vec![crate::model::Photo {
                id: "1".to_string(),
                url: format!("{base}/nope.jpg"),
            }],
            referer: None,
            title: "none".to_string(),
            id: "7".to_string(),
        };
        run(job, dir.clone(), tx, Arc::new(AtomicBool::new(false)));
        let mut failed = false;
        for message in rx {
            if matches!(message, Msg::Failed(_)) {
                failed = true;
            }
        }
        assert!(failed);
        let _ = fs::remove_dir_all(&dir);
    }

    fn tagged_meta(base: &str) -> Meta {
        Meta {
            title: Some("Blue Hour".into()),
            artist: Some("Ana Lima".into()),
            album: Some("Night Shift".into()),
            cover_urls: vec![format!("{base}/missing.jpg"), format!("{base}/cover.jpg")],
            ..Meta::default()
        }
    }

    fn cover_bytes() -> Vec<u8> {
        let mut bytes = vec![0xff, 0xd8, 0xff, 0xe0];
        bytes.extend_from_slice(&[7u8; 64]);
        bytes
    }

    fn contains(bytes: &[u8], needle: &[u8]) -> bool {
        bytes.windows(needle.len()).any(|window| window == needle)
    }

    #[test]
    fn audio_is_tagged_with_the_first_cover_that_downloads() {
        let audio = mux::synth::frames(false, 9, 2);
        let base = crate::testserver::serve(vec![
            (
                "/a.m4a".to_string(),
                mux::synth::fragmented(false, &audio, 44100, 1024),
            ),
            ("/cover.jpg".to_string(), cover_bytes()),
        ]);
        let dir = std::env::temp_dir().join(format!("nab-tag-audio-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let (tx, rx) = mpsc::channel();
        let job = Job::Audio {
            audio: Remote {
                url: format!("{base}/a.m4a"),
                ext: "m4a".to_string(),
                referer: None,
                extract: false,
            },
            title: "Blue Hour".to_string(),
            id: "abcdefghijk".to_string(),
            meta: Some(tagged_meta(&base)),
        };
        run(job, dir.clone(), tx, Arc::new(AtomicBool::new(false)));
        let (_, notes, done) = collect(rx);
        let path = done.unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(path.file_name().unwrap(), "Blue Hour.m4a");
        let bytes = fs::read(&path).unwrap();
        assert!(contains(&bytes, b"Night Shift"));
        assert!(contains(&bytes, b"covr"));
        assert!(contains(&bytes, &cover_bytes()));
        let hidden = fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".nab"))
            .count();
        assert_eq!(hidden, 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn merged_video_carries_the_tags() {
        let (base, _, _) = served();
        let dir = std::env::temp_dir().join(format!("nab-tag-video-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let (tx, rx) = mpsc::channel();
        let job = Job::Video {
            video: Remote {
                url: format!("{base}/v.mp4"),
                ext: "mp4".to_string(),
                referer: None,
                extract: false,
            },
            audio: Some(Remote {
                url: format!("{base}/a.m4a"),
                ext: "m4a".to_string(),
                referer: None,
                extract: false,
            }),
            wrap: None,
            title: "tagged".to_string(),
            id: "abcdefghijk".to_string(),
            meta: Some(tagged_meta(&base)),
        };
        run(job, dir.clone(), tx, Arc::new(AtomicBool::new(false)));
        let (_, notes, done) = collect(rx);
        let path = done.unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        let bytes = fs::read(&path).unwrap();
        assert!(contains(&bytes, b"Blue Hour"));
        assert!(!contains(&bytes, b"covr"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_cannot_be_tagged_is_saved_untouched_with_a_note() {
        let junk = vec![9u8; 4000];
        let base = crate::testserver::serve(vec![("/a.m4a".to_string(), junk.clone())]);
        let dir = std::env::temp_dir().join(format!("nab-tag-junk-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let (tx, rx) = mpsc::channel();
        let job = Job::Audio {
            audio: Remote {
                url: format!("{base}/a.m4a"),
                ext: "m4a".to_string(),
                referer: None,
                extract: false,
            },
            title: "junk".to_string(),
            id: "abcdefghijk".to_string(),
            meta: Some(tagged_meta(&base)),
        };
        run(job, dir.clone(), tx, Arc::new(AtomicBool::new(false)));
        let (_, notes, done) = collect(rx);
        let path = done.unwrap();
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("without tags"));
        assert_eq!(fs::read(&path).unwrap(), junk);
        let _ = fs::remove_dir_all(&dir);
    }
}
