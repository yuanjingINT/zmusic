use crate::models::{Playlist, Song};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Favorites {
    #[serde(default)]
    songs: Vec<Song>,
    #[serde(default)]
    playlists: Vec<Playlist>,
}

impl Favorites {
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Favorites::default(),
        }
    }

    pub fn save(&self, path: &Path) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, s);
        }
    }

    pub fn list(&self) -> &[Song] {
        &self.songs
    }

    pub fn len(&self) -> usize {
        self.songs.len()
    }

    pub fn contains(&self, id: i64) -> bool {
        self.songs.iter().any(|s| s.id == id)
    }

    pub fn add(&mut self, new: &[Song]) -> usize {
        let mut added = 0;
        for s in new {
            if !self.contains(s.id) {
                self.songs.push(s.clone());
                added += 1;
            }
        }
        added
    }

    pub fn remove_ids(&mut self, ids: &HashSet<i64>) -> usize {
        let before = self.songs.len();
        self.songs.retain(|s| !ids.contains(&s.id));
        before - self.songs.len()
    }

    pub fn playlists(&self) -> &[Playlist] {
        &self.playlists
    }

    pub fn playlist_len(&self) -> usize {
        self.playlists.len()
    }

    pub fn contains_playlist(&self, id: i64) -> bool {
        self.playlists.iter().any(|p| p.id == id)
    }

    pub fn add_playlist(&mut self, pl: &Playlist) -> usize {
        if self.contains_playlist(pl.id) {
            0
        } else {
            self.playlists.push(pl.clone());
            1
        }
    }

    pub fn remove_playlists(&mut self, ids: &HashSet<i64>) -> usize {
        let before = self.playlists.len();
        self.playlists.retain(|p| !ids.contains(&p.id));
        before - self.playlists.len()
    }
}
