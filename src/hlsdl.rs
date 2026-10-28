use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Mutex;
use std::time::Duration;

use crate::download::{Msg, Remote, USER_AGENT};
use crate::hls;

const WORKERS: usize = 6;
const ATTEMPTS: u32 = 3;

fn get_segment(
    url: &str,
    referer: Option<&str>,
    path: &Path,
    received: &AtomicU64,
    tx: &Sender<Msg>,
) -> Result<(), String> {
    let mut request = ureq::get(url)
        .header("user-agent", USER_AGENT)
        .header("accept", "*/*");
    if let Some(referer) = referer {
        request = request.header("referer", referer);
    }
    let mut response = request
        .call()
        .map_err(|error| format!("download failed: {error}"))?;
    let mut reader = response.body_mut().as_reader();
    let mut writer = BufWriter::new(
        File::create(path).map_err(|error| format!("cannot write download: {error}"))?,
    );
    let mut buffer = [0u8; 65536];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("download failed: {error}"))?;
        if read == 0 {
            break;
        }
        writer
            .write_all(&buffer[..read])
            .map_err(|error| format!("cannot write download: {error}"))?;
        let total = received.fetch_add(read as u64, Ordering::Relaxed) + read as u64;
        let _ = tx.send(Msg::Progress(total));
    }
    writer
        .flush()
        .map_err(|error| format!("cannot write download: {error}"))
}

fn container(first: &Path, has_init: bool) -> &'static str {
    if has_init {
        return "mp4";
    }
    let mut head = [0u8; 8];
    let read = File::open(first)
        .and_then(|mut file| file.read(&mut head))
        .unwrap_or(0);
    if read >= 8 && matches!(&head[4..8], b"ftyp" | b"styp" | b"moof" | b"moov") {
        "mp4"
    } else {
        "ts"
    }
}

pub fn fetch(
    remote: &Remote,
    part: &Path,
    scratch: &Path,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
) -> Result<Option<&'static str>, String> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let referer = remote.referer.as_deref();
    let mut url = remote.url.clone();
    let mut text = hls::fetch(&url, referer)?;
    if hls::is_master(&text) {
        let best = hls::parse_master(&text, &url)
            .into_iter()
            .next()
            .ok_or_else(|| "playlist lists no streams".to_string())?;
        url = best.url;
        text = hls::fetch(&url, referer)?;
    }
    let playlist = hls::parse_playlist(&text, &url);
    if playlist.encrypted {
        return Err("encrypted streams are not supported".to_string());
    }
    if playlist.segments.is_empty() {
        return Err("playlist contains no segments".to_string());
    }
    fs::create_dir_all(scratch).map_err(|error| format!("cannot write download: {error}"))?;
    let mut items: Vec<String> = Vec::new();
    items.extend(playlist.init.clone());
    items.extend(playlist.segments.iter().cloned());
    let count = items.len();
    let _ = tx.send(Msg::Total(0));
    let next = AtomicUsize::new(0);
    let finished = AtomicUsize::new(0);
    let received = AtomicU64::new(0);
    let stopped = AtomicBool::new(false);
    let failure: Mutex<Option<String>> = Mutex::new(None);
    let path_of = |index: usize| scratch.join(format!("seg-{index:06}"));
    std::thread::scope(|scope| {
        for _ in 0..WORKERS.min(count) {
            let tx = tx.clone();
            let (items, next, finished, received, stopped, failure) =
                (&items, &next, &finished, &received, &stopped, &failure);
            let path_of = &path_of;
            scope.spawn(move || loop {
                if cancel.load(Ordering::Relaxed) || stopped.load(Ordering::Relaxed) {
                    return;
                }
                let index = next.fetch_add(1, Ordering::Relaxed);
                if index >= count {
                    return;
                }
                let mut outcome = Err(String::new());
                for attempt in 0..ATTEMPTS {
                    if cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    outcome = get_segment(&items[index], referer, &path_of(index), received, &tx);
                    if outcome.is_ok() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(250 * (u64::from(attempt) + 1)));
                }
                match outcome {
                    Ok(()) => {
                        let done = finished.fetch_add(1, Ordering::Relaxed) + 1;
                        let seen = received.load(Ordering::Relaxed);
                        let estimate = seen.saturating_mul(count as u64) / done as u64;
                        let _ = tx.send(Msg::Total(estimate.max(seen)));
                    }
                    Err(error) => {
                        if failure.lock().map(|mut slot| slot.replace(error)).is_ok() {
                            stopped.store(true, Ordering::Relaxed);
                        }
                        return;
                    }
                }
            });
        }
    });
    if let Some(error) = failure.into_inner().map_err(|_| "download failed")? {
        return Err(error);
    }
    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let kind = container(&path_of(0), playlist.init.is_some());
    let mut writer = BufWriter::new(
        File::create(part).map_err(|error| format!("cannot write download: {error}"))?,
    );
    for index in 0..count {
        let mut file =
            File::open(path_of(index)).map_err(|error| format!("cannot read segment: {error}"))?;
        std::io::copy(&mut file, &mut writer)
            .map_err(|error| format!("cannot write download: {error}"))?;
        let _ = fs::remove_file(path_of(index));
    }
    writer
        .flush()
        .map_err(|error| format!("cannot write download: {error}"))?;
    let total = received.load(Ordering::Relaxed);
    let _ = tx.send(Msg::Total(total));
    let _ = tx.send(Msg::Progress(total));
    Ok(Some(kind))
}
