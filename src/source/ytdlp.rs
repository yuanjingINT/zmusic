//! yt-dlp 泛化源：同一套 `yt-dlp` 子进程逻辑按站点参数化，一次覆盖多家。
//!
//! 现接：YouTube（`ytsearch`）、SoundCloud（`scsearch`）。
//!
//! 未接入 **Bandcamp**，原因是实测过的，不是懒得写：
//! - 本机直连 `bandcamp.com` 被重置（`curl: (35) Recv failure: 连接被对方重置`）；
//! - 经 kr22 出口取搜索页/曲目页，返回 3038 字节的 `Client Challenge` 反爬页
//!   （HTTP 200，但正文没有 `tralbum`），`yt-dlp -g` 报
//!   `Unable to extract tralbum data`；
//! - `yt-dlp` 本身也没有 Bandcamp 的搜索前缀（只有 `ytsearch`/`scsearch` 这类），
//!   所以搜索这一步无论如何都得抓它的 HTML。
//!
//! 等哪天 Bandcamp 从可用出口能直连，只要给 [`Site`] 加一个变体 + 补 HTML 搜索
//! 解析即可，取流部分沿用现成的。
//!
//! 约定：
//! - 搜索：`yt-dlp "<prefix><kw>" --flat-playlist --dump-json`，按站点给超时，
//!   结果按 query 缓存 5 分钟（搜索慢，且可能被限流）
//! - 取流：`yt-dlp -g -f "bestaudio/best" --no-warnings <url>`，15s 超时，
//!   **不缓存地址**（流地址会过期）
//! - 失败一律降级：yt-dlp 不在 PATH / 非零退出 / 超时 → 标记不可用，报错给上层，
//!   绝不影响其它源
//! - 配了代理时，yt-dlp 与 mpv 必须同一出口（见 `crate::proxy`）

use crate::models::{Album, Artist, Lyrics, SearchResult, Song};
use crate::source::{SearchKind, Source, TrackId};
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const STREAM_TIMEOUT: Duration = Duration::from_secs(15);
const PROBE_TIMEOUT: Duration = Duration::from_secs(4);
const CACHE_TTL: Duration = Duration::from_secs(300);
const SEARCH_LIMIT: usize = 10;

/// 走 yt-dlp 的站点。`id()` 就是 `Song.source` 里写的值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Site {
    YouTube,
    SoundCloud,
}

impl Site {
    pub fn id(self) -> &'static str {
        match self {
            Site::YouTube => "youtube",
            Site::SoundCloud => "soundcloud",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Site::YouTube => "YouTube",
            Site::SoundCloud => "SoundCloud",
        }
    }

    /// `yt-dlp` 的搜索前缀；`None` = 该站点没有搜索入口（Bandcamp 就是这样）
    fn search_prefix(self) -> Option<&'static str> {
        match self {
            Site::YouTube => Some("ytsearch"),
            Site::SoundCloud => Some("scsearch"),
        }
    }

    /// SoundCloud 搜索实测 6–7s，给 8s 会踩线；YouTube 一般 2–4s
    fn search_timeout(self) -> Duration {
        match self {
            Site::YouTube => Duration::from_secs(8),
            Site::SoundCloud => Duration::from_secs(12),
        }
    }

    /// 把源内 ID 还原成 yt-dlp 能吃的 URL。搜索时拿到的完整网页地址优先，
    /// 只有 YouTube 这种能靠裸 id 拼的才走模板。
    fn item_url(self, s_id: &str) -> String {
        if s_id.starts_with("http://") || s_id.starts_with("https://") {
            return s_id.to_string();
        }
        match self {
            Site::YouTube => format!("https://www.youtube.com/watch?v={}", s_id),
            Site::SoundCloud => format!("https://api.soundcloud.com/tracks/{}", s_id),
        }
    }
}

pub struct YtDlpSource {
    site: Site,
    exe: String,
    /// yt-dlp 的 --proxy；和 mpv 用同一个出口，否则流地址可能 403
    proxy: Option<String>,
    /// query -> (写入时间, 搜索结果)
    cache: Mutex<HashMap<String, (Instant, Vec<Song>)>>,
    /// 探活结果缓存：避免每次搜索都跑 `--version`
    available: Mutex<Option<bool>>,
}

impl YtDlpSource {
    pub fn new(site: Site) -> Self {
        Self::with_proxy(site, "yt-dlp", None)
    }

    #[cfg(test)]
    pub fn with_exe(site: Site, exe: &str) -> Self {
        Self::with_proxy(site, exe, None)
    }

    pub fn with_proxy(site: Site, exe: &str, proxy: Option<String>) -> Self {
        YtDlpSource {
            site,
            exe: exe.to_string(),
            proxy,
            cache: Mutex::new(HashMap::new()),
            available: Mutex::new(None),
        }
    }

    /// 跑一条 yt-dlp 命令，带超时和输出收集。
    fn run(&self, args: &[&str], timeout: Duration) -> Result<String> {
        let mut full: Vec<&str> = Vec::with_capacity(args.len() + 2);
        if let Some(p) = &self.proxy {
            full.push("--proxy");
            full.push(p);
        }
        full.extend_from_slice(args);
        let mut child = Command::new(&self.exe)
            .args(&full)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| anyhow!("无法启动 {}: {}", self.exe, e))?;

        // stdout 单独线程读，避免输出超过管道缓冲时子进程卡住
        let mut stdout = child.stdout.take().expect("stdout 已 piped");
        let reader = std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = stdout.read_to_string(&mut buf);
            buf
        });

        let deadline = Instant::now() + timeout;
        let status = loop {
            match child.try_wait()? {
                Some(st) => break st,
                None if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = reader.join();
                    return Err(anyhow!("{} 超时（{}s）", self.exe, timeout.as_secs()));
                }
                None => std::thread::sleep(Duration::from_millis(20)),
            }
        };
        let text = reader.join().unwrap_or_default();
        if !status.success() {
            let mut err = String::new();
            if let Some(mut e) = child.stderr.take() {
                let _ = e.read_to_string(&mut err);
            }
            let tail: String = err.lines().last().unwrap_or("").to_string();
            return Err(anyhow!("{} 退出码 {:?}: {}", self.exe, status.code(), tail));
        }
        Ok(text)
    }

    fn probe(&self) -> bool {
        if let Some(v) = *self.available.lock().unwrap() {
            return v;
        }
        let ok = self.run(&["--version"], PROBE_TIMEOUT).is_ok();
        *self.available.lock().unwrap() = Some(ok);
        ok
    }

    fn search_uncached(&self, keyword: &str) -> Result<Vec<Song>> {
        let prefix = self
            .site
            .search_prefix()
            .ok_or_else(|| anyhow!("{} 没有搜索接口", self.site.name()))?;
        let q = format!("{prefix}{SEARCH_LIMIT}:{keyword}");
        let text = self.run(
            &[&q, "--flat-playlist", "--dump-json", "--no-warnings"],
            self.site.search_timeout(),
        )?;
        Ok(parse_items(&text, self.site))
    }

    /// 给 UI 用：这个源是否要靠 yt-dlp 取流（代理要跟着走）
    pub fn needs_proxy_for(source: &str) -> bool {
        matches!(source, "youtube" | "soundcloud")
    }
}

/// 把 yt-dlp `--dump-json` 的每行 JSON 变成 `Song`。纯函数，方便单测。
///
/// 各站点字段名不完全一样，这里只认实际出现过的：
/// - `id`：YouTube 是视频 id，SoundCloud 是数字 id
/// - `webpage_url`：SoundCloud 直接给可播放页面；YouTube 的 flat 输出不一定有
/// - 缩略图：YouTube 是 `thumbnail` 字符串，SoundCloud 是 `thumbnails` 数组
pub fn parse_items(text: &str, site: Site) -> Vec<Song> {
    let mut songs = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(id) = v.get("id").and_then(|x| x.as_str()) else {
            continue;
        };
        let title = v
            .get("title")
            .and_then(|x| x.as_str())
            .unwrap_or("(无标题)")
            .to_string();
        let uploader = v
            .get("uploader")
            .or_else(|| v.get("channel"))
            .or_else(|| v.get("artist"))
            .and_then(|x| x.as_str())
            .unwrap_or(site.name())
            .to_string();
        let duration_ms = v
            .get("duration")
            .and_then(|x| x.as_f64())
            .map(|s| (s * 1000.0) as i64)
            .unwrap_or(0);
        let thumb = v
            .get("thumbnail")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                v.get("thumbnails")
                    .and_then(|x| x.as_array())
                    .and_then(|a| a.last())
                    .and_then(|t| t.get("url"))
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string())
            });
        // YouTube 用裸 id 拼 URL；其它站点优先用产出的完整网页地址
        let s_id = match site {
            Site::YouTube => id.to_string(),
            Site::SoundCloud => v
                .get("webpage_url")
                .and_then(|x| x.as_str())
                .filter(|u| u.starts_with("http"))
                .unwrap_or(id)
                .to_string(),
        };
        songs.push(Song {
            id: 0,
            name: title,
            ar: vec![Artist {
                id: 0,
                name: uploader,
            }],
            al: Album {
                id: 0,
                name: site.name().to_string(),
                pic_url: thumb,
            },
            dt: duration_ms,
            fee: 0,
            source: site.id().to_string(),
            s_id,
        });
    }
    songs
}

impl Source for YtDlpSource {
    fn id(&self) -> &'static str {
        self.site.id()
    }

    fn name(&self) -> &'static str {
        self.site.name()
    }

    fn group(&self) -> &'static str {
        "国际"
    }

    fn capabilities(&self) -> crate::source::Capabilities {
        crate::source::Capabilities {
            kinds: vec![SearchKind::Song],
            lyrics: false,
            qualities: Vec::new(),
        }
    }

    fn is_available(&self) -> bool {
        self.probe()
    }

    fn search(&self, keyword: &str, _limit: i32, kind: SearchKind) -> Result<SearchResult> {
        // 一律只提供单曲结果
        if kind != SearchKind::Song {
            return Ok(SearchResult::default());
        }
        if !self.probe() {
            return Err(anyhow!("{} 不可用（没装或跑不起来）", self.exe));
        }

        if let Some((at, songs)) = self.cache.lock().unwrap().get(keyword) {
            if at.elapsed() < CACHE_TTL {
                return Ok(SearchResult {
                    songs: songs.clone(),
                    ..Default::default()
                });
            }
        }

        match self.search_uncached(keyword) {
            Ok(songs) => {
                self.cache
                    .lock()
                    .unwrap()
                    .insert(keyword.to_string(), (Instant::now(), songs.clone()));
                Ok(SearchResult {
                    songs,
                    ..Default::default()
                })
            }
            Err(e) => {
                // 跑失败就标不可用，UI 上把这一行置灰，别再拖累其它源
                *self.available.lock().unwrap() = Some(false);
                Err(e)
            }
        }
    }

    fn lyric(&self, _id: &TrackId) -> Result<Lyrics> {
        Ok(Lyrics::default())
    }

    fn stream_url(&self, id: &TrackId, _quality: &str) -> Result<Option<String>> {
        let url = self.site.item_url(id.as_str());
        let out = self.run(
            &["-g", "-f", "bestaudio/best", "--no-warnings", &url],
            STREAM_TIMEOUT,
        )?;
        // -g 会输出一行或多行地址，取第一条 http(s)
        Ok(out
            .lines()
            .map(|l| l.trim())
            .find(|l| l.starts_with("http"))
            .map(|l| l.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const YT: &str = r#"
{"id":"dQw4w9WgXcQ","title":"Never Gonna Give You Up","uploader":"Rick Astley","duration":213.0,"thumbnail":"https://i.ytimg.com/x.jpg"}
{"id":"abc123","title":"另一个结果","channel":"Some Channel","duration":null}
not-json-line
{"title":"没有 id 的记录"}
"#;

    const SC: &str = r#"
{"id":"88335161","title":"Get Lucky [Original Version] - Daft Punk","uploader":"DJ KB","duration":246.381,"webpage_url":"https://soundcloud.com/eldjkb/get-lucky-original-version","thumbnails":[{"url":"https://i1.sndcdn.com/x-mini.jpg"},{"url":"https://i1.sndcdn.com/x-original.jpg"}]}
"#;

    #[test]
    fn parses_youtube_lines() {
        let songs = parse_items(YT, Site::YouTube);
        assert_eq!(songs.len(), 2);
        assert_eq!(songs[0].source, "youtube");
        assert_eq!(songs[0].s_id, "dQw4w9WgXcQ");
        assert_eq!(songs[0].name, "Never Gonna Give You Up");
        assert_eq!(songs[0].ar[0].name, "Rick Astley");
        assert_eq!(songs[0].dt, 213_000);
        assert_eq!(
            songs[0].al.pic_url.as_deref(),
            Some("https://i.ytimg.com/x.jpg")
        );
        assert_eq!(songs[1].ar[0].name, "Some Channel");
        assert_eq!(songs[1].dt, 0, "duration 为 null 时按 0 处理");
    }

    #[test]
    fn parses_soundcloud_uses_webpage_url_and_last_thumbnail() {
        let songs = parse_items(SC, Site::SoundCloud);
        assert_eq!(songs.len(), 1);
        let s = &songs[0];
        assert_eq!(s.source, "soundcloud");
        assert_eq!(
            s.s_id,
            "https://soundcloud.com/eldjkb/get-lucky-original-version"
        );
        assert_eq!(s.dt, 246_381);
        assert_eq!(s.al.name, "SoundCloud");
        assert_eq!(s.ar[0].name, "DJ KB");
        assert_eq!(
            s.al.pic_url.as_deref(),
            Some("https://i1.sndcdn.com/x-original.jpg"),
            "thumbnails 数组取最后一张（最大）"
        );
    }

    #[test]
    fn item_url_prefers_absolute_and_templates_youtube() {
        assert_eq!(
            Site::YouTube.item_url("dQw4w9WgXcQ"),
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ"
        );
        assert_eq!(
            Site::SoundCloud.item_url("https://soundcloud.com/a/b"),
            "https://soundcloud.com/a/b"
        );
        assert_eq!(
            Site::SoundCloud.item_url("88335161"),
            "https://api.soundcloud.com/tracks/88335161"
        );
    }

    #[test]
    fn missing_executable_is_unavailable_and_errors() {
        for site in [Site::YouTube, Site::SoundCloud] {
            let src = YtDlpSource::with_exe(site, "yt-dlp-does-not-exist-xyz");
            assert!(!src.is_available());
            let err = src
                .search("晴天", 10, SearchKind::Song)
                .expect_err("缺 yt-dlp 必须报错");
            assert!(
                err.to_string().contains("不可用"),
                "错误应说明不可用: {err}"
            );
        }
    }

    #[test]
    fn non_song_kinds_return_empty_without_spawning() {
        let src = YtDlpSource::with_exe(Site::SoundCloud, "yt-dlp-does-not-exist-xyz");
        let r = src.search("x", 10, SearchKind::Album).unwrap();
        assert!(r.songs.is_empty());
    }

    /// 真跑 yt-dlp 的冒烟测试（默认 ignore，需要出口）：
    /// `ZMUSIC_LIVE_PROXY=socks5h://127.0.0.1:11081 cargo test --release ytdlp_live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn ytdlp_live_search_and_stream() {
        let Ok(proxy) = std::env::var("ZMUSIC_LIVE_PROXY") else {
            eprintln!("跳过：没有设置 ZMUSIC_LIVE_PROXY");
            return;
        };
        for site in [Site::YouTube, Site::SoundCloud] {
            let src = YtDlpSource::with_proxy(site, "yt-dlp", Some(proxy.clone()));
            let got = src
                .search("daft punk get lucky", 5, SearchKind::Song)
                .unwrap_or_else(|e| panic!("{} 搜索失败: {e}", site.name()));
            assert!(!got.songs.is_empty(), "{} 应有结果", site.name());
            for s in got.songs.iter().take(3) {
                eprintln!("{} | {} | {} | {}s", s.source, s.name, s.s_id, s.dt / 1000);
            }
            let track = crate::source::track_of(&got.songs[0]);
            let url = src
                .stream_url(&track, "lossless")
                .unwrap_or_else(|e| panic!("{} 取流失败: {e}", site.name()))
                .unwrap_or_else(|| panic!("{} 没有返回流地址", site.name()));
            eprintln!("{} stream: {:.60}…", site.name(), url);
        }
    }

    #[test]
    fn proxy_routing_matches_ytdlp_backed_sources() {
        assert!(YtDlpSource::needs_proxy_for("youtube"));
        assert!(YtDlpSource::needs_proxy_for("soundcloud"));
        assert!(!YtDlpSource::needs_proxy_for("netease"));
        assert!(!YtDlpSource::needs_proxy_for("meting:tencent"));
    }
}
