use crate::models::*;
use crate::util;
use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use reqwest::StatusCode;
use std::time::Duration;

#[derive(Clone)]
pub struct ApiClient {
    pub base: String,
    http: Client,
}

impl ApiClient {
    pub fn new(base: String) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(90))
            .build()
            .unwrap();
        ApiClient { base, http }
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

    pub fn search(&self, kw: &str, limit: i32, stype: i32) -> Result<SearchResult> {
        let r: SearchResp = self.get_json(
            "cloudsearch",
            &[
                ("keywords", kw),
                ("limit", &limit.to_string()),
                ("type", &stype.to_string()),
            ],
        )?;
        Ok(r.result.unwrap_or_default())
    }

    pub fn artist_songs(&self, id: i64, limit: i32) -> Result<Vec<Song>> {
        let r: ArtistSongsResp = self.get_json(
            "artist/songs",
            &[("id", &id.to_string()), ("limit", &limit.to_string())],
        )?;
        Ok(r.songs)
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
        Ok(r
            .result
            .unwrap_or_default()
            .into_iter()
            .filter_map(|item| {
                let song = item.song?;
                Some(Song {
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
                })
            })
            .collect())
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
        Ok(all)
    }

    pub fn album(&self, id: i64) -> Result<(String, Vec<Song>)> {
        let r: AlbumResp = self.get_json("album", &[("id", &id.to_string())])?;
        let name = r.album.as_ref().map(|a| a.name.clone()).unwrap_or_else(|| "专辑".to_string());
        Ok((name, r.songs))
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
