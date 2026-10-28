use std::time::Duration;

use crate::download::USER_AGENT;

pub const COOKIE: &str = "age_verified=1; platform=desktop; lang=en; x_csrf_token=1";
const RETRIES: u32 = 3;

fn transient(error: &ureq::Error) -> bool {
    match error {
        ureq::Error::StatusCode(code) => matches!(code, 408 | 425 | 429 | 500 | 502 | 503 | 504),
        ureq::Error::Io(_) | ureq::Error::Timeout(_) | ureq::Error::ConnectionFailed => true,
        _ => false,
    }
}

pub fn get(url: &str, referer: &str, xhr: bool) -> Result<String, String> {
    let mut last = String::new();
    for attempt in 0..=RETRIES {
        let mut request = ureq::get(url)
            .header("user-agent", USER_AGENT)
            .header("accept-language", "en-US,en;q=0.9")
            .header("referer", referer)
            .header("cookie", COOKIE);
        request = if xhr {
            request
                .header("accept", "*/*")
                .header("x-requested-with", "XMLHttpRequest")
                .header("x-csrf-token", "1")
        } else {
            request.header(
                "accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
        };
        match request.call() {
            Ok(mut response) => {
                return response
                    .body_mut()
                    .read_to_string()
                    .map_err(|error| format!("could not read {url}: {error}"));
            }
            Err(error) => {
                last = match &error {
                    ureq::Error::StatusCode(code) => format!("HTTP {code} for {url}"),
                    other => format!("request failed: {other}"),
                };
                if !transient(&error) || attempt == RETRIES {
                    break;
                }
                std::thread::sleep(Duration::from_millis(350 * (u64::from(attempt) + 1)));
            }
        }
    }
    Err(last)
}

pub fn encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_everything_but_unreserved() {
        assert_eq!(encode("a b&c/é~-_."), "a%20b%26c%2F%C3%A9~-_.");
    }
}
