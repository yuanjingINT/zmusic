# zmuse

A terminal player for NetEase Cloud Music. Written in Rust with
[ratatui](https://github.com/ratatui/ratatui): search, play and download from a
keyboard-only TUI.

- Playback goes through `mpv`, downloads go through the NetEase API
- Defaults, favorites and settings all live in `~/.config/zmuse/`
- One binary, no Electron, no browser

```
  ⏮  ⏯  ⏭     正在播放：晴天 - 周杰伦
  ────────────────────────────────────────────
  ▍搜索音乐      ← h 回主菜单 / s 搜索 / g 下载
  ▍歌曲推荐
  ▍专辑推荐
```

The interface itself is still Chinese; the block above is what the screen looks
like.

> **Renamed from `zmusic`.** The project used to be called zmusic; the package,
> binary and config directory are now `zmuse`. Your old setup keeps working: on
> first start `~/.config/zmusic` is copied to `~/.config/zmuse` (the old
> directory is left untouched), the cache is migrated the same way, and the old
> `ZMUSIC_*` environment variables are still read — `ZMUSE_*` wins when both are
> set. On Arch, `replaces=(zmusic)` makes `pacman -Syu` swap the old package.

## Requirements

At runtime:

| Dependency | Purpose | Required |
|------------|---------|----------|
| `mpv` | audio playback | yes |
| `node` >= 22 | local NetEase API service (`setup.sh` starts `ncm-api`) | yes |

To build from source:

| Dependency | Purpose |
|------------|---------|
| `cargo` / `rustc` (stable, edition 2021) | compile |
| C toolchain (`gcc` / `cc`) | link |
| `openssl` headers + library (Debian: `libssl-dev`) | `reqwest` uses `native-tls`, which links system OpenSSL through `openssl-sys` |
| `pkg-config` | locate OpenSSL |

> Why `native-tls` and not `rustls`: ring 0.17.14 fails to link against gcc 16 /
> LLVM 21, so we use the system OpenSSL instead.

Linux x86_64 / aarch64.

## Installation

### Arch Linux (self-hosted repo, public)

Packages are built on the developer's machine with `makepkg`, signed on the NAS,
and published to the self-hosted repo `repo.dito.asia`; both the packages and
the database carry the repo key.

> The repo currently holds the first stable release, **`0.1.1`** (`0.1.1-2`).
> `0.1.1-1` was withdrawn — it crashed on first run when `~/.config/zmuse` did
> not exist yet — so `0.1.1-2` is the one to install.

One-shot install (adds the repo, imports and locally trusts the key):

```bash
curl -fsSL https://repo.dito.asia/install-dito-repo.sh | sudo bash
sudo pacman -S zmuse
```

Manual setup:

```ini
[dito-repo]
Server = https://repo.dito.asia/x86_64
SigLevel = Required
```

```bash
sudo pacman-key --add <(curl -fsSL https://repo.dito.asia/x86_64/dito-repo.pub)
sudo pacman-key --lsign-key 6469F0BEB079C3EDCC47947627FEDD0D74F7D714
sudo pacman -Sy
sudo pacman -S zmuse
```

> Repo key fingerprint `6469 F0BE B079 C3ED CC47 9476 27FE DD0D 74F7 D714`
> (`Dito Arch Repo <repo@dito.local>`).
> The key was re-signed on 2026-11-07: machines that already had the repo need to
> run the install script once more, otherwise signature checks on new packages fail.
> On the LAN you can fall back to the NAS directly: `Server = http://192.168.1.20:8180/x86_64`.

### Prebuilt packages

Every `v*` tag triggers CI to build `.deb` / `.rpm` / Arch packages for x86_64
and aarch64 and attach them to
[GitLab Releases](https://git.tuxai.org/personal/zmuse/-/releases) (mirrored to
[GitHub Releases](https://github.com/yuanjingINT/zmuse/releases)). Arch users
should prefer `dito-repo` above.

### From source

Needs a Rust toolchain (`rustc` >= the `rust-version` in `Cargo.toml`, 1.88
right now), a C compiler, `pkg-config`, OpenSSL headers and `make`. Compiling
takes 1–3 minutes with a warm cargo cache (4–5 minutes on a cold one); the static
tarball stays the preferred route.

The install script can build it for you — same target layout, same sidecar, same
user unit, same `--uninstall` as the prebuilt route:

```bash
curl -fsSL https://repo.dito.asia/zmuse/install.sh | sh -s -- --from-source
```

It fetches the latest release tag (`--version 0.1.1` pins one; `master` is never
built implicitly), runs `cargo build --release --locked` in a temporary
directory, installs the binary, the Meting sidecar, the unit and the docs, then
deletes the source tree. `--source github` takes the tag archive from
`codeload.github.com` and never runs `git` — useful where GitHub's git over
HTTPS stalls — and the script can be fetched from raw.githubusercontent.com, so
the install does not depend on this repo's mirror at all:

```bash
curl -fsSL https://raw.githubusercontent.com/yuanjingINT/zmusic/master/install-static.sh \
     | sh -s -- --from-source --source github
```

`--source gitlab` (the default) shallow-clones the tag; if that fails it tries
the GitLab archive and then GitHub. If the build fails, the log stays at
`/tmp/zmuse-build.log` (override with `ZMUSE_BUILD_LOG`). Missing build tools are
reported with a per-distribution install command — the script installs nothing
itself and never runs `sudo`.

Or do it by hand — the main repo is on GitLab, GitHub is a mirror:

```bash
git clone https://git.tuxai.org/personal/zmusic.git
cd zmusic
cargo build --release --locked
./target/release/zmuse
```

Missing OpenSSL headers show up as `openssl-sys` / `Could not find openssl`:

```bash
# Arch
sudo pacman -S --needed base-devel openssl pkg-config
# Debian / Ubuntu
sudo apt install build-essential pkg-config libssl-dev
```

### Install script

```bash
./install.sh    # builds release, installs the launcher into ~/.local/bin and a desktop entry
```

## First run

On the very first start zmuse opens a short setup wizard: interface language
(Chinese / English), theme, waveform on/off, download folder and audio quality.
Press `Enter` to move on or `Esc` to skip with the defaults; either way the
answers are written to `config.toml` together with `first_run_done = true`, so
upgrading users never see it again. Bring it back with `zmuse config --wizard`.

The language choice is real, not just a stored string: menus, titles, the
settings screen, hints, toasts and error messages switch with it, and the wizard
redraws immediately when you change it.

The player ships without an API; you need a local NetEase API service:

```bash
./setup.sh          # first install and start (systemd --user, unit name ncm-api)
./setup.sh update   # update the API to the latest version and restart
```

> If zmuse was installed from the pacman package, `setup.sh` notices that the
> sidecar comes with the package
> (`pacman -Qo /usr/lib/zmuse/sidecar/meting.js` succeeds) and **skips the
> source sidecar install**: it will not copy an old tree into
> `~/.local/share/zmuse/meting` and will not write a user-level unit (a
> user-level unit shadows the packaged `meting-sidecar.service`, which means
> running stale code without `type=duration`). If it finds a leftover user-level
> unit it only tells you how to back it up and restart — it will not delete
> anything for you.

The launcher `install.sh` generates (`~/.local/bin/zmuse` → `run.sh` in the
repo) **never touches the network**: it only does three things — incremental
`cargo build --release` when the binary is older than the sources,
`systemctl --user start ncm-api` when the API is not running, then
`exec target/release/zmuse`. That is why the menu opens instantly.

Update the API by hand when you want to (it is no longer on the startup path):

```bash
./setup.sh update
```

## Configuration

All service and account settings live in one file:
`~/.config/zmuse/config.toml`. Most of them can be changed without touching
TOML:

```bash
zmuse config            # interactive settings screen (j/k, h/l, Enter; s saves, q saves & quits)
zmuse config --edit     # open the raw file in $EDITOR
zmuse config --print    # print the path only
zmuse config --wizard   # re-run the first-run wizard
```

The settings screen covers theme, waveform on/off, spacing and padding,
animations, spinner, download folder, super-download folder, quality and
language — most take effect immediately, the theme after a restart.

The full walkthrough (ncm-api, the Meting sidecar and cookies, the YouTube
proxy, the Jamendo key, what every source needs) is in
[docs/CONFIG.md](docs/CONFIG.md).

Environment variables still work, and win over the file:
**env > config.toml > defaults**.

| Variable | Default | Meaning |
|----------|---------|---------|
| `NCM_API` | `http://127.0.0.1:3000` | NetEase API address |
| `NCM_DIR` | `~/Music/ncm` | download directory |
| `NCM_QUALITY` | `exhigh` | quality: `standard` / `higher` / `exhigh` / `lossless` / `hires` |
| `ZMUSE_PROXY` | empty | optional proxy, e.g. `socks5h://127.0.0.1:11080`; required for YouTube / SoundCloud, see below |
| `ZMUSE_METING_URL` | `http://127.0.0.1:8123` | Meting sidecar address (Tencent / Kugou / Baidu / Kuwo) |
| `ZMUSE_METING_COOKIE` | empty | optional platform cookie used by Meting to fetch streams (prefer `[meting] cookie` in config.toml); never written to disk or logs |
| `ZMUSE_YTDLP_BIN` | `yt-dlp` | yt-dlp executable name or absolute path |
| `ZMUSE_SUPERDL_DIR` | empty | when set, **super download** writes straight into this directory and skips disk picking / unmounting / formatting (handy for testing without a USB stick) |
| `ZMUSE_WAVE_DUMP` | empty | debug: write every analysis window as CSV (columns in docs/CONFIG.md section 5); leave it off normally |

At runtime it writes:

```
~/.config/zmuse/favorites.json   favorites
~/.config/zmuse/settings.json    UI and playback settings
```

### Sources at a glance

**Only NetEase is enabled out of the box.** Everything else ships in the binary
but stays switched off: it is not registered, not probed and never shows up in
the source list until you turn it on:

```bash
zmuse source list            # what is enabled now
zmuse source list --all      # every built-in source with its enabled flag
zmuse source add meting      # enable a group (meting = QQ/Kugou/Baidu/Kuwo)
zmuse source add youtube soundcloud audius
zmuse source remove migu     # switch one off again
```

The same list is editable interactively: `zmuse config`, first row "Sources ·
enabled", `Enter` opens a checklist where `Space` toggles a source and `s` saves.

`add`/`remove` write `[sources] enabled` in `~/.config/zmuse/config.toml`,
rebuild the registry on the spot, and a running player picks it up on the next
`R` (re-probe) — no restart. `meting` expands to the four Meting sources and
`all` enables everything. Unknown ids fail with the list of valid ones.

All enabled sources behave as before: in the search screen, `S` cycles through
them by group (domestic → international → public/free); the choice is stored in
`settings.json`; only enabled sources take part in probing, the `源 x/y` counter,
the source status panel and the aggregate search.
What each source can do (which search kinds, lyrics, playable or not) comes from
`Source::capabilities()`, and the UI disables what is unsupported instead of
guessing.

| Source | `Song.source` | Group | Status |
|--------|---------------|-------|--------|
| NetEase | `netease` | domestic | default source; search / playback / lyrics all work |
| QQ Music (direct) | `qq` | domestic | free tracks stream; member tracks report "not playable" |
| QQ / Kugou / Qianqian / Kuwo (Meting) | `meting:tencent` `meting:kugou` `meting:baidu` `meting:kuwo` | domestic | goes through the sidecar; tencent/kugou search, streaming needs a cookie; baidu/kuwo return empty shells upstream |
| Migu Music | `migu` | domestic | public app endpoint; search carries no duration, member tracks report "not playable" |
| YouTube | `youtube` | international | `yt-dlp`, needs a proxy in China |
| SoundCloud | `soundcloud` | international | `yt-dlp`, needs a proxy in China; slow search (6–7s) |
| Audius | `audius` | public/free | search + streaming work |
| ccMixter | `ccmixter` | public/free | search works; direct audio links are 403 from the site, so it degrades to "not playable" at runtime |
| Wikimedia Commons | `wikimedia` | public/free | resolves IPv6 only, degrades on slow networks |
| Jamendo | `jamendo` | public/free | needs `ZMUSE_JAMENDO_CLIENT_ID`, greyed out otherwise |
| OpenGameArt | `opengameart` | public/free | scrapes the page, single tracks only |
| Internet Archive | `archive` | public/free | one row per item (its first playable MP3 with a known length); needs a proxy in China |

Not integrated (each with the reason we measured):

| Source | Reason |
|--------|--------|
| bilibili | Was integrated once (its web search API plus `yt-dlp` for streaming) and removed on request: results are mostly video clips with noisy titles and no reliable audio duration; code and registry entries are gone |
| Bandcamp | `yt-dlp` has no Bandcamp search prefix, so scraping is the only option; `bandcamp.com` is connection-reset from this machine, and through an overseas exit it returns a 3038-byte `Client Challenge` anti-bot page (no `tralbum` data), where `yt-dlp -g` reports `Unable to extract tralbum data` |
| StreetVoice | The search API works (`/api/v4/search/?q=` returns JSON), but playback `POST /api/v4/song/<id>/play/` always returns `{}` (needs a login session), and the web player itself returns 451 for region blocking — searchable, not playable, so not wired up |
| JOOX | Both `api.joox.com` and `api-jooxtt.sanook.com` answer `code=-2` with an empty `itemlist` (region / risk control), no usable results |
| MOOV | `api.moov.hk` times out directly, no public search endpoint |
| Qishui Music (Douyin) | `api.qishui.com/luna/pc/search/track` returns 200 with zero bytes and needs `a_bogus` / `X-Bogus` signatures; the web client is an SPA shell, and we do not reverse signatures |
| Free Music Archive | the old API is 404, the new one needs a key |
| Spotify / Tidal / Qobuz / Apple Music | official APIs only give metadata and 30s previews, `stream_url` can only return `None`; revisit when there are account tokens |

### Probing and aggregation

On startup zmuse probes every source concurrently in the background (4 at a
time, one minimal request each). It **stays off the render / key path**, so the
UI never waits for probing. The top bar shows `探测中 3/15` while probing and
`源 9/15` afterwards.

- `R` probes again manually (also in the background; uppercase so it never
  collides with typing a lowercase search term)
- Search screen, list area: before any search the panel shows **one line per
  source** (the UI strings are Chinese: `● 网易云  ok` / `○ YouTube  代理未配置` /
  `⚠ 酷狗(Meting)  仅搜索（无 cookie）`), ending with `14/16 可用 · R 重探`;
  once there are results the status folds into the list header
  (`搜索结果   源 14/16`)
- `V` toggles between "search results ↔ source detail"; `a` switches what the
  `S` cycle covers (available only / including offline)
- Probe results are cached for 10 minutes; a missing key (Jamendo) or a stopped
  sidecar is reported as "needs credentials", a dead network as "offline" — the
  two are never conflated

**No duration, no listing**: tracks with `dt <= 0` are dropped from search and
aggregated results (no progress bar, and you only find out it cannot play after
clicking). Meting search carries no duration, so those sources first fetch the
first 12 tracks concurrently and then filter; tracks still at 0 after the lazy
fetch are hidden as well.

**Searchable ≠ playable**: `Capabilities::can_stream` says whether a source can
produce a stream. Aggregation only merges sources with `can_stream=true`; Meting
without a cookie, and sources whose upstream returns empty shells, are marked
"search only" (red) in the list and never enter the playback queue. With a
single source selected those results are still listed, and pressing play shows
"this one comes from a search-only source".

When no playable URL can be obtained (member tracks, expired direct links, dead
sources) it does not sit there with a changed title and no sound: the track is
recorded in the background, playback skips to the next playable track in the
current order, and the top bar shows
`跳过无法播放: X（源）→ 接着放 Y`; after 5 consecutive skips it stops and says
so. In shuffle mode it will not keep hitting the same track.

The very first entry in the search screen is the **aggregate** pseudo-source: it
fans the keywords out to every available source (4 at a time, 5s each). Timed-out
sources are listed on the status line as `失败源: …` and do not affect the
others. Aggregation only searches songs; playback still dispatches on each
result's `Song.source`.

How the aggregated search runs: up to 24 concurrent sources (this machine has 24
cores and the work is pure I/O), sources that fork child processes (`yt-dlp` /
`bilibili`) are limited to 2; each source gets a 6s HTTP timeout and a 2s connect
timeout; `(source, keywords)` results are cached for 60 seconds (`R` clears it),
so fixing a typo and going back does not hit the APIs again. A NetEase search is
warmed up once at startup so the first search does not pay its upstream cold
start.

Ordering and the representative source follow a fixed priority
(`source_priority`):

```
netease > qq > meting:tencent > meting:kugou > meting:baidu > meting:kuwo
        > migu > youtube > soundcloud
        > audius > ccmixter > jamendo > wikimedia > opengameart > archive
```

- registration order = `S` cycle order = this priority; NetEase is always first
- aggregated results keep this order stably (within a source, the API order is
  preserved), so NetEase tracks come first
- the same track (title + primary artist) appears once, **represented by the
  highest-priority hit** (a later NetEase result displaces an earlier Migu one);
  the source column lists every matched source in priority order, e.g.
  `网易云+QQ(直连)`

### yt-dlp sources (YouTube / SoundCloud) and the proxy

YouTube and SoundCloud share one adapter (`src/source/ytdlp.rs`, parameterised by
`Site`) and both go through a `yt-dlp` child process. **Neither is reachable
directly from China; they need a proxy**:

```bash
# example: borrow an overseas box for a SOCKS5 tunnel
ssh -f -N -D 127.0.0.1:11080 your-vps
ZMUSE_PROXY=socks5h://127.0.0.1:11080 zmuse
```

The hard constraint: **zmuse and mpv must leave through the same IP**.
googlevideo binds the signature to the exit IP, so switching IPs yields 403. In
practice:

- `yt-dlp` is passed `--proxy` directly
- mpv's `--http-proxy` only understands HTTP proxies, not SOCKS; so when the
  proxy is `socks5(h)://`, zmuse starts a small local bridge (HTTP CONNECT →
  SOCKS5) listening on `127.0.0.1` only, used for tracks from `youtube` /
  `soundcloud`; for other sources it is cleared again and playback goes direct
- the `archive` source (Internet Archive) shares the same `[ytdlp] proxy` for
  both its searches and its playback: `archive.org` is DNS-poisoned in China, so
  without a proxy that source simply reports itself unavailable (`zmuse source
  add archive` still writes the entry; it lights up once a proxy is configured)
- with an `http://…` proxy it is handed to mpv as-is and no bridge is started

If `yt-dlp` is missing from PATH, the proxy is down, or it times out, that source
errors and is greyed out while the others keep working. Search results for
yt-dlp sources are cached per `(source, keywords)` for 5 minutes; stream URLs
are **not** cached (they expire).

Smoke test (runs the real `yt-dlp`, ignored by default):

```bash
ZMUSE_LIVE_PROXY=socks5h://127.0.0.1:11080 cargo test --release ytdlp_live -- --ignored --nocapture
```

### Meting sidecar (Tencent / Kugou / Baidu / Kuwo)

These domestic platforms are handled by `sidecar/meting.js` (Node + `@meting/core`),
listening on `127.0.0.1:8123` only:

```bash
cd sidecar && npm i && node meting.js        # or run ./setup.sh from the repo root, which installs it as systemd --user
```

On the Rust side they register as four sources: `QQ 音乐 (Meting)` /
`酷狗 (Meting)` / `千千/百度 (Meting)` / `酷我 (Meting)`. `ZMUSE_METING_URL`
changes the address (default `http://127.0.0.1:8123`). If the sidecar is not
running or cannot be installed, this whole group is greyed out and NetEase and
the other sources are unaffected. QQ also has a direct source
(`QQ 音乐 (直连)`) as a fallback.

After installing from the package the sidecar is `disabled`; enable it:

```bash
systemctl --user enable --now meting-sidecar.service
```

#### Optional: cookie (for member / restricted tracks)

Searching tencent/kugou needs no login, but many tracks need a platform cookie to
stream. zmuse only accepts a cookie from **environment variables**
(`ZMUSE_METING_COOKIE`, or the sidecar's own `METING_COOKIE`); it never writes
it to disk or into logs:

```bash
# temporary (current shell)
export ZMUSE_METING_COOKIE='uin=o0123456; qm_keyst=...'
# persistent: a 0600 env file read by systemd (the unit already sets EnvironmentFile)
mkdir -p ~/.config/zmuse && install -m 600 /dev/null ~/.config/zmuse/meting.env
printf 'METING_COOKIE=%s\n' 'uin=...; qm_keyst=...' > ~/.config/zmuse/meting.env
systemctl --user restart meting-sidecar
```

Copy the cookie from your browser's devtools (`Cookie` request header on any
request while logged in to the target site). **Using your own account cookie is
at your own risk**: it is equivalent to a logged-in session, so do not share it
and never commit it.

#### Equalizer (real audio)

The waveform on the now-playing screen comes from the **actual audio**: zmuse
starts a hidden mpv process that decodes the same track with `--ao=pcm` into an
8 kHz mono PCM FIFO, consumes it at real-time pace, and draws a symmetric braille
waveform (2×4 dots per character cell, solid fill). Columns whose amplitude is
below one dot stay blank, so quiet passages are empty rather than a flat line.

- per-song automatic gain: one value for the whole track, no pumping; the first
  play uses the fixed gain while the decoder collects the track's loudness, and
  from the second play on the cached calibration is applied from the first second
  (see docs/CONFIG.md section 5 for the numbers and how they are measured)
- attack 0.65 / release 0.12: rises fast, falls slow
- three colour tiers (low / mid / high) so intensity is readable at a glance

**Cost**: the same track is **streamed a second time** (extra bandwidth and
decoding), so this only runs while the now-playing screen is visible and
playback is actually running; switching screens, pausing or stopping kills the
decoder process immediately (no orphaned mpv).

Settings:

| Setting | Default | Meaning |
|---------|---------|---------|
| `[ui] equalizer` | `true` | master switch, off means no redraws at all |
| `[ui] equalizer_fps` | `30` | 5–60, redraw rate on the playing screen |
| `[ui] equalizer_source` | `pcm` | `pcm` real audio; `synth` a slow breathing placeholder that streams nothing (saves bandwidth) |
| `[ui] eq_reference_dbfs` | `-6` | dBFS that maps to full height; lower means it saturates sooner |
| `[ui] eq_autogain` | `true` | per-song automatic gain: one value for the whole song, applied from the second play, no extra traffic |
| `[ui] eq_gain` | `1.6` | gain multiplier for the fixed mapping used on the first play (the calibration takes over once it is known) |
| `[ui] eq_soft_clip` | `true` | soft clipping (tanh): loud passages squeeze towards the top instead of hard clipping |
| `[ui] wave_spacing` | `2` | gap between waveform rows: 1 dense / 2 default / 3 airy |
| `[ui] wave_vpad` | `1` | blank rows above and below the waveform |

`ZMUSE_EQ_REF_DBFS` / `ZMUSE_EQ_GAIN` / `ZMUSE_EQ_SOFT_CLIP` /
`ZMUSE_EQ_AUTOGAIN` / `ZMUSE_EQ_FPS` override these temporarily (see
docs/CONFIG.md section 5 for the real values and how to measure them).

#### Super download

Super download (menu → "Devices" → pick a removable disk) formats the target and
then writes the charts onto it. Failure handling: when `/song/url/v1` returns
`code!=200` or no playable stream (member tracks, unlicensed tracks, dead
third-party fallbacks), it **skips the track and continues with the next one**,
writes nothing and leaves no `.part` files; the log marks it
`跳过(无播放链接)`. Setting `ZMUSE_SUPERDL_DIR=<dir>` skips formatting and
writes into that directory instead, which is how this path is tested without a
USB stick.

#### Duration

The Meting search API returns no duration, so the list shows `--:--` first and
asks again (`type=duration`) when a track is selected or played, refreshing once
it arrives. Tencent answers (measured: `晴天` → 269s); Kugou's `playInfo` always
returns `timeLength=0` for unauthenticated hashes, so those stay at `--:--`.

## Key bindings

| Key | Action |
|-----|--------|
| `0`–`6` | jump to that main-menu entry (numbering starts at 0) |
| `h` | back to the main menu |
| `s` | search |
| `Tab` | search screen: toggle focus between the input and the result list |
| `S` | search screen: cycle through **available** sources (the first is "aggregate"; `V` shows all) |
| `R` | re-probe every source (background, never blocks the UI) |
| `V` | search screen: toggle the source list between available-only and all |
| `F1`–`F4` / `Alt+0`–`3` | search screen: switch search kind (songs / artists / playlists / albums), works from either focus |
| `g` | download list |
| `v` | now playing |
| `Enter` | play the selected item |
| `Space` | play / pause |
| `n` / `p` | next / previous |
| `j` / `k`, `↑` / `↓` | move up / down |
| `d` | download the selected item |
| `D` | batch download |
| `l` | favorite / unfavorite |
| `x` | remove from favorites |
| `A` | open the album of the selected song |
| `t` | toggle the view on the favorites screen |
| `-` / `=` | volume −5 / +5 |
| `Esc` | close an overlay; otherwise go back one screen (never quits, and does nothing on the main menu) |
| `q` | quit |

## Project layout

```
src/
  api.rs        NetEase API client
  config.rs     environment variables and paths
  downloader.rs downloads (including batches)
  favorites.rs  favorites I/O
  models.rs     data structures
  player.rs     mpv control (Unix socket)
  superdl.rs    batch download queue
  settings.rs   persisted settings
  ui/           ratatui UI (screens / theme)
  util.rs       misc helpers
PKGBUILD        Arch packaging
packaging/      nfpm packaging config (deb / rpm / Arch)
.github/workflows/build.yml   CI: multi-arch build + packaging + Release
```

## Building and packaging

Packages in the Arch repo are built locally and signed on the NAS; CI is not
involved:

```bash
makepkg -s           # needs base-devel + openssl (declared in PKGBUILD)
# output: zmuse-<ver>-<rel>-x86_64.pkg.tar.zst; the release script signs and adds it
```

Two hard rules for releases:

- **`pkgrel` must increase.** With an identical version pacman will not upgrade,
  it only offers a reinstall, so people who already have the old package never
  get the new one.
- The `PKGBUILD` `_commit` must point at a commit that is **already pushed** to
  GitLab, and `sha256sums` must be recomputed to match; archiving an unpushed
  commit returns 404.

CI (`.github/workflows/build.yml`) only produces portable packages for non-Arch
users, on push / PR / tag:

1. build once per architecture: `x86_64` (ubuntu-latest) and `aarch64` (ubuntu-24.04-arm)
2. install `libssl-dev` + `pkg-config` (needed by `native-tls`), then `cargo build --release --locked`
3. package `.deb` / `.rpm` / Arch with [nfpm](https://github.com/goreleaser/nfpm); `${GOARCH}` and `${VERSION}` in `nfpm.yaml` are injected explicitly by the workflow, never left to defaults
4. on a tag, attach the artifacts to the GitHub Release (GitLab Releases are created through the tag + API, with sha256 / fingerprint / repo URL in the body)

## Changelog

| Version | Changes |
|---------|---------|
| 0.1.1 | First stable release. Startup 243 ms → 125 ms (2.05 MB binary); braille waveform driven by the real audio, no flat-topping, adjustable spacing and padding; per-track automatic gain (measured on the first play, applied from the second, zero extra traffic); first-run wizard and Chinese/English UI; interactive `zmuse config`; fixes: NetEase single-source could not open album/playlist/artist details, the waveform decoder re-downloaded the whole track at the end of a song (+50% traffic), unplayable search results failed silently. `0.1.1-1` withdrawn (crashed on first run with no `~/.config/zmuse`) |
| 0.1.1beta2 | Fix auto-advance switching tracks without sound: with keep-open mpv ends paused, so auto-advance now unpauses explicitly; manual track changes and quality switches keep the previous paused state |
| 0.1.1beta1 | Per-screen hint bar; `i` opens the current-track popup (favorite / download / quality / play order); auto-advance when a track ends (sequential / shuffle / repeat-one); download quality follows the runtime setting |
| 0.1.0-2 | License changed to GPL-3.0, dependency fixes (`openssl` required, `mpv` / `nodejs` optional), TLS switched to native-tls |
| 0.1.0-1 | first public release |

## License

[GPL-3.0](LICENSE)

### End-to-end play script (development)

```bash
# start zmuse inside a tmux session first, then:
scripts/play-e2e.sh <session> '晴天 周杰伦' 40
```

It drives the UI by screen state (wait for a result row with a duration → make
sure the list has focus → wait for "now playing"), instead of guessing with
`sleep`; when the search hits "source unavailable" it presses `R`, re-probes and
searches again; on failure it dumps the whole screen to `/tmp/zb-fail-*.txt`.
Run it whenever you touch playback code instead of clicking through by hand.
