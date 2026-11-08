use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/156.0.0.0 Safari/537.36";
pub const ATTEMPTS: u32 = 3;

pub fn get(
    url: &str,
    referer: Option<&str>,
    range: Option<(u64, Option<u64>)>,
) -> Result<ureq::http::Response<ureq::Body>, String> {
    let mut request = ureq::get(url)
        .header("user-agent", USER_AGENT)
        .header("accept", "*/*");
    if let Some((start, end)) = range {
        let value = match end {
            Some(end) => format!("bytes={start}-{end}"),
            None => format!("bytes={start}-"),
        };
        request = request.header("range", value);
    }
    if let Some(referer) = referer {
        request = request.header("referer", referer);
        if let Some(origin) = crate::util::origin(referer) {
            request = request.header("origin", origin);
        }
    }
    request
        .call()
        .map_err(|error| format!("download failed: {error}"))
}

pub fn text(url: &str, referer: Option<&str>) -> Result<String, String> {
    get(url, referer, None)?
        .body_mut()
        .read_to_string()
        .map_err(|error| format!("download failed: {error}"))
}

pub fn pump(
    reader: &mut impl Read,
    writer: &mut impl Write,
    cancel: &AtomicBool,
    mut seen: impl FnMut(u64),
) -> Result<Option<u64>, String> {
    let mut buffer = [0u8; 65536];
    let mut written = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("download failed: {error}"))?;
        if read == 0 {
            return Ok(Some(written));
        }
        writer
            .write_all(&buffer[..read])
            .map_err(|error| format!("cannot write download: {error}"))?;
        written += read as u64;
        seen(read as u64);
    }
}

pub fn retry<T>(
    cancel: &AtomicBool,
    mut attempt: impl FnMut() -> Result<Option<T>, String>,
) -> Result<Option<T>, String> {
    let mut last = String::new();
    for round in 0..ATTEMPTS {
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        match attempt() {
            Ok(done) => return Ok(done),
            Err(error) => last = error,
        }
        std::thread::sleep(Duration::from_millis(250 * (u64::from(round) + 1)));
    }
    Err(last)
}
