//! Meting 侧车源：一个 `GET /api` 把国内平台（腾讯/酷狗/百度/酷我）覆盖掉。
//! 侧车没起时整组不可用，不影响其它源。

use crate::models::{Album, Artist, Lyrics, SearchResult, Song};
use crate::source::http;
use crate::source::{Capabilities, SearchKind, Source, TrackId};
use anyhow::Result;
use serde_json::Value;

pub struct MetingSource {
    base: String,
    server: &'static str,
    id_str: &'static str,
    label: &'static str,
}

impl MetingSource {
    pub fn new(base: String, server: &'static str) -> Self {
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
        Capabilities {
            kinds: vec![SearchKind::Song],
            lyrics: false,
            qualities: Vec::new(),
        }
    }

    fn is_available(&self) -> bool {
        http::get_json(&format!(
            "{}/api?server={}&type=search&id=a&limit=1",
            self.base, self.server
        ))
        .is_ok()
    }

    fn search(&self, keyword: &str, limit: i32, kind: SearchKind) -> Result<SearchResult> {
        if kind != SearchKind::Song {
            return Ok(SearchResult::default());
        }
        let v = http::get_json(&format!(
            "{}/api?server={}&type=search&id={}&limit={}",
            self.base,
            self.server,
            http::enc(keyword),
            limit.clamp(1, 30)
        ))?;
        Ok(SearchResult {
            songs: parse_search(&v, self.id_str),
            ..Default::default()
        })
    }

    fn lyric(&self, _id: &TrackId) -> Result<Lyrics> {
        Ok(Lyrics::default())
    }

    fn stream_url(&self, id: &TrackId, _quality: &str) -> Result<Option<String>> {
        let v = http::get_json(&format!(
            "{}/api?server={}&type=url&id={}&br=320",
            self.base,
            self.server,
            http::enc(id.as_str())
        ))?;
        Ok(parse_url(&v))
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
