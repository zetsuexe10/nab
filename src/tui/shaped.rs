use std::env;
use std::io::{self, Write};

use ratatui::backend::{Backend, ClearType, CrosstermBackend, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};
use unicode_width::UnicodeWidthStr;

pub struct Shaped<W: Write> {
    inner: CrosstermBackend<W>,
    enabled: bool,
}

impl<W: Write> Shaped<W> {
    pub fn new(writer: W, enabled: bool) -> Self {
        Self {
            inner: CrosstermBackend::new(writer),
            enabled,
        }
    }
}

pub fn supported() -> bool {
    match env::var("NAB_OSC66").ok().as_deref() {
        Some("1") => return true,
        Some("0") => return false,
        _ => {}
    }
    if env::var_os("KITTY_WINDOW_ID").is_some() {
        return true;
    }
    if env::var_os("TMUX").is_some() {
        return false;
    }
    let term = env::var("TERM").unwrap_or_default();
    term == "xterm-kitty" || term.starts_with("foot")
}

fn complex(symbol: &str) -> bool {
    symbol.chars().any(|character| {
        matches!(
            character as u32,
            0x0300..=0x036F
                | 0x0483..=0x0489
                | 0x0591..=0x05C7
                | 0x0600..=0x06FF
                | 0x0750..=0x077F
                | 0x08A0..=0x08FF
                | 0x0900..=0x0DFF
                | 0x0E00..=0x0EFF
                | 0x0F00..=0x0FFF
                | 0x1000..=0x109F
                | 0x1780..=0x17FF
                | 0x1A00..=0x1AAF
                | 0x1B00..=0x1B7F
                | 0x1AB0..=0x1AFF
                | 0x1DC0..=0x1DFF
                | 0x20D0..=0x20FF
                | 0xA800..=0xA8FF
                | 0xA980..=0xA9DF
                | 0xAA00..=0xAA5F
                | 0x11000..=0x11FFF
                | 0x1E000..=0x1E02F
        )
    })
}

fn encode(cell: &Cell) -> Cell {
    let symbol = cell.symbol();
    let width = symbol.width();
    if width == 0 || !complex(symbol) {
        return cell.clone();
    }
    let mut out = cell.clone();
    out.set_symbol(&format!("\x1b]66;w={};{}\x07", width.min(7), symbol));
    out
}

impl<W: Write> Backend for Shaped<W> {
    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        if !self.enabled {
            return self.inner.draw(content);
        }
        let cells: Vec<(u16, u16, Cell)> =
            content.map(|(x, y, cell)| (x, y, encode(cell))).collect();
        self.inner
            .draw(cells.iter().map(|(x, y, cell)| (*x, *y, cell)))
    }

    fn append_lines(&mut self, n: u16) -> io::Result<()> {
        self.inner.append_lines(n)
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        self.inner.get_cursor_position()
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        self.inner.set_cursor_position(position)
    }

    fn clear(&mut self) -> io::Result<()> {
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> io::Result<Size> {
        self.inner.size()
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> io::Result<()> {
        Backend::flush(&mut self.inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_complex_scripts_and_leaves_the_rest() {
        let wrapped = encode(&Cell::new("ভু"));
        assert!(wrapped.symbol().starts_with("\x1b]66;w=1;"));
        assert!(wrapped.symbol().ends_with('\u{7}'));
        assert_eq!(encode(&Cell::new("a")).symbol(), "a");
        assert_eq!(encode(&Cell::new("日")).symbol(), "日");
    }

    #[test]
    fn explicit_width_matches_the_cell() {
        let wrapped = encode(&Cell::new("তে"));
        assert!(wrapped.symbol().starts_with("\x1b]66;w=2;"));
    }
}
