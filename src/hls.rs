#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variant {
    pub url: String,
    pub height: u32,
    pub bandwidth: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Playlist {
    pub init: Option<String>,
    pub segments: Vec<String>,
    pub encrypted: bool,
    pub seconds: f64,
}

pub fn join(base: &str, reference: &str) -> String {
    let reference = reference.trim();
    if reference.starts_with("http://") || reference.starts_with("https://") {
        return reference.to_string();
    }
    let base = base.split(['?', '#']).next().unwrap_or(base);
    let scheme_end = base.find("://").map(|at| at + 3).unwrap_or(0);
    let host_end = base[scheme_end..]
        .find('/')
        .map(|at| scheme_end + at)
        .unwrap_or(base.len());
    if let Some(rest) = reference.strip_prefix("//") {
        return format!("{}//{rest}", base[..scheme_end - 2].replace("//", ""));
    }
    if reference.starts_with('/') {
        return format!("{}{reference}", &base[..host_end]);
    }
    let directory = match base[host_end..].rfind('/') {
        Some(at) => &base[..host_end + at + 1],
        None => &base[..host_end],
    };
    let slash = if directory.ends_with('/') { "" } else { "/" };
    format!("{directory}{slash}{reference}")
}

fn attribute<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{name}=");
    let mut from = 0;
    while let Some(found) = line[from..].find(&key) {
        let at = from + found;
        let boundary = at == 0 || matches!(line.as_bytes()[at - 1], b':' | b',' | b' ');
        if boundary {
            let value = &line[at + key.len()..];
            return Some(if let Some(quoted) = value.strip_prefix('"') {
                quoted.split('"').next().unwrap_or("")
            } else {
                value.split(',').next().unwrap_or("")
            });
        }
        from = at + key.len();
    }
    None
}

pub fn parse_master(text: &str, base: &str) -> Vec<Variant> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let mut variants = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if !line.starts_with("#EXT-X-STREAM-INF") {
            continue;
        }
        let Some(next) = lines.get(index + 1).filter(|next| !next.starts_with('#')) else {
            continue;
        };
        let height = attribute(line, "RESOLUTION")
            .and_then(|value| value.split('x').nth(1))
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        let bandwidth = attribute(line, "BANDWIDTH")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        variants.push(Variant {
            url: join(base, next),
            height,
            bandwidth,
        });
    }
    variants.sort_by(|a, b| b.height.cmp(&a.height).then(b.bandwidth.cmp(&a.bandwidth)));
    variants
}

pub fn is_master(text: &str) -> bool {
    text.contains("#EXT-X-STREAM-INF")
}

pub fn parse_playlist(text: &str, base: &str) -> Playlist {
    let mut playlist = Playlist::default();
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    for (index, line) in lines.iter().enumerate() {
        if line.starts_with("#EXT-X-MAP:") {
            if let Some(uri) = attribute(line, "URI") {
                playlist.init = Some(join(base, uri));
            }
        } else if line.starts_with("#EXT-X-KEY:") {
            let method = attribute(line, "METHOD").unwrap_or("NONE");
            if !method.eq_ignore_ascii_case("NONE") {
                playlist.encrypted = true;
            }
        } else if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let Some(next) = lines.get(index + 1).filter(|next| !next.starts_with('#')) else {
                continue;
            };
            playlist.seconds += rest
                .split(',')
                .next()
                .and_then(|value| value.parse::<f64>().ok())
                .unwrap_or(0.0);
            playlist.segments.push(join(base, next));
        }
    }
    if playlist.segments.is_empty() {
        for line in &lines {
            let bare = line
                .split(['?', '#'])
                .next()
                .unwrap_or(line)
                .to_ascii_lowercase();
            if !line.starts_with('#')
                && (bare.ends_with(".ts") || bare.ends_with(".m4s") || bare.ends_with(".mp4"))
            {
                playlist.segments.push(join(base, line));
            }
        }
    }
    playlist
}

pub fn fetch(url: &str, referer: Option<&str>) -> Result<String, String> {
    let mut request = ureq::get(url)
        .header("user-agent", crate::download::USER_AGENT)
        .header("accept", "*/*");
    if let Some(referer) = referer {
        request = request.header("referer", referer);
    }
    request
        .call()
        .map_err(|error| format!("playlist failed: {error}"))?
        .body_mut()
        .read_to_string()
        .map_err(|error| format!("playlist failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MASTER: &str = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=900000,RESOLUTION=640x360,CODECS=\"avc1.4d401e,mp4a.40.2\"\n\
360p.h264.mp4.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=2800000,RESOLUTION=1280x720\n\
/v/720p.h264.mp4.m3u8?token=abc\n\
#EXT-X-STREAM-INF:BANDWIDTH=5000000\n\
https://other.test/auto.m3u8\n";

    #[test]
    fn joins_every_kind_of_reference() {
        let base = "https://cdn.test/a/b/master.m3u8?sig=1";
        assert_eq!(join(base, "https://x/y.ts"), "https://x/y.ts");
        assert_eq!(join(base, "seg1.ts"), "https://cdn.test/a/b/seg1.ts");
        assert_eq!(join(base, "/root/seg1.ts"), "https://cdn.test/root/seg1.ts");
        assert_eq!(join(base, "//edge.test/s.ts"), "https://edge.test/s.ts");
        assert_eq!(join("https://cdn.test", "s.ts"), "https://cdn.test/s.ts");
    }

    #[test]
    fn master_variants_sort_tallest_first() {
        let variants = parse_master(MASTER, "https://cdn.test/a/master.m3u8");
        let heights: Vec<u32> = variants.iter().map(|variant| variant.height).collect();
        assert_eq!(heights, vec![720, 360, 0]);
        assert_eq!(
            variants[0].url,
            "https://cdn.test/v/720p.h264.mp4.m3u8?token=abc"
        );
        assert_eq!(variants[1].bandwidth, 900_000);
        assert_eq!(variants[2].url, "https://other.test/auto.m3u8");
        assert!(is_master(MASTER));
    }

    #[test]
    fn playlist_has_init_segments_and_length() {
        let text = "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:4.0,\nseg0.m4s\n#EXTINF:3.5,\nseg1.m4s?x=1\n#EXT-X-ENDLIST\n";
        let playlist = parse_playlist(text, "https://cdn.test/a/v.m3u8");
        assert_eq!(
            playlist.init.as_deref(),
            Some("https://cdn.test/a/init.mp4")
        );
        assert_eq!(
            playlist.segments,
            vec![
                "https://cdn.test/a/seg0.m4s",
                "https://cdn.test/a/seg1.m4s?x=1"
            ]
        );
        assert!((playlist.seconds - 7.5).abs() < 1e-9);
        assert!(!playlist.encrypted);
    }

    #[test]
    fn encryption_is_flagged_and_bare_lists_still_work() {
        let text = "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"k\"\n#EXTINF:2,\na.ts\n";
        assert!(parse_playlist(text, "https://c/x.m3u8").encrypted);
        let plain = "#EXT-X-KEY:METHOD=NONE\na.ts\nb.ts?x=1\n";
        let playlist = parse_playlist(plain, "https://c/x.m3u8");
        assert!(!playlist.encrypted);
        assert_eq!(playlist.segments.len(), 2);
    }
}
