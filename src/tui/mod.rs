mod app;
mod clip;
#[cfg(test)]
mod fixture;
mod logo;
mod store;
mod theme;
pub use theme::ThemeMode;
mod view;

use std::io::{stdout, Stdout};
use std::path::PathBuf;
use std::time::Duration;

use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::play::{self, Player};
use crate::search;
use app::{App, Flow, Mode};

type Term = Terminal<CrosstermBackend<Stdout>>;

fn failure(error: impl std::fmt::Display) -> String {
    format!("terminal error: {error}")
}

fn setup() -> Result<Term, String> {
    enable_raw_mode().map_err(failure)?;
    execute!(
        stdout(),
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste,
        Hide
    )
    .map_err(failure)?;
    Terminal::new(CrosstermBackend::new(stdout())).map_err(failure)
}

fn teardown() -> Result<(), String> {
    disable_raw_mode().map_err(failure)?;
    execute!(
        stdout(),
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen,
        Show
    )
    .map_err(failure)
}

fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(
            stdout(),
            DisableBracketedPaste,
            DisableMouseCapture,
            LeaveAlternateScreen,
            Show
        );
        original(info);
    }));
}

fn drain() {
    while event::poll(Duration::ZERO).unwrap_or(false) {
        let _ = event::read();
    }
}

fn hand_over(
    terminal: &mut Term,
    app: &mut App,
    player: Player,
    run: impl FnOnce(Player) -> Result<(), String>,
) -> Result<(), String> {
    teardown()?;
    let outcome = run(player);
    *terminal = setup()?;
    drain();
    terminal.clear().map_err(failure)?;
    if let Err(error) = outcome {
        let back = if app.mode == Mode::Done {
            Mode::Done
        } else {
            Mode::Formats
        };
        app.fail(error, back);
    }
    Ok(())
}

fn event_loop(mut terminal: Term, mut app: App) -> Result<(), String> {
    loop {
        terminal.draw(|frame| app.render(frame)).map_err(failure)?;
        let mut flow = Flow::Continue;
        if event::poll(Duration::from_millis(60)).map_err(failure)? {
            loop {
                match event::read().map_err(failure)? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => flow = app.on_key(key),
                    Event::Mouse(mouse) => flow = app.on_mouse(mouse),
                    Event::Paste(text) => flow = app.on_paste(text),
                    _ => {}
                }
                if !matches!(flow, Flow::Continue)
                    || !event::poll(Duration::ZERO).map_err(failure)?
                {
                    break;
                }
            }
        }
        app.tick();
        match flow {
            Flow::Continue => {}
            Flow::Quit => return Ok(()),
            Flow::Play {
                video,
                audio,
                title,
                player,
            } => {
                hand_over(&mut terminal, &mut app, player, |player| {
                    play::streams(video.as_ref(), audio.as_ref(), &title, player)
                })?;
            }
            Flow::PlayFile {
                path,
                video,
                player,
            } => {
                hand_over(&mut terminal, &mut app, player, |player| {
                    play::file(&path, video, player)
                })?;
            }
        }
    }
}

pub fn run(
    query: String,
    out_dir: PathBuf,
    player: Option<Player>,
    theme: Option<theme::ThemeMode>,
) -> Result<(), String> {
    install_panic_hook();
    let terminal = setup()?;
    let mut app = App::new(query, out_dir, player)
        .with_store(store::Store::locate())
        .with_theme(theme);
    app.watch_clipboard();
    let query = app.query.trim().to_string();
    if !query.is_empty() {
        if let Some(id) = search::video_id(&query) {
            app.start_resolve(search::Video::stub(&id), Mode::Query);
        } else {
            app.start_search(query);
        }
    }
    let outcome = event_loop(terminal, app);
    let _ = teardown();
    outcome
}
