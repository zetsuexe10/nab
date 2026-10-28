use std::fs;
use std::path::{Path, PathBuf};

use super::synth::{self, Frame};
use super::{merge, mp4, pairs, sniff, webm, Container};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("snag-mux-{name}-{}", std::process::id()));
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
    merge(&dir.join("v.mp4"), &dir.join("a.m4a"), &out).unwrap();
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
    merge(&dir.join("v.mp4"), &dir.join("a.m4a"), &out).unwrap();
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
    merge(&dir.join("v.mp4"), &dir.join("a.m4a"), &out).unwrap();
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
    merge(&dir.join("v.webm"), &dir.join("a.weba"), &out).unwrap();
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
    assert!(merge(&dir.join("v.mp4"), &dir.join("a.weba"), &out).is_err());
    assert!(merge(&dir.join("junk"), &dir.join("junk"), &out).is_err());
    let whole = fs::read(dir.join("v.mp4")).unwrap();
    fs::write(dir.join("cut.mp4"), &whole[..whole.len() - 30]).unwrap();
    fs::write(
        dir.join("a.m4a"),
        synth::fragmented(false, &synth::frames(false, 4, 2), 44100, 1024),
    )
    .unwrap();
    assert!(merge(&dir.join("cut.mp4"), &dir.join("a.m4a"), &out).is_err());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn pairs_matching_containers_only() {
    assert!(pairs("mp4", "m4a"));
    assert!(pairs("webm", "weba"));
    assert!(!pairs("mp4", "weba"));
    assert!(!pairs("webm", "m4a"));
    assert_eq!(Container::for_video("webm"), Container::WebM);
    assert_eq!(Container::for_video("mp4").extension(), "mp4");
}
