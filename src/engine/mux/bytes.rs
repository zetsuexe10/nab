use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

pub type Res<T> = Result<T, String>;

pub fn truncated<T>() -> Res<T> {
    Err("input ended unexpectedly".to_string())
}

pub struct Cur<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cur<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Cur { buf, pos: 0 }
    }

    pub fn left(&self) -> usize {
        self.buf.len() - self.pos
    }

    pub fn take(&mut self, count: usize) -> Res<&'a [u8]> {
        if self.left() < count {
            return truncated();
        }
        let slice = &self.buf[self.pos..self.pos + count];
        self.pos += count;
        Ok(slice)
    }

    pub fn skip(&mut self, count: usize) -> Res<()> {
        self.take(count).map(|_| ())
    }

    pub fn u8(&mut self) -> Res<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Res<u16> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    pub fn u32(&mut self) -> Res<u32> {
        let bytes = self.take(4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub fn i32(&mut self) -> Res<i32> {
        self.u32().map(|value| value as i32)
    }

    pub fn u64(&mut self) -> Res<u64> {
        let bytes = self.take(8)?;
        let mut array = [0u8; 8];
        array.copy_from_slice(bytes);
        Ok(u64::from_be_bytes(array))
    }

    pub fn i64(&mut self) -> Res<i64> {
        self.u64().map(|value| value as i64)
    }
}

pub trait Put {
    fn put_u8(&mut self, value: u8);
    fn put_u16(&mut self, value: u16);
    fn put_u32(&mut self, value: u32);
    fn put_u64(&mut self, value: u64);
    fn put_zeros(&mut self, count: usize);
}

impl Put for Vec<u8> {
    fn put_u8(&mut self, value: u8) {
        self.push(value);
    }

    fn put_u16(&mut self, value: u16) {
        self.extend_from_slice(&value.to_be_bytes());
    }

    fn put_u32(&mut self, value: u32) {
        self.extend_from_slice(&value.to_be_bytes());
    }

    fn put_u64(&mut self, value: u64) {
        self.extend_from_slice(&value.to_be_bytes());
    }

    fn put_zeros(&mut self, count: usize) {
        self.resize(self.len() + count, 0);
    }
}

pub struct Window {
    file: File,
    len: u64,
    base: u64,
    data: Vec<u8>,
}

const WINDOW: usize = 1 << 20;

impl Window {
    pub fn open(path: &std::path::Path) -> Res<Self> {
        let file =
            File::open(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let len = file
            .metadata()
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?
            .len();
        Ok(Window {
            file,
            len,
            base: 0,
            data: Vec::new(),
        })
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Res<()> {
        let end = offset
            .checked_add(out.len() as u64)
            .ok_or_else(|| "input is corrupt".to_string())?;
        if end > self.len {
            return truncated();
        }
        if out.len() > WINDOW {
            return self.read_direct(offset, out);
        }
        let held_end = self.base + self.data.len() as u64;
        if offset < self.base || end > held_end {
            let want = (self.len - offset).min(WINDOW as u64) as usize;
            self.data.resize(want, 0);
            let mut chunk = std::mem::take(&mut self.data);
            let outcome = self.read_direct(offset, &mut chunk);
            self.data = chunk;
            outcome?;
            self.base = offset;
        }
        let start = (offset - self.base) as usize;
        out.copy_from_slice(&self.data[start..start + out.len()]);
        Ok(())
    }

    fn read_direct(&mut self, offset: u64, out: &mut [u8]) -> Res<()> {
        self.file
            .seek(SeekFrom::Start(offset))
            .map_err(|error| format!("cannot read input: {error}"))?;
        self.file
            .read_exact(out)
            .map_err(|error| format!("cannot read input: {error}"))
    }

    pub fn vec_at(&mut self, offset: u64, len: u64) -> Res<Vec<u8>> {
        if offset.saturating_add(len) > self.len {
            return truncated();
        }
        let Ok(size) = usize::try_from(len) else {
            return truncated();
        };
        let mut out = vec![0u8; size];
        self.read_at(offset, &mut out)?;
        Ok(out)
    }

    pub fn copy_to<W: Write>(&mut self, offset: u64, len: u64, out: &mut W) -> Res<()> {
        if offset.saturating_add(len) > self.len {
            return truncated();
        }
        let mut buffer = vec![0u8; (len.min(WINDOW as u64) as usize).max(1)];
        let mut done = 0u64;
        while done < len {
            let step = (len - done).min(buffer.len() as u64) as usize;
            self.read_direct(offset + done, &mut buffer[..step])?;
            out.write_all(&buffer[..step])
                .map_err(|error| format!("cannot write output: {error}"))?;
            done += step as u64;
        }
        Ok(())
    }
}

pub fn write_failed(error: std::io::Error) -> String {
    format!("cannot write output: {error}")
}
