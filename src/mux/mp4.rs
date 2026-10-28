use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use super::bytes::{truncated, write_failed, Cur, Put, Res, Window};

const MAX_BOX: u64 = 1 << 28;
const MAX_SAMPLES: usize = 10_000_000;
const UNIX_TO_MP4: u64 = 2_082_844_800;
const MOVIE_SCALE: u32 = 1000;
const IDENTITY: [u8; 36] = [
    0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0x40, 0, 0, 0,
];
const FRAGMENT_BRANDS: [&[u8; 4]; 11] = [
    b"dash", b"msdh", b"msix", b"cmfc", b"cmf2", b"iso5", b"iso6", b"iso7", b"iso8", b"iso9",
    b"isoa",
];

#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub offset: u64,
    pub size: u32,
    pub dur: u32,
    pub cto: i32,
    pub sync: bool,
    pub desc: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct Edit {
    pub segment: u64,
    pub media_time: i64,
}

#[derive(Clone, Copy, Default)]
struct Defaults {
    desc: u32,
    dur: u32,
    size: u32,
    flags: u32,
}

pub struct Track {
    pub id: u32,
    pub handler: [u8; 4],
    pub timescale: u32,
    pub language: u16,
    pub stsd: Vec<u8>,
    pub hdlr: Vec<u8>,
    pub matrix: [u8; 36],
    pub width: u32,
    pub height: u32,
    pub edits: Vec<Edit>,
    pub start: u64,
    pub end: u64,
    pub shift: u64,
    pub samples: Vec<Sample>,
    defaults: Defaults,
}

impl Track {
    fn blank() -> Self {
        Track {
            id: 0,
            handler: [0; 4],
            timescale: 0,
            language: 0x55c4,
            stsd: Vec::new(),
            hdlr: Vec::new(),
            matrix: IDENTITY,
            width: 0,
            height: 0,
            edits: Vec::new(),
            start: 0,
            end: 0,
            shift: 0,
            samples: Vec::new(),
            defaults: Defaults::default(),
        }
    }

    pub fn duration(&self) -> u64 {
        self.samples.iter().map(|sample| sample.dur as u64).sum()
    }
}

pub struct Movie {
    pub reader: Window,
    pub brands: Vec<[u8; 4]>,
    pub timescale: u32,
    pub tracks: Vec<Track>,
}

pub struct Part {
    pub reader: Window,
    pub brands: Vec<[u8; 4]>,
    pub movie_scale: u32,
    pub track: Track,
}

impl Movie {
    pub fn into_part(mut self, handler: &[u8; 4]) -> Res<Part> {
        let index = self
            .tracks
            .iter()
            .position(|track| &track.handler == handler && !track.samples.is_empty())
            .ok_or_else(|| {
                format!(
                    "no {} track found",
                    if handler == b"vide" { "video" } else { "audio" }
                )
            })?;
        let mut track = self.tracks.swap_remove(index);
        if track.timescale == 0 || track.stsd.is_empty() {
            return Err("track header is missing".to_string());
        }
        if handler == b"soun" {
            for sample in &mut track.samples {
                sample.sync = true;
            }
        }
        let lowest = track
            .samples
            .iter()
            .map(|sample| sample.cto)
            .min()
            .unwrap_or(0);
        if lowest < 0 {
            let shift = -(lowest as i64);
            for sample in &mut track.samples {
                sample.cto = (sample.cto as i64 + shift) as i32;
            }
            track.shift = shift as u64;
        }
        let len = self.reader.len();
        if track
            .samples
            .iter()
            .any(|sample| sample.offset.saturating_add(sample.size as u64) > len)
        {
            return truncated();
        }
        Ok(Part {
            reader: self.reader,
            brands: self.brands,
            movie_scale: self.timescale,
            track,
        })
    }
}

struct BoxRef<'a> {
    kind: [u8; 4],
    body: &'a [u8],
    whole: &'a [u8],
}

fn children(buf: &[u8]) -> Res<Vec<BoxRef<'_>>> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while buf.len() - pos >= 8 {
        let size32 = u32::from_be_bytes([buf[pos], buf[pos + 1], buf[pos + 2], buf[pos + 3]]);
        let kind = [buf[pos + 4], buf[pos + 5], buf[pos + 6], buf[pos + 7]];
        let remaining = buf.len() - pos;
        let (header, size) = match size32 {
            0 => (8usize, remaining),
            1 => {
                if remaining < 16 {
                    return truncated();
                }
                let mut array = [0u8; 8];
                array.copy_from_slice(&buf[pos + 8..pos + 16]);
                let size = usize::try_from(u64::from_be_bytes(array))
                    .map_err(|_| "mp4 box is corrupt".to_string())?;
                (16usize, size)
            }
            other => (8usize, other as usize),
        };
        if size < header || size > remaining {
            return Err("mp4 box is corrupt".to_string());
        }
        out.push(BoxRef {
            kind,
            body: &buf[pos + header..pos + size],
            whole: &buf[pos..pos + size],
        });
        pos += size;
    }
    Ok(out)
}

fn find<'a>(boxes: &'a [BoxRef<'a>], kind: &[u8; 4]) -> Option<&'a BoxRef<'a>> {
    boxes.iter().find(|entry| &entry.kind == kind)
}

pub fn demux(path: &Path) -> Res<Movie> {
    let mut reader = Window::open(path)?;
    let len = reader.len();
    let mut brands: Vec<[u8; 4]> = Vec::new();
    let mut timescale = MOVIE_SCALE;
    let mut tracks: Vec<Track> = Vec::new();
    let mut defaults: Vec<(u32, Defaults)> = Vec::new();
    let mut seen_moov = false;
    let mut pos = 0u64;
    while len - pos >= 8 {
        let head = reader.vec_at(pos, 8)?;
        let size32 = u32::from_be_bytes([head[0], head[1], head[2], head[3]]);
        let kind = [head[4], head[5], head[6], head[7]];
        let (header, size) = match size32 {
            0 => (8u64, len - pos),
            1 => {
                if len - pos < 16 {
                    return truncated();
                }
                let wide = reader.vec_at(pos + 8, 8)?;
                let mut array = [0u8; 8];
                array.copy_from_slice(&wide);
                (16u64, u64::from_be_bytes(array))
            }
            other => (8u64, other as u64),
        };
        if size < header {
            return Err("mp4 box is corrupt".to_string());
        }
        let end = pos.saturating_add(size);
        let wanted = matches!(&kind, b"ftyp" | b"moov" | b"moof");
        if wanted {
            if end > len {
                return truncated();
            }
            if size - header > MAX_BOX {
                return Err("mp4 header is too large".to_string());
            }
            let body = reader.vec_at(pos + header, size - header)?;
            match &kind {
                b"ftyp" => read_brands(&body, &mut brands)?,
                b"moov" => {
                    seen_moov = true;
                    read_moov(&body, &mut timescale, &mut tracks, &mut defaults)?;
                }
                _ => {
                    if !seen_moov {
                        return Err("fragment appears before the movie header".to_string());
                    }
                    read_moof(&body, pos, &mut tracks)?;
                }
            }
        }
        if end > len {
            break;
        }
        pos = end;
    }
    if !seen_moov {
        return Err("mp4 has no movie header".to_string());
    }
    Ok(Movie {
        reader,
        brands,
        timescale,
        tracks,
    })
}

fn read_brands(body: &[u8], brands: &mut Vec<[u8; 4]>) -> Res<()> {
    let mut cur = Cur::new(body);
    let major = cur.take(4)?;
    cur.skip(4)?;
    let mut add = |bytes: &[u8]| {
        let brand = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if !brands.contains(&brand) {
            brands.push(brand);
        }
    };
    add(major);
    while cur.left() >= 4 {
        add(cur.take(4)?);
    }
    Ok(())
}

fn read_moov(
    body: &[u8],
    timescale: &mut u32,
    tracks: &mut Vec<Track>,
    defaults: &mut Vec<(u32, Defaults)>,
) -> Res<()> {
    for entry in children(body)? {
        match &entry.kind {
            b"mvhd" => {
                let mut cur = Cur::new(entry.body);
                let version = cur.u8()?;
                cur.skip(3)?;
                cur.skip(if version == 1 { 16 } else { 8 })?;
                let scale = cur.u32()?;
                if scale > 0 {
                    *timescale = scale;
                }
            }
            b"trak" => tracks.push(read_trak(entry.body)?),
            b"mvex" => {
                for trex in children(entry.body)? {
                    if &trex.kind != b"trex" {
                        continue;
                    }
                    let mut cur = Cur::new(trex.body);
                    cur.skip(4)?;
                    let id = cur.u32()?;
                    let value = Defaults {
                        desc: cur.u32()?,
                        dur: cur.u32()?,
                        size: cur.u32()?,
                        flags: cur.u32()?,
                    };
                    defaults.push((id, value));
                }
            }
            _ => {}
        }
    }
    for track in tracks.iter_mut() {
        if let Some((_, value)) = defaults.iter().find(|(id, _)| *id == track.id) {
            track.defaults = *value;
        }
    }
    Ok(())
}

fn read_trak(body: &[u8]) -> Res<Track> {
    let mut track = Track::blank();
    let parts = children(body)?;
    let tkhd = find(&parts, b"tkhd").ok_or_else(|| "track header is missing".to_string())?;
    {
        let mut cur = Cur::new(tkhd.body);
        let version = cur.u8()?;
        cur.skip(3)?;
        cur.skip(if version == 1 { 16 } else { 8 })?;
        track.id = cur.u32()?;
        cur.skip(4)?;
        cur.skip(if version == 1 { 8 } else { 4 })?;
        cur.skip(16)?;
        track.matrix.copy_from_slice(cur.take(36)?);
        track.width = cur.u32()?;
        track.height = cur.u32()?;
    }
    if let Some(edts) = find(&parts, b"edts") {
        for elst in children(edts.body)? {
            if &elst.kind == b"elst" {
                read_elst(elst.body, &mut track)?;
            }
        }
    }
    let mdia = find(&parts, b"mdia").ok_or_else(|| "track media is missing".to_string())?;
    let media = children(mdia.body)?;
    if let Some(mdhd) = find(&media, b"mdhd") {
        let mut cur = Cur::new(mdhd.body);
        let version = cur.u8()?;
        cur.skip(3)?;
        cur.skip(if version == 1 { 16 } else { 8 })?;
        track.timescale = cur.u32()?;
        cur.skip(if version == 1 { 8 } else { 4 })?;
        track.language = cur.u16()?;
    }
    if let Some(hdlr) = find(&media, b"hdlr") {
        let mut cur = Cur::new(hdlr.body);
        cur.skip(8)?;
        track.handler.copy_from_slice(cur.take(4)?);
        track.hdlr = hdlr.whole.to_vec();
    }
    let minf = find(&media, b"minf").ok_or_else(|| "track media is missing".to_string())?;
    let info = children(minf.body)?;
    if let Some(stbl) = find(&info, b"stbl") {
        read_stbl(stbl.body, &mut track)?;
    }
    Ok(track)
}

fn read_elst(body: &[u8], track: &mut Track) -> Res<()> {
    let mut cur = Cur::new(body);
    let version = cur.u8()?;
    cur.skip(3)?;
    let count = cur.u32()? as usize;
    for _ in 0..count {
        let (segment, media_time) = if version == 1 {
            (cur.u64()?, cur.i64()?)
        } else {
            (cur.u32()? as u64, cur.i32()? as i64)
        };
        cur.skip(4)?;
        track.edits.push(Edit {
            segment,
            media_time,
        });
    }
    Ok(())
}

fn read_stbl(body: &[u8], track: &mut Track) -> Res<()> {
    let parts = children(body)?;
    if let Some(stsd) = find(&parts, b"stsd") {
        track.stsd = stsd.whole.to_vec();
    }
    let Some(stsz) = find(&parts, b"stsz") else {
        return Ok(());
    };
    let mut cur = Cur::new(stsz.body);
    cur.skip(4)?;
    let uniform = cur.u32()?;
    let count = cur.u32()? as usize;
    if count == 0 {
        return Ok(());
    }
    if count > MAX_SAMPLES || (uniform == 0 && cur.left() / 4 < count) {
        return Err("sample table is corrupt".to_string());
    }
    let mut sizes = Vec::with_capacity(count);
    for _ in 0..count {
        sizes.push(if uniform == 0 { cur.u32()? } else { uniform });
    }
    let stts = find(&parts, b"stts").ok_or_else(|| "sample table is corrupt".to_string())?;
    let mut durs: Vec<u32> = Vec::with_capacity(count);
    {
        let mut cur = Cur::new(stts.body);
        cur.skip(4)?;
        let entries = cur.u32()? as usize;
        for _ in 0..entries {
            let repeat = cur.u32()? as usize;
            let delta = cur.u32()?;
            if durs.len() + repeat > count {
                return Err("sample table is corrupt".to_string());
            }
            durs.extend(std::iter::repeat_n(delta, repeat));
        }
    }
    if durs.len() != count {
        return Err("sample table is corrupt".to_string());
    }
    let mut ctos = vec![0i32; count];
    if let Some(ctts) = find(&parts, b"ctts") {
        let mut cur = Cur::new(ctts.body);
        cur.skip(4)?;
        let entries = cur.u32()? as usize;
        let mut at = 0usize;
        for _ in 0..entries {
            let repeat = cur.u32()? as usize;
            let offset = cur.i32()?;
            if at + repeat > count {
                return Err("sample table is corrupt".to_string());
            }
            ctos[at..at + repeat].fill(offset);
            at += repeat;
        }
    }
    let mut sync = vec![true; count];
    if let Some(stss) = find(&parts, b"stss") {
        sync.fill(false);
        let mut cur = Cur::new(stss.body);
        cur.skip(4)?;
        let entries = cur.u32()? as usize;
        for _ in 0..entries {
            let number = cur.u32()? as usize;
            if number == 0 || number > count {
                return Err("sample table is corrupt".to_string());
            }
            sync[number - 1] = true;
        }
    }
    let mut runs: Vec<(u32, u32, u32)> = Vec::new();
    let stsc = find(&parts, b"stsc").ok_or_else(|| "sample table is corrupt".to_string())?;
    {
        let mut cur = Cur::new(stsc.body);
        cur.skip(4)?;
        let entries = cur.u32()? as usize;
        for _ in 0..entries {
            runs.push((cur.u32()?, cur.u32()?, cur.u32()?));
        }
    }
    let mut offsets: Vec<u64> = Vec::new();
    if let Some(stco) = find(&parts, b"stco") {
        let mut cur = Cur::new(stco.body);
        cur.skip(4)?;
        let entries = cur.u32()? as usize;
        if cur.left() / 4 < entries {
            return truncated();
        }
        for _ in 0..entries {
            offsets.push(cur.u32()? as u64);
        }
    } else if let Some(co64) = find(&parts, b"co64") {
        let mut cur = Cur::new(co64.body);
        cur.skip(4)?;
        let entries = cur.u32()? as usize;
        if cur.left() / 8 < entries {
            return truncated();
        }
        for _ in 0..entries {
            offsets.push(cur.u64()?);
        }
    } else {
        return Err("sample table is corrupt".to_string());
    }
    let mut samples = Vec::with_capacity(count);
    let mut run = 0usize;
    for (index, &chunk_offset) in offsets.iter().enumerate() {
        let number = index as u32 + 1;
        while run + 1 < runs.len() && runs[run + 1].0 <= number {
            run += 1;
        }
        let Some(&(_, per_chunk, desc)) = runs.get(run) else {
            return Err("sample table is corrupt".to_string());
        };
        let mut offset = chunk_offset;
        for _ in 0..per_chunk {
            let at = samples.len();
            if at >= count {
                break;
            }
            samples.push(Sample {
                offset,
                size: sizes[at],
                dur: durs[at],
                cto: ctos[at],
                sync: sync[at],
                desc,
            });
            offset += sizes[at] as u64;
        }
    }
    if samples.len() != count {
        return Err("sample table is corrupt".to_string());
    }
    track.end = durs.iter().map(|&dur| dur as u64).sum();
    track.samples = samples;
    Ok(())
}

fn read_moof(body: &[u8], moof_start: u64, tracks: &mut [Track]) -> Res<()> {
    for traf in children(body)? {
        if &traf.kind != b"traf" {
            continue;
        }
        let parts = children(traf.body)?;
        let tfhd = find(&parts, b"tfhd").ok_or_else(|| "fragment header is missing".to_string())?;
        let mut cur = Cur::new(tfhd.body);
        let flags = cur.u32()? & 0x00ff_ffff;
        let id = cur.u32()?;
        let base = if flags & 0x1 != 0 {
            cur.u64()?
        } else {
            moof_start
        };
        let Some(track) = tracks.iter_mut().find(|track| track.id == id) else {
            continue;
        };
        let mut defaults = track.defaults;
        if flags & 0x2 != 0 {
            defaults.desc = cur.u32()?;
        }
        if flags & 0x8 != 0 {
            defaults.dur = cur.u32()?;
        }
        if flags & 0x10 != 0 {
            defaults.size = cur.u32()?;
        }
        if flags & 0x20 != 0 {
            defaults.flags = cur.u32()?;
        }
        if let Some(tfdt) = find(&parts, b"tfdt") {
            let mut cur = Cur::new(tfdt.body);
            let version = cur.u8()?;
            cur.skip(3)?;
            let decode = if version == 1 {
                cur.u64()?
            } else {
                cur.u32()? as u64
            };
            if track.samples.is_empty() {
                track.start = decode;
                track.end = decode;
            }
        }
        let mut next = base;
        for trun in parts.iter().filter(|entry| &entry.kind == b"trun") {
            let mut cur = Cur::new(trun.body);
            let head = cur.u32()?;
            let version = (head >> 24) as u8;
            let flags = head & 0x00ff_ffff;
            let count = cur.u32()? as usize;
            if track.samples.len() + count > MAX_SAMPLES {
                return Err("fragment is corrupt".to_string());
            }
            if flags & 0x1 != 0 {
                let delta = cur.i32()? as i64;
                next = (base as i64 + delta) as u64;
            }
            let first_flags = if flags & 0x4 != 0 {
                Some(cur.u32()?)
            } else {
                None
            };
            for index in 0..count {
                let dur = if flags & 0x100 != 0 {
                    cur.u32()?
                } else {
                    defaults.dur
                };
                let size = if flags & 0x200 != 0 {
                    cur.u32()?
                } else {
                    defaults.size
                };
                let sample_flags = if flags & 0x400 != 0 {
                    cur.u32()?
                } else if index == 0 {
                    first_flags.unwrap_or(defaults.flags)
                } else {
                    defaults.flags
                };
                let cto = if flags & 0x800 != 0 {
                    if version == 0 {
                        cur.u32()? as i32
                    } else {
                        cur.i32()?
                    }
                } else {
                    0
                };
                track.samples.push(Sample {
                    offset: next,
                    size,
                    dur,
                    cto,
                    sync: sample_flags & 0x0001_0000 == 0,
                    desc: defaults.desc.max(1),
                });
                track.end += dur as u64;
                next += size as u64;
            }
        }
    }
    Ok(())
}

struct Chunk {
    part: usize,
    first: usize,
    count: usize,
    desc: u32,
    time: f64,
    offset: u64,
}

fn plan(parts: &[Part]) -> (Vec<Chunk>, u64) {
    let mut chunks: Vec<Chunk> = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        let scale = part.track.timescale as u64;
        let target = (scale / 2).max(1);
        let samples = &part.track.samples;
        let mut clock = part.track.start;
        let mut at = 0usize;
        while at < samples.len() {
            let first = at;
            let desc = samples[at].desc;
            let begin = clock;
            let mut spent = 0u64;
            while at < samples.len() && samples[at].desc == desc && spent < target {
                spent += samples[at].dur as u64;
                clock += samples[at].dur as u64;
                at += 1;
            }
            chunks.push(Chunk {
                part: index,
                first,
                count: at - first,
                desc,
                time: begin as f64 / scale as f64,
                offset: 0,
            });
        }
    }
    chunks.sort_by(|left, right| left.time.total_cmp(&right.time));
    let mut running = 0u64;
    for chunk in &mut chunks {
        chunk.offset = running;
        running += parts[chunk.part].track.samples[chunk.first..chunk.first + chunk.count]
            .iter()
            .map(|sample| sample.size as u64)
            .sum::<u64>();
    }
    (chunks, running)
}

pub(super) fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 8);
    out.put_u32((body.len() + 8) as u32);
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

pub(super) fn full(kind: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
    let mut inner = Vec::with_capacity(body.len() + 4);
    inner.put_u8(version);
    inner.put_u8((flags >> 16) as u8);
    inner.put_u8((flags >> 8) as u8);
    inner.put_u8(flags as u8);
    inner.extend_from_slice(body);
    atom(kind, &inner)
}

fn rescale(value: u64, from: u64, to: u64) -> u64 {
    if from == 0 {
        return 0;
    }
    ((value as u128 * to as u128 + from as u128 / 2) / from as u128) as u64
}

fn edit_list(part: &Part) -> Vec<(u64, i64)> {
    let track = &part.track;
    let scale = track.timescale as u64;
    let media = track.duration();
    let movie_scale = if part.movie_scale == 0 {
        MOVIE_SCALE
    } else {
        part.movie_scale
    } as u64;
    if track.edits.is_empty() {
        let skipped = rescale(media.saturating_sub(track.shift), scale, MOVIE_SCALE as u64);
        if track.start > 0 {
            return vec![
                (rescale(track.start, scale, MOVIE_SCALE as u64), -1),
                (skipped, track.shift as i64),
            ];
        }
        if track.shift > 0 {
            return vec![(skipped, track.shift as i64)];
        }
        return Vec::new();
    }
    let last = track.edits.len() - 1;
    track
        .edits
        .iter()
        .enumerate()
        .map(|(index, edit)| {
            let media_time = if edit.media_time >= 0 {
                (edit.media_time - track.start as i64).max(0) + track.shift as i64
            } else {
                -1
            };
            let segment = if edit.segment == 0 && index == last {
                rescale(
                    media.saturating_sub(media_time.max(0) as u64),
                    scale,
                    MOVIE_SCALE as u64,
                )
            } else {
                rescale(edit.segment, movie_scale, MOVIE_SCALE as u64)
            };
            (segment, media_time)
        })
        .collect()
}

fn runs<T: PartialEq + Copy>(values: impl Iterator<Item = T>) -> Vec<(u32, T)> {
    let mut out: Vec<(u32, T)> = Vec::new();
    for value in values {
        match out.last_mut() {
            Some((count, last)) if *last == value => *count += 1,
            _ => out.push((1, value)),
        }
    }
    out
}

fn build_trak(
    part: &Part,
    id: u32,
    chunks: &[&Chunk],
    base: u64,
    wide: bool,
    now: u64,
) -> (Vec<u8>, u64) {
    let track = &part.track;
    let samples = &track.samples;
    let video = &track.handler == b"vide";
    let media = track.duration();
    let edits = edit_list(part);
    let movie_duration = if edits.is_empty() {
        rescale(media, track.timescale as u64, MOVIE_SCALE as u64)
    } else {
        edits.iter().map(|(segment, _)| *segment).sum()
    };

    let mut tkhd = Vec::new();
    let long = movie_duration > u32::MAX as u64;
    if long {
        tkhd.put_u64(now);
        tkhd.put_u64(now);
        tkhd.put_u32(id);
        tkhd.put_u32(0);
        tkhd.put_u64(movie_duration);
    } else {
        tkhd.put_u32(now as u32);
        tkhd.put_u32(now as u32);
        tkhd.put_u32(id);
        tkhd.put_u32(0);
        tkhd.put_u32(movie_duration as u32);
    }
    tkhd.put_zeros(8);
    tkhd.put_u16(0);
    tkhd.put_u16(0);
    tkhd.put_u16(if video { 0 } else { 0x0100 });
    tkhd.put_u16(0);
    tkhd.extend_from_slice(&track.matrix);
    tkhd.put_u32(track.width);
    tkhd.put_u32(track.height);
    let tkhd = full(b"tkhd", long as u8, 3, &tkhd);

    let mut edts = Vec::new();
    if !edits.is_empty() {
        let wide_edits = edits
            .iter()
            .any(|(segment, time)| *segment > u32::MAX as u64 || *time > i32::MAX as i64);
        let mut body = Vec::new();
        body.put_u32(edits.len() as u32);
        for (segment, time) in &edits {
            if wide_edits {
                body.put_u64(*segment);
                body.put_u64(*time as u64);
            } else {
                body.put_u32(*segment as u32);
                body.put_u32(*time as i32 as u32);
            }
            body.put_u16(1);
            body.put_u16(0);
        }
        edts = atom(b"edts", &full(b"elst", wide_edits as u8, 0, &body));
    }

    let mut mdhd = Vec::new();
    let long_media = media > u32::MAX as u64;
    if long_media {
        mdhd.put_u64(now);
        mdhd.put_u64(now);
        mdhd.put_u32(track.timescale);
        mdhd.put_u64(media);
    } else {
        mdhd.put_u32(now as u32);
        mdhd.put_u32(now as u32);
        mdhd.put_u32(track.timescale);
        mdhd.put_u32(media as u32);
    }
    mdhd.put_u16(track.language);
    mdhd.put_u16(0);
    let mdhd = full(b"mdhd", long_media as u8, 0, &mdhd);

    let hdlr = if track.hdlr.is_empty() {
        let mut body = Vec::new();
        body.put_u32(0);
        body.extend_from_slice(&track.handler);
        body.put_zeros(12);
        body.put_u8(0);
        full(b"hdlr", 0, 0, &body)
    } else {
        track.hdlr.clone()
    };

    let media_header = if video {
        let mut body = Vec::new();
        body.put_zeros(8);
        full(b"vmhd", 0, 1, &body)
    } else {
        let mut body = Vec::new();
        body.put_zeros(4);
        full(b"smhd", 0, 0, &body)
    };
    let mut dref = Vec::new();
    dref.put_u32(1);
    dref.extend_from_slice(&full(b"url ", 0, 1, &[]));
    let dinf = atom(b"dinf", &full(b"dref", 0, 0, &dref));

    let mut stts = Vec::new();
    let timing = runs(samples.iter().map(|sample| sample.dur));
    stts.put_u32(timing.len() as u32);
    for (count, dur) in &timing {
        stts.put_u32(*count);
        stts.put_u32(*dur);
    }
    let stts = full(b"stts", 0, 0, &stts);

    let mut ctts = Vec::new();
    if samples.iter().any(|sample| sample.cto != 0) {
        let offsets = runs(samples.iter().map(|sample| sample.cto));
        let signed = offsets.iter().any(|(_, offset)| *offset < 0);
        let mut body = Vec::new();
        body.put_u32(offsets.len() as u32);
        for (count, offset) in &offsets {
            body.put_u32(*count);
            body.put_u32(*offset as u32);
        }
        ctts = full(b"ctts", signed as u8, 0, &body);
    }

    let mut stss = Vec::new();
    if video && samples.iter().any(|sample| !sample.sync) {
        let marks: Vec<u32> = samples
            .iter()
            .enumerate()
            .filter(|(_, sample)| sample.sync)
            .map(|(index, _)| index as u32 + 1)
            .collect();
        let mut body = Vec::new();
        body.put_u32(marks.len() as u32);
        for mark in marks {
            body.put_u32(mark);
        }
        stss = full(b"stss", 0, 0, &body);
    }

    let layout = runs(chunks.iter().map(|chunk| (chunk.count as u32, chunk.desc)));
    let mut stsc = Vec::new();
    let mut first_chunk = 1u32;
    stsc.put_u32(layout.len() as u32);
    for (repeat, (per_chunk, desc)) in &layout {
        stsc.put_u32(first_chunk);
        stsc.put_u32(*per_chunk);
        stsc.put_u32(*desc);
        first_chunk += *repeat;
    }
    let stsc = full(b"stsc", 0, 0, &stsc);

    let mut stsz = Vec::new();
    stsz.put_u32(0);
    stsz.put_u32(samples.len() as u32);
    for sample in samples {
        stsz.put_u32(sample.size);
    }
    let stsz = full(b"stsz", 0, 0, &stsz);

    let mut stco = Vec::new();
    stco.put_u32(chunks.len() as u32);
    for chunk in chunks {
        if wide {
            stco.put_u64(base + chunk.offset);
        } else {
            stco.put_u32((base + chunk.offset) as u32);
        }
    }
    let stco = if wide {
        full(b"co64", 0, 0, &stco)
    } else {
        full(b"stco", 0, 0, &stco)
    };

    let mut stbl = Vec::new();
    for piece in [&track.stsd, &stts, &ctts, &stss, &stsc, &stsz, &stco] {
        stbl.extend_from_slice(piece);
    }
    let mut minf = Vec::new();
    for piece in [&media_header, &dinf, &atom(b"stbl", &stbl)] {
        minf.extend_from_slice(piece);
    }
    let mut mdia = Vec::new();
    for piece in [&mdhd, &hdlr, &atom(b"minf", &minf)] {
        mdia.extend_from_slice(piece);
    }
    let mut trak = Vec::new();
    for piece in [&tkhd, &edts, &atom(b"mdia", &mdia)] {
        trak.extend_from_slice(piece);
    }
    (atom(b"trak", &trak), movie_duration)
}

fn build_moov(parts: &[Part], chunks: &[Chunk], base: u64, wide: bool, now: u64) -> Vec<u8> {
    let mut traks = Vec::new();
    let mut duration = 0u64;
    for (index, part) in parts.iter().enumerate() {
        let own: Vec<&Chunk> = chunks.iter().filter(|chunk| chunk.part == index).collect();
        let (trak, length) = build_trak(part, index as u32 + 1, &own, base, wide, now);
        duration = duration.max(length);
        traks.extend_from_slice(&trak);
    }
    let long = duration > u32::MAX as u64;
    let mut mvhd = Vec::new();
    if long {
        mvhd.put_u64(now);
        mvhd.put_u64(now);
        mvhd.put_u32(MOVIE_SCALE);
        mvhd.put_u64(duration);
    } else {
        mvhd.put_u32(now as u32);
        mvhd.put_u32(now as u32);
        mvhd.put_u32(MOVIE_SCALE);
        mvhd.put_u32(duration as u32);
    }
    mvhd.put_u32(0x0001_0000);
    mvhd.put_u16(0x0100);
    mvhd.put_zeros(10);
    mvhd.extend_from_slice(&IDENTITY);
    mvhd.put_zeros(24);
    mvhd.put_u32(parts.len() as u32 + 1);
    let mut body = full(b"mvhd", long as u8, 0, &mvhd);
    body.extend_from_slice(&traks);
    atom(b"moov", &body)
}

fn build_ftyp(parts: &[Part]) -> Vec<u8> {
    let mut brands: Vec<[u8; 4]> = vec![*b"isom", *b"iso2"];
    for part in parts {
        for brand in &part.brands {
            if !brands.contains(brand) && !FRAGMENT_BRANDS.contains(&brand) {
                brands.push(*brand);
            }
        }
    }
    if !brands.contains(b"mp41") {
        brands.push(*b"mp41");
    }
    let mut body = Vec::new();
    body.extend_from_slice(b"isom");
    body.put_u32(0x200);
    for brand in &brands {
        body.extend_from_slice(brand);
    }
    atom(b"ftyp", &body)
}

pub fn merge(video: &Path, audio: &Path, out: &Path) -> Res<()> {
    let mut parts = vec![
        demux(video)?.into_part(b"vide")?,
        demux(audio)?.into_part(b"soun")?,
    ];
    let (chunks, total) = plan(&parts);
    let wide = total > 0xffff_ffff - (64 << 20);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
        + UNIX_TO_MP4;
    let ftyp = build_ftyp(&parts);
    let sized = build_moov(&parts, &chunks, 0, wide, now);
    let mdat_header: u64 = if total + 8 > 0xffff_ffff { 16 } else { 8 };
    let base = ftyp.len() as u64 + sized.len() as u64 + mdat_header;
    let moov = build_moov(&parts, &chunks, base, wide, now);
    if moov.len() != sized.len() {
        return Err("could not lay out the file".to_string());
    }
    let file = File::create(out).map_err(|error| format!("cannot write output: {error}"))?;
    let mut writer = BufWriter::with_capacity(1 << 20, file);
    writer.write_all(&ftyp).map_err(write_failed)?;
    writer.write_all(&moov).map_err(write_failed)?;
    let mut header = Vec::new();
    if mdat_header == 16 {
        header.put_u32(1);
        header.extend_from_slice(b"mdat");
        header.put_u64(total + 16);
    } else {
        header.put_u32((total + 8) as u32);
        header.extend_from_slice(b"mdat");
    }
    writer.write_all(&header).map_err(write_failed)?;
    for chunk in &chunks {
        let part = &mut parts[chunk.part];
        let samples = &part.track.samples[chunk.first..chunk.first + chunk.count];
        let mut at = 0usize;
        while at < samples.len() {
            let start = samples[at].offset;
            let mut length = samples[at].size as u64;
            at += 1;
            while at < samples.len() && samples[at].offset == start + length {
                length += samples[at].size as u64;
                at += 1;
            }
            part.reader.copy_to(start, length, &mut writer)?;
        }
    }
    writer.flush().map_err(write_failed)
}
