mod download;
mod hls;
mod hlsdl;
mod innertube;
pub mod mux;
mod play;
mod search;
mod site;
mod source;
#[cfg(test)]
mod testserver;
mod tui;
mod util;
mod xh;
mod ytultra;

use std::env;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use download::Job;
use search::Video;
use site::{Entry, Site};

struct Args {
    query: String,
    audio: bool,
    video: bool,
    play: bool,
    quality: Option<String>,
    out: Option<PathBuf>,
    limit: usize,
    player: Option<String>,
    theme: Option<tui::ThemeMode>,
    site: Option<Site>,
    list: bool,
    help: bool,
    version: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("snag: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let args = parse_args()?;
    if args.help {
        print_help();
        return Ok(ExitCode::SUCCESS);
    }
    if args.version {
        println!("snag {}", env!("CARGO_PKG_VERSION"));
        return Ok(ExitCode::SUCCESS);
    }
    if args.audio && args.video {
        return Err("choose either --audio or --video".to_string());
    }
    let plain = !args.audio && !args.video && !args.play && !args.list && args.quality.is_none();
    let terminal = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let out_dir = args.out.clone().unwrap_or_else(default_dir);
    if plain && terminal {
        let player = match args.player.as_deref() {
            Some(name) => Some(play::choose(Some(name))?),
            None => play::detect(),
        };
        return tui::run(args.query, out_dir, player, args.theme, args.site)
            .map(|_| ExitCode::SUCCESS);
    }
    if args.query.trim().is_empty() {
        print_help();
        return Ok(ExitCode::FAILURE);
    }
    let mut site = args.site.unwrap_or(Site::YouTube);
    let mut videos: Vec<Video>;
    match site::classify(&args.query) {
        Entry::Video(video) => videos = vec![video],
        Entry::Profile(url) => {
            let profile = site::profile(&url, "featured", 1)?;
            videos = profile.videos;
        }
        Entry::Search(query) => {
            if query != args.query.trim() {
                site = Site::XHamster;
            }
            let page = site::search(site, &query)?;
            videos = page.videos;
            let mut cursor = page.cursor;
            while videos.len() < args.limit {
                let Some(next) = cursor.take() else {
                    break;
                };
                let page = site::more(&next)?;
                if page.videos.is_empty() {
                    break;
                }
                page.extend(&mut videos);
                cursor = page.cursor;
            }
        }
    }
    videos.truncate(args.limit);
    if videos.is_empty() {
        return Err("no results".to_string());
    }
    if args.list || (!args.audio && !args.video && !args.play) {
        print_results(&videos);
        return Ok(ExitCode::SUCCESS);
    }
    let target = &videos[0];
    let sources = site::resolve(target)?;
    let title = sources
        .title
        .clone()
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| target.title.clone());
    let max_height = args
        .quality
        .as_deref()
        .and_then(|quality| quality.trim_end_matches('p').parse::<u32>().ok());
    if args.play {
        let player = play::choose(args.player.as_deref())?;
        if args.video || max_height.is_some() {
            let video = sources
                .best_video(max_height)
                .ok_or_else(|| "no video stream".to_string())?;
            let audio = sources.audio_for(video).map(source::Stream::remote);
            play::streams(Some(&video.remote()), audio.as_ref(), &title, player)?;
        } else {
            let audio = sources
                .audio()
                .ok_or_else(|| "no audio stream".to_string())?;
            play::streams(None, Some(&audio.remote()), &title, player)?;
        }
        return Ok(ExitCode::SUCCESS);
    }
    let job = if args.audio {
        let audio = sources
            .audio()
            .ok_or_else(|| "no audio stream".to_string())?;
        Job::Audio {
            audio: audio.remote(),
            title,
            id: target.id.clone(),
        }
    } else {
        let video = sources
            .best_video(max_height)
            .ok_or_else(|| "no video stream".to_string())?;
        Job::Video {
            video: video.remote(),
            audio: sources.audio_for(video).map(source::Stream::remote),
            title,
            id: target.id.clone(),
        }
    };
    let path = download::run_blocking(job, out_dir)?;
    println!("{}", path.display());
    Ok(ExitCode::SUCCESS)
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        query: String::new(),
        audio: false,
        video: false,
        play: false,
        quality: None,
        out: None,
        limit: 20,
        player: None,
        theme: None,
        site: None,
        list: false,
        help: false,
        version: false,
    };
    let mut words: Vec<String> = Vec::new();
    let mut iter = env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => args.help = true,
            "-V" | "--version" => args.version = true,
            "-a" | "--audio" => args.audio = true,
            "-v" | "--video" => args.video = true,
            "-p" | "--play" => args.play = true,
            "-l" | "--list" => args.list = true,
            "-q" | "--quality" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--quality needs a height".to_string())?;
                args.quality = Some(normalize_quality(&value)?);
            }
            "-o" | "--out" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--out needs a directory".to_string())?;
                args.out = Some(PathBuf::from(value));
            }
            "-n" | "--limit" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--limit needs a number".to_string())?;
                args.limit = value
                    .parse::<usize>()
                    .map_err(|_| "--limit needs a number".to_string())?
                    .max(1);
            }
            "--player" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--player needs mpv or vlc".to_string())?;
                args.player = Some(value);
            }
            "--site" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--site needs youtube or xhamster".to_string())?;
                args.site = Some(
                    Site::parse(&value)
                        .ok_or_else(|| "--site needs youtube or xhamster".to_string())?,
                );
            }
            "--theme" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--theme needs auto, light or dark".to_string())?;
                args.theme = Some(
                    tui::ThemeMode::parse(&value)
                        .ok_or_else(|| "--theme needs auto, light or dark".to_string())?,
                );
            }
            "--" => {
                for rest in iter.by_ref() {
                    words.push(rest);
                }
            }
            other if other.starts_with('-') && other != "-" => {
                return Err(format!("unknown option {other}"));
            }
            other => words.push(other.to_string()),
        }
    }
    args.query = words.join(" ");
    Ok(args)
}

fn normalize_quality(value: &str) -> Result<String, String> {
    let trimmed = value.trim().to_ascii_lowercase();
    let digits = trimmed.trim_end_matches('p');
    if digits.is_empty() || !digits.chars().all(|character| character.is_ascii_digit()) {
        return Err(format!("unknown quality {value}"));
    }
    Ok(format!("{digits}p"))
}

fn print_results(videos: &[Video]) {
    for (index, video) in videos.iter().enumerate() {
        println!(
            "{:>2}  {:>6}  {:<12}  {}",
            index + 1,
            video.duration,
            video.views,
            video.title
        );
    }
}

fn default_dir() -> PathBuf {
    if let Ok(dir) = env::var("XDG_DOWNLOAD_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    if let Some(home) = env::var_os("HOME") {
        let downloads = PathBuf::from(&home).join("Downloads");
        if downloads.is_dir() {
            return downloads;
        }
        return PathBuf::from(home);
    }
    PathBuf::from(".")
}

fn print_help() {
    let version = env!("CARGO_PKG_VERSION");
    println!(
        "snag {version}
search youtube or xhamster, then snag the audio or video

usage:
  snag <query>                        open the picker
  snag --audio <query>                save audio of the top result
  snag --video <query>                save video of the top result
  snag --video --quality 480p <query> pick a height
  snag --play [--video] <query>       play with mpv or vlc
  snag --list <query>                 print results
  snag --site xhamster <query>        search xhamster instead of youtube
  snag <xhamster link>                video, profile or search links work too

options:
  -a, --audio             save audio
  -v, --video             save video
  -q, --quality <height>  144p, 240p, 360p, 480p, 720p
  -p, --play              play instead of saving
  -o, --out <dir>         output directory
  -n, --limit <count>     search results to show (default 20)
      --player <name>     mpv or vlc, default whatever is installed
      --site <name>       youtube or xhamster (ctrl-s switches in the picker)
      --theme <mode>      auto, light or dark (ctrl-t cycles in the picker)
  -l, --list              print results and exit
  -h, --help              show this help
  -V, --version           show the version

files land in $XDG_DOWNLOAD_DIR, ~/Downloads or $HOME.
video and audio are merged by snag itself; playback needs mpv or vlc.
youtube streams come straight from youtube with ytultra alongside; xhamster
streams come from the page's initials data and from the page source. in the
picker press tab to switch which provider's sources you are looking at.
the picker suggests searches as you type, opens creator, channel and pornstar
profiles, takes mouse clicks, remembers searches (up arrow) and offers a link
from your clipboard (tab)."
    );
}
