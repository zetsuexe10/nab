# snag

Search YouTube from the terminal, pick a stream, and snag it. One small Rust binary:
a full-screen picker, audio or video saved under clean names, and playback through
mpv or vlc. No ffmpeg, no yt-dlp, no cookies.

```
                      █▀▀ █▀▄█ █▀█ █▀▀▀
                      ▀▀▓ █  ▓ █▀█ █ ▀▓
                      ▀▀▀ ▀  ▀ ▀ ▀ ▀▀▀▀

                search youtube, pick a stream, snag it

   ╭─ Search or paste a link ─────────────────────────▄▄▄▄▄▄▄▄
   │ ❯ lofi beats to study to                           snag
   ╰──────────────────────────────────────────────────▀▀▀▀▀▀▀▀

     ↵ search  ·  ↑ history  ·  esc quit  ·  ^c quit  ·  ^t theme:auto
```

## Features

- One flow: search or paste a link, pick a result, pick a format from the Download panel,
  watch the progress bar. A pasted YouTube link skips the search.
- Mouse works everywhere: buttons, rows, hints, the logo and the scroll wheel.
- Search history on `↑`, a YouTube link in your clipboard offered on `tab`, and auto,
  light and dark themes on `ctrl+t`.
- Audio or video in any offered quality, saved as `Title.m4a` or `Title.mp4`.
- Video and audio are merged by snag's own muxer, so nothing else needs installing.
- mpv and vlc are both first-class: pick either one per play, with a key or a flag.
- Fast downloads: ranged requests, fetched as parallel segments.
- Scriptable: every action also works from flags, with the saved path on stdout.

## Install

From source:

```bash
cargo install --git https://github.com/zetsuexe10/snag --locked
```

Or clone and build:

```bash
gh repo clone zetsuexe10/snag
cd snag
cargo install --path . --locked
```

Release binaries are attached to every tag: static `x86_64` and `aarch64` Linux (musl),
both macOS architectures, and Windows in 64-bit (`x86_64-pc-windows-msvc`) and 32-bit
(`i686-pc-windows-msvc`).

```bash
gh release download --repo zetsuexe10/snag \
  --pattern 'snag-x86_64-unknown-linux-musl.tar.gz' \
  --dir /tmp/snag
tar -xzf /tmp/snag/snag-x86_64-unknown-linux-musl.tar.gz -C ~/.local/bin
```

On Linux and macOS the helper script picks the right archive for you:

```bash
curl -fsSL https://raw.githubusercontent.com/zetsuexe10/snag/master/install.sh | bash
```

## Use

```bash
snag lofi beats
snag https://www.youtube.com/watch?v=dQw4w9WgXcQ
```

### Keys

| Where | Key | Action |
| --- | --- | --- |
| search | `enter` | search, or open a pasted link |
| search | `↑` `↓` | walk through earlier searches; `↓` with none to recall jumps to the results |
| search | `tab` | paste the YouTube link from the clipboard, or jump to the results |
| search | `←` `→` `home` `end` | move the cursor |
| search | `ctrl+u` `ctrl+w` `ctrl+k` | clear, delete word, delete to end |
| search | `esc` | jump to the results (quits when there are none) |
| results | `j` `k` `↑` `↓` | move; `pgup` `pgdn` jump; `g` `G` first and last |
| results | `enter` | show the formats |
| results | `/` `esc` | edit the query |
| formats | `enter` | download the highlighted format |
| formats | `p` | play it with the current player |
| formats | `m` / `v` | play with mpv / vlc, and keep that choice |
| formats | `tab` | switch source (innertube, ytultra) |
| formats | `esc` | back |
| download | `esc` | cancel (not while merging) |
| done | `enter` | snag another |
| done | `p` `m` `v` | play the saved file |
| done | `esc` | back to the formats |
| anywhere | `ctrl+t` | cycle the theme: auto, light, dark |
| anywhere | `ctrl+c` | quit |

Clicking does what the matching key does: the `snag` button submits, a row opens, a hint
presses its key, the logo goes home and the wheel moves the selection.

Search pulls the next page as the selection nears the bottom, so a query keeps going
without a new search. The Download panel lists videos first and then audio, each with its
size. The line next to it says whether a video gets a matching audio track merged in or is
ready as is.

The theme and the search history are remembered in `~/.config/snag` (`%APPDATA%\snag` on
Windows). `SNAG_CONFIG_DIR` moves that folder and `--theme auto|light|dark` overrides the
theme for one run. `auto` uses your terminal's own colours.

### Flags

```bash
snag --list "koyaanisqatsi"                       # print results
snag --audio "koyaanisqatsi"                      # save the first result as m4a
snag --video "koyaanisqatsi"                      # save the first result as mp4
snag --video --quality 480p "koyaanisqatsi"       # pick a height
snag --play "koyaanisqatsi"                       # listen now, no file
snag --play --video --player vlc "koyaanisqatsi"  # watch now, in vlc
snag --audio --out ~/Music "koyaanisqatsi"        # choose the folder
```

The saved path is printed on stdout, progress on stderr.

| Flag | Meaning |
| --- | --- |
| `-a`, `--audio` | save audio |
| `-v`, `--video` | save video |
| `-q`, `--quality <height>` | 144p, 240p, 360p, 480p, 720p and up |
| `-p`, `--play` | play instead of saving |
| `--player <name>` | `mpv` or `vlc` |
| `--theme <mode>` | `auto`, `light` or `dark` |
| `-o`, `--out <dir>` | output folder |
| `-n`, `--limit <count>` | results to show, default 20 |
| `-l`, `--list` | print results and exit |

## Players

Pick either player explicitly, per play, from the keys above or with `--player`. Without
a choice, mpv is used when it is installed and vlc otherwise. Both are looked up on
`PATH` first, then in their usual install folders: Program Files on Windows,
`/Applications` and Homebrew on macOS, snap on Linux.

- **mpv** streams the video and audio URLs side by side and starts immediately. snag
  passes `--ytdl=no` so mpv uses the resolved URLs instead of calling yt-dlp.
- **vlc** cannot reliably play a separate audio stream over http, so snag downloads both,
  merges them into one temporary file, plays it, and deletes it afterwards. If the merge
  is not possible it falls back to vlc's `input-slave`.

## Files

Names come from the video title, with characters that break file systems replaced. If the
name is taken, the video id is appended, then a counter:

```
Khola Janala - SWAT (Original version).m4a
Khola Janala - SWAT (Original version) [JZKoY1Lfs4Q].mp4
```

The default folder is `$XDG_DOWNLOAD_DIR`, then `~/Downloads`, then `$HOME`. Temporary
parts are removed on failure or cancellation.

## Muxer

`src/mux` is a small pure-Rust muxer that joins one video track and one audio track into a
single file. Samples are copied byte for byte, nothing is re-encoded.

| Container | Video | Audio |
| --- | --- | --- |
| MP4 (fragmented or regular) | h264, hevc, av1 | aac |
| WebM | vp9, av1 | opus, vorbis |

- MP4 output is progressive with the index up front, so it starts playing and seeks
  without scanning. Offsets switch to 64-bit past 4 GB.
- WebM output carries a seek head and cues for every video keyframe.
- Edit lists, composition offsets, start offsets and rotation are preserved, so audio and
  video stay in sync.
- Truncated or corrupt input is reported as an error instead of producing a broken file.
- A video is paired with audio of the same container: `mp4` with `m4a`, `webm` with
  `weba`. If no matching audio exists, the video is saved alone and snag says so.

## How it finds streams

Search and stream lookup talk to YouTube's innertube JSON API directly, the same
endpoints the site and its apps use. The player endpoint is asked with a visionOS client,
which answers with plain googlevideo URLs, real file sizes and no signature step. ytultra
is queried alongside as a second source that fills in anything innertube does not hand
out. Fresh URLs are requested for every download, so nothing is cached.

YouTube throttles a plain sequential request to a crawl. snag uses ranged requests and
fetches large files as parallel segments: on a 5.5 MB track that was about 10 KB/s
without a range header and roughly 1 MB/s with one.

Other converter sites were tried and left out: their links are signed or encrypted in
browser-only JavaScript and are not usable from a plain HTTP client. Download only
material you have the rights to.

## Layout

```
src/
  main.rs        flags and non-interactive mode
  search.rs      search and paging
  innertube.rs   stream lookup
  ytultra.rs     second stream source
  source.rs      merged stream list and ranking
  download.rs    ranged downloads, merge step, file naming
  mux/           mp4 and webm muxer
  play.rs        mpv and vlc discovery and launching
  tui/           app state, views, logo, theme, history, clipboard, event loop
```

## Build and test

```bash
cargo build --release
cargo test
```

The suite covers the muxer on hand-built fragmented, progressive and WebM files, an
end-to-end download and merge against a local range server, player launching through
stand-in player scripts, and every screen rendered at several terminal sizes. CI runs
`cargo fmt --check`, `cargo clippy -D warnings` and `cargo test` on Linux, then clippy and
the tests again on macOS and on Windows, 64-bit and 32-bit (`i686-pc-windows-msvc`). Every
push builds Linux (musl, x86 and arm), macOS (Intel and Apple Silicon) and Windows (64-bit
and 32-bit) binaries, and a `v*` tag publishes them as a release.

## License

MIT
