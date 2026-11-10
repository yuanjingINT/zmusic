//! Internet Archive（archive.org）：公有领域与老唱片资料库。
//!
//! 搜索走 `advancedsearch.php`（Lucene 查询），筛 `mediatype:audio`；取流用
//! `metadata/<identifier>` 里列出的直链 mp3。一个 item 只出一行（item 的第一首
//! 可播 mp3，带已知时长），嵌套目录的文件也能拼出下载地址。
//!
//! archive.org 在中国大陆被 DNS 污染，要能搜就得配代理（和 yt-dlp 共用
//! `[ytdlp] proxy`）。没配代理时该源自报不可用，不影响其它源。

use crate::models::{Album, Artist, Lyrics, SearchResult, Song};
use crate::source::http;
use crate::source::{Capabilities, SearchKind, Source, TrackId};
use anyhow::{anyhow, Context, Result};
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::OnceLock;
use std::time::Duration;

const BASE: &str = "https://archive.org";
/// 单曲时长上限：6 小时。超过的多半是拼接坏掉的文件（见过 length=374491）
const MAX_TRACK_SECS: i64 = 6 * 3600;
/// 搜索每个 item 都要再取一次 metadata（含长度），并发压在 6 个
const META_CONCURRENCY: usize = 6;
const SEARCH_TIMEOUT: Duration = Duration::from_secs(10);
const META_TIMEOUT: Duration = Duration::from_secs(25);

pub struct ArchiveSource {
    base: String,
    proxy: Option<String>,
    /// 建 client 失败也要缓存住，别每次按键都去起一次代理桥
    client: OnceLock<Result<Client, String>>,
}

/// 搜索命中的 item（还没取 metadata）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawItem {
    pub identifier: String,
    pub title: String,
    pub creator: String,
}

impl ArchiveSource {
    pub fn new(proxy: Option<String>) -> Self {
        ArchiveSource {
            base: BASE.to_string(),
            proxy: proxy.map(|p| p.trim().to_string()).filter(|p| !p.is_empty()),
            client: OnceLock::new(),
        }
    }

    #[cfg(test)]
    pub fn with_base(base: &str, proxy: Option<&str>) -> Self {
        ArchiveSource {
            base: base.to_string(),
            proxy: proxy.map(|p| p.to_string()),
            client: OnceLock::new(),
        }
    }

    fn client(&self) -> Result<&Client> {
        match self
            .client
            .get_or_init(|| build_client(self.proxy.as_deref()).map_err(|e| e.to_string()))
        {
            Ok(c) => Ok(c),
            Err(e) => Err(anyhow!("{e}")),
        }
    }
}

/// 带超时的 blocking client；配了代理就经内置的 SOCKS→HTTP 桥
fn build_client(proxy: Option<&str>) -> Result<Client> {
    let mut b = Client::builder()
        .timeout(Duration::from_secs(120))
        .connect_timeout(Duration::from_secs(4))
        .pool_max_idle_per_host(8)
        .tcp_nodelay(true)
        .user_agent("zmuse/0.1 (+https://git.tuxai.org/personal/zmuse)");
    if let Some(url) = crate::proxy::http_proxy(proxy)? {
        b = b.proxy(
            reqwest::Proxy::all(&url)
                .with_context(|| format!("代理地址不合法: {url}"))?,
        );
    }
    b.build().context("构建 archive.org HTTP client 失败")
}

fn get_json(client: &Client, url: &str, timeout: Duration) -> Result<Value> {
    let resp = client
        .get(url)
        .timeout(timeout)
        .send()
        .with_context(|| format!("请求失败: {url}"))?;
    if !resp.status().is_success() {
        return Err(anyhow!("HTTP {}: {url}", resp.status()));
    }
    resp.json::<Value>()
        .with_context(|| format!("JSON 解析失败: {url}"))
}

fn text_field(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str())
            .map(|x| x.trim())
            .filter(|x| !x.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        _ => String::new(),
    }
}

/// 解析 `response.docs`：只留 identifier 非空的
pub fn parse_search(v: &Value) -> Vec<RawItem> {
    let Some(docs) = v.pointer("/response/docs").and_then(|d| d.as_array()) else {
        return Vec::new();
    };
    docs.iter()
        .filter_map(|d| {
            let identifier = text_field(d, "identifier");
            if identifier.is_empty() {
                return None;
            }
            let title = text_field(d, "title");
            Some(RawItem {
                title: if title.is_empty() {
                    identifier.clone()
                } else {
                    title
                },
                creator: text_field(d, "creator"),
                identifier,
            })
        })
        .collect()
}

/// 从 metadata 里挑第一首可播 mp3：时长已知且不离谱。
/// 返回 (文件名, 毫秒)。嵌套目录的文件名保留 `/`。
pub fn pick_track(meta: &Value) -> Option<(String, i64)> {
    let files = meta.get("files")?.as_array()?;
    for f in files {
        let name = f.get("name").and_then(|n| n.as_str()).unwrap_or("");
        if name.is_empty() || !name.to_ascii_lowercase().ends_with(".mp3") {
            continue;
        }
        // 受限 item 的文件带 private:true，取流会 403
        let private = f
            .get("private")
            .map(|p| p.as_str() == Some("true") || p.as_bool() == Some(true))
            .unwrap_or(false);
        if private {
            continue;
        }
        let Some(len) = f.get("length").map(length_ms) else {
            continue;
        };
        if len <= 0 || len > MAX_TRACK_SECS * 1000 {
            continue;
        }
        return Some((name.to_string(), len));
    }
    None
}

/// `length` 可能是 `"246.38"`（秒）也可能是 `"04:52"`：都认，认不出给 0
fn length_ms(v: &Value) -> i64 {
    match v {
        Value::String(s) => http::duration_ms(s),
        Value::Number(n) => n.as_f64().map(|x| (x * 1000.0).round() as i64).unwrap_or(0),
        _ => 0,
    }
}

/// `identifier/文件名` → 下载地址。文件名按段编码，`/` 保留（info 里会有子目录）
pub fn stream_url_from_id(base: &str, s_id: &str) -> Option<String> {
    let (identifier, file) = s_id.split_once('/')?;
    if identifier.trim().is_empty() || file.trim().is_empty() {
        return None;
    }
    let path: Vec<String> = file.split('/').filter(|p| !p.is_empty()).map(http::enc).collect();
    if path.is_empty() {
        return None;
    }
    Some(format!(
        "{}/download/{}/{}",
        base.trim_end_matches('/'),
        http::enc(identifier),
        path.join("/")
    ))
}

/// 取一个 item 的 metadata → 一行 Song；拿不到可播文件就 None（不报错，别的 item 还能用）
fn fetch_item(client: &Client, base: &str, item: &RawItem) -> Option<Song> {
    let url = format!("{base}/metadata/{}", http::enc(&item.identifier));
    let meta = get_json(client, &url, META_TIMEOUT).ok()?;
    let (file, dt) = pick_track(&meta)?;
    let album = if item.title.is_empty() {
        item.identifier.clone()
    } else {
        item.title.clone()
    };
    let artist = if item.creator.is_empty() {
        "Internet Archive".to_string()
    } else {
        item.creator.clone()
    };
    Some(Song {
        id: 0,
        name: album.clone(),
        ar: vec![Artist {
            id: 0,
            name: artist,
        }],
        al: Album {
            id: 0,
            name: album,
            pic_url: Some(format!("{base}/services/img/{}", http::enc(&item.identifier))),
        },
        dt,
        fee: 0,
        source: "archive".to_string(),
        s_id: format!("{}/{}", item.identifier, file),
    })
}

impl Source for ArchiveSource {
    fn id(&self) -> &'static str {
        "archive"
    }

    fn name(&self) -> &'static str {
        "Internet Archive"
    }

    fn group(&self) -> &'static str {
        "公开免费"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            kinds: vec![SearchKind::Song],
            lyrics: false,
            qualities: Vec::new(),
            can_stream: true,
        }
    }

    fn is_available(&self) -> bool {
        let Ok(client) = self.client() else {
            return false;
        };
        let url = format!(
            "{}/advancedsearch.php?q=mediatype%3Aaudio&fl%5B%5D=identifier&rows=1&page=1&output=json",
            self.base
        );
        get_json(client, &url, SEARCH_TIMEOUT).is_ok()
    }

    fn search(&self, keyword: &str, limit: i32, kind: SearchKind) -> Result<SearchResult> {
        if kind != SearchKind::Song {
            return Ok(SearchResult::default());
        }
        let kw = keyword.trim();
        if kw.is_empty() {
            return Ok(SearchResult::default());
        }
        let client = self.client()?;
        let rows = limit.clamp(1, 30);
        // 先把结果限定成 audio，用户关键词原样放进括号里（archive 用 Lucene 语法）
        let q = format!("mediatype:audio AND ({kw})");
        let url = format!(
            "{}/advancedsearch.php?q={}&fl%5B%5D=identifier&fl%5B%5D=title&fl%5B%5D=creator\
             &rows={rows}&page=1&output=json&sort%5B%5D=downloads%20desc",
            self.base,
            http::enc(&q),
        );
        let v = get_json(client, &url, SEARCH_TIMEOUT)?;
        let items = parse_search(&v);
        if items.is_empty() {
            return Ok(SearchResult::default());
        }

        // 每个 item 还要一次 metadata 才知道时长和可播文件：分小批并发，
        // 时长未知的按项目规则丢掉（retain_timed 在 UI 侧还会兜一次）
        let mut songs: Vec<Song> = Vec::with_capacity(items.len());
        std::thread::scope(|s| {
            for chunk in items.chunks(META_CONCURRENCY) {
                let handles: Vec<_> = chunk
                    .iter()
                    .map(|it| {
                        let client = client.clone();
                        let base = self.base.clone();
                        s.spawn(move || fetch_item(&client, &base, it))
                    })
                    .collect();
                for h in handles {
                    if let Some(song) = h.join().ok().flatten() {
                        songs.push(song);
                    }
                }
            }
        });
        crate::source::agg::retain_timed(&mut songs);
        songs.truncate(rows as usize);
        Ok(SearchResult {
            songs,
            ..Default::default()
        })
    }

    fn lyric(&self, _id: &TrackId) -> Result<Lyrics> {
        Ok(Lyrics::default())
    }

    fn stream_url(&self, id: &TrackId, _quality: &str) -> Result<Option<String>> {
        Ok(stream_url_from_id(&self.base, id.as_str()))
    }

    /// 懒补时长：收藏/队列里存下来的老数据可能没时长，问一次 metadata
    fn duration_ms(&self, id: &TrackId) -> Result<Option<i64>> {
        let Some((identifier, file)) = id.as_str().split_once('/') else {
            return Ok(None);
        };
        if identifier.is_empty() || file.is_empty() {
            return Ok(None);
        }
        let client = self.client()?;
        let url = format!("{}/metadata/{}", self.base, http::enc(identifier));
        let meta = get_json(client, &url, META_TIMEOUT)?;
        let Some(files) = meta.get("files").and_then(|f| f.as_array()) else {
            return Ok(None);
        };
        for f in files {
            if f.get("name").and_then(|n| n.as_str()) != Some(file) {
                continue;
            }
            let ms = f.get("length").map(length_ms).unwrap_or(0);
            return Ok(if ms > 0 { Some(ms) } else { None });
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_search_docs_and_skips_empty_identifiers() {
        let v = json!({"response":{"numFound":2,"docs":[
            {"identifier":"Torley_Wong_-_The_Final_Selection","title":"Torley Wong - The Final Selection","creator":"Torley Wong"},
            {"identifier":"OTRR_Dark_Fantasy_Singles","title":"Dark Fantasy - Single Episodes","creator":["Old Time Radio","Group"]},
            {"title":"no identifier"},
            {"identifier":"NoTitle"}
        ]}});
        let items = parse_search(&v);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].identifier, "Torley_Wong_-_The_Final_Selection");
        assert_eq!(items[1].creator, "Old Time Radio, Group", "creator 数组拼起来");
        assert_eq!(items[2].title, "NoTitle", "没标题就用 identifier");
        assert!(parse_search(&json!({"error":"x"})).is_empty());
    }

    #[test]
    fn picks_first_mp3_with_a_sane_length() {
        let meta = json!({"files":[
            {"name":"cover.png","format":"PNG"},
            {"name":"__ia_thumb.jpg","format":"JPEG"},
            {"name":"a.flac","format":"Flac","length":"500"},
            {"name":"notes.txt","format":"Text"},
            {"name":"01 track.mp3","format":"VBR MP3","length":"04:52"},
            {"name":"02 track.mp3","format":"192Kbps MP3","length":"186.88"}
        ]});
        assert_eq!(
            pick_track(&meta),
            Some(("01 track.mp3".to_string(), 292_000))
        );
    }

    #[test]
    fn drops_unknown_absurd_and_private_files() {
        let meta = json!({"files":[
            {"name":"private.mp3","length":"100","private":"true"},
            {"name":"broken.mp3"},
            {"name":"weird.mp3","length":"374491.55"},
            {"name":"ok.mp3","length":"200"}
        ]});
        assert_eq!(pick_track(&meta), Some(("ok.mp3".to_string(), 200_000)));
        assert_eq!(pick_track(&json!({"files":[]})), None);
        assert_eq!(pick_track(&json!({})), None);
    }

    #[test]
    fn builds_download_url_with_nested_paths_and_encoding() {
        assert_eq!(
            stream_url_from_id(BASE, "Torley_Wong_-_The_Final_Selection/Torley_Wong-1981_A.D..mp3")
                .as_deref(),
            Some(
                "https://archive.org/download/Torley_Wong_-_The_Final_Selection/Torley_Wong-1981_A.D..mp3"
            )
        );
        assert_eq!(
            stream_url_from_id(
                BASE,
                "MusopenCollectionAsFlac/Bach_GoldbergVariations/Johann Sebastian Bach-01-Aria.mp3"
            )
            .as_deref(),
            Some(
                "https://archive.org/download/MusopenCollectionAsFlac/Bach_GoldbergVariations/Johann%20Sebastian%20Bach-01-Aria.mp3"
            )
        );
        assert_eq!(stream_url_from_id(BASE, "no-slash"), None);
        assert_eq!(stream_url_from_id(BASE, "/onlyfile.mp3"), None);
        assert_eq!(stream_url_from_id(BASE, "id/"), None);
    }

    #[test]
    fn unreachable_or_bad_proxy_degrades_without_panicking() {
        let s = ArchiveSource::with_base("http://127.0.0.1:9", None);
        assert!(!s.is_available());
        assert!(s.search("beethoven", 5, SearchKind::Song).is_err());

        // 代理串不合法：自报不可用，而不是 panic
        let s = ArchiveSource::with_base(BASE, Some("nonsense"));
        assert!(!s.is_available());
        assert!(s.search("x", 5, SearchKind::Song).is_err());
    }

    #[test]
    fn song_source_and_id_shape() {
        let meta = json!({"files":[{"name":"sub/01.mp3","length":"100"}]});
        let item = RawItem {
            identifier: "item-id".to_string(),
            title: "Album".to_string(),
            creator: String::new(),
        };
        // fetch_item 需要网络，这里只验可播文件解析 + 拼地址这条链
        let (file, dt) = pick_track(&meta).expect("应有可播文件");
        assert_eq!(dt, 100_000);
        let s_id = format!("{}/{}", item.identifier, file);
        assert_eq!(s_id, "item-id/sub/01.mp3");
        assert_eq!(
            stream_url_from_id(BASE, &s_id).as_deref(),
            Some("https://archive.org/download/item-id/sub/01.mp3")
        );
    }

    /// 真网络：`cargo test --release archive_live -- --ignored --nocapture`
    /// （archive.org 在国内被墙，本机跑要 `ZDROP` 代理：ZMUSE_LIVE_PROXY=socks5h://127.0.0.1:11081）
    #[test]
    #[ignore]
    fn archive_live_search() {
        let proxy = std::env::var("ZMUSE_LIVE_PROXY").ok();
        let s = ArchiveSource::new(proxy);
        let r = s.search("Beethoven symphony no 5", 5, SearchKind::Song).expect("搜索失败");
        println!("archive_live_search: {} 条", r.songs.len());
        for song in &r.songs {
            println!("  {} | {} | {}ms | {}", song.name, song.ar[0].name, song.dt, song.s_id);
            assert!(song.dt > 0);
        }
        assert!(!r.songs.is_empty());
        let first = &r.songs[0];
        let url = s
            .stream_url(&TrackId(first.s_id.clone()), "hires")
            .expect("取流不应报错")
            .expect("应有直链");
        println!("  stream: {url}");
        assert!(url.starts_with("https://archive.org/download/"));
    }

    /// 真 mpv：把配置里的 SOCKS 代理经内置桥转成 HTTP 代理喂给 mpv，
    /// 确认 archive.org 的直链真的能放（`--no-audio`，不碰用户的音频设备）。
    /// `ZMUSE_LIVE_PROXY=socks5h://127.0.0.1:11081 cargo test --release archive_live_playback -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn archive_live_playback() {
        let proxy = std::env::var("ZMUSE_LIVE_PROXY").expect("要 ZMUSE_LIVE_PROXY");
        let s = ArchiveSource::new(Some(proxy.clone()));
        let r = s.search("Beethoven symphony no 5", 3, SearchKind::Song).expect("搜索失败");
        let song = r.songs.first().expect("应有结果");
        let url = s
            .stream_url(&TrackId(song.s_id.clone()), "hires")
            .expect("取流不该报错")
            .expect("应有直链");
        let http_proxy = crate::proxy::http_proxy(Some(&proxy))
            .expect("起代理桥失败")
            .expect("应有桥地址");
        println!("  stream: {url}\n  bridge: {http_proxy}");
        let out = std::process::Command::new("mpv")
            .args([
                // 只把声音丢给 null 输出设备：解码/取流都真跑，用户的音频设备不碰
                "--ao=null",
                "--no-video",
                "--force-window=no",
                "--end=5",
                &format!("--http-proxy={http_proxy}"),
                &url,
            ])
            .output()
            .expect("mpv 起不来");
        println!("  mpv exit: {:?}", out.status.code());
        println!("--- mpv stdout ---\n{}", String::from_utf8_lossy(&out.stdout));
        println!("--- mpv stderr ---\n{}", String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success(), "mpv 播放失败");
    }
}
