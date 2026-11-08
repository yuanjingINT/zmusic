//! YouTube 源：走 `yt-dlp` 子进程，不引入 Rust 侧 HTTP/解密依赖。
//!
//! 约定（与 Chief 定的规格一致）：
//! - 搜索：`yt-dlp "ytsearch10:<kw>" --flat-playlist --dump-json`，8s 超时，
//!   结果按 query 缓存 5 分钟（搜索慢，且可能被限流）
//! - 取流：`yt-dlp -g -f "bestaudio/best" --no-warnings <url>`，15s 超时，
//!   **不缓存地址**（YouTube 的流地址约 6 小时过期）
//! - 失败一律降级：yt-dlp 不在 PATH / 非零退出 / 超时 → 标记不可用，报错给上层，
//!   绝不影响其它源

use crate::models::{Album, Artist, Lyrics, SearchResult, Song};
use crate::source::{SearchKind, Source, TrackId};
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const SEARCH_TIMEOUT: Duration = Duration::from_secs(8);
const STREAM_TIMEOUT: Duration = Duration::from_secs(15);
const PROBE_TIMEOUT: Duration = Duration::from_secs(4);
const CACHE_TTL: Duration = Duration::from_secs(300);
const SEARCH_LIMIT: usize = 10;

pub struct YouTubeSource {
    exe: String,
    /// yt-dlp 的 --proxy；和 mpv 用同一个出口，否则流地址会 403
    proxy: Option<String>,
    /// query -> (写入时间, 搜索结果)
    cache: Mutex<HashMap<String, (Instant, Vec<Song>)>>,
    /// 探活结果缓存：避免每次搜索都跑 `--version`
    available: Mutex<Option<bool>>,
}

impl YouTubeSource {
    pub fn new() -> Self {
        Self::with_exe("yt-dlp")
    }

    pub fn with_exe(exe: &str) -> Self {
        Self::with_proxy(exe, None)
    }

    pub fn with_proxy(exe: &str, proxy: Option<String>) -> Self {
        YouTubeSource {
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
        let q = format!("ytsearch{SEARCH_LIMIT}:{keyword}");
        let text = self.run(
            &[&q, "--flat-playlist", "--dump-json", "--no-warnings"],
            SEARCH_TIMEOUT,
        )?;
        Ok(parse_items(&text))
    }
}

/// 把 yt-dlp `--dump-json` 的每行 JSON 变成 `Song`。纯函数，方便单测。
pub fn parse_items(text: &str) -> Vec<Song> {
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
            .and_then(|x| x.as_str())
            .unwrap_or("YouTube")
            .to_string();
        let duration_ms = v
            .get("duration")
            .and_then(|x| x.as_f64())
            .map(|s| (s * 1000.0) as i64)
            .unwrap_or(0);
        let thumb = v
            .get("thumbnail")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());
        songs.push(Song {
            id: 0,
            name: title,
            ar: vec![Artist {
                id: 0,
                name: uploader,
            }],
            al: Album {
                id: 0,
                name: "YouTube".to_string(),
                pic_url: thumb,
            },
            dt: duration_ms,
            fee: 0,
            source: "youtube".to_string(),
            s_id: id.to_string(),
        });
    }
    songs
}

impl Source for YouTubeSource {
    fn id(&self) -> &'static str {
        "youtube"
    }

    fn name(&self) -> &'static str {
        "YouTube"
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
        // YouTube 只提供单曲结果
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
                // 跑失败就标不可用，UI 上把这一行置灰，别再拖累网易云
                *self.available.lock().unwrap() = Some(false);
                Err(e)
            }
        }
    }

    fn lyric(&self, _id: &TrackId) -> Result<Lyrics> {
        Ok(Lyrics::default())
    }

    fn stream_url(&self, id: &TrackId, _quality: &str) -> Result<Option<String>> {
        let url = format!("https://www.youtube.com/watch?v={}", id.as_str());
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

    #[test]
    fn parses_flat_playlist_json_lines() {
        let text = r#"
{"id":"dQw4w9WgXcQ","title":"Never Gonna Give You Up","uploader":"Rick Astley","duration":213.0,"thumbnail":"https://i.ytimg.com/x.jpg"}
{"id":"abc123","title":"另一个结果","channel":"Some Channel","duration":null}
not-json-line
{"title":"没有 id 的记录"}
"#;
        let songs = parse_items(text);
        assert_eq!(songs.len(), 2);
        assert_eq!(songs[0].source, "youtube");
        assert_eq!(songs[0].s_id, "dQw4w9WgXcQ");
        assert_eq!(songs[0].name, "Never Gonna Give You Up");
        assert_eq!(songs[0].ar[0].name, "Rick Astley");
        assert_eq!(songs[0].dt, 213_000);
        assert_eq!(songs[1].ar[0].name, "Some Channel");
        assert_eq!(songs[1].dt, 0, "duration 为 null 时按 0 处理");
    }

    #[test]
    fn missing_executable_is_unavailable_and_errors() {
        let src = YouTubeSource::with_exe("yt-dlp-does-not-exist-xyz");
        assert!(!src.is_available());
        let err = src
            .search("晴天", 10, SearchKind::Song)
            .expect_err("缺 yt-dlp 必须报错");
        assert!(err.to_string().contains("不可用"), "错误应说明不可用: {err}");
    }

    #[test]
    fn non_song_kinds_return_empty_without_spawning() {
        let src = YouTubeSource::with_exe("yt-dlp-does-not-exist-xyz");
        let r = src.search("x", 10, SearchKind::Album).unwrap();
        assert!(r.songs.is_empty());
    }
}
