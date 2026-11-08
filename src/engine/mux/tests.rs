use std::fs;
use std::path::{Path, PathBuf};

use super::synth::{self, Frame};
use super::{merge, merge_into, mp4, pairs, sniff, webm, Container};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nab-mux-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn payloads(part: &mut mp4::Part) -> Vec<Vec<u8>> {
    let samples = part.track.samples.clone();
    samples
        .iter()
        .map(|sample| {
            part.reader
                .vec_at(sample.offset, u64::from(sample.size))
                .unwrap()
        })
        .collect()
}

fn check_mp4(out: &Path, video: &[Frame], audio: &[Frame]) {
    let mut v = mp4::demux(out).unwrap().into_part(b"vide").unwrap();
    let mut a = mp4::demux(out).unwrap().into_part(b"soun").unwrap();
    assert_eq!(v.track.samples.len(), video.len());
    assert_eq!(a.track.samples.len(), audio.len());
    let got = payloads(&mut v);
    for (index, frame) in video.iter().enumerate() {
        assert_eq!(got[index], frame.data);
        assert_eq!(v.track.samples[index].sync, frame.key);
        assert_eq!(v.track.samples[index].cto, frame.cto);
        assert_eq!(v.track.samples[index].dur, 512);
    }
    let got = payloads(&mut a);
    for (index, frame) in audio.iter().enumerate() {
        assert_eq!(got[index], frame.data);
        assert_eq!(a.track.samples[index].dur, 1024);
    }
    assert_eq!(v.track.timescale, 15360);
    assert_eq!(a.track.timescale, 44100);
    assert!(!v.brands.contains(b"dash"));
}

#[test]
fn merges_fragmented_mp4() {
    let dir = scratch("frag");
    let video = synth::frames(true, 9, 1);
    let audio = synth::frames(false, 14, 2);
    fs::write(
        dir.join("v.mp4"),
        synth::fragmented(true, &video, 15360, 512),
    )
    .unwrap();
    fs::write(
        dir.join("a.m4a"),
        synth::fragmented(false, &audio, 44100, 1024),
    )
    .unwrap();
    let out = dir.join("o.mp4");
    merge(&dir.join("v.mp4"), &dir.join("a.m4a"), &out, None).unwrap();
    check_mp4(&out, &video, &audio);
    assert_eq!(sniff(&out).unwrap(), Container::Mp4);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn merges_progressive_mp4() {
    let dir = scratch("prog");
    let video = synth::frames(true, 10, 3);
    let audio = synth::frames(false, 11, 4);
    fs::write(
        dir.join("v.mp4"),
        synth::progressive(true, &video, 15360, 512),
    )
    .unwrap();
    fs::write(
        dir.join("a.m4a"),
        synth::progressive(false, &audio, 44100, 1024),
    )
    .unwrap();
    let out = dir.join("o.mp4");
    merge(&dir.join("v.mp4"), &dir.join("a.m4a"), &out, None).unwrap();
    check_mp4(&out, &video, &audio);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn mixes_fragmented_and_progressive() {
    let dir = scratch("mixed");
    let video = synth::frames(true, 7, 5);
    let audio = synth::frames(false, 5, 6);
    fs::write(
        dir.join("v.mp4"),
        synth::fragmented(true, &video, 15360, 512),
    )
    .unwrap();
    fs::write(
        dir.join("a.m4a"),
        synth::progressive(false, &audio, 44100, 1024),
    )
    .unwrap();
    let out = dir.join("o.mp4");
    merge(&dir.join("v.mp4"), &dir.join("a.m4a"), &out, None).unwrap();
    check_mp4(&out, &video, &audio);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn merges_webm() {
    let dir = scratch("webm");
    let video: Vec<(i64, Frame)> = synth::frames(true, 12, 7)
        .into_iter()
        .enumerate()
        .map(|(index, frame)| (index as i64 * 33, frame))
        .collect();
    let audio: Vec<(i64, Frame)> = synth::frames(false, 20, 8)
        .into_iter()
        .enumerate()
        .map(|(index, frame)| (index as i64 * 20, frame))
        .collect();
    fs::write(dir.join("v.webm"), synth::webm(true, &video)).unwrap();
    fs::write(dir.join("a.weba"), synth::webm(false, &audio)).unwrap();
    let out = dir.join("o.webm");
    merge(&dir.join("v.webm"), &dir.join("a.weba"), &out, None).unwrap();
    assert_eq!(sniff(&out).unwrap(), Container::WebM);
    let mut v = webm::demux_for_test(&out, 1).unwrap();
    let mut a = webm::demux_for_test(&out, 2).unwrap();
    assert_eq!(v.count(), video.len());
    assert_eq!(a.count(), audio.len());
    for (index, (time, frame)) in video.iter().enumerate() {
        let (stamp, key, data) = v.block(index);
        assert_eq!((stamp, key), (*time, frame.key));
        assert_eq!(data, frame.data);
    }
    for (index, (time, frame)) in audio.iter().enumerate() {
        let (stamp, _, data) = a.block(index);
        assert_eq!(stamp, *time);
        assert_eq!(data, frame.data);
    }
    assert!(v.has_cues());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn rejects_mismatched_and_broken_input() {
    let dir = scratch("bad");
    let video = synth::frames(true, 4, 1);
    let audio: Vec<(i64, Frame)> = synth::frames(false, 4, 1)
        .into_iter()
        .enumerate()
        .map(|(index, frame)| (index as i64 * 20, frame))
        .collect();
    fs::write(
        dir.join("v.mp4"),
        synth::fragmented(true, &video, 15360, 512),
    )
    .unwrap();
    fs::write(dir.join("a.weba"), synth::webm(false, &audio)).unwrap();
    fs::write(dir.join("junk"), b"not media at all").unwrap();
    let out = dir.join("o");
    assert!(merge(&dir.join("v.mp4"), &dir.join("a.weba"), &out, None).is_err());
    assert!(merge(&dir.join("junk"), &dir.join("junk"), &out, None).is_err());
    let whole = fs::read(dir.join("v.mp4")).unwrap();
    fs::write(dir.join("cut.mp4"), &whole[..whole.len() - 30]).unwrap();
    fs::write(
        dir.join("a.m4a"),
        synth::fragmented(false, &synth::frames(false, 4, 2), 44100, 1024),
    )
    .unwrap();
    assert!(merge(&dir.join("cut.mp4"), &dir.join("a.m4a"), &out, None).is_err());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn pairs_matching_containers_only() {
    assert!(pairs("mp4", "m4a"));
    assert!(pairs("webm", "weba"));
    assert!(pairs("MKV", "mka"));
    assert!(!pairs("mkv", "m4a"));
    assert!(!pairs("mp4", "weba"));
    assert!(!pairs("webm", "m4a"));
    assert_eq!(Container::for_video("webm"), Container::WebM);
    assert_eq!(Container::for_video("mp4").extension(), "mp4");
}

fn survives(dir: &Path, video: &[u8], audio: &[u8], v_ext: &str, a_ext: &str) {
    let v = dir.join(format!("v.{v_ext}"));
    let a = dir.join(format!("a.{a_ext}"));
    let out = dir.join("out.bin");
    fs::write(&v, video).unwrap();
    fs::write(&a, audio).unwrap();
    let _ = merge(&v, &a, &out, None);
}

#[test]
fn damaged_inputs_fail_cleanly_instead_of_panicking() {
    let dir = scratch("damaged");
    let video = synth::frames(true, 6, 1);
    let audio = synth::frames(false, 9, 2);
    let sets: [(Vec<u8>, Vec<u8>, &str, &str); 3] = [
        (
            synth::progressive(true, &video, 15360, 512),
            synth::progressive(false, &audio, 44100, 1024),
            "mp4",
            "m4a",
        ),
        (
            synth::fragmented(true, &video, 15360, 512),
            synth::fragmented(false, &audio, 44100, 1024),
            "mp4",
            "m4a",
        ),
        (
            synth::webm(
                true,
                &synth::frames(true, 6, 1)
                    .into_iter()
                    .enumerate()
                    .map(|(index, frame)| (index as i64 * 40, frame))
                    .collect::<Vec<_>>(),
            ),
            synth::webm(
                false,
                &synth::frames(false, 9, 2)
                    .into_iter()
                    .enumerate()
                    .map(|(index, frame)| (index as i64 * 23, frame))
                    .collect::<Vec<_>>(),
            ),
            "webm",
            "weba",
        ),
    ];
    for (v, a, v_ext, a_ext) in &sets {
        for cut in (0..v.len()).step_by(3) {
            survives(&dir, &v[..cut], a, v_ext, a_ext);
        }
        for at in (0..v.len()).step_by(5) {
            let mut bent = v.clone();
            bent[at] = bent[at].wrapping_add(0x55);
            survives(&dir, &bent, a, v_ext, a_ext);
            let mut bent = a.clone();
            let at = at % a.len();
            bent[at] ^= 0xff;
            survives(&dir, v, &bent, v_ext, a_ext);
        }
    }
}

#[test]
fn extracts_the_audio_track_alone_from_joined_and_fragmented_mp4() {
    let dir = scratch("extract");
    let video = synth::frames(true, 9, 1);
    let audio = synth::frames(false, 14, 2);
    let joined = dir.join("joined.mp4");
    fs::write(
        dir.join("v.mp4"),
        synth::progressive(true, &video, 15360, 512),
    )
    .unwrap();
    fs::write(
        dir.join("a.m4a"),
        synth::fragmented(false, &audio, 44100, 1024),
    )
    .unwrap();
    merge(&dir.join("v.mp4"), &dir.join("a.m4a"), &joined, None).unwrap();
    for source in [joined.clone(), dir.join("a.m4a")] {
        let out = dir.join("only.m4a");
        super::extract_audio(&source, &out, None).unwrap();
        assert_eq!(sniff(&out).unwrap(), Container::Mp4);
        let mut part = mp4::demux(&out).unwrap().into_part(b"soun").unwrap();
        assert!(mp4::demux(&out).unwrap().into_part(b"vide").is_err());
        assert_eq!(part.track.samples.len(), audio.len());
        let got = payloads(&mut part);
        for (index, frame) in audio.iter().enumerate() {
            assert_eq!(got[index], frame.data);
        }
    }
    let first = synth::frames(true, 1, 1).remove(0);
    fs::write(dir.join("v.webm"), synth::webm(true, &[(0, first)])).unwrap();
    assert!(super::extract_audio(&dir.join("v.webm"), &dir.join("x.m4a"), None).is_err());
    assert!(super::extract_audio(&dir.join("v.mp4"), &dir.join("x.m4a"), None).is_err());
}

fn timed(video: bool, count: usize, seed: u8, step: i64) -> Vec<(i64, Frame)> {
    synth::frames(video, count, seed)
        .into_iter()
        .enumerate()
        .map(|(index, frame)| (index as i64 * step, frame))
        .collect()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn ms(ticks: i64, scale: i64) -> i64 {
    (ticks * 1000 + scale / 2) / scale
}

#[test]
fn remuxes_mp4_into_matroska() {
    let dir = scratch("mkv-from-mp4");
    let video = synth::frames(true, 10, 3);
    let audio = synth::frames(false, 12, 4);
    for fragmented in [true, false] {
        let (v, a) = if fragmented {
            (
                synth::fragmented(true, &video, 15360, 512),
                synth::fragmented(false, &audio, 44100, 1024),
            )
        } else {
            (
                synth::progressive(true, &video, 15360, 512),
                synth::progressive(false, &audio, 44100, 1024),
            )
        };
        fs::write(dir.join("v.mp4"), v).unwrap();
        fs::write(dir.join("a.m4a"), a).unwrap();
        let out = dir.join("o.mkv");
        merge_into(
            &dir.join("v.mp4"),
            &dir.join("a.m4a"),
            &out,
            Container::Mkv,
            None,
        )
        .unwrap();
        assert_eq!(sniff(&out).unwrap(), Container::Mkv);
        let bytes = fs::read(&out).unwrap();
        assert!(contains(&bytes, b"V_MPEG4/ISO/AVC"));
        assert!(contains(&bytes, b"A_AAC"));
        assert!(contains(&bytes, &synth::AVC_CONFIG));
        assert!(contains(&bytes, &synth::AAC_CONFIG));
        let mut v = webm::demux_for_test(&out, 1).unwrap();
        let mut a = webm::demux_for_test(&out, 2).unwrap();
        assert_eq!((v.count(), a.count()), (video.len(), audio.len()));
        for (index, frame) in video.iter().enumerate() {
            let (stamp, key, data) = v.block(index);
            let due = ms(index as i64 * 512 + frame.cto as i64, 15360);
            assert_eq!((stamp, key, data), (due, frame.key, frame.data.clone()));
        }
        for (index, frame) in audio.iter().enumerate() {
            let (stamp, _, data) = a.block(index);
            assert_eq!(stamp, ms(index as i64 * 1024, 44100));
            assert_eq!(data, frame.data);
        }
        assert!(v.has_cues());
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn matroska_audio_only_files_keep_their_cues() {
    let dir = scratch("mka");
    let audio = synth::frames(false, 30, 6);
    fs::write(
        dir.join("a.m4a"),
        synth::fragmented(false, &audio, 44100, 1024),
    )
    .unwrap();
    let out = dir.join("only.mka");
    super::extract_audio_into(&dir.join("a.m4a"), &out, Container::Mkv, None).unwrap();
    assert_eq!(sniff(&out).unwrap(), Container::Mkv);
    let mut probe = webm::demux_for_test(&out, 2).unwrap();
    assert_eq!(probe.count(), audio.len());
    assert_eq!(probe.block(5).2, audio[5].data);
    assert!(probe.has_cues());
    assert!(webm::demux_for_test(&out, 1).is_err());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn webm_streams_can_be_stored_as_matroska_and_back_out() {
    let dir = scratch("webm-to-mkv");
    let video = timed(true, 9, 7, 33);
    let audio = timed(false, 15, 8, 20);
    fs::write(dir.join("v.webm"), synth::webm(true, &video)).unwrap();
    fs::write(dir.join("a.weba"), synth::webm(false, &audio)).unwrap();
    let mkv = dir.join("o.mkv");
    merge_into(
        &dir.join("v.webm"),
        &dir.join("a.weba"),
        &mkv,
        Container::Mkv,
        None,
    )
    .unwrap();
    assert_eq!(sniff(&mkv).unwrap(), Container::Mkv);
    let plain = dir.join("o.webm");
    merge(&dir.join("v.webm"), &dir.join("a.weba"), &plain, None).unwrap();
    assert_eq!(sniff(&plain).unwrap(), Container::WebM);
    assert_eq!(super::track_counts(&mkv).unwrap(), (9, 15));
    for (source, target, name) in [
        (&mkv, Container::Mkv, "x.mka"),
        (&plain, Container::WebM, "x.weba"),
    ] {
        let out = dir.join(name);
        super::extract_audio_into(source, &out, target, None).unwrap();
        assert_eq!(sniff(&out).unwrap(), target);
        let mut only = webm::demux_for_test(&out, 2).unwrap();
        assert_eq!(only.count(), audio.len());
        assert_eq!(only.block(3).2, audio[3].1.data);
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn refuses_conversions_it_cannot_do_honestly() {
    let dir = scratch("no-convert");
    let video = timed(true, 4, 1, 33);
    let audio = timed(false, 4, 2, 20);
    fs::write(dir.join("v.webm"), synth::webm(true, &video)).unwrap();
    fs::write(dir.join("a.weba"), synth::webm(false, &audio)).unwrap();
    fs::write(
        dir.join("v.mp4"),
        synth::fragmented(true, &synth::frames(true, 4, 1), 15360, 512),
    )
    .unwrap();
    fs::write(
        dir.join("a.m4a"),
        synth::fragmented(false, &synth::frames(false, 4, 2), 44100, 1024),
    )
    .unwrap();
    let out = dir.join("o");
    assert!(merge_into(
        &dir.join("v.webm"),
        &dir.join("a.weba"),
        &out,
        Container::Mp4,
        None
    )
    .is_err());
    assert!(merge_into(
        &dir.join("v.mp4"),
        &dir.join("a.m4a"),
        &out,
        Container::WebM,
        None
    )
    .is_err());
    assert!(super::extract_audio_into(&dir.join("a.m4a"), &out, Container::WebM, None).is_err());
    assert!(super::extract_audio_into(&dir.join("a.weba"), &out, Container::Mp4, None).is_err());
    assert_eq!(Container::parse("MKV"), Some(Container::Mkv));
    assert_eq!(Container::parse("mka"), Some(Container::Mkv));
    assert_eq!(Container::parse("avi"), None);
    assert_eq!(Container::for_video("mkv"), Container::Mkv);
    assert_eq!(Container::Mkv.audio_extension(), "mka");
    let _ = fs::remove_dir_all(&dir);
}

fn has(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle)
}

fn sample_meta() -> crate::model::Meta {
    crate::model::Meta {
        title: Some("Blue Hour".into()),
        artist: Some("Ana Lima; Sam Roe".into()),
        album: Some("Night Shift".into()),
        album_artist: Some("Ana Lima".into()),
        composer: Some("Ana Lima".into()),
        track: Some(3),
        track_total: Some(9),
        date: Some("2021-03-05".into()),
        comment: Some("liner notes".into()),
        url: Some("https://www.youtube.com/watch?v=abcdefghijk".into()),
        cover: crate::model::Cover::sniff(vec![0xff, 0xd8, 0xff, 0xe0, 1, 2, 3, 4]),
        ..crate::model::Meta::default()
    }
}

#[test]
fn mp4_tags_are_written_and_the_streams_survive() {
    let dir = scratch("tags-mp4");
    let video = synth::frames(true, 9, 1);
    let audio = synth::frames(false, 14, 2);
    fs::write(
        dir.join("v.mp4"),
        synth::fragmented(true, &video, 15360, 512),
    )
    .unwrap();
    fs::write(
        dir.join("a.m4a"),
        synth::fragmented(false, &audio, 44100, 1024),
    )
    .unwrap();
    let out = dir.join("o.mp4");
    let meta = sample_meta();
    merge(&dir.join("v.mp4"), &dir.join("a.m4a"), &out, Some(&meta)).unwrap();
    check_mp4(&out, &video, &audio);
    let bytes = fs::read(&out).unwrap();
    for needle in [
        &b"ilst"[..],
        &b"\xa9nam"[..],
        &b"Blue Hour"[..],
        &b"\xa9ART"[..],
        &b"Ana Lima; Sam Roe"[..],
        &b"aART"[..],
        &b"\xa9alb"[..],
        &b"Night Shift"[..],
        &b"\xa9day"[..],
        &b"2021-03-05"[..],
        &b"\xa9wrt"[..],
        &b"trkn"[..],
        &b"covr"[..],
        &[0xff, 0xd8, 0xff, 0xe0, 1, 2, 3, 4][..],
        &b"liner notes\n\nhttps://www.youtube.com/watch?v=abcdefghijk"[..],
    ] {
        assert!(has(&bytes, needle), "{}", String::from_utf8_lossy(needle));
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn untagged_output_has_no_metadata_atoms() {
    let dir = scratch("tags-none");
    let video = synth::frames(true, 5, 1);
    let audio = synth::frames(false, 6, 2);
    fs::write(
        dir.join("v.mp4"),
        synth::fragmented(true, &video, 15360, 512),
    )
    .unwrap();
    fs::write(
        dir.join("a.m4a"),
        synth::fragmented(false, &audio, 44100, 1024),
    )
    .unwrap();
    let out = dir.join("o.mp4");
    merge(&dir.join("v.mp4"), &dir.join("a.m4a"), &out, None).unwrap();
    assert!(!has(&fs::read(&out).unwrap(), b"ilst"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn retagging_audio_only_mp4_keeps_every_sample() {
    let dir = scratch("tags-retag-mp4");
    let audio = synth::frames(false, 21, 4);
    fs::write(
        dir.join("a.m4a"),
        synth::fragmented(false, &audio, 44100, 1024),
    )
    .unwrap();
    let out = dir.join("t.m4a");
    super::retag(&dir.join("a.m4a"), &out, &sample_meta()).unwrap();
    assert_eq!(sniff(&out).unwrap(), Container::Mp4);
    let mut part = mp4::demux(&out).unwrap().into_part(b"soun").unwrap();
    assert!(mp4::demux(&out).unwrap().into_part(b"vide").is_err());
    let got = payloads(&mut part);
    assert_eq!(got.len(), audio.len());
    for (index, frame) in audio.iter().enumerate() {
        assert_eq!(got[index], frame.data);
    }
    assert!(has(&fs::read(&out).unwrap(), b"Night Shift"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn matroska_tags_and_cover_are_written_and_the_streams_survive() {
    let dir = scratch("tags-mkv");
    let video = timed(true, 9, 7, 33);
    let audio = timed(false, 15, 8, 20);
    fs::write(dir.join("v.webm"), synth::webm(true, &video)).unwrap();
    fs::write(dir.join("a.weba"), synth::webm(false, &audio)).unwrap();
    let out = dir.join("o.mkv");
    let meta = sample_meta();
    merge_into(
        &dir.join("v.webm"),
        &dir.join("a.weba"),
        &out,
        Container::Mkv,
        Some(&meta),
    )
    .unwrap();
    assert_eq!(super::track_counts(&out).unwrap(), (9, 15));
    assert!(webm::demux_for_test(&out, 1).unwrap().has_cues());
    let bytes = fs::read(&out).unwrap();
    for needle in [
        &b"TITLE"[..],
        &b"Blue Hour"[..],
        &b"ARTIST"[..],
        &b"ALBUM"[..],
        &b"Night Shift"[..],
        &b"DATE_RELEASED"[..],
        &b"PART_NUMBER"[..],
        &b"cover.jpg"[..],
        &b"image/jpeg"[..],
        &[0x19, 0x41, 0xA4, 0x69][..],
    ] {
        assert!(has(&bytes, needle), "{}", String::from_utf8_lossy(needle));
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn webm_keeps_tags_but_never_attachments() {
    let dir = scratch("tags-webm");
    let video = timed(true, 9, 7, 33);
    let audio = timed(false, 15, 8, 20);
    fs::write(dir.join("v.webm"), synth::webm(true, &video)).unwrap();
    fs::write(dir.join("a.weba"), synth::webm(false, &audio)).unwrap();
    let out = dir.join("o.webm");
    merge(
        &dir.join("v.webm"),
        &dir.join("a.weba"),
        &out,
        Some(&sample_meta()),
    )
    .unwrap();
    assert_eq!(sniff(&out).unwrap(), Container::WebM);
    assert_eq!(super::track_counts(&out).unwrap(), (9, 15));
    let bytes = fs::read(&out).unwrap();
    assert!(has(&bytes, b"Blue Hour"));
    assert!(!has(&bytes, &[0x19, 0x41, 0xA4, 0x69]));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn retagging_webm_audio_keeps_every_block() {
    let dir = scratch("tags-retag-webm");
    let audio = timed(false, 30, 6, 20);
    fs::write(dir.join("a.weba"), synth::webm(false, &audio)).unwrap();
    let out = dir.join("t.weba");
    super::retag(&dir.join("a.weba"), &out, &sample_meta()).unwrap();
    assert_eq!(sniff(&out).unwrap(), Container::WebM);
    let mut probe = webm::demux_for_test(&out, 2).unwrap();
    assert_eq!(probe.count(), audio.len());
    assert_eq!(probe.block(7).2, audio[7].1.data);
    assert!(webm::demux_for_test(&out, 1).is_err());
    assert!(has(&fs::read(&out).unwrap(), b"Blue Hour"));
    let _ = fs::remove_dir_all(&dir);
}
