//! bilibili 源：搜索用它自己的 web API，取流借 `yt-dlp`。
//!
//! 实测结论（2026-11）：
//! - `api.bilibili.com/x/web-interface/search/type` 直接打会 412（风控），
//!   必须先拿 `buvid3`：`GET /x/frontend/finger/spi` → `data.b_3`（不用抓首页 HTML）
//! - 带上 `Cookie: buvid3=…` 后 `code=0`、一次 20 条，`title`/`duration`/`author`/`bvid` 齐全
//! - 取流走 `yt-dlp -g -f bestaudio/best https://www.bilibili.com/video/<bvid>`，
//!   拿到的 CDN 直链**不需要 Referer**（实测无 Referer 也 200），可以直接交给 mpv
//!
//! 失败一律降级：拿不到 buvid3 / 接口改版 / yt-dlp 不在 → 该源报错置灰，不拖累其它源。

use crate::models::{Album, Artist, Lyrics, SearchResult, Song};
use crate::source::http;
use crate::source::ytdlp;
use crate::source::{Capabilities, SearchKind, Source, TrackId};
use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::sync::Mutex;
use std::time::Duration;

const API: &str = "https://api.bilibili.com";
/// 取流比搜索慢（要先解析 DASH），给足
const STREAM_TIMEOUT: Duration = Duration::from_secs(20);

pub struct BilibiliSource {
    api: String,
    exe: String,
    /// buvid3 风控 cookie，拿到就复用；失效（412）时刷新一次
    cookie: Mutex<Option<String>>,
}

impl BilibiliSource {
    pub fn new() -> Self {
        BilibiliSource {
            api: API.to_string(),
            exe: "yt-dlp".to_string(),
            cookie: Mutex::new(None),
        }
    }

    #[cfg(test)]
    pub fn with_base(api: &str) -> Self {
        BilibiliSource {
            api: api.to_string(),
            exe: "yt-dlp".to_string(),
            cookie: Mutex::new(None),
        }
    }

    #[cfg(test)]
    pub fn with_base_and_exe(api: &str, exe: &str) -> Self {
        BilibiliSource {
            api: api.to_string(),
            exe: exe.to_string(),
            cookie: Mutex::new(None),
        }
    }

    fn buvid(&self) -> Result<String> {
        if let Some(c) = self.cookie.lock().unwrap().clone() {
            return Ok(c);
        }
        let url = format!("{}/x/frontend/finger/spi", self.api);
        let v = http::get_json(&url).context("拿 buvid3 失败（bilibili 风控接口）")?;
        let b3 = v
            .get("data")
            .and_then(|d| d.get("b_3"))
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("bilibili 没返回 b_3: {v}"))?;
        let cookie = format!("buvid3={b3}");
        *self.cookie.lock().unwrap() = Some(cookie.clone());
        Ok(cookie)
    }

    /// 带 cookie 发一次搜索；`code=-412` 说明 cookie 失效，调用方刷新重试
    fn search_raw(&self, keyword: &str, cookie: &str) -> Result<(i64, Value)> {
        let url = format!(
            "{}/x/web-interface/search/type?search_type=video&page=1&keyword={}",
            self.api,
            http::enc(keyword)
        );
        let resp = http::client()
            .get(&url)
            .header("Cookie", cookie)
            .header("Referer", "https://www.bilibili.com/")
            .send()
            .with_context(|| format!("bilibili 搜索请求失败: {keyword}"))?;
        if !resp.status().is_success() {
            return Err(anyhow!("bilibili HTTP {}", resp.status()));
        }
        let v: Value = resp.json().context("bilibili 搜索返回的不是 JSON")?;
        let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        Ok((code, v))
    }
}

/// 去掉 bilibili 标题里的高亮标签和 HTML 实体。纯函数，方便单测。
pub fn clean_title(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .trim()
        .to_string()
}

/// 搜索接口 `data.result[]` → `Song`。纯函数，方便单测。
pub fn parse_search(v: &Value, limit: usize) -> Vec<Song> {
    let arr = v
        .get("data")
        .and_then(|d| d.get("result"))
        .and_then(|r| r.as_array());
    let Some(arr) = arr else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|it| {
            let bvid = it.get("bvid").and_then(|x| x.as_str())?;
            let title = clean_title(it.get("title").and_then(|x| x.as_str()).unwrap_or(""));
            let author = it
                .get("author")
                .and_then(|x| x.as_str())
                .unwrap_or("bilibili")
                .to_string();
            let dt = http::duration_ms(it.get("duration").and_then(|x| x.as_str()).unwrap_or(""));
            let pic = it.get("pic").and_then(|x| x.as_str()).map(|p| {
                if p.starts_with("//") {
                    format!("https:{p}")
                } else {
                    p.to_string()
                }
            });
            Some(Song {
                id: 0,
                name: if title.is_empty() {
                    bvid.to_string()
                } else {
                    title
                },
                ar: vec![Artist {
                    id: 0,
                    name: author,
                }],
                al: Album {
                    id: 0,
                    name: "bilibili".to_string(),
                    pic_url: pic,
                },
                dt,
                fee: 0,
                source: "bilibili".to_string(),
                s_id: bvid.to_string(),
            })
        })
        .take(limit)
        .collect()
}

impl Source for BilibiliSource {
    fn id(&self) -> &'static str {
        "bilibili"
    }

    fn name(&self) -> &'static str {
        "bilibili"
    }

    fn group(&self) -> &'static str {
        "国内"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            kinds: vec![SearchKind::Song],
            lyrics: false,
            qualities: Vec::new(),
        }
    }

    fn search(&self, keyword: &str, limit: i32, kind: SearchKind) -> Result<SearchResult> {
        if kind != SearchKind::Song {
            return Ok(SearchResult::default());
        }
        let limit = limit.clamp(1, 50) as usize;
        let cookie = self.buvid()?;
        let (mut code, mut v) = self.search_raw(keyword, &cookie)?;
        if code == -412 {
            // cookie 过期：清掉重拿一次，还不行就降级
            *self.cookie.lock().unwrap() = None;
            let fresh = self.buvid()?;
            let (c2, v2) = self.search_raw(keyword, &fresh)?;
            code = c2;
            v = v2;
        }
        if code != 0 {
            let msg = v.get("message").and_then(|m| m.as_str()).unwrap_or("");
            return Err(anyhow!("bilibili 搜索失败 code={code} {msg}"));
        }
        Ok(SearchResult {
            songs: parse_search(&v, limit),
            ..Default::default()
        })
    }

    fn lyric(&self, _id: &TrackId) -> Result<Lyrics> {
        Ok(Lyrics::default())
    }

    fn stream_url(&self, id: &TrackId, _quality: &str) -> Result<Option<String>> {
        let url = format!("https://www.bilibili.com/video/{}", id.as_str());
        ytdlp::stream_url_of(&self.exe, None, &url, STREAM_TIMEOUT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"code":0,"data":{"result":[
{"bvid":"BV1BZbSzZEGT","title":"【𝐇𝐢-𝐑𝐞𝐬无损音质】｜《<em class=\"keyword\">晴天</em>》","duration":"4:30","author":"VV音乐局","pic":"//i0.hdslb.com/bfs/archive/a.jpg"},
{"bvid":"BV1d4411N7zD","title":"循环歌曲《<em class=\"keyword\">晴天</em>》&quot;","duration":"1:02:03","author":"Echo"}
]}}"#;

    #[test]
    fn cleans_highlight_tags_and_entities() {
        assert_eq!(
            clean_title("【无损】《<em class=\"keyword\">晴天</em>》&quot;"),
            "【无损】《晴天》\""
        );
        assert_eq!(clean_title("a &amp; b"), "a & b");
        assert_eq!(clean_title("  <b></b>  "), "");
    }

    #[test]
    fn parses_search_result_with_duration_and_cover() {
        let v: Value = serde_json::from_str(SAMPLE).unwrap();
        let songs = parse_search(&v, 10);
        assert_eq!(songs.len(), 2);
        assert_eq!(songs[0].source, "bilibili");
        assert_eq!(songs[0].s_id, "BV1BZbSzZEGT");
        assert!(songs[0].name.contains("晴天"), "{}", songs[0].name);
        assert!(!songs[0].name.contains('<'), "标签要清掉");
        assert_eq!(songs[0].dt, 270_000);
        assert_eq!(songs[0].ar[0].name, "VV音乐局");
        assert_eq!(
            songs[0].al.pic_url.as_deref(),
            Some("https://i0.hdslb.com/bfs/archive/a.jpg")
        );
        assert_eq!(songs[1].dt, 3_723_000, "支持 h:mm:ss");
        assert_eq!(songs[1].al.pic_url, None);
    }

    #[test]
    fn non_song_kind_returns_empty() {
        let src = BilibiliSource::with_base("http://127.0.0.1:9");
        let r = src.search("x", 10, SearchKind::Album).unwrap();
        assert!(r.songs.is_empty(), "非歌曲类型不联网");
    }

    #[test]
    fn unreachable_api_degrades_to_error() {
        let src = BilibiliSource::with_base("http://127.0.0.1:9");
        let err = src
            .search("晴天", 10, SearchKind::Song)
            .expect_err("接口不可达必须报错，不能 panic");
        assert!(
            err.to_string().contains("buvid3") || err.to_string().contains("请求失败"),
            "错误应说明原因: {err}"
        );
    }

    /// 真联网冒烟（默认 ignore）：`cargo test --release bilibili_live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bilibili_live_search_and_stream() {
        let src = BilibiliSource::new();
        let got = src
            .search("周杰伦 晴天", 5, SearchKind::Song)
            .expect("bilibili 搜索应成功");
        assert!(!got.songs.is_empty());
        for s in got.songs.iter().take(3) {
            eprintln!("{} | {} | {} | {}s", s.source, s.name, s.s_id, s.dt / 1000);
        }
        let url = src
            .stream_url(&crate::source::track_of(&got.songs[0]), "exhigh")
            .expect("取流不应报错")
            .expect("应拿到直链");
        eprintln!("bilibili stream: {:.70}…", url);
    }

    #[test]
    fn missing_ytdlp_degrades_to_error() {
        let src =
            BilibiliSource::with_base_and_exe("http://127.0.0.1:9", "yt-dlp-does-not-exist-xy");
        let err = src
            .stream_url(&TrackId::from("BV1d4411N7zD"), "exhigh")
            .expect_err("缺 yt-dlp 取流必须报错");
        assert!(
            err.to_string().contains("无法启动"),
            "错误应说明原因: {err}"
        );
    }
}
