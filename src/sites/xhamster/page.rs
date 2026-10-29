use super::sources::Source;

pub fn sources(html: &str) -> Vec<Source> {
    let text = html.replace("\\/", "/");
    let mut found: Vec<Source> = Vec::new();
    let mut from = 0;
    while let Some(at) = next_url(&text, from) {
        let end = text[at..]
            .find(|c: char| {
                c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | '<' | '>' | '(' | ')')
            })
            .map(|offset| at + offset)
            .unwrap_or(text.len());
        from = end.max(at + 1);
        let url = text[at..end].trim_end_matches(['.', ',', ';', ':']);
        let lower = url.to_ascii_lowercase();
        let hls = has_extension(&lower, ".m3u8");
        let mp4 = !hls && has_extension(&lower, ".mp4");
        if !hls && !mp4 {
            continue;
        }
        let host = lower.split("://").nth(1).unwrap_or("");
        if host.starts_with("thumb-") || host.starts_with("thumb.") {
            continue;
        }
        if has_extension(&lower, ".t.mp4") || has_extension(&lower, ".t.webm") {
            continue;
        }
        if found.iter().any(|source| source.url == url) {
            continue;
        }
        let height = heights(url).into_iter().max().unwrap_or(0);
        let codec = ["av1", "h265", "h264", "vp9"]
            .iter()
            .find(|name| lower.contains(*name))
            .copied()
            .unwrap_or("h264");
        found.push(Source {
            url: url.to_string(),
            height,
            hls,
            codec: codec.to_string(),
            bitrate: None,
            vr: false,
        });
    }
    found.sort_by(|a, b| a.hls.cmp(&b.hls).then(b.height.cmp(&a.height)));
    found
}

fn next_url(text: &str, from: usize) -> Option<usize> {
    let slice = &text[from..];
    let http = slice.find("http://");
    let https = slice.find("https://");
    let at = match (http, https) {
        (Some(a), Some(b)) => a.min(b),
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => return None,
    };
    Some(from + at)
}

fn has_extension(lower: &str, extension: &str) -> bool {
    lower.match_indices(extension).any(|(at, _)| {
        matches!(
            lower.as_bytes().get(at + extension.len()),
            None | Some(b'?')
        )
    })
}

fn heights(url: &str) -> Vec<u32> {
    let bytes = url.as_bytes();
    let mut found = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index].is_ascii_digit() {
            let start = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            let digits = index - start;
            if (3..=4).contains(&digits) && bytes.get(index) == Some(&b'p') {
                if let Ok(value) = url[start..index].parse() {
                    found.push(value);
                }
            }
        } else {
            index += 1;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrapes_streams_out_of_markup_and_skips_previews() {
        let html = r#"
            <link rel="preload" href="https:\/\/video.cdn.test\/a\/1080p.h264.mp4.m3u8?k=1">
            <script>var u="https://video.cdn.test/a/720p.h264.mp4";var v="https://video.cdn.test/a/480p.av1.mp4?x=2";</script>
            <img src="https://thumb-p1.cdn.test/x.jpg"><video src="https://thumb-p1.cdn.test/a.mp4">
            <a href="https://video.cdn.test/a/preview.t.mp4">p</a>
            <a href="https://video.cdn.test/a/720p.h264.mp4">dup</a>
            <a href="https://video.cdn.test/doc.html">no</a>
        "#;
        let found = sources(html);
        let described: Vec<(u32, bool, &str)> = found
            .iter()
            .map(|source| (source.height, source.hls, source.codec.as_str()))
            .collect();
        assert_eq!(
            described,
            vec![
                (720, false, "h264"),
                (480, false, "av1"),
                (1080, true, "h264")
            ]
        );
        assert_eq!(
            found[2].url,
            "https://video.cdn.test/a/1080p.h264.mp4.m3u8?k=1"
        );
    }

    #[test]
    fn no_streams_means_an_empty_list() {
        assert!(sources("<html>nothing here</html>").is_empty());
    }
}
