use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::theme::Theme;

const ART: [&str; 3] = [
    "█▀▀ █▀▄█ █▀█ █▀▀▀",
    "▀▀▓ █  ▓ █▀█ █ ▀▓",
    "▀▀▀ ▀  ▀ ▀ ▀ ▀▀▀▀",
];

pub const HEIGHT: u16 = 3;
pub const WIDTH: u16 = 17;

const INTRO_MS: u64 = 900;
const SPREAD_MS: u64 = 550;
const SWEEP_MS: u64 = 1000;
const EVERY_MS: u64 = 7000;
const TILT: f64 = 2.0;
const HALF: f64 = 2.4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Intro(u64),
    Idle,
    Sweep(u64),
}

fn phase(elapsed: u64) -> Phase {
    if elapsed < INTRO_MS {
        return Phase::Intro(elapsed);
    }
    let cycle = (elapsed - INTRO_MS) % (EVERY_MS + SWEEP_MS);
    if cycle < EVERY_MS {
        Phase::Idle
    } else {
        Phase::Sweep(cycle - EVERY_MS)
    }
}

fn delay(seed: u64, row: usize, col: usize) -> u64 {
    let mut value = seed ^ ((row as u64 + 1) << 32) ^ (col as u64 + 1).wrapping_mul(0x9E37_79B9);
    value ^= value << 13;
    value ^= value >> 7;
    value ^= value << 17;
    value % SPREAD_MS
}

fn ease(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
}

fn half_block(ch: char) -> bool {
    ch == '▀' || ch == '▄'
}

fn lighter(ch: char) -> char {
    match ch {
        '█' => '▒',
        '▓' => '░',
        other => other,
    }
}

fn cell(theme: &Theme, ch: char, row: usize, col: usize, phase: Phase, seed: u64) -> (char, Style) {
    let solid = theme.text();
    let faint = theme.soft();
    if ch == ' ' {
        return (ch, solid);
    }
    match phase {
        Phase::Idle => (ch, solid),
        Phase::Intro(t) => {
            let at = t as i64 - delay(seed, row, col) as i64;
            if at < 0 {
                (' ', solid)
            } else if at < 110 {
                (if half_block(ch) { ch } else { '░' }, faint)
            } else if at < 220 {
                (if half_block(ch) { ch } else { '▒' }, faint)
            } else {
                (ch, solid)
            }
        }
        Phase::Sweep(t) => {
            let cols = ART[0].chars().count() as f64;
            let rows = ART.len() as f64;
            let low = -TILT * rows - HALF;
            let high = cols + HALF;
            let position = low + ease(t as f64 / SWEEP_MS as f64) * (high - low);
            let distance = (col as f64 - (rows - 1.0 - row as f64) * TILT - position).abs();
            if distance <= HALF && 1.0 - distance / HALF > 0.35 {
                if half_block(ch) {
                    (ch, faint)
                } else {
                    (lighter(ch), solid)
                }
            } else {
                (ch, solid)
            }
        }
    }
}

pub fn lines(theme: &Theme, elapsed: u64, seed: u64, animate: bool) -> Vec<Line<'static>> {
    let phase = if animate { phase(elapsed) } else { Phase::Idle };
    ART.iter()
        .enumerate()
        .map(|(row, text)| {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut run = String::new();
            let mut run_style: Option<Style> = None;
            for (col, ch) in text.chars().enumerate() {
                let (shown, style) = cell(theme, ch, row, col, phase, seed);
                if run_style == Some(style) || shown == ' ' {
                    run.push(shown);
                } else {
                    if let Some(previous) = run_style {
                        spans.push(Span::styled(std::mem::take(&mut run), previous));
                    }
                    run.push(shown);
                    run_style = Some(style);
                }
            }
            if let Some(style) = run_style {
                spans.push(Span::styled(run, style));
            }
            Line::from(spans)
        })
        .collect()
}
