#![allow(dead_code)]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artist {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Album {
    pub id: i64,
    pub name: String,
    pub pic_url: Option<String>,
}

impl Default for Album {
    fn default() -> Self {
        Album {
            id: 0,
            name: String::new(),
            pic_url: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Song {
    pub id: i64,
    pub name: String,
    pub ar: Vec<Artist>,
    pub al: Album,
    pub dt: i64,
    pub fee: i32,
}

impl Default for Song {
    fn default() -> Self {
        Song {
            id: 0,
            name: String::new(),
            ar: Vec::new(),
            al: Album::default(),
            dt: 0,
            fee: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub play_count: i64,
    pub track_count: i32,
    pub cover_img_url: Option<String>,
}

impl Default for Playlist {
    fn default() -> Self {
        Playlist {
            id: 0,
            name: String::new(),
            play_count: 0,
            track_count: 0,
            cover_img_url: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PlaylistDetail {
    pub id: i64,
    pub name: String,
    pub track_count: i32,
    pub cover_img_url: Option<String>,
}

impl Default for PlaylistDetail {
    fn default() -> Self {
        PlaylistDetail {
            id: 0,
            name: String::new(),
            track_count: 0,
            cover_img_url: None,
        }
    }
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
pub struct ArtistInfo {
    pub id: i64,
    pub name: String,
    pub pic_url: Option<String>,
    pub music_size: i32,
    pub album_size: i32,
}

impl Default for ArtistInfo {
    fn default() -> Self {
        ArtistInfo {
            id: 0,
            name: String::new(),
            pic_url: None,
            music_size: 0,
            album_size: 0,
        }
    }
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
pub struct Toplist {
    pub id: i64,
    pub name: String,
}

impl Default for Toplist {
    fn default() -> Self {
        Toplist {
            id: 0,
            name: String::new(),
        }
    }
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
pub struct SongUrl {
    pub id: i64,
    pub url: Option<String>,
    pub fee: Option<i32>,
    pub free_trial_info: Option<serde_json::Value>,
    pub size: Option<i64>,
}

impl Default for SongUrl {
    fn default() -> Self {
        SongUrl {
            id: 0,
            url: None,
            fee: None,
            free_trial_info: None,
            size: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Lyrics {
    pub lines: Vec<(u64, String)>,
    pub trans: Vec<(u64, String)>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NewsongItem {
    pub id: i64,
    pub name: String,
    pub song: Option<RawSong>,
}

impl Default for NewsongItem {
    fn default() -> Self {
        NewsongItem {
            id: 0,
            name: String::new(),
            song: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RawSong {
    pub id: i64,
    pub name: String,
    pub artists: Vec<Artist>,
    pub album: RawAlbum,
    pub duration: i64,
    pub fee: i32,
}

impl Default for RawSong {
    fn default() -> Self {
        RawSong {
            id: 0,
            name: String::new(),
            artists: Vec::new(),
            album: RawAlbum::default(),
            duration: 0,
            fee: 0,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RawAlbum {
    pub id: i64,
    pub name: String,
    pub pic_url: Option<String>,
}

impl Default for RawAlbum {
    fn default() -> Self {
        RawAlbum {
            id: 0,
            name: String::new(),
            pic_url: None,
        }
    }
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
pub struct RecAlbum {
    pub id: i64,
    pub name: String,
    pub artist: Option<Artist>,
    pub artists: Vec<Artist>,
    pub pic_url: Option<String>,
    pub publish_time: Option<i64>,
}

impl Default for RecAlbum {
    fn default() -> Self {
        RecAlbum {
            id: 0,
            name: String::new(),
            artist: None,
            artists: Vec::new(),
            pic_url: None,
            publish_time: None,
        }
    }
}
