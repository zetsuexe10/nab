# nab

Search YouTube or xHamster from the terminal, pick a stream, and nab it. One small Rust binary:
a full-screen picker, audio or video saved under clean names, and playback through
mpv or vlc. No ffmpeg, no yt-dlp, no cookies.

```
                         █▄ █ ▄▀▀▄ █▀▀▄
                         █ ▀█ █▀▀█ █▀▀▄
                         ▀  ▀ ▀  ▀ ▀▀▀

                search youtube, pick a stream, nab it

   ╭─ Search or paste a link ──────────────────────────▄▄▄▄▄▄▄
   │ ❯ lofi beats to study to                            nab
   ╰───────────────────────────────────────────────────▀▀▀▀▀▀▀

     ↵ search  ·  ↑ history  ·  esc quit  ·  ^c quit  ·  ^t theme:auto
```

## Features

- One flow: search or paste a link, pick a result, pick a format from the Download panel,
  watch the progress bar. A pasted YouTube link skips the search.
- Two sites: YouTube and xHamster. `ctrl+s` switches, `--site` picks one, and a pasted
  link picks the site for you. xHamster videos list two stream sources, `initials` and
  `page source`, and `tab` switches between them.
- Search suggestions drop down as you type, with creators, channels and pornstars marked.
  Picking a person opens a profile screen with their details and a paged, sortable list
  of videos. `o` on a result or in the formats opens the uploader's profile.
- Mouse works everywhere: buttons, rows, hints, the logo and the scroll wheel.
- Search history on `↑`, a YouTube link in your clipboard offered on `tab`, and auto,
  light and dark themes on `ctrl+t`.
- Videos, audio only, and photo galleries, each filed into its own folder per site
  (see [Files](#files)). Audio works on every site: where a site has no audio-only
  stream, nab takes the sound track out of the smallest video it can find.
- Audio or video in any offered quality, saved as `Title.m4a` or `Title.mp4`.
- Modular: a site is one module that implements the `Provider` trait plus a line in the
  registry (see [Adding a site](#adding-a-site)).
- Video and audio are merged by nab's own muxer, so nothing else needs installing.
- mpv and vlc are both first-class: pick either one per play, with a key or a flag.
- Fast downloads: ranged requests, fetched as parallel segments.
- Scriptable: every action also works from flags, with the saved path on stdout.

## Install

From source:

```bash
cargo install --git https://github.com/zetsuexe10/nab --locked
```

Or clone and build:

```bash
gh repo clone zetsuexe10/nab
cd nab
cargo install --path . --locked
```

Release binaries are attached to every tag: static `x86_64` and `aarch64` Linux (musl),
and Windows in 64-bit (`x86_64-pc-windows-msvc`) and 32-bit (`i686-pc-windows-msvc`).

```bash
gh release download --repo zetsuexe10/nab \
  --pattern 'nab-x86_64-unknown-linux-musl.tar.gz' \
  --dir /tmp/nab
tar -xzf /tmp/nab/nab-x86_64-unknown-linux-musl.tar.gz -C ~/.local/bin
```

On Linux the helper script picks the right archive for you:

```bash
curl -fsSL https://raw.githubusercontent.com/zetsuexe10/nab/master/install.sh | bash
```

## Use

```bash
nab lofi beats
nab https://www.youtube.com/watch?v=dQw4w9WgXcQ
```

### Keys

| Where | Key | Action |
| --- | --- | --- |
| search | `enter` | search, or open a pasted link |
| search | `↓` `↑` | walk through the suggestions, `enter` takes one, `esc` closes them |
| search | `↑` `↓` | with no suggestions open, walk through earlier searches or jump to the results |
| search | `tab` | fill the highlighted suggestion, paste the clipboard link, or jump to the results |
| search | `ctrl+s` | switch between YouTube and xHamster |
| search | `ctrl+g` | switch what you search: videos, galleries, stars (where the site has them) |
| search | `←` `→` `home` `end` | move the cursor |
| search | `ctrl+u` `ctrl+w` `ctrl+k` | clear, delete word, delete to end |
| search | `esc` | jump to the results (quits when there are none) |
| results | `j` `k` `↑` `↓` | move; `pgup` `pgdn` jump; `g` `G` first and last |
| results | `enter` | show the formats |
| results | `o` | open the uploader's profile, when the site has one |
| results | `c` / `r` | comments / related videos of the highlighted video (xHamster) |
| results | `enter` on a star | open the star's profile |
| results | `/` `esc` | edit the query |
| profile | `j` `k` `↑` `↓` | move; `pgup` `pgdn` jump; `g` `G` first and last |
| profile | `enter` | show the formats of the highlighted video |
| profile | `←` `→` or `p` `n` | previous and next page |
| profile | `t` | switch between the person's videos and galleries |
| profile | `s` | cycle the sort: featured, newest, best, most viewed, most commented |
| profile | `c` | comments of the highlighted video |
| profile | `esc` | back to where you came from |
| comments | `j` `k` `pgup` `pgdn` | move; more pages load as you near the end |
| comments | `esc` | back |
| formats | `enter` | download the highlighted format |
| formats | `p` | play it with the current player (galleries are saved, not played) |
| formats | `m` / `v` | play with mpv / vlc, and keep that choice |
| formats | `tab` | switch source (innertube, ytultra, initials, page source) |
| formats | `o` | open the uploader's profile |
| formats | `c` / `r` | comments / related videos |
| formats | `esc` | back |
| download | `esc` | cancel (not while merging or extracting audio) |
| done | `enter` | nab another |
| done | `p` `m` `v` | play the saved file |
| done | `esc` | back to the formats |
| anywhere | `ctrl+t` | cycle the theme: auto, light, dark |
| anywhere | `ctrl+c` | quit |

Clicking does what the matching key does: the `nab` button submits, a row opens, a hint
presses its key, the logo goes home and the wheel moves the selection.

Search pulls the next page as the selection nears the bottom, so a query keeps going
without a new search. The Download panel lists videos first and then audio, each with its
size. The line next to it says whether a video gets a matching audio track merged in or is
ready as is.

The theme, the site and the search history are remembered in `~/.config/nab` (`%APPDATA%\nab` on
Windows). `NAB_CONFIG_DIR` moves that folder and `--theme auto|light|dark` overrides the
theme for one run. `auto` uses your terminal's own colours.

### Flags

```bash
nab --list "koyaanisqatsi"                       # print results
nab --audio "koyaanisqatsi"                      # save the first result as m4a
nab --video "koyaanisqatsi"                      # save the first result as mp4
nab --video --quality 480p "koyaanisqatsi"       # pick a height
nab --play "koyaanisqatsi"                       # listen now, no file
nab --play --video --player vlc "koyaanisqatsi"  # watch now, in vlc
nab --site xhamster --list "some query"          # search the other site
nab --site xhamster --galleries --video "query"  # save a gallery's photos
nab https://xhamster.com/creators/some-creator --list  # a profile's videos
nab --audio --out ~/Music "koyaanisqatsi"        # choose the folder
```

The saved path is printed on stdout, progress on stderr.

| Flag | Meaning |
| --- | --- |
| `-a`, `--audio` | save audio |
| `-v`, `--video` | save video |
| `-g`, `--galleries` | search galleries instead of videos |
| `-q`, `--quality <height>` | 144p, 240p, 360p, 480p, 720p and up |
| `-p`, `--play` | play instead of saving |
| `--player <name>` | `mpv` or `vlc` |
| `--site <name>` | `youtube` or `xhamster` |
| `--theme <mode>` | `auto`, `light` or `dark` |
| `-o`, `--out <dir>` | output root, default `Downloads/Nab` |
| `-n`, `--limit <count>` | results to show, default 20 |
| `-l`, `--list` | print results and exit |

## Players

Pick either player explicitly, per play, from the keys above or with `--player`. Without
a choice, mpv is used when it is installed and vlc otherwise. Both are looked up on
`PATH` first, then in their usual install folders: Program Files on Windows,
and snap on Linux.

- **mpv** streams the video and audio URLs side by side and starts immediately. nab
  passes `--ytdl=no` so mpv uses the resolved URLs instead of calling yt-dlp.
- **vlc** cannot reliably play a separate audio stream over http, so nab downloads both,
  merges them into one temporary file, plays it, and deletes it afterwards. If the merge
  is not possible it falls back to vlc's `input-slave`.

## Files

Everything lands under one root, sorted by site and then by what it is:

```
~/Downloads/Nab/Youtube/Videos
~/Downloads/Nab/Youtube/Audios
~/Downloads/Nab/Xhamster/Videos
~/Downloads/Nab/Xhamster/Audios
~/Downloads/Nab/Xhamster/Galleries
```

The root follows the system. On Linux it is `$XDG_DOWNLOAD_DIR`, then the `DOWNLOAD` entry
of `~/.config/user-dirs.dirs`, then `~/Downloads`. On Windows it is
`%USERPROFILE%\Downloads\Nab`, for example `C:\Users\you\Downloads\Nab\Youtube\Videos`.
`--out <dir>` replaces `Downloads/Nab` and keeps the site and kind folders beneath it.

Names come from the title, with characters that break file systems replaced. If the name
is taken, the id is appended, then a counter:

```
Khola Janala - SWAT (Original version).m4a
Khola Janala - SWAT (Original version) [JZKoY1Lfs4Q].mp4
```

A gallery is a folder named after it, holding `001.jpg`, `002.jpg` and so on, fetched four
photos at a time. A photo that cannot be fetched is skipped with a note, and the gallery
fails only if none could be saved. Temporary parts are removed on failure or cancellation.

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
  `weba`. If no matching audio exists, the video is saved alone and nab says so.

## How it finds streams

Search and stream lookup talk to YouTube's innertube JSON API directly, the same
endpoints the site and its apps use. The player endpoint is asked with a visionOS client,
which answers with plain googlevideo URLs, real file sizes and no signature step. ytultra
is queried alongside as a second source that fills in anything innertube does not hand
out. Fresh URLs are requested for every download, so nothing is cached.

YouTube throttles a plain sequential request to a crawl. nab uses ranged requests and
fetches large files as parallel segments: on a 5.5 MB track that was about 10 KB/s
without a range header and roughly 1 MB/s with one.

Other converter sites were tried and left out: their links are signed or encrypted in
browser-only JavaScript and are not usable from a plain HTTP client. Download only
material you have the rights to.

## xHamster

Pages embed their data as `window.initials`. The `initials` source reads the stream
list out of it; media links there are either plain or XOR-ciphered with one of seven
keystream generators, and nab deciphers them and drops decoys that do not decode to a
playable URL. The `page source` source scrapes the raw HTML for mp4 and m3u8 links
instead, so one still works when the other breaks.

- Progressive mp4 goes up to 720p; taller sizes come as HLS. Each height is listed once
  per packaging, preferring h264, then h265, av1 and vp9.
- HLS is downloaded by nab as parallel segments and joined without ffmpeg. Transport
  stream playlists are saved as `.ts`, fragmented mp4 playlists as a fragmented `.mp4`.
  Encrypted (AES) playlists are not supported and say so.
- Requests carry an age-verified cookie and an xHamster referer; mpv and vlc get the same
  referer. Links locked to one IP are ranked below open ones.
- Geo-blocked and removed videos are reported as such.
- Uploader (`/users/…`) profiles read their videos from the `/videos` tab and page by number; the hero details come from the landing page.
- Related videos use `/api/front/video/related` and page on as you scroll. Comments come from the site's `/x-api` call and load page by page.
- Stars: `ctrl+g` to the stars scope lists the pornstar directory (type `creators`, `channels` or `celebrities` for the others) or searches stars by name; `enter` opens a profile.
- xHamster has no audio-only stream, so `Audios` is filled by cutting the sound track
  out of the smallest progressive mp4 (the smallest HLS stream when there is none), with
  no re-encoding, as `.m4a`. The clip is fetched whole first; only mp4 sources can be cut.
- Galleries are searched, listed on profiles (`t`) and read from the gallery page's
  `initials`, then saved photo by photo.

## Layout

```
src/
  main.rs          flags and non-interactive mode
  model.rs         Video, Profile, Photo, Scope and the other shared types
  layout.rs        output folders per OS, site and kind
  source.rs        merged stream list and ranking
  download.rs      ranged downloads, audio extraction, merge step, file naming
  photos.rs        parallel gallery downloads
  hls.rs hlsdl.rs  playlist parsing, segment download and join
  mux/             mp4 and webm muxer, audio extraction
  play.rs          mpv and vlc discovery and launching
  sites/
    mod.rs         Provider trait, registry, link and query classification
    youtube/       search, innertube, ytultra
    xhamster/      cipher, initials, page scraper, sources, profiles, galleries, suggestions
  tui/             app state, views, logo, theme, history, clipboard, event loop
```

### Adding a site

1. Create `src/sites/<name>/mod.rs` with a type that implements `Provider`: an `id`, a
   `title`, the `folder` name it files under, its `scopes` (videos, galleries), and
   `classify`, `search`, `more`, `resolve`, `profile` and `suggest`. Anything a site does
   not have can return an empty result or an error saying so.
2. Add the provider to `REGISTRY` in `src/sites/mod.rs`.

Search, `ctrl+s`, `--site`, link detection, folders and downloads pick it up from there.

## Build and test

```bash
cargo build --release
cargo test
```

The suite covers the muxer on hand-built fragmented, progressive and WebM files, an
end-to-end download and merge against a local range server, player launching through
stand-in player scripts, and every screen rendered at several terminal sizes. CI runs
`cargo fmt --check`, `cargo clippy -D warnings` and `cargo test` on Linux, then clippy and
the tests again on Windows, 64-bit and 32-bit (`i686-pc-windows-msvc`). Every
push builds Linux (musl, x86 and arm) and Windows (64-bit and 32-bit) binaries, and a `v*` tag publishes them as a release.

## License

MIT
