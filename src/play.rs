use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::download::{self, Job, Remote};
use crate::mux;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/156.0.0.0 Safari/537.36";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Player {
    Mpv,
    Vlc,
}

impl Player {
    pub fn name(self) -> &'static str {
        match self {
            Player::Mpv => "mpv",
            Player::Vlc => "vlc",
        }
    }

    fn binary(self) -> &'static str {
        self.name()
    }

    fn extra_locations(self) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = Vec::new();
        match self {
            Player::Vlc => {
                for var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
                    if let Some(base) = std::env::var_os(var) {
                        found.push(
                            PathBuf::from(base)
                                .join("VideoLAN")
                                .join("VLC")
                                .join("vlc.exe"),
                        );
                    }
                }
                found.push(PathBuf::from("/Applications/VLC.app/Contents/MacOS/VLC"));
                if let Some(home) = std::env::var_os("HOME") {
                    found.push(PathBuf::from(home).join("Applications/VLC.app/Contents/MacOS/VLC"));
                }
                found.push(PathBuf::from("/snap/bin/vlc"));
            }
            Player::Mpv => {
                found.push(PathBuf::from("/opt/homebrew/bin/mpv"));
                found.push(PathBuf::from("/usr/local/bin/mpv"));
                found.push(PathBuf::from("/Applications/mpv.app/Contents/MacOS/mpv"));
                found.push(PathBuf::from("/snap/bin/mpv"));
                if let Some(base) = std::env::var_os("ProgramFiles") {
                    found.push(PathBuf::from(base).join("mpv").join("mpv.exe"));
                }
            }
        }
        found
    }
}

pub fn locate(player: Player) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    locate_in(player, &dirs, &player.extra_locations())
}

fn locate_in(player: Player, dirs: &[PathBuf], extras: &[PathBuf]) -> Option<PathBuf> {
    let name = player.binary();
    for dir in dirs {
        for candidate in [dir.join(name), dir.join(format!("{name}.exe"))] {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    extras.iter().find(|candidate| candidate.is_file()).cloned()
}

pub fn detect() -> Option<Player> {
    [Player::Mpv, Player::Vlc]
        .into_iter()
        .find(|player| locate(*player).is_some())
}

pub fn choose(preference: Option<&str>) -> Result<Player, String> {
    match preference {
        Some(name) => {
            let player = match name.trim().to_ascii_lowercase().as_str() {
                "mpv" => Player::Mpv,
                "vlc" => Player::Vlc,
                other => return Err(format!("unknown player {other}")),
            };
            if locate(player).is_some() {
                Ok(player)
            } else {
                Err(format!("{} is not installed", player.name()))
            }
        }
        None => detect().ok_or_else(|| "install mpv or vlc to play".to_string()),
    }
}

fn os(text: impl Into<OsString>) -> OsString {
    text.into()
}

fn mpv_args(
    video: Option<&str>,
    audio: Option<&str>,
    title: &str,
) -> Result<Vec<OsString>, String> {
    let mut args = vec![
        os(format!("--force-media-title={title}")),
        os(format!("--user-agent={USER_AGENT}")),
        os("--ytdl=no"),
    ];
    match (video, audio) {
        (Some(video), Some(audio)) => {
            args.push(os(format!("--audio-file={audio}")));
            args.push(os("--"));
            args.push(os(video));
        }
        (Some(video), None) => {
            args.push(os("--"));
            args.push(os(video));
        }
        (None, Some(audio)) => {
            args.push(os("--no-video"));
            args.push(os("--force-window=no"));
            args.push(os("--"));
            args.push(os(audio));
        }
        (None, None) => return Err("nothing to play".to_string()),
    }
    Ok(args)
}

fn vlc_args(
    video: Option<&str>,
    audio: Option<&str>,
    title: &str,
) -> Result<Vec<OsString>, String> {
    let mut args = vec![os("--play-and-exit")];
    let target = match (video, audio) {
        (Some(video), _) => video,
        (None, Some(audio)) => {
            args.push(os("--no-video"));
            audio
        }
        (None, None) => return Err("nothing to play".to_string()),
    };
    args.push(os(target));
    args.push(os(format!(":meta-title={title}")));
    args.push(os(format!(":http-user-agent={USER_AGENT}")));
    if let (Some(_), Some(audio)) = (video, audio) {
        args.push(os(format!(":input-slave={audio}")));
    }
    Ok(args)
}

fn file_args(player: Player, path: &Path, video: bool) -> Vec<OsString> {
    let mut args = Vec::new();
    match player {
        Player::Mpv => {
            if !video {
                args.push(os("--no-video"));
                args.push(os("--force-window=no"));
            }
            args.push(os("--"));
        }
        Player::Vlc => {
            args.push(os("--play-and-exit"));
            if !video {
                args.push(os("--no-video"));
            }
        }
    }
    args.push(path.as_os_str().to_os_string());
    args
}

fn launch(program: &Path, args: &[OsString], player: Player) -> Result<(), String> {
    let status = Command::new(program)
        .args(args)
        .status()
        .map_err(|error| format!("could not start {}: {error}", player.name()))?;
    if !status.success() {
        return Err(format!("{} exited with {status}", player.name()));
    }
    Ok(())
}

fn program(player: Player) -> Result<PathBuf, String> {
    locate(player).ok_or_else(|| format!("{} is not installed", player.name()))
}

pub fn streams(
    video: Option<&Remote>,
    audio: Option<&Remote>,
    title: &str,
    player: Player,
) -> Result<(), String> {
    play_streams(&program(player)?, video, audio, title, player)
}

fn play_streams(
    program: &Path,
    video: Option<&Remote>,
    audio: Option<&Remote>,
    title: &str,
    player: Player,
) -> Result<(), String> {
    if player == Player::Vlc {
        if let (Some(video), Some(audio)) = (video, audio) {
            if mux::pairs(&video.ext, &audio.ext) {
                match play_merged(program, video, audio, title) {
                    Ok(()) => return Ok(()),
                    Err(Fallback::Failed(error)) => return Err(error),
                    Err(Fallback::Unavailable) => {}
                }
            }
        }
    }
    let video_url = video.map(|remote| remote.url.as_str());
    let audio_url = audio.map(|remote| remote.url.as_str());
    let args = match player {
        Player::Mpv => mpv_args(video_url, audio_url, title)?,
        Player::Vlc => vlc_args(video_url, audio_url, title)?,
    };
    launch(program, &args, player)
}

enum Fallback {
    Unavailable,
    Failed(String),
}

fn play_merged(
    program: &Path,
    video: &Remote,
    audio: &Remote,
    title: &str,
) -> Result<(), Fallback> {
    let dir = std::env::temp_dir().join(format!("snag-play-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let cleanup = Cleanup(dir.clone());
    let job = Job::Video {
        video: video.clone(),
        audio: Some(audio.clone()),
        title: title.to_string(),
        id: "play".to_string(),
    };
    let merged = download::run_blocking(job, dir).map_err(|_| Fallback::Unavailable)?;
    let args = file_args(Player::Vlc, &merged, true);
    let outcome = launch(program, &args, Player::Vlc).map_err(Fallback::Failed);
    drop(cleanup);
    outcome
}

struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn file(path: &Path, video: bool, player: Player) -> Result<(), String> {
    launch(&program(player)?, &file_args(player, path, video), player)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn names_players() {
        assert_eq!(Player::Mpv.name(), "mpv");
        assert_eq!(Player::Vlc.name(), "vlc");
        assert!(choose(Some("nope")).is_err());
    }

    #[test]
    fn mpv_streams_both_urls_without_ytdl() {
        let args = text(&mpv_args(Some("https://v"), Some("https://a"), "Clip").unwrap());
        assert!(args.contains(&"--force-media-title=Clip".to_string()));
        assert!(args.contains(&"--ytdl=no".to_string()));
        assert!(args.contains(&"--audio-file=https://a".to_string()));
        assert_eq!(args.last().unwrap(), "https://v");
        let dash = args.iter().position(|arg| arg == "--").unwrap();
        assert_eq!(dash, args.len() - 2);
    }

    #[test]
    fn mpv_audio_only_hides_the_window() {
        let args = text(&mpv_args(None, Some("https://a"), "Clip").unwrap());
        assert!(args.contains(&"--no-video".to_string()));
        assert!(args.contains(&"--force-window=no".to_string()));
        assert!(mpv_args(None, None, "Clip").is_err());
    }

    #[test]
    fn vlc_gets_item_options_after_the_target() {
        let args = text(&vlc_args(Some("https://v"), Some("https://a"), "Clip").unwrap());
        assert_eq!(args[0], "--play-and-exit");
        assert_eq!(args[1], "https://v");
        assert!(args.contains(&":meta-title=Clip".to_string()));
        assert!(args.contains(&":input-slave=https://a".to_string()));
        assert!(args.iter().any(|arg| arg.starts_with(":http-user-agent=")));
        let audio = text(&vlc_args(None, Some("https://a"), "Clip").unwrap());
        assert!(audio.contains(&"--no-video".to_string()));
        assert!(!audio.iter().any(|arg| arg.starts_with(":input-slave")));
    }

    #[test]
    fn file_args_follow_the_player() {
        let path = Path::new("/tmp/a b.mp4");
        let mpv = text(&file_args(Player::Mpv, path, false));
        assert_eq!(
            mpv,
            vec!["--no-video", "--force-window=no", "--", "/tmp/a b.mp4"]
        );
        let vlc = text(&file_args(Player::Vlc, path, true));
        assert_eq!(vlc, vec!["--play-and-exit", "/tmp/a b.mp4"]);
    }

    #[cfg(unix)]
    fn fake_player(dir: &Path, name: &str) -> (PathBuf, PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let log = dir.join(format!("{name}.log"));
        let copy = dir.join(format!("{name}.copy"));
        let script = dir.join(name);
        fs::write(
            &script,
            format!(
                "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\" >> '{}'; done\nif [ -f \"$2\" ]; then cp \"$2\" '{}'; fi\n",
                log.display(),
                copy.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        (script, log, copy)
    }

    #[cfg(unix)]
    #[test]
    fn mpv_receives_both_stream_urls() {
        let dir = std::env::temp_dir().join(format!("snag-fake-mpv-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let (script, log, _) = fake_player(&dir, "mpv");
        let video = Remote {
            url: "https://v.example/video".to_string(),
            ext: "mp4".to_string(),
        };
        let audio = Remote {
            url: "https://a.example/audio".to_string(),
            ext: "m4a".to_string(),
        };
        play_streams(&script, Some(&video), Some(&audio), "Clip", Player::Mpv).unwrap();
        let seen = fs::read_to_string(&log).unwrap();
        assert!(seen.contains("--audio-file=https://a.example/audio"));
        assert!(seen.trim_end().ends_with("https://v.example/video"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn vlc_plays_a_file_merged_by_snag() {
        let dir = std::env::temp_dir().join(format!("snag-fake-vlc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let (script, log, copy) = fake_player(&dir, "vlc");
        let video = crate::mux::synth::frames(true, 6, 1);
        let audio = crate::mux::synth::frames(false, 6, 2);
        let base = crate::testserver::serve(vec![
            (
                "/v.mp4".to_string(),
                crate::mux::synth::fragmented(true, &video, 15360, 512),
            ),
            (
                "/a.m4a".to_string(),
                crate::mux::synth::fragmented(false, &audio, 44100, 1024),
            ),
        ]);
        let video = Remote {
            url: format!("{base}/v.mp4"),
            ext: "mp4".to_string(),
        };
        let audio = Remote {
            url: format!("{base}/a.m4a"),
            ext: "m4a".to_string(),
        };
        play_streams(&script, Some(&video), Some(&audio), "Clip", Player::Vlc).unwrap();
        let seen = fs::read_to_string(&log).unwrap();
        assert!(seen.starts_with("--play-and-exit"));
        assert!(!seen.contains("input-slave"));
        assert_eq!(crate::mux::track_counts(&copy).unwrap(), (6, 6));
        let leftovers = std::env::temp_dir().join(format!("snag-play-{}", std::process::id()));
        assert!(!leftovers.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_players_in_listed_places() {
        let dir = std::env::temp_dir().join(format!("snag-locate-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert!(locate_in(Player::Vlc, std::slice::from_ref(&dir), &[]).is_none());
        let named = dir.join("vlc.exe");
        fs::write(&named, b"x").unwrap();
        assert_eq!(
            locate_in(Player::Vlc, std::slice::from_ref(&dir), &[]),
            Some(named)
        );
        let extra = dir.join("elsewhere");
        fs::write(&extra, b"x").unwrap();
        assert_eq!(
            locate_in(Player::Mpv, &[], &[dir.join("missing"), extra.clone()]),
            Some(extra)
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
