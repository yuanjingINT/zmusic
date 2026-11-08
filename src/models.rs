#![allow(dead_code)]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artist {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[derive(Default)]
pub struct Album {
    pub id: i64,
    pub name: String,
    pub pic_url: Option<String>,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[derive(Default)]
pub struct Song {
    pub id: i64,
    pub name: String,
    pub ar: Vec<Artist>,
    pub al: Album,
    pub dt: i64,
    pub fee: i32,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[derive(Default)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub play_count: i64,
    pub track_count: i32,
    pub cover_img_url: Option<String>,
}


#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[derive(Default)]
pub struct PlaylistDetail {
    pub id: i64,
    pub name: String,
    pub track_count: i32,
    pub cover_img_url: Option<String>,
}


#[derive(Debug, Clone, Deserialize)]
pub struct SearchResp {
    pub result: Option<SearchResult>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SearchResult {
    pub songs: Vec<Song>,
    pub playlists: Vec<Playlist>,
    pub albums: Vec<RecAlbum>,
    pub artists: Vec<ArtistInfo>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[derive(Default)]
pub struct ArtistInfo {
    pub id: i64,
    pub name: String,
    pub pic_url: Option<String>,
    pub music_size: i32,
    pub album_size: i32,
}


#[derive(Debug, Clone, Deserialize)]
pub struct ArtistSongsResp {
    pub songs: Vec<Song>,
    pub code: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToplistResp {
    pub list: Vec<Toplist>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[derive(Default)]
pub struct Toplist {
    pub id: i64,
    pub name: String,
}


#[derive(Debug, Clone, Deserialize)]
pub struct PersonalizedResp {
    pub result: Vec<Playlist>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaylistDetailResp {
    pub playlist: PlaylistDetail,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaylistTrackAllResp {
    pub songs: Vec<Song>,
    pub code: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AlbumResp {
    pub album: Option<Album>,
    pub songs: Vec<Song>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LyricResp {
    pub lrc: Option<LrcWrap>,
    pub tlyric: Option<LrcWrap>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LrcWrap {
    pub lyric: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SongUrlResp {
    pub data: Vec<SongUrl>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[derive(Default)]
pub struct SongUrl {
    pub id: i64,
    pub url: Option<String>,
    pub fee: Option<i32>,
    pub free_trial_info: Option<serde_json::Value>,
    pub size: Option<i64>,
}


#[derive(Debug, Clone)]
pub struct Lyrics {
    pub lines: Vec<(u64, String)>,
    pub trans: Vec<(u64, String)>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[derive(Default)]
pub struct NewsongItem {
    pub id: i64,
    pub name: String,
    pub song: Option<RawSong>,
}


#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[derive(Default)]
pub struct RawSong {
    pub id: i64,
    pub name: String,
    pub artists: Vec<Artist>,
    pub album: RawAlbum,
    pub duration: i64,
    pub fee: i32,
}


#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[derive(Default)]
pub struct RawAlbum {
    pub id: i64,
    pub name: String,
    pub pic_url: Option<String>,
}


#[derive(Debug, Clone, Deserialize)]
pub struct NewsongResp {
    pub result: Option<Vec<NewsongItem>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AlbumNewResp {
    pub albums: Vec<RecAlbum>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[derive(Default)]
pub struct RecAlbum {
    pub id: i64,
    pub name: String,
    pub artist: Option<Artist>,
    pub artists: Vec<Artist>,
    pub pic_url: Option<String>,
    pub publish_time: Option<i64>,
}

