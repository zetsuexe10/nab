use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

use crate::download::{Msg, USER_AGENT};
use crate::model::Photo;

const WORKERS: usize = 4;
const ATTEMPTS: u32 = 3;
const LIMIT: u64 = 128 << 20;

#[derive(Debug, PartialEq, Eq)]
pub struct Outcome {
    pub saved: usize,
    pub failed: usize,
}

pub fn extension(url: &str, content_type: Option<&str>) -> &'static str {
    let kind = content_type
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match kind.as_str() {
        "image/webp" => return "webp",
        "image/jpeg" | "image/jpg" | "image/pjpeg" => return "jpg",
        "image/png" | "image/apng" => return "png",
        "image/gif" => return "gif",
        "image/avif" => return "avif",
        _ => {}
    }
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    match path.rsplit('.').next().unwrap_or("") {
        "webp" => "webp",
        "png" => "png",
        "gif" => "gif",
        "avif" => "avif",
        _ => "jpg",
    }
}

fn get(photo: &Photo, index: usize, referer: Option<&str>, folder: &Path) -> Result<u64, String> {
    let mut request = ureq::get(&photo.url)
        .header("user-agent", USER_AGENT)
        .header("accept", "image/avif,image/webp,image/*,*/*;q=0.8");
    if let Some(referer) = referer {
        request = request.header("referer", referer);
    }
    let mut response = request
        .call()
        .map_err(|error| format!("photo {index} failed: {error}"))?;
    let kind = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let mut body = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(LIMIT)
        .read_to_end(&mut body)
        .map_err(|error| format!("photo {index} failed: {error}"))?;
    if body.is_empty() {
        return Err(format!("photo {index} was empty"));
    }
    let name = format!("{index:03}.{}", extension(&photo.url, kind.as_deref()));
    let part = folder.join(format!("{name}.part"));
    let mut file =
        File::create(&part).map_err(|error| format!("cannot write photo {index}: {error}"))?;
    file.write_all(&body)
        .and_then(|_| file.flush())
        .map_err(|error| format!("cannot write photo {index}: {error}"))?;
    fs::rename(&part, folder.join(name))
        .map_err(|error| format!("cannot write photo {index}: {error}"))?;
    Ok(body.len() as u64)
}

pub fn fetch(
    photos: &[Photo],
    referer: Option<&str>,
    folder: &Path,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
) -> Result<Option<Outcome>, String> {
    fs::create_dir_all(folder)
        .map_err(|error| format!("cannot create {}: {error}", folder.display()))?;
    let count = photos.len();
    let next = AtomicUsize::new(0);
    let finished = AtomicUsize::new(0);
    let saved = AtomicUsize::new(0);
    let failed = AtomicUsize::new(0);
    let bytes = AtomicU64::new(0);
    let _ = tx.send(Msg::Total(0));
    std::thread::scope(|scope| {
        for _ in 0..WORKERS.min(count.max(1)) {
            scope.spawn(|| loop {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                let at = next.fetch_add(1, Ordering::Relaxed);
                let Some(photo) = photos.get(at) else {
                    return;
                };
                let mut stored = false;
                for attempt in 0..ATTEMPTS {
                    if cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    match get(photo, at + 1, referer, folder) {
                        Ok(size) => {
                            bytes.fetch_add(size, Ordering::Relaxed);
                            stored = true;
                            break;
                        }
                        Err(_) => {
                            std::thread::sleep(Duration::from_millis(300 * u64::from(attempt + 1)))
                        }
                    }
                }
                if stored {
                    saved.fetch_add(1, Ordering::Relaxed);
                } else {
                    failed.fetch_add(1, Ordering::Relaxed);
                }
                let done = finished.fetch_add(1, Ordering::Relaxed) + 1;
                let have = bytes.load(Ordering::Relaxed);
                let estimate = have.saturating_mul(count as u64) / done as u64;
                let _ = tx.send(Msg::Total(estimate.max(have)));
                let _ = tx.send(Msg::Progress(have));
            });
        }
    });
    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    Ok(Some(Outcome {
        saved: saved.load(Ordering::Relaxed),
        failed: failed.load(Ordering::Relaxed),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_prefers_the_response_type() {
        assert_eq!(
            extension("https://i/a.jpg", Some("image/webp; charset=x")),
            "webp"
        );
        assert_eq!(extension("https://i/a.png?x=1", None), "png");
        assert_eq!(extension("https://i/a", Some("text/html")), "jpg");
        assert_eq!(extension("https://i/a.GIF", None), "gif");
    }
}
