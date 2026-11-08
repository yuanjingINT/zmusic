use crate::models::*;
use crate::source::{SearchKind, TrackId};
use anyhow::anyhow;
use crate::util;
use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use reqwest::StatusCode;
use std::time::Duration;

#[derive(Clone)]
pub struct NetEaseSource {
    pub base: String,
    http: Client,
}

impl NetEaseSource {
    pub fn new(base: String) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(90))
            .build()
            .unwrap();
        NetEaseSource { base, http }
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str, params: &[(&str, &str)]) -> Result<T> {
        let url = format!("{}/{}", self.base, path);
        let resp = self
            .http
            .get(&url)
            .query(params)
            .send()
            .with_context(|| format!("请求失败: {}", url))?;
        if resp.status() != StatusCode::OK {
            bail!("HTTP {}", resp.status());
        }
        let body = resp.text()?;
        let preview = body.chars().take(200).collect::<String>();
        serde_json::from_str(&body).with_context(|| format!("解析失败: {}", preview))
    }

    pub fn search(&self, kw: &str, limit: i32, kind: SearchKind) -> Result<SearchResult> {
        // 网易云私有协议：1 单曲 / 100 歌手 / 1000 歌单 / 10 专辑
        let stype = match kind {
            SearchKind::Song => 1,
            SearchKind::Artist => 100,
            SearchKind::Playlist => 1000,
            SearchKind::Album => 10,
        };
        let r: SearchResp = self.get_json(
            "cloudsearch",
            &[
                ("keywords", kw),
                ("limit", &limit.to_string()),
                ("type", &stype.to_string()),
            ],
        )?;
        let mut res = r.result.unwrap_or_default();
        tag(&mut res.songs);
        Ok(res)
    }

    pub fn artist_songs(&self, id: i64, limit: i32) -> Result<Vec<Song>> {
        let r: ArtistSongsResp = self.get_json(
            "artist/songs",
            &[("id", &id.to_string()), ("limit", &limit.to_string())],
        )?;
        let mut songs = r.songs;
        tag(&mut songs);
        Ok(songs)
    }

    pub fn toplist(&self) -> Result<Vec<Toplist>> {
        let r: ToplistResp = self.get_json("toplist", &[])?;
        Ok(r.list)
    }

    pub fn personalized(&self, limit: i32) -> Result<Vec<Playlist>> {
        let r: PersonalizedResp = self.get_json("personalized", &[("limit", &limit.to_string())])?;
        Ok(r.result)
    }

    pub fn personalized_newsong(&self, limit: i32) -> Result<Vec<Song>> {
        let r: NewsongResp = self.get_json(
            "personalized/newsong",
            &[("limit", &limit.to_string())],
        )?;
        let mut songs: Vec<Song> = r
            .result
            .unwrap_or_default()
            .into_iter()
            .filter_map(|item| {
                let song = item.song?;
                let id = song.id;
                Some(Song {
                    source: "netease".to_string(),
                    s_id: id.to_string(),
                    id: song.id,
                    name: song.name,
                    ar: song.artists,
                    al: Album {
                        id: song.album.id,
                        name: song.album.name,
                        pic_url: song.album.pic_url,
                    },
                    dt: song.duration,
                    fee: song.fee,
                    ..Default::default()
                })
            })
            .collect();
        tag(&mut songs);
        Ok(songs)
    }

    pub fn album_new(&self, limit: i32) -> Result<Vec<RecAlbum>> {
        let r: AlbumNewResp = self.get_json("album/new", &[("limit", &limit.to_string())])?;
        Ok(r.albums)
    }

    pub fn playlist_detail(&self, id: i64) -> Result<PlaylistDetail> {
        let r: PlaylistDetailResp = self.get_json("playlist/detail", &[("id", &id.to_string())])?;
        Ok(r.playlist)
    }

    pub fn playlist_track_all(&self, id: i64) -> Result<Vec<Song>> {
        let mut all = Vec::new();
        let mut offset = 0i64;
        loop {
            let r: PlaylistTrackAllResp = self.get_json(
                "playlist/track/all",
                &[
                    ("id", &id.to_string()),
                    ("limit", "500"),
                    ("offset", &offset.to_string()),
                ],
            )?;
            let n = r.songs.len();
            all.extend(r.songs);
            if n < 500 || offset > 10000 {
                break;
            }
            offset += 500;
        }
        tag(&mut all);
        Ok(all)
    }

    pub fn album(&self, id: i64) -> Result<(String, Vec<Song>)> {
        let r: AlbumResp = self.get_json("album", &[("id", &id.to_string())])?;
        let name = r.album.as_ref().map(|a| a.name.clone()).unwrap_or_else(|| "专辑".to_string());
        let mut songs = r.songs;
        tag(&mut songs);
        Ok((name, songs))
    }

    pub fn lyric(&self, id: i64) -> Result<Lyrics> {
        let r: LyricResp = self.get_json("lyric", &[("id", &id.to_string())])?;
        let lines = r
            .lrc
            .as_ref()
            .and_then(|l| l.lyric.clone())
            .map(|t| util::parse_lrc(&t))
            .unwrap_or_default();
        let trans = r
            .tlyric
            .as_ref()
            .and_then(|l| l.lyric.clone())
            .map(|t| util::parse_lrc(&t))
            .unwrap_or_default();
        Ok(Lyrics { lines, trans })
    }

    pub fn song_url_v1(&self, ids: &[i64], level: &str) -> Result<Vec<SongUrl>> {
        let ids = ids
            .iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let r: SongUrlResp = self.get_json("song/url/v1", &[("id", &ids), ("level", level)])?;
        Ok(r.data)
    }
}

/// 网易云对 [`crate::source::Source`] 的实现。
///
/// 这里只做转发：接口语义与拆出来之前完全一致（同一批端点、同样的参数）。
/// 给一批歌打上来源标记：混源队列靠它知道该去哪个源取流
fn tag(songs: &mut [Song]) {
    for s in songs.iter_mut() {
        s.source = "netease".to_string();
        s.s_id = s.id.to_string();
    }
}

/// 网易云的 ID 必须是数字，字符串 ID 是给其它源的
fn numeric_id(id: &TrackId) -> Result<i64> {
    id.as_str()
        .parse::<i64>()
        .map_err(|_| anyhow!("网易云 ID 必须是数字，收到: {}", id))
}

impl crate::source::Source for NetEaseSource {
    fn id(&self) -> &'static str {
        "netease"
    }

    fn name(&self) -> &'static str {
        "网易云音乐"
    }

    fn search(&self, keyword: &str, limit: i32, kind: SearchKind) -> Result<SearchResult> {
        NetEaseSource::search(self, keyword, limit, kind)
    }

    fn lyric(&self, id: &TrackId) -> Result<Lyrics> {
        NetEaseSource::lyric(self, numeric_id(id)?)
    }

    fn stream_url(&self, id: &TrackId, quality: &str) -> Result<Option<String>> {
        Ok(self
            .song_url_v1(&[numeric_id(id)?], quality)?
            .into_iter()
            .next()
            .and_then(|u| u.url))
    }
}
