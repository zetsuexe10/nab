use ratatui::style::{Color, Modifier, Style};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThemeMode {
    Auto,
    Light,
    Dark,
}

impl ThemeMode {
    pub fn parse(text: &str) -> Option<ThemeMode> {
        match text.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(ThemeMode::Auto),
            "light" => Some(ThemeMode::Light),
            "dark" => Some(ThemeMode::Dark),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::Auto => "auto",
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        }
    }

    pub fn next(self) -> ThemeMode {
        match self {
            ThemeMode::Auto => ThemeMode::Light,
            ThemeMode::Light => ThemeMode::Dark,
            ThemeMode::Dark => ThemeMode::Auto,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Theme {
    pub mode: ThemeMode,
    primary: Color,
    gray: Color,
    select: Color,
    button_text: Color,
    pub background: Option<Color>,
    dim: bool,
}

impl Theme {
    pub fn of(mode: ThemeMode) -> Theme {
        match mode {
            ThemeMode::Auto => Theme {
                mode,
                primary: Color::Reset,
                gray: Color::Reset,
                select: Color::Cyan,
                button_text: Color::Reset,
                background: None,
                dim: true,
            },
            ThemeMode::Light => Theme {
                mode,
                primary: Color::Rgb(0x18, 0x18, 0x1b),
                gray: Color::Rgb(0x52, 0x52, 0x5b),
                select: Color::Rgb(0x1d, 0x5f, 0xa8),
                button_text: Color::Rgb(0xff, 0xff, 0xff),
                background: Some(Color::Rgb(0xff, 0xff, 0xff)),
                dim: false,
            },
            ThemeMode::Dark => Theme {
                mode,
                primary: Color::Rgb(0xff, 0xff, 0xff),
                gray: Color::Rgb(0xa1, 0xa1, 0xaa),
                select: Color::Rgb(0x7a, 0xa2, 0xc8),
                button_text: Color::Rgb(0x18, 0x18, 0x1b),
                background: Some(Color::Rgb(0x18, 0x18, 0x1b)),
                dim: false,
            },
        }
    }

    pub fn base(&self) -> Style {
        let mut style = Style::new().fg(self.primary);
        if let Some(background) = self.background {
            style = style.bg(background);
        }
        style
    }

    pub fn text(&self) -> Style {
        Style::new().fg(self.primary)
    }

    pub fn bold(&self) -> Style {
        self.text().add_modifier(Modifier::BOLD)
    }

    pub fn soft(&self) -> Style {
        let style = Style::new().fg(self.gray);
        if self.dim {
            style.add_modifier(Modifier::DIM)
        } else {
            style
        }
    }

    pub fn pick(&self) -> Style {
        Style::new().fg(self.select).add_modifier(Modifier::BOLD)
    }

    pub fn fill(&self, dim: bool) -> Style {
        let style = Style::new().fg(if dim { self.gray } else { self.primary });
        let style = style.add_modifier(Modifier::BOLD);
        if dim && self.dim {
            style.add_modifier(Modifier::DIM)
        } else {
            style
        }
    }

    pub fn button(&self, dim: bool) -> Style {
        if self.mode == ThemeMode::Auto {
            if dim {
                Style::new().add_modifier(Modifier::BOLD | Modifier::DIM)
            } else {
                Style::new().add_modifier(Modifier::BOLD | Modifier::REVERSED)
            }
        } else {
            let fill = if dim { self.gray } else { self.primary };
            Style::new()
                .bg(fill)
                .fg(self.button_text)
                .add_modifier(Modifier::BOLD)
        }
    }
}

pub fn width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

pub fn fit(text: &str, room: usize) -> String {
    if width(text) <= room {
        return text.to_string();
    }
    if room == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for character in text.chars() {
        let step = UnicodeWidthChar::width(character).unwrap_or(0);
        if used + step + 1 > room {
            break;
        }
        out.push(character);
        used += step;
    }
    out.push('…');
    out
}

pub fn pad(text: &str, room: usize) -> String {
    let used = width(text);
    if used >= room {
        text.to_string()
    } else {
        format!("{text}{}", " ".repeat(room - used))
    }
}

pub fn join(parts: &[&str]) -> String {
    parts
        .iter()
        .copied()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

pub fn wrap(text: &str, room: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if line.is_empty() {
            line = word.to_string();
        } else if width(&line) + 1 + width(word) <= room {
            line.push(' ');
            line.push_str(word);
        } else {
            lines.push(std::mem::take(&mut line));
            line = word.to_string();
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
        .into_iter()
        .flat_map(|line| {
            if width(&line) <= room {
                vec![line]
            } else {
                line.chars()
                    .collect::<Vec<_>>()
                    .chunks(room.max(1))
                    .map(|chunk| chunk.iter().collect::<String>())
                    .collect()
            }
        })
        .collect()
}

pub fn clock(seconds: u64) -> String {
    let (hours, minutes, secs) = (seconds / 3600, (seconds % 3600) / 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes}:{secs:02}")
    }
}

pub fn tilde(path: &std::path::Path) -> String {
    let text = path.display().to_string();
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    if let Some(home) = home {
        let home = home.to_string_lossy().into_owned();
        if !home.is_empty() {
            if let Some(rest) = text.strip_prefix(&home) {
                return format!("~{rest}");
            }
        }
    }
    text
}
