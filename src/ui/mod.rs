pub mod screens;
mod theme;

use crate::api::ApiClient;
use crate::config::Config;
use crate::downloader::Downloader;
use crate::favorites::Favorites;
use crate::models::{ArtistInfo, Lyrics, Playlist, RecAlbum, SearchResult, Song};
use crate::player::Player;
use crate::settings::Settings;
use crate::superdl::{self, UsbDevice};
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;
use std::collections::HashSet;
use std::io::Stdout;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

pub const MENU_ITEMS: [&str; 8] = [
    "搜索音乐",
    "歌曲推荐",
    "专辑推荐",
    "歌单推荐",
    "我的下载",
    "我的收藏",
    "超级下载",
    "设置",
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ScreenKind {
    Menu,
    Search,
    Songs,
    Albums,
    Playlists,
    Library,
    NowPlaying,
    Downloads,
    Favorites,
    SuperDl,
    Settings,
}

impl ScreenKind {
    fn name(&self) -> &'static str {
        match self {
            ScreenKind::Menu => "主菜单",
            ScreenKind::Search => "搜索音乐",
            ScreenKind::Songs => "歌曲推荐",
            ScreenKind::Albums => "专辑推荐",
            ScreenKind::Playlists => "歌单推荐",
            ScreenKind::Library => "曲目列表",
            ScreenKind::NowPlaying => "正在播放",
            ScreenKind::Downloads => "我的下载",
            ScreenKind::Favorites => "我的收藏",
            ScreenKind::SuperDl => "超级下载",
            ScreenKind::Settings => "设置",
        }
    }
}

pub(crate) enum AppMsg {
    Playlists(Result<Vec<Playlist>, String>),
    Songs(Result<Vec<Song>, String>),
    Albums(Result<Vec<RecAlbum>, String>),
    Search(Result<SearchResult, String>),
    PlaylistDetail(Result<(String, Vec<Song>), String>),
    AlbumDetail(Result<(String, Vec<Song>), String>),
    ArtistDetail(Result<(String, Vec<Song>), String>),
    SuperDevices(Result<Vec<UsbDevice>, String>),
    SettingsUsb(Result<Vec<UsbDevice>, String>),
    SuperProgress(String),
    SuperDone(Result<String, String>),
    RunSudo {
        cmd: String,
        resp: Sender<Result<(), String>>,
    },
    Enqueue(Result<Vec<Song>, String>),
    PlayUrl(u64, Option<String>),
    Lyric(u64, Option<Lyrics>),
}

fn spawn<F: FnOnce() -> AppMsg + Send + 'static>(tx: Sender<AppMsg>, f: F) {
    thread::spawn(move || {
        let _ = tx.send(f());
    });
}

pub struct App {
    cfg: Config,
    api: ApiClient,
    player: Player,
    downloader: Downloader,
    favorites: Favorites,
    tx: Sender<AppMsg>,
    rx: Receiver<AppMsg>,

    screen: ScreenKind,
    back: Option<ScreenKind>,
    menu_sel: usize,

    selected: HashSet<i64>,

    playlists: Vec<Playlist>,
    playlists_loading: bool,
    playlists_sel: usize,

    songs: Vec<Song>,
    songs_loading: bool,
    songs_sel: usize,

    albums: Vec<RecAlbum>,
    albums_loading: bool,
    albums_sel: usize,

    search_input: String,
    search_input_focus: bool,
    search_type: usize,
    search: Vec<Song>,
    search_playlists: Vec<Playlist>,
    search_albums: Vec<RecAlbum>,
    search_artists: Vec<ArtistInfo>,
    search_loading: bool,
    search_sel: usize,

    lib_title: String,
    lib: Vec<Song>,
    lib_loading: bool,
    lib_sel: usize,

    fav_sel: usize,
    fav_view: usize,

    ctx_open: bool,
    ctx_kind: u8,
    ctx_song: Option<Song>,
    ctx_playlist: Option<Playlist>,
    ctx_sel: usize,
    ctx_x: u16,
    ctx_y: u16,

    super_devices: Vec<UsbDevice>,
    super_loading: bool,
    super_sel: usize,
    super_running: bool,
    super_lines: Vec<String>,

    settings: Settings,
    settings_sel: usize,
    settings_usb: Vec<UsbDevice>,
    settings_usb_loading: bool,

    queue: Vec<Song>,
    queue_idx: Option<usize>,
    now_playing: Option<Song>,
    lyric: Option<Lyrics>,
    play_gen: u64,

    prev_eof: bool,
    toast: Option<(String, Instant)>,
    quit: bool,
    started: Instant,
}

impl App {
    pub fn new(cfg: Config, api: ApiClient, player: Player, downloader: Downloader) -> Self {
        let favorites = Favorites::load(&cfg.favorites_path);
        let settings = Settings::load(&cfg.settings_path);
        let (tx, rx) = channel();
        let mut app = App {
            cfg,
            api,
            player,
            downloader,
            favorites,
            tx,
            rx,
            screen: ScreenKind::Menu,
            back: None,
            menu_sel: 0,
            selected: HashSet::new(),
            playlists: Vec::new(),
            playlists_loading: false,
            playlists_sel: 0,
            songs: Vec::new(),
            songs_loading: false,
            songs_sel: 0,
            albums: Vec::new(),
            albums_loading: false,
            albums_sel: 0,
            search_input: String::new(),
            search_input_focus: false,
            search_type: 0,
            search: Vec::new(),
            search_playlists: Vec::new(),
            search_albums: Vec::new(),
            search_artists: Vec::new(),
            search_loading: false,
            search_sel: 0,
            lib_title: String::new(),
            lib: Vec::new(),
            lib_loading: false,
            lib_sel: 0,
            fav_sel: 0,
            fav_view: 0,
            ctx_open: false,
            ctx_kind: 0,
            ctx_song: None,
            ctx_playlist: None,
            ctx_sel: 0,
            ctx_x: 0,
            ctx_y: 0,
            super_devices: Vec::new(),
            super_loading: false,
            super_sel: 0,
            super_running: false,
            super_lines: Vec::new(),
            settings: settings,
            settings_sel: 0,
            settings_usb: Vec::new(),
            settings_usb_loading: false,
            queue: Vec::new(),
            queue_idx: None,
            now_playing: None,
            lyric: None,
            play_gen: 0,
            prev_eof: false,
            toast: None,
            quit: false,
            started: Instant::now(),
        };
        app.apply_download_dir();
        app
    }

    fn apply_download_dir(&mut self) {
        if self.settings.is_usb() && !self.settings.usb_mount.is_empty() {
            let p = std::path::PathBuf::from(&self.settings.usb_mount);
            if p.is_dir() {
                self.downloader.set_dir(p);
                return;
            }
        }
        self.downloader.set_dir(self.cfg.download_dir.clone());
    }

    pub fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> anyhow::Result<()> {
        loop {
            terminal.draw(|f| screens::render_all(f, self))?;
            if self.quit {
                break;
            }
            while event::poll(Duration::from_millis(60))? {
                match event::read()? {
                    Event::Key(k) => self.handle_key(k),
                    Event::Mouse(m) => self.handle_mouse(m),
                    _ => {}
                }
            }
            self.handle_msgs();
            self.tick();
        }
        Ok(())
    }

    fn load_playlists(&mut self) {
        if self.playlists_loading {
            return;
        }
        self.playlists_loading = true;
        let api = self.api.clone();
        let tx = self.tx.clone();
        spawn(tx, move || match api.personalized(50) {
            Ok(v) => AppMsg::Playlists(Ok(v)),
            Err(e) => AppMsg::Playlists(Err(e.to_string())),
        });
    }

    fn load_songs(&mut self) {
        if self.songs_loading {
            return;
        }
        self.songs_loading = true;
        let api = self.api.clone();
        let tx = self.tx.clone();
        spawn(tx, move || match api.personalized_newsong(30) {
            Ok(v) => AppMsg::Songs(Ok(v)),
            Err(e) => AppMsg::Songs(Err(e.to_string())),
        });
    }

    fn load_albums(&mut self) {
        if self.albums_loading {
            return;
        }
        self.albums_loading = true;
        let api = self.api.clone();
        let tx = self.tx.clone();
        spawn(tx, move || match api.album_new(30) {
            Ok(v) => AppMsg::Albums(Ok(v)),
            Err(e) => AppMsg::Albums(Err(e.to_string())),
        });
    }

    fn load_super_devices(&mut self) {
        if self.super_loading {
            return;
        }
        self.super_loading = true;
        let tx = self.tx.clone();
        spawn(tx, move || match superdl::detect_usb() {
            Ok(v) => AppMsg::SuperDevices(Ok(v)),
            Err(e) => AppMsg::SuperDevices(Err(e.to_string())),
        });
    }

    fn run_sudo(&mut self, cmd: &str) -> Result<(), String> {
        use crossterm::cursor::{Hide, Show};
        use crossterm::execute;
        use crossterm::terminal::{
            disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
        };
        use std::io::Write;
        let mut out = std::io::stdout();
        let _ = execute!(out, LeaveAlternateScreen, Show);
        let _ = disable_raw_mode();
        let _ = out.flush();
        let _ = writeln!(out, "\n[zmusic] 正在执行: {}", cmd);
        let _ = out.flush();
        let status = std::process::Command::new("sh").args(["-c", cmd]).status();
        let _ = enable_raw_mode();
        let _ = execute!(out, EnterAlternateScreen, Hide);
        match status {
            Ok(s) if s.success() => Ok(()),
            Ok(s) => Err(format!("命令退出码 {:?}", s.code())),
            Err(e) => Err(e.to_string()),
        }
    }

    fn start_super_download(&mut self) {
        if self.super_running {
            return;
        }
        let Some(dev) = self.super_devices.get(self.super_sel).cloned() else {
            return;
        };
        self.super_running = true;
        self.super_lines.clear();
        self.super_lines.push(format!(
            "正在准备: {} ({}) - 注意! 该设备将被格式化, 数据会全部丢失",
            dev.path,
            dev.model
        ));
        let api_base = self.cfg.api_base.clone();
        let quality = self.cfg.quality.clone();
        let tx = self.tx.clone();
        let sudo_tx = self.tx.clone();
        let sudo_run = move |cmd: &str| -> (bool, String) {
            let (rtx, rrx) = std::sync::mpsc::channel();
            if sudo_tx
                .send(AppMsg::RunSudo {
                    cmd: cmd.to_string(),
                    resp: rtx,
                })
                .is_err()
            {
                return (false, "UI 已退出".to_string());
            }
            match rrx.recv() {
                Ok(Ok(())) => (true, String::new()),
                Ok(Err(e)) => (false, e),
                Err(_) => (false, "未收到响应".to_string()),
            }
        };
        thread::spawn(move || {
            superdl::run_super_download(&dev, &api_base, &quality, tx, sudo_run);
        });
    }

    fn load_settings_usb(&mut self) {
        if self.settings_usb_loading {
            return;
        }
        self.settings_usb_loading = true;
        let tx = self.tx.clone();
        spawn(tx, move || match superdl::detect_usb() {
            Ok(v) => AppMsg::SettingsUsb(Ok(v)),
            Err(e) => AppMsg::SettingsUsb(Err(e.to_string())),
        });
    }

    fn switch_to_local(&mut self) {
        self.settings.download_location = "local".to_string();
        self.settings.usb_mount.clear();
        self.settings.save(&self.cfg.settings_path);
        self.apply_download_dir();
        self.toast = Some((
            format!("下载位置: 本地 {}", self.cfg.download_dir.display()),
            Instant::now(),
        ));
    }

    fn select_usb_for_download(&mut self, idx: usize) {
        let Some(dev) = self.settings_usb.get(idx).cloned() else {
            if self.settings_usb.is_empty() && !self.settings_usb_loading {
                self.load_settings_usb();
            }
            self.toast = Some(("未检测到 U盘设备".to_string(), Instant::now()));
            return;
        };
        let target = dev.part.clone().unwrap_or_else(|| dev.path.clone());
        let mount = if let Some(m) = &dev.mount {
            Some(m.clone())
        } else {
            let out = std::process::Command::new("udisksctl")
                .args(["mount", "-b", &target])
                .output();
            match out {
                Ok(o) if o.status.success() => {
                    let s = String::from_utf8_lossy(&o.stdout).into_owned();
                    s.find(" at ").map(|pos| s[pos + 4..].trim().to_string())
                }
                _ => None,
            }
        };
        match mount {
            Some(m) if !m.is_empty() => {
                self.settings.download_location = "usb".to_string();
                self.settings.usb_mount = m.clone();
                self.settings.usb_device = dev.path.clone();
                self.settings.save(&self.cfg.settings_path);
                self.apply_download_dir();
                self.toast = Some((format!("下载位置: U盘 {}", m), Instant::now()));
            }
            _ => {
                self.toast = Some(("无法挂载 U盘 (请检查设备)".to_string(), Instant::now()));
            }
        }
    }

    fn toggle_download_location(&mut self) {
        if self.settings.is_usb() {
            self.switch_to_local();
        } else {
            let idx = self.settings_sel.saturating_sub(1);
            self.select_usb_for_download(idx);
        }
    }

    fn tick(&mut self) {
        let st = self.player.state();
        if st.eof && !self.prev_eof && !st.paused && self.queue_idx.is_some() {
            self.next(1);
        }
        self.prev_eof = st.eof;
        if let Some((_, t)) = &self.toast {
            if t.elapsed() > Duration::from_secs(3) {
                self.toast = None;
            }
        }
    }

    fn handle_msgs(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                AppMsg::Playlists(r) => {
                    self.playlists_loading = false;
                    match r {
                        Ok(v) => self.playlists = v,
                        Err(e) => self.toast = Some((format!("歌单推荐加载失败: {}", e), Instant::now())),
                    }
                }
                AppMsg::Songs(r) => {
                    self.songs_loading = false;
                    match r {
                        Ok(v) => {
                            self.songs = v;
                            self.songs_sel = 0;
                        }
                        Err(e) => self.toast = Some((format!("歌曲推荐加载失败: {}", e), Instant::now())),
                    }
                }
                AppMsg::Albums(r) => {
                    self.albums_loading = false;
                    match r {
                        Ok(v) => {
                            self.albums = v;
                            self.albums_sel = 0;
                        }
                        Err(e) => self.toast = Some((format!("专辑推荐加载失败: {}", e), Instant::now())),
                    }
                }
                AppMsg::Search(r) => {
                    self.search_loading = false;
                    match r {
                        Ok(v) => {
                            self.search = v.songs;
                            self.search_playlists = v.playlists;
                            self.search_albums = v.albums;
                            self.search_artists = v.artists;
                            self.search_sel = 0;
                        }
                        Err(e) => self.toast = Some((format!("搜索失败: {}", e), Instant::now())),
                    }
                }
                AppMsg::PlaylistDetail(r) => {
                    self.lib_loading = false;
                    match r {
                        Ok((title, songs)) => {
                            self.lib_title = title;
                            self.lib = songs;
                            self.lib_sel = 0;
                        }
                        Err(e) => self.toast = Some((format!("歌单加载失败: {}", e), Instant::now())),
                    }
                }
                AppMsg::AlbumDetail(r) => {
                    self.lib_loading = false;
                    match r {
                        Ok((title, songs)) => {
                            self.lib_title = title;
                            self.lib = songs;
                            self.lib_sel = 0;
                        }
                        Err(e) => self.toast = Some((format!("专辑加载失败: {}", e), Instant::now())),
                    }
                }
                AppMsg::ArtistDetail(r) => {
                    self.lib_loading = false;
                    match r {
                        Ok((title, songs)) => {
                            self.lib_title = title;
                            self.lib = songs;
                            self.lib_sel = 0;
                        }
                        Err(e) => self.toast = Some((format!("歌手歌曲加载失败: {}", e), Instant::now())),
                    }
                }
                AppMsg::SuperDevices(r) => {
                    self.super_loading = false;
                    match r {
                        Ok(v) => {
                            self.super_devices = v;
                            self.super_sel = 0;
                        }
                        Err(e) => self.toast = Some((format!("USB 设备检测失败: {}", e), Instant::now())),
                    }
                }
                AppMsg::SettingsUsb(r) => {
                    self.settings_usb_loading = false;
                    match r {
                        Ok(v) => self.settings_usb = v,
                        Err(e) => self.toast = Some((format!("USB 设备检测失败: {}", e), Instant::now())),
                    }
                }
                AppMsg::SuperProgress(s) => {
                    self.super_lines.push(s);
                    if self.super_lines.len() > 200 {
                        self.super_lines.drain(..self.super_lines.len() - 200);
                    }
                }
                AppMsg::SuperDone(r) => {
                    self.super_running = false;
                    self.super_lines.push(match r {
                        Ok(s) => format!("✅ {}", s),
                        Err(e) => format!("❌ {}", e),
                    });
                    let toast = match &self.super_lines[self.super_lines.len() - 1] {
                        s if s.starts_with("❌") => s.clone(),
                        _ => "超级下载完成".to_string(),
                    };
                    self.toast = Some((toast, Instant::now()));
                }
                AppMsg::RunSudo { cmd, resp } => {
                    let r = self.run_sudo(&cmd);
                    let _ = resp.send(r);
                }
                AppMsg::Enqueue(r) => match r {
                    Ok(v) => self.enqueue_songs(v),
                    Err(e) => self.toast = Some((format!("获取歌单曲目失败: {}", e), Instant::now())),
                },
                AppMsg::PlayUrl(gen, url) => {
                    if gen == self.play_gen {
                        match url {
                            Some(u) => self.player.load(&u),
                            None => {
                                let name = self
                                    .now_playing
                                    .as_ref()
                                    .map(|s| s.name.clone())
                                    .unwrap_or_default();
                                self.toast = Some((format!("无法获取播放链接: {}", name), Instant::now()));
                            }
                        }
                    }
                }
                AppMsg::Lyric(gen, l) => {
                    if gen == self.play_gen {
                        self.lyric = l;
                    }
                }
            }
        }
    }

    fn goto(&mut self, kind: ScreenKind) {
        if self.screen == kind {
            return;
        }
        self.back = Some(self.screen);
        self.screen = kind;
        self.selected.clear();
        self.ctx_open = false;
    }

    fn list_len(&self) -> usize {
        match self.screen {
            ScreenKind::Menu => MENU_ITEMS.len(),
            ScreenKind::Playlists => self.playlists.len(),
            ScreenKind::Songs => self.songs.len(),
            ScreenKind::Albums => self.albums.len(),
            ScreenKind::Search => match self.search_type {
                0 => self.search.len(),
                1 => self.search_artists.len(),
                2 => self.search_playlists.len(),
                3 => self.search_albums.len(),
                _ => 0,
            },
            ScreenKind::Library => self.lib.len(),
            ScreenKind::Favorites => {
                if self.fav_view == 1 {
                    self.favorites.playlist_len()
                } else {
                    self.favorites.len()
                }
            }
            ScreenKind::SuperDl => self.super_devices.len(),
            ScreenKind::Settings => 1 + self.settings_usb.len(),
            _ => 0,
        }
    }

    fn list_sel(&self) -> usize {
        match self.screen {
            ScreenKind::Menu => self.menu_sel,
            ScreenKind::Playlists => self.playlists_sel,
            ScreenKind::Songs => self.songs_sel,
            ScreenKind::Albums => self.albums_sel,
            ScreenKind::Search => self.search_sel,
            ScreenKind::Library => self.lib_sel,
            ScreenKind::Favorites => self.fav_sel,
            ScreenKind::SuperDl => self.super_sel,
            ScreenKind::Settings => self.settings_sel,
            _ => 0,
        }
    }

    fn set_list_sel(&mut self, idx: usize) {
        match self.screen {
            ScreenKind::Menu => self.menu_sel = idx,
            ScreenKind::Playlists => self.playlists_sel = idx,
            ScreenKind::Songs => self.songs_sel = idx,
            ScreenKind::Albums => self.albums_sel = idx,
            ScreenKind::Search => self.search_sel = idx,
            ScreenKind::Library => self.lib_sel = idx,
            ScreenKind::Favorites => self.fav_sel = idx,
            ScreenKind::SuperDl => self.super_sel = idx,
            ScreenKind::Settings => self.settings_sel = idx,
            _ => {}
        }
    }

    fn compute_top(sel: usize, len: usize, h: usize) -> usize {
        let h = h.max(1);
        if len == 0 {
            0
        } else if sel >= h {
            (sel - h + 1).min(len.saturating_sub(h))
        } else {
            0
        }
    }

    fn hit_index(&self, row: u16) -> Option<usize> {
        let (_, term_h) = crossterm::terminal::size().ok()?;
        let content_h = term_h.saturating_sub(4) as usize;
        let len = self.list_len();
        if len == 0 {
            return None;
        }
        let item_start: u16 = match self.screen {
            ScreenKind::Menu | ScreenKind::Playlists | ScreenKind::Albums | ScreenKind::SuperDl | ScreenKind::Settings => 2,
            ScreenKind::Songs | ScreenKind::Library | ScreenKind::Favorites => 3,
            ScreenKind::Search => {
                if self.search_type == 0 {
                    7
                } else {
                    6
                }
            }
            _ => return None,
        };
        if row < item_start {
            return None;
        }
        let top = Self::compute_top(self.list_sel(), len, content_h);
        let idx = top + (row - item_start) as usize;
        if idx < len {
            Some(idx)
        } else {
            None
        }
    }

    fn handle_mouse(&mut self, m: MouseEvent) {
        let (col, row) = (m.column, m.row);
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if self.ctx_open {
                    if let Some(r) = self.ctx_rect() {
                        if col >= r.x
                            && col < r.x + r.width
                            && row >= r.y
                            && row < r.y + r.height
                        {
                            let item = (row.saturating_sub(r.y + 1)) as usize;
                            if item < self.ctx_items().len() {
                                self.ctx_sel = item;
                                self.ctx_execute();
                            }
                            return;
                        }
                    }
                    self.ctx_open = false;
                    return;
                }
                if let Some(idx) = self.hit_index(row) {
                    self.set_list_sel(idx);
                }
            }
            MouseEventKind::Down(MouseButton::Right) => self.right_click(col, row),
            MouseEventKind::ScrollDown => self.move_sel(1),
            MouseEventKind::ScrollUp => self.move_sel(-1),
            _ => {}
        }
    }

    fn right_click(&mut self, col: u16, row: u16) {
        let Some(idx) = self.hit_index(row) else { return };
        self.set_list_sel(idx);
        match self.screen {
            ScreenKind::Search => match self.search_type {
                0 => self.open_song_ctx(self.search.get(idx).cloned(), col, row),
                2 => self.open_playlist_ctx(self.search_playlists.get(idx).cloned(), col, row),
                3 => self.open_album_ctx(self.search_albums.get(idx).cloned(), col, row),
                _ => {}
            },
            ScreenKind::Songs => self.open_song_ctx(self.songs.get(idx).cloned(), col, row),
            ScreenKind::Library => self.open_song_ctx(self.lib.get(idx).cloned(), col, row),
            ScreenKind::Favorites => {
                if self.fav_view == 0 {
                    self.open_song_ctx(self.favorites.list().get(idx).cloned(), col, row);
                } else {
                    self.open_playlist_ctx(self.favorites.playlists().get(idx).cloned(), col, row);
                }
            }
            ScreenKind::Playlists => self.open_playlist_ctx(self.playlists.get(idx).cloned(), col, row),
            ScreenKind::Albums => self.open_album_ctx(self.albums.get(idx).cloned(), col, row),
            _ => {}
        }
    }

    fn open_song_ctx(&mut self, song: Option<Song>, x: u16, y: u16) {
        self.ctx_kind = 0;
        self.ctx_song = song;
        self.ctx_playlist = None;
        self.ctx_sel = 0;
        self.ctx_x = x;
        self.ctx_y = y;
        self.ctx_open = self.ctx_song.is_some();
    }

    fn open_playlist_ctx(&mut self, pl: Option<Playlist>, x: u16, y: u16) {
        self.ctx_kind = 1;
        self.ctx_playlist = pl;
        self.ctx_song = None;
        self.ctx_sel = 0;
        self.ctx_x = x;
        self.ctx_y = y;
        self.ctx_open = self.ctx_playlist.is_some();
    }

    fn open_album_ctx(&mut self, al: Option<RecAlbum>, x: u16, y: u16) {
        self.ctx_kind = 2;
        self.ctx_playlist = al.map(|a| Playlist {
            id: a.id,
            name: a.name,
            play_count: 0,
            track_count: 0,
            cover_img_url: a.pic_url,
        });
        self.ctx_song = None;
        self.ctx_sel = 0;
        self.ctx_x = x;
        self.ctx_y = y;
        self.ctx_open = self.ctx_playlist.is_some();
    }

    fn ctx_items(&self) -> Vec<&'static str> {
        match self.ctx_kind {
            0 => vec!["1  下载", "2  收藏"],
            1 => vec!["1  下载歌单", "2  收藏"],
            _ => vec!["1  下载专辑"],
        }
    }

    fn ctx_rect(&self) -> Option<Rect> {
        let (w, h) = crossterm::terminal::size().ok()?;
        let items = self.ctx_items();
        let bw = items.iter().map(|s| s.len() as u16).max().unwrap_or(8) + 4;
        let bh = items.len() as u16 + 2;
        let x = self.ctx_x.min(w.saturating_sub(bw));
        let y = self.ctx_y.min(h.saturating_sub(bh));
        Some(Rect::new(x, y, bw, bh))
    }

    fn ctx_execute(&mut self) {
        let (kind, sel) = (self.ctx_kind, self.ctx_sel);
        let song = self.ctx_song.clone();
        let pl = self.ctx_playlist.clone();
        self.ctx_open = false;
        match (kind, sel) {
            (0, 0) => {
                if let Some(s) = song {
                    self.enqueue_songs(vec![s]);
                }
            }
            (0, 1) => {
                if let Some(s) = song {
                    let name = s.name.clone();
                    let n = self.favorites.add(&[s]);
                    self.favorites.save(&self.cfg.favorites_path);
                    self.toast = Some((
                        if n > 0 {
                            format!("已收藏: {}", name)
                        } else {
                            "已在收藏中".to_string()
                        },
                        Instant::now(),
                    ));
                }
            }
            (1, 0) | (2, 0) => {
                if let Some(p) = pl {
                    self.toast = Some((format!("正在获取: {}...", p.name), Instant::now()));
                    let api = self.api.clone();
                    let tx = self.tx.clone();
                    let id = p.id;
                    spawn(tx, move || match api.playlist_track_all(id) {
                        Ok(v) => AppMsg::Enqueue(Ok(v)),
                        Err(e) => AppMsg::Enqueue(Err(e.to_string())),
                    });
                }
            }
            (1, 1) => {
                if let Some(p) = pl {
                    let n = self.favorites.add_playlist(&p);
                    self.favorites.save(&self.cfg.favorites_path);
                    self.toast = Some((
                        if n > 0 {
                            format!("已收藏歌单: {}", p.name)
                        } else {
                            "歌单已在收藏中".to_string()
                        },
                        Instant::now(),
                    ));
                }
            }
            _ => {}
        }
    }

    fn handle_key(&mut self, k: KeyEvent) {
        if self.started.elapsed() < Duration::from_millis(150) {
            return;
        }
        if self.ctx_open {
            let n = self.ctx_items().len();
            match k.code {
                KeyCode::Esc => self.ctx_open = false,
                KeyCode::Up => self.ctx_sel = (self.ctx_sel + n - 1) % n,
                KeyCode::Down => self.ctx_sel = (self.ctx_sel + 1) % n,
                KeyCode::Char('1') => {
                    self.ctx_sel = 0;
                    self.ctx_execute();
                }
                KeyCode::Char('2') => {
                    self.ctx_sel = 1;
                    self.ctx_execute();
                }
                KeyCode::Enter => self.ctx_execute(),
                _ => {}
            }
            return;
        }
        if self.screen == ScreenKind::Search && k.code == KeyCode::Tab {
            self.search_input_focus = false;
            self.set_search_type((self.search_type + 1) % 4);
            return;
        }
        if self.screen == ScreenKind::Search && self.search_input_focus {
            self.handle_search_input(k);
            return;
        }
        match k.code {
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => self.quit = true,
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Esc => {
                if let Some(back) = self.back.take() {
                    self.screen = back;
                }
            }
            KeyCode::Char('h') => self.goto(ScreenKind::Menu),
            KeyCode::Char('s') => {
                self.goto(ScreenKind::Search);
                self.search_input_focus = true;
            }
            KeyCode::Char('g') => self.goto(ScreenKind::Downloads),
            KeyCode::Char('v') => {
                if self.now_playing.is_some() {
                    self.goto(ScreenKind::NowPlaying);
                }
            }
            KeyCode::Enter => {
                if k.modifiers.contains(KeyModifiers::ALT) {
                    self.toggle_select();
                } else {
                    self.enter();
                }
            }
            KeyCode::Char(' ') => {
                if k.modifiers.contains(KeyModifiers::ALT) {
                    self.toggle_select();
                } else {
                    self.player.toggle();
                }
            }
            KeyCode::Char('d') => self.download_selected(),
            KeyCode::Char('D') => self.batch_download(),
            KeyCode::Char('l') => self.favorite_selected(),
            KeyCode::Char('x') => self.remove_favorite(),
            KeyCode::Char('A') => self.open_album_of_selected(),
            KeyCode::Char('t') => {
                if self.screen == ScreenKind::Favorites {
                    self.fav_view ^= 1;
                    self.fav_sel = 0;
                    self.selected.clear();
                } else if self.screen == ScreenKind::Search {
                    self.set_search_type((self.search_type + 1) % 4);
                }
            }
            KeyCode::Char('0') => self.set_search_type(0),
            KeyCode::Char('1') => self.set_search_type(1),
            KeyCode::Char('2') => self.set_search_type(2),
            KeyCode::Char('3') => self.set_search_type(3),
            KeyCode::Char('n') => self.next(1),
            KeyCode::Char('p') => self.next(-1),
            KeyCode::Right => {
                if self.screen == ScreenKind::Settings {
                    self.toggle_download_location();
                } else {
                    self.player.seek_rel(5.0);
                }
            }
            KeyCode::Left => {
                if self.screen == ScreenKind::Settings {
                    self.toggle_download_location();
                } else {
                    self.player.seek_rel(-5.0);
                }
            }
            KeyCode::Char('[') => self.player.seek_rel(-15.0),
            KeyCode::Char(']') => self.player.seek_rel(15.0),
            KeyCode::Char('+') | KeyCode::Char('=') => self.player.volume(5),
            KeyCode::Char('-') => self.player.volume(-5),
            KeyCode::Up => self.move_sel(-1),
            KeyCode::Down => self.move_sel(1),
            KeyCode::Char('k') => self.move_sel(-1),
            KeyCode::Char('j') => self.move_sel(1),
            KeyCode::PageUp => self.move_sel(-10),
            KeyCode::PageDown => self.move_sel(10),
            _ => {}
        }
    }

    fn handle_search_input(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Char(c) => self.search_input.push(c),
            KeyCode::Backspace => {
                self.search_input.pop();
            }
            KeyCode::Enter => {
                let q = self.search_input.trim().to_string();
                if !q.is_empty() {
                    self.do_search(q);
                }
            }
            KeyCode::Esc => self.search_input_focus = false,
            _ => {}
        }
    }

    fn search_type_api(&self) -> i32 {
        match self.search_type {
            0 => 1,
            1 => 100,
            2 => 1000,
            3 => 10,
            _ => 1,
        }
    }

    fn set_search_type(&mut self, t: usize) {
        if t == self.search_type {
            return;
        }
        self.search_type = t;
        self.search_sel = 0;
        let q = self.search_input.trim().to_string();
        if !q.is_empty() {
            self.do_search(q);
        }
    }

    fn do_search(&mut self, q: String) {
        self.search_input_focus = false;
        self.search_loading = true;
        let stype = self.search_type_api();
        let api = self.api.clone();
        let tx = self.tx.clone();
        spawn(tx, move || match api.search(&q, 60, stype) {
            Ok(v) => AppMsg::Search(Ok(v)),
            Err(e) => AppMsg::Search(Err(e.to_string())),
        });
    }

    fn move_sel(&mut self, delta: i32) {
        let len = self.list_len();
        if len == 0 {
            return;
        }
        let sel = match self.screen {
            ScreenKind::Menu => &mut self.menu_sel,
            ScreenKind::Playlists => &mut self.playlists_sel,
            ScreenKind::Songs => &mut self.songs_sel,
            ScreenKind::Albums => &mut self.albums_sel,
            ScreenKind::Search => &mut self.search_sel,
            ScreenKind::Library => &mut self.lib_sel,
            ScreenKind::Favorites => &mut self.fav_sel,
            ScreenKind::SuperDl => &mut self.super_sel,
            ScreenKind::Settings => &mut self.settings_sel,
            _ => return,
        };
        *sel = (*sel as i32 + delta).rem_euclid(len as i32) as usize;
    }

    fn enter(&mut self) {
        match self.screen {
            ScreenKind::Menu => self.activate_menu(),
            ScreenKind::Search => self.enter_search_result(),
            ScreenKind::Songs => {
                if self.songs_sel < self.songs.len() {
                    self.play_list(self.songs.clone(), self.songs_sel);
                }
            }
            ScreenKind::Albums => {
                if let Some(a) = self.albums.get(self.albums_sel) {
                    self.open_album(a.id);
                }
            }
            ScreenKind::Playlists => {
                if let Some(pl) = self.playlists.get(self.playlists_sel) {
                    self.open_playlist(pl.id);
                }
            }
            ScreenKind::Library => {
                if self.lib_sel < self.lib.len() {
                    self.play_list(self.lib.clone(), self.lib_sel);
                }
            }
            ScreenKind::Favorites => {
                if self.fav_view == 1 {
                    if let Some(p) = self.favorites.playlists().get(self.fav_sel) {
                        self.open_playlist(p.id);
                    }
                } else {
                    let list = self.favorites.list().to_vec();
                    if self.fav_sel < list.len() {
                        self.play_list(list, self.fav_sel);
                    }
                }
            }
            ScreenKind::SuperDl => self.start_super_download(),
            ScreenKind::Settings => {
                if self.settings_sel == 0 {
                    self.toggle_download_location();
                } else {
                    self.select_usb_for_download(self.settings_sel - 1);
                }
            }
            ScreenKind::NowPlaying => self.player.toggle(),
            ScreenKind::Downloads => {}
        }
    }

    fn enter_search_result(&mut self) {
        match self.search_type {
            0 => {
                if self.search_sel < self.search.len() {
                    self.play_list(self.search.clone(), self.search_sel);
                }
            }
            1 => {
                if let Some(a) = self.search_artists.get(self.search_sel) {
                    self.open_artist(a.id, a.name.clone());
                }
            }
            2 => {
                if let Some(p) = self.search_playlists.get(self.search_sel) {
                    self.open_playlist(p.id);
                }
            }
            3 => {
                if let Some(a) = self.search_albums.get(self.search_sel) {
                    self.open_album(a.id);
                }
            }
            _ => {}
        }
    }

    fn open_artist(&mut self, id: i64, name: String) {
        self.goto(ScreenKind::Library);
        self.lib_loading = true;
        let api = self.api.clone();
        let tx = self.tx.clone();
        spawn(tx, move || match api.artist_songs(id, 50) {
            Ok(songs) => AppMsg::ArtistDetail(Ok((name, songs))),
            Err(e) => AppMsg::ArtistDetail(Err(e.to_string())),
        });
    }

    fn activate_menu(&mut self) {
        match self.menu_sel {
            0 => {
                self.goto(ScreenKind::Search);
                self.search_input_focus = true;
            }
            1 => {
                self.goto(ScreenKind::Songs);
                if self.songs.is_empty() {
                    self.load_songs();
                }
            }
            2 => {
                self.goto(ScreenKind::Albums);
                if self.albums.is_empty() {
                    self.load_albums();
                }
            }
            3 => {
                self.goto(ScreenKind::Playlists);
                if self.playlists.is_empty() {
                    self.load_playlists();
                }
            }
            4 => self.goto(ScreenKind::Downloads),
            5 => self.goto(ScreenKind::Favorites),
            6 => {
                self.goto(ScreenKind::SuperDl);
                if self.super_devices.is_empty() {
                    self.load_super_devices();
                }
            }
            7 => {
                self.goto(ScreenKind::Settings);
                if self.settings_usb.is_empty() && !self.settings_usb_loading {
                    self.load_settings_usb();
                }
            }
            _ => {}
        }
    }

    fn open_playlist(&mut self, id: i64) {
        self.goto(ScreenKind::Library);
        self.lib_loading = true;
        let api = self.api.clone();
        let tx = self.tx.clone();
        spawn(tx, move || {
            let res = (|| -> anyhow::Result<(String, Vec<Song>)> {
                let d = api.playlist_detail(id)?;
                let tracks = api.playlist_track_all(id)?;
                Ok((d.name, tracks))
            })();
            match res {
                Ok(v) => AppMsg::PlaylistDetail(Ok(v)),
                Err(e) => AppMsg::PlaylistDetail(Err(e.to_string())),
            }
        });
    }

    fn open_album(&mut self, id: i64) {
        self.goto(ScreenKind::Library);
        self.lib_loading = true;
        let api = self.api.clone();
        let tx = self.tx.clone();
        spawn(tx, move || {
            match api.album(id) {
                Ok((name, songs)) => AppMsg::AlbumDetail(Ok((name, songs))),
                Err(e) => AppMsg::AlbumDetail(Err(e.to_string())),
            }
        });
    }

    fn open_album_of_selected(&mut self) {
        let id = match self.screen {
            ScreenKind::Search => {
                if self.search_type == 0 {
                    self.search.get(self.search_sel).map(|s| s.al.id)
                } else {
                    None
                }
            }
            ScreenKind::Songs => self.songs.get(self.songs_sel).map(|s| s.al.id),
            ScreenKind::Library => self.lib.get(self.lib_sel).map(|s| s.al.id),
            _ => None,
        };
        if let Some(id) = id {
            self.open_album(id);
        }
    }

    fn play_list(&mut self, list: Vec<Song>, idx: usize) {
        if list.is_empty() {
            return;
        }
        self.queue = list;
        self.queue_idx = Some(idx);
        self.play_gen += 1;
        self.play_current();
        self.selected.clear();
        self.screen = ScreenKind::NowPlaying;
    }

    fn play_current(&mut self) {
        let Some(idx) = self.queue_idx else { return };
        let Some(song) = self.queue.get(idx).cloned() else {
            return;
        };
        self.now_playing = Some(song.clone());
        self.lyric = None;
        let gen = self.play_gen;
        let api = self.api.clone();
        let tx = self.tx.clone();
        let quality = self.cfg.quality.clone();
        let id = song.id;
        spawn(tx.clone(), move || {
            let url = api
                .song_url_v1(&[id], &quality)
                .ok()
                .and_then(|v| v.into_iter().next())
                .and_then(|u| u.url);
            AppMsg::PlayUrl(gen, url)
        });
        let api2 = self.api.clone();
        spawn(tx, move || {
            let lyr = api2.lyric(id).ok();
            AppMsg::Lyric(gen, lyr)
        });
    }

    fn next(&mut self, step: i32) {
        let n = self.queue.len();
        if n == 0 {
            return;
        }
        let cur = self.queue_idx.unwrap_or(0);
        let ni = ((cur as i32 + step).rem_euclid(n as i32)) as usize;
        self.queue_idx = Some(ni);
        self.play_gen += 1;
        self.play_current();
    }

    fn current_song(&self) -> Option<Song> {
        match self.screen {
            ScreenKind::Search => {
                if self.search_type == 0 {
                    self.search.get(self.search_sel).cloned()
                } else {
                    None
                }
            }
            ScreenKind::Songs => self.songs.get(self.songs_sel).cloned(),
            ScreenKind::Library => self.lib.get(self.lib_sel).cloned(),
            ScreenKind::Favorites => {
                if self.fav_view == 0 {
                    self.favorites.list().get(self.fav_sel).cloned()
                } else {
                    None
                }
            }
            ScreenKind::NowPlaying => self.now_playing.clone(),
            _ => None,
        }
    }

    fn selected_songs(&self) -> Vec<Song> {
        match self.screen {
            ScreenKind::Search => {
                if self.search_type == 0 {
                    self.search
                        .iter()
                        .filter(|s| self.selected.contains(&s.id))
                        .cloned()
                        .collect()
                } else {
                    Vec::new()
                }
            }
            ScreenKind::Songs => self
                .songs
                .iter()
                .filter(|s| self.selected.contains(&s.id))
                .cloned()
                .collect(),
            ScreenKind::Library => self
                .lib
                .iter()
                .filter(|s| self.selected.contains(&s.id))
                .cloned()
                .collect(),
            ScreenKind::Favorites => {
                if self.fav_view == 0 {
                    self.favorites
                        .list()
                        .iter()
                        .filter(|s| self.selected.contains(&s.id))
                        .cloned()
                        .collect()
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }

    fn toggle_select(&mut self) {
        let id = self.current_song().map(|s| s.id);
        if let Some(id) = id {
            if self.selected.contains(&id) {
                self.selected.remove(&id);
            } else {
                self.selected.insert(id);
            }
            self.move_sel(1);
        }
    }

    fn favorite_selected(&mut self) {
        let songs = self.selected_songs();
        let songs = if songs.is_empty() {
            self.current_song().map(|s| vec![s]).unwrap_or_default()
        } else {
            songs
        };
        if songs.is_empty() {
            return;
        }
        let added = self.favorites.add(&songs);
        self.favorites.save(&self.cfg.favorites_path);
        self.selected.clear();
        let n = songs.len();
        if added > 0 {
            self.toast = Some((format!("已收藏 {} 首", n), Instant::now()));
        } else {
            self.toast = Some((format!("已在收藏中 (共 {} 首)", n), Instant::now()));
        }
    }

    fn remove_favorite(&mut self) {
        if self.screen == ScreenKind::Favorites && self.fav_view == 1 {
            let ids: HashSet<i64> = self
                .favorites
                .playlists()
                .get(self.fav_sel)
                .map(|p| p.id)
                .into_iter()
                .collect();
            if ids.is_empty() {
                return;
            }
            let n = self.favorites.remove_playlists(&ids);
            self.favorites.save(&self.cfg.favorites_path);
            if self.fav_sel >= self.favorites.playlist_len() {
                self.fav_sel = self.favorites.playlist_len().saturating_sub(1);
            }
            if n > 0 {
                self.toast = Some((format!("已取消收藏 {} 个歌单", n), Instant::now()));
            }
            return;
        }
        let ids: HashSet<i64> = if self.selected.is_empty() {
            self.current_song().map(|s| s.id).into_iter().collect()
        } else {
            self.selected.iter().copied().collect()
        };
        if ids.is_empty() {
            return;
        }
        let n = self.favorites.remove_ids(&ids);
        self.favorites.save(&self.cfg.favorites_path);
        self.selected.clear();
        if self.fav_sel >= self.favorites.len() {
            self.fav_sel = self.favorites.len().saturating_sub(1);
        }
        if n > 0 {
            self.toast = Some((format!("已从收藏移除 {} 首", n), Instant::now()));
        }
    }

    fn download_selected(&mut self) {
        let songs = self.selected_songs();
        let songs = if songs.is_empty() {
            self.current_song().map(|s| vec![s]).unwrap_or_default()
        } else {
            songs
        };
        if songs.is_empty() {
            return;
        }
        let n = songs.len();
        let added = self.downloader.enqueue(songs);
        self.selected.clear();
        if added > 0 {
            self.toast = Some((format!("已加入 {} 首下载", added), Instant::now()));
        } else {
            self.toast = Some((format!("{} 首已在下载队列", n), Instant::now()));
        }
    }

    fn enqueue_songs(&mut self, list: Vec<Song>) {
        let n = self.downloader.enqueue(list);
        if n > 0 {
            self.toast = Some((format!("已加入 {} 首下载", n), Instant::now()));
        }
    }

    fn batch_download(&mut self) {
        match self.screen {
            ScreenKind::Search => {
                if self.search_type == 0 {
                    self.enqueue_songs(self.search.clone());
                }
            }
            ScreenKind::Songs => self.enqueue_songs(self.songs.clone()),
            ScreenKind::Library => self.enqueue_songs(self.lib.clone()),
            ScreenKind::Favorites => {
                if self.fav_view == 1 {
                    if let Some(p) = self.favorites.playlists().get(self.fav_sel) {
                        self.toast = Some((format!("正在获取: {}...", p.name), Instant::now()));
                        let api = self.api.clone();
                        let tx = self.tx.clone();
                        let id = p.id;
                        spawn(tx, move || match api.playlist_track_all(id) {
                            Ok(v) => AppMsg::Enqueue(Ok(v)),
                            Err(e) => AppMsg::Enqueue(Err(e.to_string())),
                        });
                    }
                } else {
                    self.enqueue_songs(self.favorites.list().to_vec());
                }
            }
            ScreenKind::Playlists => {
                if let Some(pl) = self.playlists.get(self.playlists_sel) {
                    self.toast = Some((format!("正在获取歌单曲目: {}...", pl.name), Instant::now()));
                    let api = self.api.clone();
                    let tx = self.tx.clone();
                    let id = pl.id;
                    spawn(tx, move || match api.playlist_track_all(id) {
                        Ok(v) => AppMsg::Enqueue(Ok(v)),
                        Err(e) => AppMsg::Enqueue(Err(e.to_string())),
                    });
                }
            }
            ScreenKind::Albums => {
                if let Some(a) = self.albums.get(self.albums_sel) {
                    self.toast = Some((format!("正在获取专辑曲目: {}...", a.name), Instant::now()));
                    let api = self.api.clone();
                    let tx = self.tx.clone();
                    let id = a.id;
                    spawn(tx, move || match api.album(id) {
                        Ok((_, songs)) => AppMsg::Enqueue(Ok(songs)),
                        Err(e) => AppMsg::Enqueue(Err(e.to_string())),
                    });
                }
            }
            _ => {}
        }
    }
}
