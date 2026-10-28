use super::bytes::Put;
use super::mp4::{atom, full};
use super::webm::{element, uint_element};

pub struct Frame {
    pub data: Vec<u8>,
    pub key: bool,
    pub cto: i32,
}

pub fn frames(video: bool, count: usize, seed: u8) -> Vec<Frame> {
    (0..count)
        .map(|index| Frame {
            data: (0..40 + index * 7)
                .map(|at| (at as u8).wrapping_mul(31).wrapping_add(seed ^ index as u8))
                .collect(),
            key: !video || index % 3 == 0,
            cto: if video && index % 3 == 1 { 2 * 512 } else { 0 },
        })
        .collect()
}

fn stsd(video: bool) -> Vec<u8> {
    let mut entry = Vec::new();
    entry.put_zeros(6);
    entry.put_u16(1);
    entry.extend_from_slice(&[7u8; 20]);
    let kind: &[u8; 4] = if video { b"avc1" } else { b"mp4a" };
    full(b"stsd", 0, 0, &{
        let mut body = Vec::new();
        body.put_u32(1);
        body.extend_from_slice(&atom(kind, &entry));
        body
    })
}

fn head(video: bool, tables: &[u8], dur: u64, timescale: u32, mvex: bool) -> Vec<u8> {
    let mut tkhd = Vec::new();
    tkhd.put_u32(0);
    tkhd.put_u32(0);
    tkhd.put_u32(1);
    tkhd.put_u32(0);
    tkhd.put_u32(dur as u32);
    tkhd.put_zeros(8);
    tkhd.put_u32(0);
    tkhd.put_u16(if video { 0 } else { 0x100 });
    tkhd.put_u16(0);
    let mut matrix = vec![0u8; 36];
    matrix[1] = 1;
    matrix[17] = 1;
    matrix[32] = 0x40;
    tkhd.extend_from_slice(&matrix);
    tkhd.put_u32(if video { 320 << 16 } else { 0 });
    tkhd.put_u32(if video { 180 << 16 } else { 0 });
    let mut mdhd = Vec::new();
    mdhd.put_u32(0);
    mdhd.put_u32(0);
    mdhd.put_u32(timescale);
    mdhd.put_u32(dur as u32);
    mdhd.put_u16(0x55c4);
    mdhd.put_u16(0);
    let mut hdlr = Vec::new();
    hdlr.put_u32(0);
    hdlr.extend_from_slice(if video { b"vide" } else { b"soun" });
    hdlr.put_zeros(12);
    hdlr.put_u8(0);
    let mut dref = Vec::new();
    dref.put_u32(1);
    dref.extend_from_slice(&full(b"url ", 0, 1, &[]));
    let mut minf = Vec::new();
    minf.extend_from_slice(&full(
        if video { b"vmhd" } else { b"smhd" },
        0,
        1,
        &[0u8; 8],
    ));
    minf.extend_from_slice(&atom(b"dinf", &full(b"dref", 0, 0, &dref)));
    minf.extend_from_slice(&atom(b"stbl", tables));
    let mut mdia = Vec::new();
    mdia.extend_from_slice(&full(b"mdhd", 0, 0, &mdhd));
    mdia.extend_from_slice(&full(b"hdlr", 0, 0, &hdlr));
    mdia.extend_from_slice(&atom(b"minf", &minf));
    let mut trak = full(b"tkhd", 0, 3, &tkhd);
    trak.extend_from_slice(&atom(b"mdia", &mdia));
    let mut mvhd = Vec::new();
    mvhd.put_u32(0);
    mvhd.put_u32(0);
    mvhd.put_u32(1000);
    mvhd.put_u32(0);
    mvhd.put_u32(0x10000);
    mvhd.put_u16(0x100);
    mvhd.put_zeros(10);
    mvhd.extend_from_slice(&matrix);
    mvhd.put_zeros(24);
    mvhd.put_u32(2);
    let mut moov = full(b"mvhd", 0, 0, &mvhd);
    moov.extend_from_slice(&atom(b"trak", &trak));
    if mvex {
        let mut trex = Vec::new();
        trex.put_u32(1);
        trex.put_u32(1);
        trex.put_u32(0);
        trex.put_u32(0);
        trex.put_u32(0);
        moov.extend_from_slice(&atom(b"mvex", &full(b"trex", 0, 0, &trex)));
    }
    atom(b"moov", &moov)
}

fn ftyp(brand: &[u8; 4]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(brand);
    body.put_u32(0);
    body.extend_from_slice(b"isomiso5dash");
    atom(b"ftyp", &body)
}

pub fn progressive(video: bool, items: &[Frame], timescale: u32, dur: u32) -> Vec<u8> {
    let lead = ftyp(b"isom");
    let total: usize = items.iter().map(|frame| frame.data.len()).sum();
    let mut stts = Vec::new();
    stts.put_u32(1);
    stts.put_u32(items.len() as u32);
    stts.put_u32(dur);
    let mut stsc = Vec::new();
    stsc.put_u32(1);
    stsc.put_u32(1);
    stsc.put_u32(items.len() as u32);
    stsc.put_u32(1);
    let mut stsz = Vec::new();
    stsz.put_u32(0);
    stsz.put_u32(items.len() as u32);
    for frame in items {
        stsz.put_u32(frame.data.len() as u32);
    }
    let mut stco = Vec::new();
    stco.put_u32(1);
    stco.put_u32((lead.len() + 8) as u32);
    let mut tables = stsd(video);
    tables.extend_from_slice(&full(b"stts", 0, 0, &stts));
    if items.iter().any(|frame| frame.cto != 0) {
        let mut ctts = Vec::new();
        ctts.put_u32(items.len() as u32);
        for frame in items {
            ctts.put_u32(1);
            ctts.put_u32(frame.cto as u32);
        }
        tables.extend_from_slice(&full(b"ctts", 0, 0, &ctts));
    }
    if items.iter().any(|frame| !frame.key) {
        let marks: Vec<u32> = items
            .iter()
            .enumerate()
            .filter(|(_, frame)| frame.key)
            .map(|(index, _)| index as u32 + 1)
            .collect();
        let mut stss = Vec::new();
        stss.put_u32(marks.len() as u32);
        for mark in marks {
            stss.put_u32(mark);
        }
        tables.extend_from_slice(&full(b"stss", 0, 0, &stss));
    }
    tables.extend_from_slice(&full(b"stsc", 0, 0, &stsc));
    tables.extend_from_slice(&full(b"stsz", 0, 0, &stsz));
    tables.extend_from_slice(&full(b"stco", 0, 0, &stco));
    let mut out = lead;
    let mut data = Vec::with_capacity(total);
    for frame in items {
        data.extend_from_slice(&frame.data);
    }
    out.extend_from_slice(&atom(b"mdat", &data));
    out.extend_from_slice(&head(
        video,
        &tables,
        items.len() as u64 * dur as u64,
        timescale,
        false,
    ));
    out
}

pub fn fragmented(video: bool, items: &[Frame], timescale: u32, dur: u32) -> Vec<u8> {
    let mut tables = stsd(video);
    for kind in [b"stts", b"stsc", b"stco"] {
        tables.extend_from_slice(&full(kind, 0, 0, &[0u8; 4]));
    }
    tables.extend_from_slice(&full(b"stsz", 0, 0, &[0u8; 8]));
    let mut out = ftyp(b"iso5");
    out.extend_from_slice(&head(video, &tables, 0, timescale, true));
    let mut decode = 0u64;
    for (number, group) in items.chunks(2).enumerate() {
        let mut trun = Vec::new();
        trun.put_u32(group.len() as u32);
        trun.put_u32(0);
        for frame in group {
            trun.put_u32(dur);
            trun.put_u32(frame.data.len() as u32);
            trun.put_u32(if frame.key { 0x0200_0000 } else { 0x0101_0000 });
            trun.put_u32(frame.cto as u32);
        }
        let mut tfhd = Vec::new();
        tfhd.put_u32(1);
        let mut tfdt = Vec::new();
        tfdt.put_u64(decode);
        let mut mfhd = Vec::new();
        mfhd.put_u32(number as u32 + 1);
        let build = |offset: u32| {
            let mut run = trun.clone();
            run[4..8].copy_from_slice(&offset.to_be_bytes());
            let mut traf = full(b"tfhd", 0, 0x020000, &tfhd);
            traf.extend_from_slice(&full(b"tfdt", 1, 0, &tfdt));
            traf.extend_from_slice(&full(b"trun", 0, 0xF01, &run));
            let mut moof = full(b"mfhd", 0, 0, &mfhd);
            moof.extend_from_slice(&atom(b"traf", &traf));
            atom(b"moof", &moof)
        };
        let size = build(0).len();
        out.extend_from_slice(&build(size as u32 + 8));
        let mut data = Vec::new();
        for frame in group {
            data.extend_from_slice(&frame.data);
        }
        out.extend_from_slice(&atom(b"mdat", &data));
        decode += group.len() as u64 * dur as u64;
    }
    out
}

pub fn webm(video: bool, items: &[(i64, Frame)]) -> Vec<u8> {
    let mut ebml = uint_element(0x4286, 1);
    ebml.extend_from_slice(&element(0x4282, b"webm"));
    let mut out = element(0x1A45_DFA3, &ebml);
    let mut info = uint_element(0x2A_D7B1, 1_000_000);
    info.extend_from_slice(&element(0x4489, &1000f64.to_be_bytes()));
    let mut entry = uint_element(0xD7, 1);
    entry.extend_from_slice(&uint_element(0x73C5, 99));
    entry.extend_from_slice(&uint_element(0x83, if video { 1 } else { 2 }));
    entry.extend_from_slice(&element(0x86, if video { b"V_VP9" } else { b"A_OPUS" }));
    let mut segment = element(0x1549_A966, &info);
    segment.extend_from_slice(&element(0x1654_AE6B, &element(0xAE, &entry)));
    for group in items.chunks(4) {
        let base = group[0].0;
        let mut cluster = uint_element(0xE7, base as u64);
        for (time, frame) in group {
            let mut block = vec![0x81];
            block.extend_from_slice(&((time - base) as i16).to_be_bytes());
            block.push(if frame.key { 0x80 } else { 0 });
            block.extend_from_slice(&frame.data);
            cluster.extend_from_slice(&element(0xA3, &block));
        }
        segment.extend_from_slice(&element(0x1F43_B675, &cluster));
    }
    out.extend_from_slice(&element(0x1853_8067, &segment));
    out
}
