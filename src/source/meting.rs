//! Meting 侧车源：一个 `GET /api` 把国内平台（腾讯/酷狗/百度/酷我）覆盖掉。
//! 侧车没起时整组不可用，不影响其它源。

use crate::models::{Album, Artist, Lyrics, SearchResult, Song};
use crate::source::http;
use crate::source::{Capabilities, SearchKind, Source, TrackId};
use anyhow::{Context, Result};
use serde_json::Value;
use std::sync::Arc;

pub struct MetingSource {
    base: String,
    server: &'static str,
    id_str: &'static str,
    label: &'static str,
    /// 平台 cookie（config.toml 的 `[meting] cookie`，或 ZMUSE_METING_COOKIE）
    cookie: Option<String>,
}

impl MetingSource {
    #[cfg(test)]
    pub fn new(base: String, server: &'static str) -> Self {
        Self::with_cookie(base, server, None)
    }

    pub fn with_cookie(base: String, server: &'static str, cookie: Option<String>) -> Self {
        let (id_str, label) = match server {
            "tencent" => ("meting:tencent", "QQ 音乐 (Meting)"),
            "kugou" => ("meting:kugou", "酷狗 (Meting)"),
            "baidu" => ("meting:baidu", "千千/百度 (Meting)"),
            "kuwo" => ("meting:kuwo", "酷我 (Meting)"),
            _ => ("meting:netease", "网易云 (Meting)"),
        };
        MetingSource {
            base,
            server,
            id_str,
            label,
            cookie,
        }
    }
}

/// 解析 Meting 统一格式的搜索数组
pub fn parse_search(v: &Value, source: &'static str) -> Vec<Song> {
    let Some(arr) = v.as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|s| {
            let id = s
                .get("id")?
                .as_str()
                .or_else(|| s.get("url_id")?.as_str())?;
            if id.is_empty() {
                return None;
            }
            let name = s.get("name").and_then(|x| x.as_str()).unwrap_or("");
            if name.is_empty() {
                return None; // Meting 对不支持的平台会返回只有 source 的空对象
            }
            let url_id = s
                .get("url_id")
                .and_then(|x| x.as_str())
                .filter(|x| !x.is_empty())
                .unwrap_or(id);
            Some(Song {
                id: 0,
                name: name.to_string(),
                ar: s
                    .get("artist")
                    .and_then(|a| a.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str())
                            .filter(|x| !x.is_empty())
                            .map(|x| Artist {
                                id: 0,
                                name: x.to_string(),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                al: Album {
                    id: 0,
                    name: s
                        .get("album")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                    pic_url: None,
                },
                dt: 0,
                fee: 0,
                source: source.to_string(),
                s_id: url_id.to_string(),
            })
        })
        .collect()
}

/// Meting 的 url 响应：`{"url": "...", "br": 320}`，空串表示拿不到流
pub fn parse_url(v: &Value) -> Option<String> {
    v.get("url")
        .and_then(|u| u.as_str())
        .filter(|u| !u.is_empty())
        .map(|u| u.to_string())
}

impl MetingSource {
    /// cookie 来自 config.toml / 环境变量，不落盘、不进日志。
    fn cookie(&self) -> Option<String> {
        self.cookie
            .clone()
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty() && c.len() < 8192)
    }

    /// 带 cookie 头发一次 GET；侧车的 cookie 优先取这个头
    fn get(&self, path: &str) -> Result<Value> {
        let url = format!("{}/api?{}", self.base, path);
        let mut req = http::client().get(&url);
        if let Some(c) = self.cookie() {
            req = req.header("x-meting-cookie", c);
        }
        let resp = req.send().with_context(|| format!("请求侧车失败: {url}"))?;
        if !resp.status().is_success() {
            return Err(anyhow::anyhow!("侧车 HTTP {}: {url}", resp.status()));
        }
        resp.json().context("侧车返回的不是 JSON")
    }
}

/// 取单曲时长（秒级 API 返回毫秒）；拿不到回 None
fn duration_of(base: &str, server: &'static str, cookie: Option<&str>, id: &str) -> Option<i64> {
    let url = format!(
        "{}/api?server={}&type=duration&id={}",
        base,
        server,
        http::enc(id)
    );
    let mut req = http::client().get(&url);
    if let Some(c) = cookie {
        req = req.header("x-meting-cookie", c);
    }
    let v: Value = req.send().ok()?.json().ok()?;
    v.get("duration_ms")
        .and_then(|x| x.as_i64())
        .filter(|ms| *ms > 0)
}

impl MetingSource {
    /// 搜索接口不给时长：并发补前 `max` 条，仍拿不到时长的直接丢掉
    /// （没有时长的曲目在列表里没有进度条，点了才知道能不能放）。
    fn fill_durations(&self, songs: Vec<Song>, max: usize) -> Vec<Song> {
        use std::collections::HashMap;
        let songs: Vec<Song> = songs.into_iter().take(max).collect();
        if songs.is_empty() {
            return songs;
        }
        let cookie = self.cookie().map(Arc::new);
        let (tx, rx) = std::sync::mpsc::channel::<(usize, i64)>();
        for (i, song) in songs.iter().enumerate() {
            if song.dt > 0 {
                let _ = tx.send((i, song.dt));
                continue;
            }
            let base = self.base.clone();
            let server = self.server;
            let cookie = cookie.clone();
            let id = song.s_id.clone();
            let tx = tx.clone();
            std::thread::spawn(move || {
                let ms = duration_of(&base, server, cookie.as_ref().map(|c| c.as_str()), &id)
                    .unwrap_or(0);
                let _ = tx.send((i, ms));
            });
        }
        drop(tx);
        let mut got: HashMap<usize, i64> = HashMap::new();
        while let Ok((i, ms)) = rx.recv() {
            got.insert(i, ms);
        }
        songs
            .into_iter()
            .enumerate()
            .filter_map(|(i, mut s)| {
                let dt = got.get(&i).copied().unwrap_or(0);
                if dt <= 0 {
                    return None;
                }
                s.dt = dt;
                Some(s)
            })
            .collect()
    }
}

impl Source for MetingSource {
    fn id(&self) -> &'static str {
        self.id_str
    }

    fn name(&self) -> &'static str {
        self.label
    }

    fn group(&self) -> &'static str {
        "国内"
    }

    fn capabilities(&self) -> Capabilities {
        // 没 cookie 时 tencent/kugou 能搜不能出流：标「仅搜索」，聚合不并进播放队列。
        // baidu/kuwo 上游本身返回空壳，配了 cookie 也当不能播。
        let with_creds = self.cookie().is_some() && matches!(self.server, "tencent" | "kugou");
        Capabilities {
            kinds: vec![SearchKind::Song],
            lyrics: false,
            qualities: Vec::new(),
            can_stream: with_creds,
        }
    }

    fn is_available(&self) -> bool {
        self.get(&format!("server={}&type=search&id=a&limit=1", self.server))
            .is_ok()
    }

    fn search(&self, keyword: &str, limit: i32, kind: SearchKind) -> Result<SearchResult> {
        if kind != SearchKind::Song {
            return Ok(SearchResult::default());
        }
        let v = self.get(&format!(
            "server={}&type=search&id={}&limit={}",
            self.server,
            http::enc(keyword),
            limit.clamp(1, 30)
        ))?;
        // 最多补前 12 条，避免一屏超出部分白打接口
        let songs = self.fill_durations(parse_search(&v, self.id_str), 12);
        Ok(SearchResult {
            songs,
            ..Default::default()
        })
    }

    fn lyric(&self, _id: &TrackId) -> Result<Lyrics> {
        Ok(Lyrics::default())
    }

    fn stream_url(&self, id: &TrackId, _quality: &str) -> Result<Option<String>> {
        let v = self.get(&format!(
            "server={}&type=url&id={}&br=320",
            self.server,
            http::enc(id.as_str())
        ))?;
        Ok(parse_url(&v))
    }

    /// 懒补时长：Meting 搜索不给时长，选中/播放时补一次
    fn duration_ms(&self, id: &TrackId) -> Result<Option<i64>> {
        let v = self.get(&format!(
            "server={}&type=duration&id={}",
            self.server,
            http::enc(id.as_str())
        ))?;
        let ms = v
            .get("duration_ms")
            .and_then(|x| x.as_i64())
            .filter(|ms| *ms > 0);
        Ok(ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_meting_search() {
        let v = json!([
            {"id":"0039MnYb0qxYhV","name":"晴天","artist":["周杰伦"],
             "album":"叶惠美","url_id":"0039MnYb0qxYhV","source":"tencent"},
            {"artist":[],"album":"","source":"baidu"}
        ]);
        let songs = parse_search(&v, "meting:tencent");
        assert_eq!(songs.len(), 1, "空壳结果应被丢掉");
        assert_eq!(songs[0].name, "晴天");
        assert_eq!(songs[0].source, "meting:tencent");
        assert_eq!(songs[0].s_id, "0039MnYb0qxYhV");
    }

    #[test]
    fn empty_url_is_none() {
        assert_eq!(parse_url(&json!({"url":"","br":-1})), None);
        assert_eq!(
            parse_url(&json!({"url":"http://x/a.mp3","br":320})).as_deref(),
            Some("http://x/a.mp3")
        );
    }

    #[test]
    fn sidecar_down_degrades() {
        let s = MetingSource::new("http://127.0.0.1:9".to_string(), "tencent");
        assert!(!s.is_available());
        assert!(s.search("x", 5, SearchKind::Song).is_err());
    }
}
