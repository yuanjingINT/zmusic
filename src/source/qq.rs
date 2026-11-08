//! QQ 音乐：搜索走 `c.y.qq.com` 的公开 JSON；取流走 `u.y.qq.com` 的 vkey 接口。
//! 会员/无版权曲目 vkey 会返回空 purl，这时返回 Ok(None)，UI 显示无法获取链接。

use crate::models::{Album, Artist, Lyrics, SearchResult, Song};
use crate::source::http;
use crate::source::{Capabilities, SearchKind, Source, TrackId};
use anyhow::Result;
use serde_json::{json, Value};

const SEARCH_BASE: &str = "https://c.y.qq.com";
const VKEY_BASE: &str = "https://u.y.qq.com";

pub struct QqSource {
    search_base: String,
    vkey_base: String,
}

impl QqSource {
    pub fn new() -> Self {
        QqSource {
            search_base: SEARCH_BASE.to_string(),
            vkey_base: VKEY_BASE.to_string(),
        }
    }

    #[cfg(test)]
    pub fn with_base(search_base: &str, vkey_base: &str) -> Self {
        QqSource {
            search_base: search_base.to_string(),
            vkey_base: vkey_base.to_string(),
        }
    }
}

/// 解析 `client_search_cp` 的 `data.song.list`
pub fn parse_search(v: &Value) -> Vec<Song> {
    v.pointer("/data/song/list")
        .and_then(|l| l.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| {
                    let mid = s.get("songmid")?.as_str()?;
                    Some(Song {
                        id: 0,
                        name: s
                            .get("songname")
                            .and_then(|x| x.as_str())
                            .unwrap_or("(无标题)")
                            .to_string(),
                        ar: s
                            .get("singer")
                            .and_then(|x| x.as_array())
                            .map(|a| {
                                a.iter()
                                    .filter_map(|x| x.get("name").and_then(|n| n.as_str()))
                                    .map(|n| Artist {
                                        id: 0,
                                        name: n.to_string(),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                        al: Album {
                            id: 0,
                            name: s
                                .get("albumname")
                                .and_then(|x| x.as_str())
                                .unwrap_or("QQ 音乐 (直连)")
                                .to_string(),
                            pic_url: None,
                        },
                        dt: s.get("interval").and_then(|x| x.as_i64()).unwrap_or(0) * 1000,
                        fee: 0,
                        source: "qq".to_string(),
                        s_id: mid.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 从 vkey 响应里拼出可播地址；会员曲目 purl 为空 -> None
pub fn parse_vkey(v: &Value) -> Option<String> {
    let data = v.pointer("/req_0/data")?;
    let purl = data.pointer("/midurlinfo/0/purl")?.as_str()?.to_string();
    if purl.is_empty() {
        return None;
    }
    let sip = data
        .pointer("/sip/0")
        .and_then(|x| x.as_str())
        .unwrap_or("http://dl.stream.qqmusic.qq.com/");
    Some(format!("{sip}{purl}"))
}

impl Source for QqSource {
    fn id(&self) -> &'static str {
        "qq"
    }

    fn name(&self) -> &'static str {
        "QQ 音乐"
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
            "{}/soso/fcgi-bin/client_search_cp?format=json&p=1&n=1&w=a",
            self.search_base
        ))
        .is_ok()
    }

    fn search(&self, keyword: &str, limit: i32, kind: SearchKind) -> Result<SearchResult> {
        if kind != SearchKind::Song {
            return Ok(SearchResult::default());
        }
        let v = http::get_json(&format!(
            "{}/soso/fcgi-bin/client_search_cp?format=json&p=1&n={}&w={}",
            self.search_base,
            limit.clamp(1, 30),
            http::enc(keyword)
        ))?;
        Ok(SearchResult {
            songs: parse_search(&v),
            ..Default::default()
        })
    }

    fn lyric(&self, _id: &TrackId) -> Result<Lyrics> {
        Ok(Lyrics::default())
    }

    fn stream_url(&self, id: &TrackId, _quality: &str) -> Result<Option<String>> {
        let payload = json!({
            "req_0": {
                "module": "vkey.GetVkeyServer",
                "method": "CgiGetVkey",
                "param": {
                    "guid": "10000",
                    "songmid": [id.as_str()],
                    "songtype": [0],
                    "uin": "0",
                    "loginflag": 1,
                    "platform": "20"
                }
            },
            "comm": {"uin": 0, "format": "json", "ct": 24, "cv": 0}
        });
        let v = http::get_json(&format!(
            "{}/cgi-bin/musicu.fcg?data={}",
            self.vkey_base,
            http::enc(&payload.to_string())
        ))?;
        Ok(parse_vkey(&v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_search_payload() {
        let v = json!({"data":{"song":{"list":[
            {"songmid":"0039MnYb0qxYhV","songname":"晴天","interval":269,
             "singer":[{"name":"周杰伦"}],"albumname":"叶惠美"}
        ]}}});
        let songs = parse_search(&v);
        assert_eq!(songs.len(), 1);
        assert_eq!(songs[0].name, "晴天");
        assert_eq!(songs[0].s_id, "0039MnYb0qxYhV");
        assert_eq!(songs[0].dt, 269_000);
        assert_eq!(songs[0].ar[0].name, "周杰伦");
        assert_eq!(songs[0].source, "qq");
    }

    #[test]
    fn empty_purl_means_not_playable() {
        let v = json!({"req_0":{"data":{"sip":["http://x/"],"midurlinfo":[{"purl":""}]}}});
        assert_eq!(parse_vkey(&v), None);
        let ok = json!({"req_0":{"data":{"sip":["http://x/"],"midurlinfo":[{"purl":"C400abc.m4a?vkey=1"}]}}});
        assert_eq!(parse_vkey(&ok).as_deref(), Some("http://x/C400abc.m4a?vkey=1"));
    }

    #[test]
    fn unreachable_base_degrades_to_error() {
        let s = QqSource::with_base("http://127.0.0.1:9", "http://127.0.0.1:9");
        assert!(!s.is_available());
        assert!(s.search("x", 5, SearchKind::Song).is_err());
    }

    #[test]
    #[ignore]
    fn qq_live_search() {
        let s = QqSource::new();
        let r = s.search("晴天", 3, SearchKind::Song).expect("QQ 搜索失败");
        println!("qq_live_search: {} 条", r.songs.len());
        for song in r.songs.iter().take(3) {
            println!("  - {}  [{} ms]  s_id={}", song.name, song.dt, song.s_id);
        }
    }
}
