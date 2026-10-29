mod download;
mod hls;
mod hlsdl;
mod layout;
mod model;
pub mod mux;
mod photos;
mod play;
mod sites;
mod source;
#[cfg(test)]
mod testserver;
mod tui;
mod util;

use std::env;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use download::Job;
use layout::Layout;
use model::{Media, Scope, Video};
use sites::{Entry, Site};

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
    galleries: bool,
    list: bool,
    help: bool,
    version: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("nab: {error}");
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
        println!("nab {}", env!("CARGO_PKG_VERSION"));
        return Ok(ExitCode::SUCCESS);
    }
    if args.audio && args.video {
        return Err("choose either --audio or --video".to_string());
    }
    let plain = !args.audio && !args.video && !args.play && !args.list && args.quality.is_none();
    let terminal = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let layout = Layout::new(args.out.clone().unwrap_or_else(layout::default_root));
    if plain && terminal {
        let player = match args.player.as_deref() {
            Some(name) => Some(play::choose(Some(name))?),
            None => play::detect(),
        };
        return tui::run(args.query, layout, player, args.theme, args.site)
            .map(|_| ExitCode::SUCCESS);
    }
    if args.query.trim().is_empty() {
        print_help();
        return Ok(ExitCode::FAILURE);
    }
    let mut site = args.site.unwrap_or_else(Site::first);
    let scope = if args.galleries {
        Scope::Galleries
    } else {
        Scope::Videos
    };
    if !site.supports(scope) {
        return Err(format!("{} has no galleries", site.title()));
    }
    if let Some(linked) = sites::link_site(&args.query) {
        site = linked;
    }
    let mut videos: Vec<Video>;
    match sites::classify(&args.query, site, scope) {
        Entry::Video(video) => videos = vec![video],
        Entry::Profile(owner, url) => {
            videos = owner.profile(&url, "featured", scope, 1)?.videos;
        }
        Entry::Search(query, scope) => {
            let page = site.search(&query, scope)?;
            videos = page.videos;
            let mut cursor = page.cursor;
            while videos.len() < args.limit {
                let Some(next) = cursor.take() else {
                    break;
                };
                let page = sites::more(&next)?;
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
    let sources = sites::resolve(target)?;
    let title = sources
        .title
        .clone()
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| target.title.clone());
    let owner = Site::named(target.site).unwrap_or(site);
    let max_height = args
        .quality
        .as_deref()
        .and_then(|quality| quality.trim_end_matches('p').parse::<u32>().ok());
    if target.media == Media::Gallery {
        if args.play {
            return Err("galleries are saved, not played".to_string());
        }
        if args.audio {
            return Err("galleries have no audio".to_string());
        }
        let stream = sources
            .gallery()
            .ok_or_else(|| "no photos found".to_string())?;
        let job = Job::Gallery {
            photos: sources.photos.clone(),
            referer: stream.referer.map(str::to_string),
            title,
            id: target.id.clone(),
        };
        let dir = layout.dir(owner.folder(), job.folder());
        let path = download::run_blocking(job, dir)?;
        println!("{}", path.display());
        return Ok(ExitCode::SUCCESS);
    }
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
    let dir = layout.dir(owner.folder(), job.folder());
    let path = download::run_blocking(job, dir)?;
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
        galleries: false,
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
            "-g" | "--galleries" => args.galleries = true,
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
                    .ok_or_else(|| format!("--site needs {}", Site::names()))?;
                args.site = Some(
                    Site::parse(&value).ok_or_else(|| format!("--site needs {}", Site::names()))?,
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

fn print_help() {
    let version = env!("CARGO_PKG_VERSION");
    let sites = Site::names();
    println!(
        "nab {version}
search {sites}, then nab the audio, video or photo gallery

usage:
  nab <query>                        open the picker
  nab <link>                         video, gallery or profile links work too
  nab --audio <query>                save audio of the top result
  nab --video <query>                save video of the top result
  nab --video --quality 480p <query> pick a height
  nab --play [--video] <query>       play with mpv or vlc
  nab --list <query>                 print results
  nab --site xhamster <query>        search another site
  nab --galleries <query>            search photo galleries, where a site has them

options:
  -a, --audio             save audio
  -v, --video             save video (a gallery's photos too)
  -q, --quality <height>  144p, 240p, 360p, 480p, 720p
  -p, --play              play instead of saving
  -g, --galleries         search galleries instead of videos
  -o, --out <dir>         output folder, default Downloads/Nab
      --site <name>       {sites} (ctrl-s switches in the picker)
      --player <name>     mpv or vlc, default whatever is installed
      --theme <mode>      auto, light or dark (ctrl-t cycles in the picker)
  -n, --limit <count>     search results to show (default 20)
  -l, --list              print results and exit
  -h, --help              show this help
  -V, --version           show the version

files are sorted into <out>/<Site>/<Videos|Audios|Galleries>, for example
Downloads/Nab/Youtube/Videos. video and audio are merged by nab itself;
playback needs mpv or vlc. in the picker tab switches a video's sources,
ctrl-g flips between videos and galleries, and profiles open from results."
    );
}
