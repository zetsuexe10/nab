const MIN_HEX: usize = 12;

fn constant(value: u32) -> i32 {
    value as i32
}

fn shr(value: i32, bits: u32) -> i32 {
    ((value as u32) >> bits) as i32
}

fn generator(id: u8, seed: i32) -> Option<Box<dyn FnMut() -> u8>> {
    let mut state = seed;
    let next: Box<dyn FnMut() -> u8> = match id {
        1 => Box::new(move || {
            state = state
                .wrapping_mul(1_664_525)
                .wrapping_add(constant(0x3c6e_f35f));
            (state & 0xff) as u8
        }),
        2 => Box::new(move || {
            state ^= state << 13;
            state ^= shr(state, 17);
            state ^= state << 5;
            (state & 0xff) as u8
        }),
        3 => Box::new(move || {
            state = state.wrapping_add(constant(0x9e37_79b9));
            let mut mixed = state;
            mixed ^= shr(mixed, 16);
            mixed = mixed.wrapping_mul(constant(0x85eb_ca77));
            mixed ^= shr(mixed, 13);
            mixed = mixed.wrapping_mul(constant(0xc2b2_ae3d));
            mixed ^= shr(mixed, 16);
            (mixed & 0xff) as u8
        }),
        4 => Box::new(move || {
            state = state.wrapping_add(constant(0x6d2b_79f5));
            let mut mixed = (state << 7) | shr(state, 25);
            mixed = mixed.wrapping_add(constant(0x9e37_79b9));
            mixed ^= shr(mixed, 11);
            mixed = mixed.wrapping_mul(constant(0x27d4_eb2d));
            (mixed & 0xff) as u8
        }),
        5 => Box::new(move || {
            state ^= state << 7;
            state ^= shr(state, 9);
            state ^= state << 8;
            state = state.wrapping_add(constant(0xa5a5_a5a5));
            (state & 0xff) as u8
        }),
        6 => Box::new(move || {
            state = state
                .wrapping_mul(constant(0x2c92_77b5))
                .wrapping_add(constant(0xac56_4b05));
            let mixed = (state ^ shr(state, 18)) as u32;
            ((mixed >> (shr(state, 27) & 31)) & 0xff) as u8
        }),
        7 => Box::new(move || {
            state = state.wrapping_add(constant(0x9e37_79b9));
            let mut mixed = state ^ (state << 5);
            mixed = mixed.wrapping_mul(constant(0x7feb_352d));
            mixed ^= shr(mixed, 15);
            mixed = mixed.wrapping_mul(constant(0x846c_a68b));
            (mixed & 0xff) as u8
        }),
        _ => return None,
    };
    Some(next)
}

pub fn is_blob(value: &str) -> bool {
    value.len() >= MIN_HEX
        && value.len().is_multiple_of(2)
        && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn unhex(value: &str) -> Option<Vec<u8>> {
    value
        .as_bytes()
        .chunks(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).ok()?;
            u8::from_str_radix(text, 16).ok()
        })
        .collect()
}

pub fn decipher(blob: &str) -> Option<String> {
    if !is_blob(blob) {
        return None;
    }
    let bytes = unhex(blob)?;
    if bytes.len() < 6 {
        return None;
    }
    let seed = i32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
    let mut next = generator(bytes[0], seed)?;
    Some(
        bytes[5..]
            .iter()
            .map(|byte| (byte ^ next()) as char)
            .collect(),
    )
}

fn playable(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return false;
    }
    if url
        .chars()
        .any(|c| c.is_whitespace() || matches!(c as u32, 0..=8 | 14..=31))
    {
        return false;
    }
    [".mp4", ".m3u8", ".ts", ".m4s", ".webm"].iter().any(|ext| {
        lower.match_indices(ext).any(|(at, _)| {
            matches!(
                lower.as_bytes().get(at + ext.len()),
                None | Some(b'?') | Some(b',')
            )
        })
    })
}

pub fn decipher_url(value: &str) -> Option<String> {
    if is_blob(value) {
        let plain = decipher(value)?;
        return playable(&plain).then_some(plain);
    }
    plain_url(value)
}

fn plain_url(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return None;
    }
    let after_scheme = value.find("://")? + 3;
    let path_start = after_scheme + value[after_scheme..].find('/')? + 1;
    let rest = &value[path_start..];
    let hex_len = rest.bytes().take_while(u8::is_ascii_hexdigit).count();
    let boundary = rest.as_bytes().get(hex_len);
    if hex_len >= MIN_HEX && matches!(boundary, Some(b'/') | Some(b',')) {
        let segment = &rest[..hex_len];
        let plain = decipher(segment)?;
        let rebuilt = format!("{}{}{}", &value[..path_start], plain, &rest[hex_len..]);
        return playable(&rebuilt).then_some(rebuilt);
    }
    playable(value).then(|| value.to_string())
}

#[cfg(test)]
pub fn encrypt(id: u8, seed: i32, plain: &str) -> String {
    let mut next = generator(id, seed).unwrap();
    let mut bytes = vec![id];
    bytes.extend_from_slice(&seed.to_le_bytes());
    bytes.extend(plain.bytes().map(|byte| byte ^ next()));
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const VECTORS: [(u8, &str); 7] = [
        (
            1,
            "01785634121f1eb55ce804aa0f9c7cf17b9289fafab74a7f149938215e62522d",
        ),
        (
            2,
            "02ffffffff778b37aae2d69fd4912197d30687d5b7532ad2482a2055d77ded77",
        ),
        (
            3,
            "037856341249f5a20ae9ea44b7dc94012a93fc0a4f4d165d304ae78172ac27ca",
        ),
        (
            4,
            "0400000000e59b0923e935569b8aa466b92c7f74fc4f991af145e3c7bbda55fa",
        ),
        (
            5,
            "054f9721c51490e7b01d6422a8ba0753f3eb4b702ef3fd050cf9df695233bc0a",
        ),
        (
            6,
            "062a00000069d2af0074e0561405e34ddc3834544179222d6b310f4f2f2a7e88",
        ),
        (
            7,
            "070700000068c0389db4db35246d7f9e37d6a1e60e64fef3fabb0db16f53b878",
        ),
    ];

    #[test]
    fn every_generator_matches_the_site_script() {
        for (id, blob) in VECTORS {
            assert_eq!(
                decipher(blob).as_deref(),
                Some(format!("https://c.x/{id}/720p.h264.mp4").as_str()),
                "generator {id}"
            );
        }
    }

    #[test]
    fn urls_are_validated_after_deciphering() {
        assert_eq!(
            decipher_url(VECTORS[2].1).as_deref(),
            Some("https://c.x/3/720p.h264.mp4")
        );
        assert_eq!(decipher_url("0a0b0c0d0e0f0a0b0c0d0e0f"), None);
        assert_eq!(decipher_url("0900000000aabbccddee"), None);
        assert_eq!(decipher("zz"), None);
        assert_eq!(decipher("010000"), None);
    }

    #[test]
    fn plain_urls_pass_and_leading_segments_are_deciphered() {
        assert_eq!(
            decipher_url("https://cdn.x/a/b/720p.mp4?x=1").as_deref(),
            Some("https://cdn.x/a/b/720p.mp4?x=1")
        );
        assert_eq!(decipher_url("https://cdn.x/page.html"), None);
        assert_eq!(decipher_url("ftp://cdn.x/a.mp4"), None);
        let bytes = VECTORS[0].1;
        let url = format!("https://cdn.x/{bytes}/more.mp4");
        assert_eq!(
            decipher_url(&url).as_deref(),
            Some("https://cdn.x/https://c.x/1/720p.h264.mp4/more.mp4")
        );
    }

    #[test]
    fn encrypt_round_trips_for_every_generator() {
        for id in 1..=7u8 {
            let blob = encrypt(id, -123_456, "https://cdn.test/a/720p.h264.mp4?x=1");
            assert_eq!(
                decipher_url(&blob).as_deref(),
                Some("https://cdn.test/a/720p.h264.mp4?x=1")
            );
        }
    }

    #[test]
    fn tricky_extension_boundaries() {
        assert!(playable("https://a/b.m3u8,x"));
        assert!(playable("https://a/b.ts"));
        assert!(!playable("https://a/b.mp4x"));
        assert!(!playable("https://a/b\u{1}.mp4"));
    }
}
