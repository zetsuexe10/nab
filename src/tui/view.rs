use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use super::app::{App, Hit, Mode, Zone, SUGGEST_MAX};
use super::logo;
use super::theme::{clock, fit, join, pad, tilde, width, wrap, Theme};
use crate::download::Stage;
use crate::play::Player;
use crate::search::Video;
use crate::site::Suggestion;
use crate::source::Kind;
use crate::util::{human_size, short_count};

const SPINNER: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
const BUTTON: &str = "snag";
const AGAIN: &str = "↵ snag another";
const BAR: usize = 30;
const PANEL: u16 = 40;
const SEPARATOR: &str = "  ·  ";
const HEADER: u16 = 7;
const WIDE: u16 = 76;
const STACK_BELOW: u16 = 64;
const SIDE: u16 = 28;

struct Hint {
    key: String,
    label: String,
    press: Option<(KeyCode, KeyModifiers)>,
}

fn hint(key: &str, label: impl Into<String>) -> Hint {
    let press = match key {
        "↵" => Some((KeyCode::Enter, KeyModifiers::NONE)),
        "esc" => Some((KeyCode::Esc, KeyModifiers::NONE)),
        "⇥" => Some((KeyCode::Tab, KeyModifiers::NONE)),
        "^c" => Some((KeyCode::Char('c'), KeyModifiers::CONTROL)),
        "^t" => Some((KeyCode::Char('t'), KeyModifiers::CONTROL)),
        "^s" => Some((KeyCode::Char('s'), KeyModifiers::CONTROL)),
        "←" => Some((KeyCode::Left, KeyModifiers::NONE)),
        "→" => Some((KeyCode::Right, KeyModifiers::NONE)),
        "↓" => Some((KeyCode::Down, KeyModifiers::NONE)),
        other if other.chars().count() == 1 => other
            .chars()
            .next()
            .map(|character| (KeyCode::Char(character), KeyModifiers::NONE)),
        _ => None,
    };
    Hint {
        key: key.to_string(),
        label: label.into(),
        press,
    }
}

fn window(text: &str, caret: usize, room: usize) -> (String, u16) {
    let chars: Vec<char> = text.chars().collect();
    let cells = |slice: &[char]| -> usize {
        slice
            .iter()
            .map(|character| width(&character.to_string()))
            .sum()
    };
    let mut start = 0usize;
    while start < caret && cells(&chars[start..caret]) >= room {
        start += 1;
    }
    let mut shown = String::new();
    let mut used = 0usize;
    for character in &chars[start..] {
        let step = width(&character.to_string());
        if used + step > room {
            break;
        }
        shown.push(*character);
        used += step;
    }
    (shown, cells(&chars[start..caret]) as u16)
}

fn rect(area: Rect, x: u16, y: u16, wide: u16, tall: u16) -> Rect {
    Rect::new(x, y, wide, tall).intersection(area)
}

fn put(frame: &mut Frame, x: u16, y: u16, wide: u16, line: Line<'static>) {
    let target = rect(frame.area(), x, y, wide, 1);
    if target.width > 0 && target.height > 0 {
        frame.render_widget(Paragraph::new(line), target);
    }
}

fn centered(frame: &mut Frame, x: u16, y: u16, wide: u16, line: Line<'static>) {
    let used = line.width() as u16;
    let shift = wide.saturating_sub(used) / 2;
    put(frame, x + shift, y, wide.saturating_sub(shift), line);
}

fn padded(mut line: Line<'static>, room: usize) -> Line<'static> {
    let used = line.width();
    if used < room {
        line.spans.push(Span::raw(" ".repeat(room - used)));
    }
    line
}

fn panel(theme: &Theme, title: &str, wide: u16, rows: Vec<Line<'static>>) -> Vec<Line<'static>> {
    let soft = theme.soft();
    let inner = (wide as usize).saturating_sub(2);
    let title = fit(title, inner.saturating_sub(4));
    let tail = inner.saturating_sub(width(&title) + 3);
    let room = (wide as usize).saturating_sub(6);
    let mut lines = vec![Line::from(vec![
        Span::styled("╭─ ", soft),
        Span::styled(title, theme.text()),
        Span::styled(format!(" {}╮", "─".repeat(tail)), soft),
    ])];
    for row in rows {
        let mut spans = vec![Span::styled("│  ", soft)];
        spans.extend(padded(row, room).spans);
        spans.push(Span::styled("  │", soft));
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(Span::styled(
        format!("╰{}╯", "─".repeat(inner)),
        soft,
    )));
    lines
}

fn bar(theme: &Theme, ratio: f64, wide: usize) -> Line<'static> {
    let ratio = ratio.clamp(0.0, 1.0);
    let filled = (ratio * wide as f64).round() as usize;
    Line::from(vec![
        Span::styled("█".repeat(filled), theme.text()),
        Span::styled("░".repeat(wide - filled), theme.soft()),
        Span::styled(
            format!(" {:>4}", format!("{}%", (ratio * 100.0).round() as u32)),
            theme.text(),
        ),
    ])
}

impl App {
    fn spinner(&self) -> &'static str {
        let frame = (self.clock.elapsed().as_millis() / 80) as usize;
        SPINNER[frame % SPINNER.len()]
    }

    fn zone(&mut self, rect: Rect, hit: Hit) {
        if rect.width > 0 && rect.height > 0 {
            self.zones.push(Zone { rect, hit });
        }
    }

    fn input_title(&self) -> &'static str {
        match self.mode {
            Mode::Query => "Search or paste a link",
            Mode::Busy if self.busy == "searching" => "Searching",
            Mode::Busy if self.busy == "loading profile" => "Loading profile",
            Mode::Busy => "Resolving",
            _ => "Search",
        }
    }

    fn hints(&self) -> Vec<Hint> {
        let mut hints = Vec::new();
        match self.mode {
            Mode::Query => {
                hints.push(hint("↵", "search"));
                if !self.history.is_empty() {
                    hints.push(hint("↑", "history"));
                }
                if self.suggest_sel.is_some() {
                    hints.push(hint("⇥", "fill"));
                } else if self.offered().is_some() {
                    hints.push(hint("⇥", "paste"));
                } else if self.dropdown() {
                    hints.push(hint("↓", "suggestions"));
                } else if !self.results.is_empty() {
                    hints.push(hint("↓", "results"));
                }
                hints.push(hint(
                    "esc",
                    if self.results.is_empty() {
                        "quit"
                    } else {
                        "back"
                    },
                ));
                hints.push(hint("^s", format!("site:{}", self.site.label())));
                hints.push(hint("^c", "quit"));
            }
            Mode::Busy => {
                hints.push(hint("esc", "cancel"));
                hints.push(hint("^c", "quit"));
            }
            Mode::Results => {
                hints.push(hint("↵", "open"));
                hints.push(hint("↑↓", "move"));
                if self
                    .results
                    .get(self.sel)
                    .is_some_and(|video| video.author_url.is_some())
                {
                    hints.push(hint("o", "profile"));
                }
                hints.push(hint("/", "search"));
                hints.push(hint("^c", "quit"));
            }
            Mode::Profile => {
                hints.push(hint("↵", "open"));
                hints.push(hint("↑↓", "move"));
                hints.push(hint("←", "prev"));
                hints.push(hint("→", "next"));
                hints.push(hint("s", format!("sort:{}", self.sort_label())));
                hints.push(hint("esc", "back"));
            }
            Mode::Formats => {
                hints.push(hint("↵", "download"));
                hints.push(hint("↑↓", "move"));
                hints.push(hint("p", "play"));
                self.player_hints(&mut hints);
                if self
                    .current
                    .as_ref()
                    .is_some_and(|video| video.author_url.is_some())
                {
                    hints.push(hint("o", "profile"));
                }
                if self.providers().len() > 1 {
                    hints.push(hint("⇥", self.provider.unwrap_or("source")));
                }
                hints.push(hint("esc", "back"));
            }
            Mode::Download => {
                if self.stage != Stage::Merging && !self.cancelling {
                    hints.push(hint("esc", "cancel"));
                }
                hints.push(hint("^c", "quit"));
            }
            Mode::Done => {
                hints.push(hint("↵", "snag another"));
                hints.push(hint("p", "play"));
                self.player_hints(&mut hints);
                hints.push(hint("esc", "back"));
            }
            Mode::Error => {
                hints.push(hint("↵", "back"));
                hints.push(hint("^c", "quit"));
            }
        }
        hints.push(hint("^t", format!("theme:{}", self.theme.label())));
        hints
    }

    fn player_hints(&self, hints: &mut Vec<Hint>) {
        for player in [Player::Mpv, Player::Vlc] {
            if self.installed(player) {
                let key = &player.name()[..1];
                hints.push(hint(key, player.name()));
            }
        }
    }

    pub fn render(&mut self, frame: &mut Frame) {
        self.zones.clear();
        let area = frame.area();
        let theme = Theme::of(self.theme);
        frame.render_widget(Block::new().style(theme.base()), area);
        if area.width < 16 || area.height < 5 {
            put(frame, area.x, area.y, area.width, Line::from("too small"));
            return;
        }
        let wide = area.width.saturating_sub(4).clamp(14, WIDE).min(area.width);
        let header = if area.height >= 24 { HEADER } else { 0 };
        let footer = if area.height >= 14 { 3 } else { 1 };
        let avail = area.height.saturating_sub(header + footer);
        let body = self.body_height(avail, wide);
        let total = header + body + footer;
        let x = area.x + (area.width - wide) / 2;
        let mut y = area.y + area.height.saturating_sub(total) / 2;
        if header > 0 {
            self.draw_header(frame, &theme, area, y);
            y += header;
        }
        self.draw_body(frame, &theme, x, y, wide, body);
        y += body + footer - 1;
        if footer >= 3 && self.mode != Mode::Query {
            if let Some((text, _)) = self.notice.clone() {
                centered(
                    frame,
                    area.x,
                    y - 1,
                    area.width,
                    Line::styled(fit(&text, area.width as usize), theme.soft()),
                );
            }
        }
        self.draw_hints(frame, &theme, area, y);
    }

    fn draw_header(&mut self, frame: &mut Frame, theme: &Theme, area: Rect, y: u16) {
        let lines = logo::lines(
            theme,
            self.born.elapsed().as_millis() as u64,
            0x5EED,
            self.animate,
        );
        let left = area.x + area.width.saturating_sub(logo::WIDTH) / 2;
        for (row, line) in lines.into_iter().enumerate() {
            put(frame, left, y + row as u16, logo::WIDTH, line);
        }
        let target = rect(
            area,
            left.saturating_sub(2),
            y,
            logo::WIDTH + 4,
            logo::HEIGHT,
        );
        self.zone(target, Hit::Logo);
        centered(
            frame,
            area.x,
            y + 4,
            area.width,
            Line::styled(
                fit(
                    &format!("search {}, pick a stream, snag it", self.site.label()),
                    area.width as usize,
                ),
                theme.text(),
            ),
        );
        centered(
            frame,
            area.x,
            y + 5,
            area.width,
            Line::styled(
                fit(
                    "audio · video · mpv · vlc · built-in muxer, no ffmpeg",
                    area.width as usize,
                ),
                theme.soft(),
            ),
        );
    }

    fn body_height(&self, avail: u16, wide: u16) -> u16 {
        match self.mode {
            Mode::Query | Mode::Busy => 4 + (SUGGEST_MAX as u16 + 2).min(avail.saturating_sub(4)),
            Mode::Profile => {
                let want = 2 + 2 * self.profile_videos().max(1) as u16;
                if wide < STACK_BELOW {
                    if avail < 8 {
                        want.min(avail.max(3))
                    } else {
                        3 + want.min(avail - 3)
                    }
                } else {
                    want.min(avail.max(3))
                        .max((self.profile_left(SIDE as usize).len() as u16).min(avail))
                }
            }
            Mode::Results => {
                let want = 2 + 2 * self.results.len().max(1) as u16;
                if avail < 8 {
                    want.min(avail.max(4))
                } else {
                    3 + want.min(avail - 3)
                }
            }
            Mode::Formats => {
                let want = 2 + self.visible().len().max(1) as u16;
                if wide < STACK_BELOW {
                    if avail < 8 {
                        want.min(avail.max(3))
                    } else {
                        3 + want.min(avail - 3)
                    }
                } else {
                    want.min(avail.max(3))
                        .max(self.left_lines(wide).len() as u16)
                }
            }
            Mode::Download => 5,
            Mode::Done => 6,
            Mode::Error => (wrap(&format!("✗ {}", self.error), wide as usize).len() as u16)
                .clamp(1, avail.max(1)),
        }
    }

    fn draw_body(
        &mut self,
        frame: &mut Frame,
        theme: &Theme,
        x: u16,
        y: u16,
        wide: u16,
        tall: u16,
    ) {
        match self.mode {
            Mode::Query | Mode::Busy => {
                self.draw_input(frame, theme, x, y, wide);
                if self.dropdown() {
                    self.draw_dropdown(frame, theme, x, y + 3, wide, tall.saturating_sub(3));
                } else {
                    self.draw_status(frame, theme, x, y + 3, wide);
                }
            }
            Mode::Profile => self.draw_profile(frame, theme, x, y, wide, tall),
            Mode::Results => self.draw_results(frame, theme, x, y, wide, tall),
            Mode::Formats => self.draw_formats(frame, theme, x, y, wide, tall),
            Mode::Download => self.draw_download(frame, theme, x, y, wide),
            Mode::Done => self.draw_done(frame, theme, x, y, wide),
            Mode::Error => {
                for (row, line) in wrap(&format!("✗ {}", self.error), wide as usize)
                    .into_iter()
                    .take(tall as usize)
                    .enumerate()
                {
                    centered(
                        frame,
                        x,
                        y + row as u16,
                        wide,
                        Line::styled(line, theme.bold()),
                    );
                }
            }
        }
    }

    fn draw_status(&self, frame: &mut Frame, theme: &Theme, x: u16, y: u16, wide: u16) {
        let text = if let Some((notice, _)) = &self.notice {
            Some(notice.clone())
        } else if self.mode == Mode::Query && self.offered().is_some() {
            Some("link in your clipboard — ⇥ to paste it".to_string())
        } else if self.mode == Mode::Query && self.clip_taken && !self.query.is_empty() {
            Some("↵ to snag it".to_string())
        } else {
            None
        };
        if let Some(text) = text {
            centered(
                frame,
                x,
                y,
                wide,
                Line::styled(fit(&text, wide as usize), theme.soft()),
            );
        }
    }

    fn input_span(wide: u16) -> u16 {
        wide.saturating_sub((BUTTON.len() + 4) as u16).max(8)
    }

    fn suggestion_row(&self, index: usize, room: usize) -> Line<'static> {
        let theme = Theme::of(self.theme);
        let row: &Suggestion = &self.suggestions[index];
        let selected = self.suggest_sel == Some(index);
        let count = row.count.map(short_count).unwrap_or_default();
        let tag = join(&[row.tag, &count]);
        let head = room.saturating_sub(2 + width(&tag) + 1);
        let style = if selected { theme.pick() } else { theme.text() };
        Line::from(vec![
            Span::styled(if selected { "❯ " } else { "  " }, theme.pick()),
            Span::styled(pad(&fit(&row.text, head), head), style),
            Span::styled(format!(" {tag}"), theme.soft()),
        ])
    }

    fn draw_dropdown(
        &mut self,
        frame: &mut Frame,
        theme: &Theme,
        x: u16,
        y: u16,
        wide: u16,
        tall: u16,
    ) {
        let span = Self::input_span(wide);
        let rows = (tall.saturating_sub(2) as usize).min(self.suggestions.len());
        if rows == 0 {
            return;
        }
        let inner = (span as usize).saturating_sub(6);
        let lines: Vec<Line<'static>> = (0..rows)
            .map(|index| self.suggestion_row(index, inner))
            .collect();
        let area = frame.area();
        for (offset, line) in panel(theme, "Suggestions", span, lines)
            .into_iter()
            .enumerate()
        {
            put(frame, x, y + offset as u16, span, line);
        }
        for index in 0..rows {
            self.zone(
                rect(area, x, y + 1 + index as u16, span, 1),
                Hit::Suggest(index),
            );
        }
    }

    fn profile_left(&self, room: usize) -> Vec<Line<'static>> {
        let theme = Theme::of(self.theme);
        let Some(profile) = &self.profile else {
            return Vec::new();
        };
        let soft = |text: String| Line::styled(fit(&format!("▸ {text}"), room), theme.soft());
        let name = if profile.verified {
            format!("{} ✓", profile.name)
        } else {
            profile.name.clone()
        };
        let mut lines: Vec<Line<'static>> = wrap(&name, room)
            .into_iter()
            .take(2)
            .map(|line| Line::styled(line, theme.bold()))
            .collect();
        let handle = profile
            .handle
            .as_ref()
            .map(|handle| format!("@{}", handle.trim_start_matches('@')))
            .unwrap_or_default();
        lines.push(soft(join(&[profile.kind.label(), &handle])));
        let place = join(&[
            profile.country.as_deref().unwrap_or(""),
            profile.gender.as_deref().unwrap_or(""),
        ]);
        if !place.is_empty() {
            lines.push(soft(place));
        }
        lines.push(Line::raw(""));
        if let Some(count) = profile.subscribers {
            lines.push(soft(format!("{} subscribers", short_count(count))));
        }
        if let Some(count) = profile.video_count {
            lines.push(soft(format!("{} videos", short_count(count))));
        }
        if let Some(count) = profile.views {
            lines.push(soft(format!("{} views", short_count(count))));
        }
        if !profile.aliases.is_empty() {
            lines.push(soft(format!("aka {}", profile.aliases.join(", "))));
        }
        for (key, value) in profile.info.iter().take(3) {
            lines.push(soft(format!("{key}: {value}")));
        }
        if !profile.socials.is_empty() {
            let names: Vec<&str> = profile
                .socials
                .iter()
                .map(|(name, _)| name.as_str())
                .collect();
            lines.push(soft(format!("links: {}", names.join(", "))));
        }
        if let Some(bio) = profile.bio.as_ref().filter(|bio| !bio.is_empty()) {
            lines.push(Line::raw(""));
            for line in wrap(bio, room).into_iter().take(3) {
                lines.push(Line::styled(line, theme.soft()));
            }
        }
        lines
    }

    fn draw_profile(
        &mut self,
        frame: &mut Frame,
        theme: &Theme,
        x: u16,
        y: u16,
        wide: u16,
        tall: u16,
    ) {
        let Some(profile) = self.profile.as_ref() else {
            return;
        };
        let stacked = wide < STACK_BELOW;
        let paging = profile.paging.clone();
        let summary = join(&[
            profile.kind.label(),
            &profile
                .subscribers
                .map(|count| format!("{} subscribers", short_count(count)))
                .unwrap_or_default(),
            &profile
                .video_count
                .map(|count| format!("{} videos", short_count(count)))
                .unwrap_or_default(),
        ]);
        let heading = if profile.verified {
            format!("{} ✓", profile.name)
        } else {
            profile.name.clone()
        };
        let (px, py, pw, ph) = if stacked {
            let head = tall.min(3);
            if head >= 2 {
                put(
                    frame,
                    x,
                    y,
                    wide,
                    Line::styled(fit(&heading, wide as usize), theme.bold()),
                );
                put(
                    frame,
                    x,
                    y + 1,
                    wide,
                    Line::styled(fit(&summary, wide as usize), theme.soft()),
                );
            }
            (x, y + head, wide, tall.saturating_sub(head).max(3))
        } else {
            for (row, line) in self
                .profile_left(SIDE as usize)
                .into_iter()
                .take(tall as usize)
                .enumerate()
            {
                put(frame, x, y + row as u16, SIDE, line);
            }
            (x + SIDE + 2, y, wide - SIDE - 2, tall)
        };
        let rows = (ph.saturating_sub(2) / 2).max(1) as usize;
        if self.psel < self.pscroll {
            self.pscroll = self.psel;
        }
        if self.psel >= self.pscroll + rows {
            self.pscroll = self.psel + 1 - rows;
        }
        let inner = (pw as usize).saturating_sub(6);
        let count = self.profile_videos();
        let last = (self.pscroll + rows).min(count);
        let mut lines = Vec::new();
        if let Some(profile) = &self.profile {
            for index in self.pscroll..last {
                let [first, second] =
                    self.result_rows(&profile.videos[index], index == self.psel, inner);
                lines.push(first);
                lines.push(second);
            }
        }
        if lines.is_empty() {
            lines.push(Line::styled("no videos here", theme.soft()));
        }
        let place = if paging.last > 0 {
            format!("page {}/{}", paging.page, paging.last)
        } else {
            format!("page {}", paging.page)
        };
        let title = join(&["Videos", &place, &self.sort_label()]);
        let area = frame.area();
        for (offset, line) in panel(theme, &title, pw, lines).into_iter().enumerate() {
            put(frame, px, py + offset as u16, pw, line);
        }
        for index in self.pscroll..last {
            let row = py + 1 + ((index - self.pscroll) * 2) as u16;
            self.zone(rect(area, px, row, pw, 2), Hit::Result(index));
        }
    }

    fn draw_input(&mut self, frame: &mut Frame, theme: &Theme, x: u16, y: u16, wide: u16) {
        let button = (BUTTON.len() + 4) as u16;
        let span = Self::input_span(wide);
        let inner = span as usize - 2;
        let editing = self.mode == Mode::Query;
        let dim = self.mode == Mode::Busy;
        let soft = theme.soft();
        let title = fit(self.input_title(), inner.saturating_sub(4));
        let tail = inner.saturating_sub(width(&title) + 3);
        put(
            frame,
            x,
            y,
            span,
            Line::from(vec![
                Span::styled("╭─ ", soft),
                Span::styled(title, theme.text()),
                Span::styled(format!(" {}─", "─".repeat(tail)), soft),
            ]),
        );
        let room = (span as usize).saturating_sub(4);
        let mut spans = vec![Span::styled("│ ", soft), Span::styled("❯ ", theme.text())];
        if self.query.is_empty() && editing {
            spans.push(Span::styled(
                fit(
                    &format!("search {} or paste a link…", self.site.label()),
                    room,
                ),
                soft,
            ));
            frame.set_cursor_position((x + 4, y + 1));
        } else if editing {
            let (shown, column) = window(&self.query, self.caret, room.saturating_sub(1));
            spans.push(Span::styled(shown, theme.text()));
            frame.set_cursor_position((x + 4 + column, y + 1));
        } else {
            spans.push(Span::styled(fit(&self.query, room), soft));
        }
        put(frame, x, y + 1, span, Line::from(spans));
        put(
            frame,
            x,
            y + 2,
            span,
            Line::styled(format!("╰{}─", "─".repeat(inner)), soft),
        );
        let left = x + span;
        put(
            frame,
            left,
            y,
            button,
            Line::styled("▄".repeat(button as usize), theme.fill(dim)),
        );
        put(
            frame,
            left,
            y + 1,
            button,
            Line::styled(format!("  {BUTTON}  "), theme.button(dim)),
        );
        put(
            frame,
            left,
            y + 2,
            button,
            Line::styled("▀".repeat(button as usize), theme.fill(dim)),
        );
        let area = frame.area();
        if !dim {
            self.zone(
                rect(area, left, y, button, 3),
                Hit::Key(KeyCode::Enter, KeyModifiers::NONE),
            );
            if !editing {
                self.zone(
                    rect(area, x, y, span, 3),
                    Hit::Key(KeyCode::Char('/'), KeyModifiers::NONE),
                );
            }
        }
    }

    fn result_rows(&self, video: &Video, selected: bool, room: usize) -> [Line<'static>; 2] {
        let theme = Theme::of(self.theme);
        let marker = if selected { "❯ " } else { "  " };
        let length = width(&video.duration);
        let head = room.saturating_sub(2 + length + 1);
        let title = if video.title.is_empty() {
            video.id.clone()
        } else {
            video.title.clone()
        };
        let title_style = if selected { theme.pick() } else { theme.bold() };
        let first = Line::from(vec![
            Span::styled(marker, theme.pick()),
            Span::styled(pad(&fit(&title, head), head), title_style),
            Span::styled(format!(" {}", video.duration), theme.soft()),
        ]);
        let meta = join(&[&video.channel, &video.views, &video.published]);
        let second = Line::from(vec![
            Span::raw("  "),
            Span::styled(fit(&meta, room.saturating_sub(2)), theme.soft()),
        ]);
        [first, second]
    }

    fn draw_results(
        &mut self,
        frame: &mut Frame,
        theme: &Theme,
        x: u16,
        y: u16,
        wide: u16,
        tall: u16,
    ) {
        let mut top = y;
        let mut room = tall;
        if tall >= 8 {
            self.draw_input(frame, theme, x, y, wide);
            top += 3;
            room -= 3;
        }
        let rows = ((room.saturating_sub(2)) / 2).max(1) as usize;
        if self.sel < self.rscroll {
            self.rscroll = self.sel;
        }
        if self.sel >= self.rscroll + rows {
            self.rscroll = self.sel + 1 - rows;
        }
        let inner = (wide as usize).saturating_sub(6);
        let mut lines = Vec::new();
        let last = (self.rscroll + rows).min(self.results.len());
        for index in self.rscroll..last {
            let [first, second] = self.result_rows(&self.results[index], index == self.sel, inner);
            lines.push(first);
            lines.push(second);
        }
        let more = if self.loading_more {
            format!("Results · {} · loading", self.results.len())
        } else {
            format!("Results · {}", self.results.len())
        };
        let drawn = panel(theme, &more, wide, lines);
        let area = frame.area();
        for (offset, line) in drawn.into_iter().enumerate() {
            put(frame, x, top + offset as u16, wide, line);
        }
        for index in self.rscroll..last {
            let row = top + 1 + ((index - self.rscroll) * 2) as u16;
            self.zone(rect(area, x, row, wide, 2), Hit::Result(index));
        }
    }

    fn left_lines(&self, wide: u16) -> Vec<Line<'static>> {
        let theme = Theme::of(self.theme);
        let room = (wide.saturating_sub(PANEL + 3) as usize).max(10);
        let mut lines: Vec<Line<'static>> = wrap(&self.title(), room)
            .into_iter()
            .take(3)
            .map(|line| Line::styled(line, theme.bold()))
            .collect();
        lines.push(Line::raw(""));
        if let Some(video) = &self.current {
            let meta = join(&[&video.channel, &video.duration, &video.views]);
            if !meta.is_empty() {
                lines.push(Line::styled(fit(&format!("▸ {meta}"), room), theme.soft()));
            }
        }
        if let Some(provider) = self.provider {
            lines.push(Line::styled(
                fit(&format!("▸ source {provider}"), room),
                theme.soft(),
            ));
        }
        if let Some(note) = self.note() {
            lines.push(Line::styled(fit(&format!("▸ {note}"), room), theme.soft()));
        }
        lines
    }

    fn note(&self) -> Option<&'static str> {
        let streams = self.visible();
        let stream = streams.get(self.choice)?;
        Some(match stream.kind {
            Kind::Audio => "audio only",
            Kind::Muxed => "video and audio in one file",
            Kind::Video => {
                let paired = self
                    .sources
                    .as_ref()
                    .and_then(|sources| sources.audio_for(stream))
                    .is_some();
                if paired {
                    "video and audio merged by snag"
                } else {
                    "video only, no matching audio"
                }
            }
        })
    }

    fn choice_row(&self, index: usize, room: usize) -> Line<'static> {
        let theme = Theme::of(self.theme);
        let streams = self.visible();
        let stream = streams[index];
        let selected = index == self.choice;
        let name = if stream.is_audio() {
            format!("♪ {}", stream.quality)
        } else {
            stream.quality.clone()
        };
        let size = if stream.size > 0 {
            format!("~{}", human_size(stream.size))
        } else {
            String::new()
        };
        let head = format!("{}{}", pad(&name, 7), pad(&stream.ext, 5));
        let gap = room.saturating_sub(2 + width(&head) + width(&size));
        let style = if selected { theme.pick() } else { theme.text() };
        Line::from(vec![
            Span::styled(if selected { "❯ " } else { "  " }, theme.pick()),
            Span::styled(head, style),
            Span::raw(" ".repeat(gap)),
            Span::styled(size, if selected { theme.pick() } else { theme.soft() }),
        ])
    }

    fn draw_formats(
        &mut self,
        frame: &mut Frame,
        theme: &Theme,
        x: u16,
        y: u16,
        wide: u16,
        tall: u16,
    ) {
        let stacked = wide < STACK_BELOW;
        let area = frame.area();
        let (panel_x, panel_y, panel_w, panel_h) = if stacked {
            let head = tall.min(3);
            let title = fit(&self.title(), wide as usize);
            if head >= 2 {
                put(frame, x, y, wide, Line::styled(title, theme.bold()));
            }
            (x, y + head, wide, tall.saturating_sub(head).max(3))
        } else {
            let left = self.left_lines(wide);
            let room = wide.saturating_sub(PANEL + 3);
            for (row, line) in left.into_iter().take(tall as usize).enumerate() {
                put(frame, x, y + row as u16, room, line);
            }
            (x + wide - PANEL, y, PANEL, tall)
        };
        let rows = (panel_h.saturating_sub(2)).max(1) as usize;
        let count = self.visible().len();
        if self.choice < self.cscroll {
            self.cscroll = self.choice;
        }
        if self.choice >= self.cscroll + rows {
            self.cscroll = self.choice + 1 - rows;
        }
        let inner = (panel_w as usize).saturating_sub(6);
        let last = (self.cscroll + rows).min(count);
        let lines: Vec<Line<'static>> = (self.cscroll..last)
            .map(|index| self.choice_row(index, inner))
            .collect();
        for (offset, line) in panel(theme, "Download", panel_w, lines)
            .into_iter()
            .enumerate()
        {
            put(frame, panel_x, panel_y + offset as u16, panel_w, line);
        }
        for index in self.cscroll..last {
            let row = panel_y + 1 + (index - self.cscroll) as u16;
            self.zone(rect(area, panel_x, row, panel_w, 1), Hit::Choice(index));
        }
    }

    fn draw_download(&self, frame: &mut Frame, theme: &Theme, x: u16, y: u16, wide: u16) {
        let room = wide as usize;
        let head = join(&[&fit(&self.title(), 42.min(room)), &self.picked]);
        centered(
            frame,
            x,
            y,
            wide,
            Line::styled(fit(&head, room), theme.soft()),
        );
        let span = BAR.min(room.saturating_sub(6)).max(4);
        let merging = self.stage == Stage::Merging;
        let spin = self.spinner();
        let status = |text: String| {
            Line::from(vec![
                Span::styled(spin, theme.text()),
                Span::styled(format!(" {text}"), theme.soft()),
            ])
        };
        if self.cancelling {
            centered(frame, x, y + 2, wide, bar(theme, self.ratio(), span));
            centered(frame, x, y + 4, wide, status("cancelling…".to_string()));
        } else if merging {
            centered(frame, x, y + 2, wide, bar(theme, 1.0, span));
            centered(frame, x, y + 4, wide, status("merging…".to_string()));
        } else if self.total > 0 {
            centered(frame, x, y + 2, wide, bar(theme, self.ratio(), span));
            centered(
                frame,
                x,
                y + 4,
                wide,
                Line::styled(fit(&self.meta(), room), theme.soft()),
            );
        } else if self.progress > 0 {
            centered(frame, x, y + 2, wide, status("downloading…".to_string()));
            centered(
                frame,
                x,
                y + 4,
                wide,
                Line::styled(fit(&self.meta(), room), theme.soft()),
            );
        } else {
            centered(frame, x, y + 2, wide, bar(theme, 0.0, span));
            centered(
                frame,
                x,
                y + 4,
                wide,
                status("starting download…".to_string()),
            );
        }
    }

    fn ratio(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.progress as f64 / self.total as f64
        }
    }

    fn meta(&self) -> String {
        let step = if self.steps.len() > 1 {
            let at = self
                .steps
                .iter()
                .position(|stage| *stage == self.stage)
                .unwrap_or(0);
            let label = match self.stage {
                Stage::FetchingAudio => "audio",
                _ => "video",
            };
            format!("{label} {}/{}", at + 1, self.steps.len())
        } else {
            String::new()
        };
        let seconds = self.phase_started.elapsed().as_secs_f64();
        let rate = if seconds > 0.5 {
            (self.progress as f64 / seconds) as u64
        } else {
            0
        };
        let amount = if self.total > 0 {
            format!("{} / {}", human_size(self.progress), human_size(self.total))
        } else {
            human_size(self.progress)
        };
        let speed = if rate > 0 {
            format!("{}/s", human_size(rate))
        } else {
            String::new()
        };
        let left = if rate > 0 && self.total > self.progress {
            format!("{} left", clock((self.total - self.progress) / rate))
        } else {
            String::new()
        };
        join(&[&step, &amount, &speed, &left])
    }

    fn draw_done(&mut self, frame: &mut Frame, theme: &Theme, x: u16, y: u16, wide: u16) {
        centered(
            frame,
            x,
            y,
            wide,
            Line::from(vec![
                Span::styled("✓ snagged! ", theme.bold()),
                Span::styled("find your file in:", theme.text()),
            ]),
        );
        let path = self
            .saved
            .as_ref()
            .map(|path| tilde(path))
            .unwrap_or_default();
        centered(
            frame,
            x,
            y + 1,
            wide,
            Line::styled(fit(&path, (wide as usize).min(60)), theme.soft()),
        );
        let label = width(AGAIN) as u16;
        let box_w = label + 8;
        let left = x + wide.saturating_sub(box_w) / 2;
        let soft = theme.soft();
        let inner = box_w as usize - 2;
        put(
            frame,
            left,
            y + 3,
            box_w,
            Line::styled(format!("╭{}╮", "─".repeat(inner)), soft),
        );
        put(
            frame,
            left,
            y + 4,
            box_w,
            Line::from(vec![
                Span::styled("│   ", soft),
                Span::styled(AGAIN, theme.bold()),
                Span::styled("   │", soft),
            ]),
        );
        put(
            frame,
            left,
            y + 5,
            box_w,
            Line::styled(format!("╰{}╯", "─".repeat(inner)), soft),
        );
        let area = frame.area();
        self.zone(
            rect(area, left, y + 3, box_w, 3),
            Hit::Key(KeyCode::Enter, KeyModifiers::NONE),
        );
    }

    fn draw_hints(&mut self, frame: &mut Frame, theme: &Theme, area: Rect, y: u16) {
        let mut hints = self.hints();
        let lead = if self.mode == Mode::Busy {
            Some(format!("{} {}", self.spinner(), self.busy))
        } else {
            None
        };
        let lead_width = lead
            .as_ref()
            .map(|text| width(text) + SEPARATOR.len() - 2)
            .unwrap_or(0);
        let measure = |hints: &[Hint]| -> usize {
            let items: usize = hints
                .iter()
                .map(|item| width(&item.key) + 1 + width(&item.label))
                .sum();
            items + hints.len().saturating_sub(1) * (SEPARATOR.len() - 2) + lead_width
        };
        while hints.len() > 1 && measure(&hints) > area.width as usize {
            hints.pop();
        }
        let total = measure(&hints);
        let mut column = area.x + (area.width as usize).saturating_sub(total) as u16 / 2;
        let soft = theme.soft();
        if let Some(text) = lead {
            let used = width(&text) as u16;
            put(
                frame,
                column,
                y,
                used,
                Line::from(vec![
                    Span::styled(text.chars().next().unwrap().to_string(), theme.text()),
                    Span::styled(text.chars().skip(1).collect::<String>(), soft),
                ]),
            );
            column += used;
            put(frame, column, y, 5, Line::styled(SEPARATOR, soft));
            column += 5;
        }
        for (index, item) in hints.iter().enumerate() {
            if index > 0 {
                put(frame, column, y, 5, Line::styled(SEPARATOR, soft));
                column += 5;
            }
            let used = (width(&item.key) + 1 + width(&item.label)) as u16;
            put(
                frame,
                column,
                y,
                used,
                Line::from(vec![
                    Span::styled(item.key.clone(), theme.text()),
                    Span::styled(format!(" {}", item.label), soft),
                ]),
            );
            if let Some((code, modifiers)) = item.press {
                self.zone(rect(area, column, y, used, 1), Hit::Key(code, modifiers));
            }
            column += used;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixture::{app, profiled, suggested};
    use super::super::theme::ThemeMode;
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::PathBuf;

    fn draw(app: &mut App, wide: u16, tall: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(wide, tall)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let mut out = String::new();
        for row in 0..tall {
            let mut line = String::new();
            let mut column = 0;
            while column < wide {
                let symbol = buffer[(column, row)].symbol();
                line.push_str(symbol);
                column += width(symbol).max(1) as u16;
            }
            out.push_str(line.trim_end());
            out.push('\n');
        }
        out
    }

    #[test]
    fn query_screen_has_logo_framed_input_and_button() {
        let mut app = app();
        app.mode = Mode::Query;
        app.query.clear();
        app.caret = 0;
        let screen = draw(&mut app, 90, 30);
        assert!(screen.contains("█▀▀ █▀▄█ █▀█ █▀▀▀"));
        assert!(screen.contains("╭─ Search or paste a link"));
        assert!(screen.contains("search youtube or paste a link…"));
        assert!(screen.contains("▄▄▄▄▄▄▄▄") && screen.contains("▀▀▀▀▀▀▀▀"));
        assert!(screen.lines().any(|line| line.ends_with("  snag")));
        assert!(screen.contains("↵ search"));
        assert!(screen.contains("^t theme:auto"));
    }

    #[test]
    fn dropdown_lists_suggestions_with_tags_inside_the_frame() {
        let mut app = suggested();
        app.suggest_sel = Some(1);
        let screen = draw(&mut app, 90, 30);
        assert!(screen.contains("╭─ Suggestions"));
        assert!(screen.contains("  sam smith"));
        assert!(screen.contains("❯ Sample Creator"));
        assert!(screen.contains("creator · 88"));
        assert!(screen.contains("^s site:youtube"));
        for line in screen.lines() {
            assert!(width(line) <= 90, "{line}");
        }
        assert!(!app.zones.is_empty());
    }

    #[test]
    fn profile_shows_details_beside_a_paged_video_panel() {
        let mut app = profiled();
        let screen = draw(&mut app, 100, 32);
        assert!(screen.contains("Sample Creator ✓"));
        assert!(screen.contains("creator · @sample"));
        assert!(screen.contains("Norway · female"));
        assert!(screen.contains("120K subscribers"));
        assert!(screen.contains("aka Sam, Sammy"));
        assert!(screen.contains("╭─ Videos · page 1/3 · featured"));
        assert!(screen.contains("❯ Profile clip number 0"));
        assert!(screen.contains("sort:featured"));
        for line in screen.lines() {
            assert!(width(line) <= 100, "{line}");
        }
    }

    #[test]
    fn profile_stacks_and_survives_tiny_terminals() {
        let mut app = profiled();
        let screen = draw(&mut app, 50, 24);
        assert!(screen.contains("Sample Creator ✓"));
        assert!(screen.contains("Videos · page 1/3"));
        for (wide, tall) in [(16, 5), (30, 8), (40, 12), (64, 14), (120, 50)] {
            let mut app = profiled();
            app.psel = 5;
            let screen = draw(&mut app, wide, tall);
            for line in screen.lines() {
                assert!(width(line) <= wide as usize, "{line}");
            }
        }
    }

    #[test]
    fn query_placeholder_names_the_site() {
        let mut app = app();
        app.mode = Mode::Query;
        app.query.clear();
        app.caret = 0;
        app.site = crate::site::Site::XHamster;
        let screen = draw(&mut app, 90, 30);
        assert!(screen.contains("search xhamster or paste a link…"));
        assert!(screen.contains("^s site:xhamster"));
    }

    #[test]
    fn every_screen_renders_at_every_size_without_panicking() {
        let sizes = [
            (1, 1),
            (16, 5),
            (20, 6),
            (30, 9),
            (45, 13),
            (63, 20),
            (64, 24),
            (80, 40),
            (200, 60),
        ];
        for (wide, tall) in sizes {
            for mode in [
                Mode::Query,
                Mode::Results,
                Mode::Formats,
                Mode::Profile,
                Mode::Busy,
                Mode::Download,
                Mode::Done,
                Mode::Error,
            ] {
                let mut screen = profiled();
                screen.suggestions = suggested().suggestions;
                screen.suggest_sel = Some(1);
                screen.query = "sam".to_string();
                screen.caret = 3;
                screen.error = "something failed with a long message ".repeat(4);
                screen.mode = mode;
                let text = draw(&mut screen, wide, tall);
                for line in text.lines() {
                    assert!(
                        width(line) <= wide as usize,
                        "{mode:?} {wide}x{tall}: {line}"
                    );
                }
            }
        }
    }

    #[test]
    fn results_clip_wide_text_inside_the_frame() {
        let mut app = app();
        let screen = draw(&mut app, 70, 30);
        assert!(screen.contains("Results · 9"));
        assert!(screen.contains("❯ Result number 0"));
        for line in screen.lines() {
            assert!(width(line) <= 70, "{line}");
        }
        assert!(screen.contains("日本語"));
    }

    #[test]
    fn formats_panel_lists_videos_then_audio_with_sizes() {
        let mut app = app();
        app.mode = Mode::Formats;
        let screen = draw(&mut app, 100, 30);
        assert!(screen.contains("╭─ Download"));
        let video = screen.find("720p").unwrap();
        let muxed = screen.find("360p").unwrap();
        let audio = screen.find("♪ 128k").unwrap();
        assert!(video < muxed && muxed < audio);
        assert!(screen.contains("~19.1 MB") || screen.contains("~20.0 MB"));
        assert!(screen.contains("video and audio merged by snag"));
        assert!(screen.contains("↵ download"));
        assert!(screen.contains("m mpv"));
        assert!(screen.contains("v vlc"));
    }

    #[test]
    fn click_zones_cover_rows_buttons_and_hints() {
        let mut app = app();
        draw(&mut app, 90, 30);
        assert!(app
            .zones
            .iter()
            .any(|zone| matches!(zone.hit, Hit::Result(0))));
        assert!(app.zones.iter().any(|zone| zone.hit == Hit::Logo));
        assert!(app
            .zones
            .iter()
            .any(|zone| zone.hit == Hit::Key(KeyCode::Enter, KeyModifiers::NONE)));
        app.mode = Mode::Formats;
        draw(&mut app, 90, 30);
        assert!(app
            .zones
            .iter()
            .any(|zone| matches!(zone.hit, Hit::Choice(2))));
    }

    #[test]
    fn every_screen_renders_in_every_theme_at_small_sizes() {
        for theme in [ThemeMode::Auto, ThemeMode::Light, ThemeMode::Dark] {
            for mode in [
                Mode::Query,
                Mode::Results,
                Mode::Formats,
                Mode::Busy,
                Mode::Download,
                Mode::Done,
                Mode::Error,
            ] {
                for (wide, tall) in [
                    (14, 4),
                    (16, 5),
                    (24, 8),
                    (40, 12),
                    (60, 20),
                    (80, 24),
                    (200, 60),
                ] {
                    let mut app = app();
                    app.theme = theme;
                    app.mode = mode;
                    app.error = "something went wrong while talking to the server".into();
                    app.saved = Some(PathBuf::from("/tmp/out/file.mp4"));
                    app.steps = vec![Stage::Fetching, Stage::FetchingAudio, Stage::Merging];
                    app.total = 1000;
                    app.progress = 400;
                    app.choice = 2;
                    app.sel = 8;
                    app.query = "a fairly long query that has to scroll inside the frame".into();
                    app.caret = app.query.chars().count();
                    app.notice = Some(("note".into(), std::time::Instant::now()));
                    draw(&mut app, wide, tall);
                }
            }
        }
    }

    #[test]
    fn download_and_done_screens_describe_progress() {
        let mut app = app();
        app.mode = Mode::Download;
        app.picked = "720p mp4".into();
        app.steps = vec![Stage::Fetching, Stage::FetchingAudio, Stage::Merging];
        app.total = 20_000_000;
        app.progress = 5_000_000;
        let screen = draw(&mut app, 90, 30);
        assert!(screen.contains("A title") || screen.contains("Result number 0"));
        assert!(screen.contains("720p mp4"));
        assert!(screen.contains("25%"));
        assert!(screen.contains("video 1/3"));
        assert!(screen.contains("esc cancel"));
        app.stage = Stage::Merging;
        let screen = draw(&mut app, 90, 30);
        assert!(screen.contains("100%"));
        assert!(screen.contains("merging…"));
        assert!(!screen.contains("esc cancel"));
        app.mode = Mode::Done;
        app.saved = Some(PathBuf::from("/tmp/out/A title.mp4"));
        let screen = draw(&mut app, 90, 30);
        assert!(screen.contains("✓ snagged! find your file in:"));
        assert!(screen.contains("↵ snag another"));
        assert!(screen.contains("p play"));
    }

    #[test]
    fn error_screen_wraps_the_message() {
        let mut app = app();
        app.fail("x ".repeat(60), Mode::Query);
        let screen = draw(&mut app, 50, 20);
        assert!(screen.contains("✗ x x"));
        for line in screen.lines() {
            assert!(width(line) <= 50);
        }
    }

    #[test]
    fn light_theme_paints_a_background() {
        let mut app = app();
        app.theme = ThemeMode::Light;
        let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        assert_eq!(
            buffer[(0, 0)].bg,
            ratatui::style::Color::Rgb(0xff, 0xff, 0xff)
        );
    }

    #[test]
    fn window_keeps_the_caret_visible() {
        let (shown, column) = window("abcdefghij", 10, 5);
        assert_eq!(shown, "ghij");
        assert_eq!(column, 4);
        let (shown, column) = window("abcdefghij", 3, 5);
        assert_eq!(shown, "abcde");
        assert_eq!(column, 3);
    }
}
