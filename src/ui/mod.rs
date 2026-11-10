pub mod anim;
pub mod eq;
mod hints;
mod nav;
mod playback;
pub mod screens;
pub mod spectrum;
mod search_keys;
mod status;
mod theme;

use crate::config::Config;
use crate::downloader::Downloader;
use crate::favorites::Favorites;
use crate::models::{ArtistInfo, Lyrics, Playlist, RecAlbum, SearchResult, Song};
use crate::player::Player;
use crate::settings::Settings;
use crate::source::health::{self, SourceState};
use crate::source::netease::NetEaseSource;
use crate::source::SourceRegistry;
use crate::superdl::{self, UsbDevice};
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use nav::NavStack;
use playback::PlayMode;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;
use std::collections::HashSet;
use std::io::Stdout;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

/// 聚合伪源：搜索时并发问所有 `Ok` 的源再合并
pub const AGGREGATE_ID: &str = "aggregate";

/// 源分组名按界面语言显示（`国内` / `国际` / `公开免费`）
pub fn group_display(group: &str) -> &'static str {
    match (group, crate::i18n::en()) {
        ("国内", false) => "国内",
        ("国内", true) => "domestic",
        ("国际", false) => "国际",
        ("国际", true) => "international",
        ("公开免费", false) => "公开免费",
        ("公开免费", true) => "public/free",
        (_, false) => "其它",
        (_, true) => "other",
    }
}

/// 打不开详情时的提示文案（纯函数，方便单测）。
///
/// 单源网易云是**能**打开详情的情况，永远不该走到这里；只有聚合里没有网易云条目、
/// 或者当前源压根没有详情接口（咪咕/QQ 等）才会用这条文案。
pub fn detail_unsupported_msg(source_id: &str, what: &str) -> String {
    // what 是「专辑 / 歌单 / 歌手」，按语言取词，避免英文句子里嵌中文
    let what = match (what, crate::i18n::en()) {
        ("专辑", false) => "专辑",
        ("专辑", true) => "album",
        ("歌单", false) => "歌单",
        ("歌单", true) => "playlist",
        ("歌手", false) => "歌手",
        ("歌手", true) => "artist",
        (w, _) => w,
    };
    if source_id == AGGREGATE_ID {
        crate::tr!(
            "这个{}没有网易云的条目，打不开详情（按 S 换源试试）",
            "This {} has no NetEase entry, so its details cannot be opened (press S to switch source)",
            what
        )
    } else {
        crate::tr!(
            "{}源暂不支持{}详情，按 S 换到网易云或聚合再打开",
            "{} does not support {} details yet; press S to switch to NetEase or aggregate",
            crate::source::source_display(source_id),
            what
        )
    }
}

/// 主菜单；编号从 0 开始（数字键 0–6 直达）。文案随界面语言变化。
pub fn menu_items() -> [&'static str; 7] {
    [
        crate::i18n::t("menu.search"),
        crate::i18n::t("menu.songs"),
        crate::i18n::t("menu.albums"),
        crate::i18n::t("menu.playlists"),
        crate::i18n::t("menu.downloads"),
        crate::i18n::t("menu.favorites"),
        crate::i18n::t("menu.superdl"),
    ]
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
            ScreenKind::Menu => crate::i18n::t("menu.title"),
            ScreenKind::Search => crate::i18n::t("title.search"),
            ScreenKind::Songs => crate::i18n::t("title.songs"),
            ScreenKind::Albums => crate::i18n::t("title.albums"),
            ScreenKind::Playlists => crate::i18n::t("title.playlists"),
            ScreenKind::Library => crate::i18n::t("title.library"),
            ScreenKind::NowPlaying => crate::i18n::t("title.nowplaying"),
            ScreenKind::Downloads => crate::i18n::t("title.downloads"),
            ScreenKind::Favorites => crate::i18n::t("title.favorites"),
            ScreenKind::SuperDl => crate::i18n::t("title.superdl"),
            ScreenKind::Settings => crate::i18n::t("title.settings"),
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
    /// gen, url, autoplay, mpv_proxy, 实际成功的源, 各源失败原因
    PlayUrl(
        u64,
        Option<String>,
        bool,
        Option<String>,
        Option<(String, String)>,
        Vec<(String, String)>,
    ),
    Lyric(u64, Option<Lyrics>),
    /// 后台探测单个源的结果
    Probe {
        gen: u64,
        id: String,
        state: SourceState,
    },
    ProbeDone {
        gen: u64,
    },
    /// 聚合搜索：某个源的一批结果（按到达顺序合并）
    AggPart {
        gen: u64,
        id: String,
        result: Result<SearchResult, String>,
    },
    AggDone {
        gen: u64,
        failed: Vec<String>,
    },
    /// 懒补时长回来了
    Duration {
        source: String,
        s_id: String,
        dt: i64,
    },
}

fn spawn<F: FnOnce() -> AppMsg + Send + 'static>(tx: Sender<AppMsg>, f: F) {
    thread::spawn(move || {
        let _ = tx.send(f());
    });
}

pub struct App {
    cfg: Config,
    api: NetEaseSource,
    player: Player,
    downloader: Downloader,
    /// 与下载器共享的音质，弹窗里改完两边同时生效
    quality: Arc<RwLock<String>>,
    /// 音乐源注册表 + 当前搜索用的源
    registry: Arc<SourceRegistry>,
    source_id: String,
    /// 搜索失败过的源（UI 置灰用），成功一次就清掉
    failed_sources: std::collections::HashSet<String>,
    /// 源健康状态：后台探测写，渲染只读，绝不在这里发网络请求
    health: health::HealthTable,
    probe_gen: u64,
    probe_total: usize,
    probe_done: usize,
    probe_started: Option<Instant>,
    /// 聚合去重：key → 结果下标（歌曲用，代表源按优先级替换）
    agg_index: std::collections::HashMap<String, usize>,
    /// 聚合去重：所有类型共用（key 带类型前缀 a|/p|l|/s|）
    agg_seen: std::collections::HashSet<String>,
    agg_failed: Vec<String>,
    /// 搜索代数：切源/新搜索后旧结果直接丢
    search_gen: u64,
    /// 源列表是否显示离线的（S 循环范围）
    show_all_sources: bool,
    /// 搜索页当前看的是源状态明细（V 切换），还是搜索结果
    show_source_list: bool,
    source_sel: usize,
    /// 已经补过时长的曲目，避免重复请求
    duration_asked: std::collections::HashSet<String>,
    /// 聚合去重后，每一行被哪些源命中（来源列显示「网易云+QQ(直连)」）
    agg_sources: std::collections::HashMap<String, Vec<String>>,
    /// 同一首歌在各源上的完整条目：主源取不到流时按优先级换源重试
    agg_candidates: std::collections::HashMap<String, Vec<Song>>,
    /// 歌手 / 专辑 / 歌单 的候选：key → [(source, 对方 id)]，Enter 时按优先级试
    agg_artist_candidates: std::collections::HashMap<String, Vec<(String, i64)>>,
    agg_album_candidates: std::collections::HashMap<String, Vec<(String, i64)>>,
    agg_playlist_candidates: std::collections::HashMap<String, Vec<(String, i64)>>,
    /// 取不到流的曲目（本次会话跳过，随机模式不再撞）
    bad_songs: std::collections::HashSet<String>,
    /// 连续跳过次数，超过 5 就停
    skip_streak: u32,
    /// 已经处理过的 mpv 失败序号（未播放过是 0）
    last_mpv_error: u64,
    /// 动效开关（[ui] 段）
    anim: crate::config::Animations,
    /// 切歌淡入的起点
    fade_start: Option<Instant>,
    /// 真频谱解码器（只在正在播放页 + 播放时存在）
    eq_analyzer: Option<crate::ui::eq::PcmAnalyzer>,
    /// 解码失败次数 / 是否已降级到正弦兜底
    eq_fail: u32,
    eq_fallback: bool,
    /// 对称波形的滚动缓冲（暂停时不再推送，画面自然静止）
    wave: crate::ui::spectrum::WaveRing,
    /// 跳转后等播放位置稳定再重开解码器（否则会拿过期的位置去解码）
    eq_hold: Option<Instant>,
    /// 上一拍的播放位置（拖动判定用）
    eq_last_pos: Option<f64>,
    /// 这首歌的整曲自动增益（dB，给界面显示「自动 +X dB」）；None = 首次播放
    eq_auto_gain: Option<f32>,
    /// 上面那个值是按哪首歌算的（换歌才重算）
    eq_auto_key: Option<String>,
    /// 当前这首歌里解码器是第几次启动（dump 的段号，换歌归 1）
    eq_seg: usize,
    /// 哪一代播放已经「唱完」了：这一代内不再重建解码器（防止歌尾重拉整首）
    eq_done_gen: Option<u64>,
    /// 逐曲响度缓存：歌曲 key → p90 dB（存 ~/.cache/zmuse/eq_gain.json）
    eq_cache: std::collections::HashMap<String, f32>,
    /// 缓存是否已从磁盘读过
    eq_cache_loaded: bool,
    /// 当前播放的流地址（频谱解码要重拉一次）
    now_url: Option<String>,
    favorites: Favorites,
    tx: Sender<AppMsg>,
    rx: Receiver<AppMsg>,

    screen: ScreenKind,
    nav: NavStack,
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

    song_cfg_open: bool,
    song_cfg_sel: usize,
    play_mode: PlayMode,
    rng: u64,
    pending_seek: Option<f64>,

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
    pub fn new(
        cfg: Config,
        api: NetEaseSource,
        player: Player,
        downloader: Downloader,
        quality: Arc<RwLock<String>>,
        registry: Arc<SourceRegistry>,
    ) -> Self {
        let favorites = Favorites::load(&cfg.favorites_path);
        let settings = Settings::load(&cfg.settings_path);
        let mut cfg = cfg;
        if !settings.quality.is_empty() {
            cfg.quality = settings.quality.clone();
        }
        let play_mode = PlayMode::parse(&settings.play_mode);
        let anim = cfg.animations;
        // settings 里的音质可能覆盖了环境变量，共享给下载器
        *quality.write().unwrap() = cfg.quality.clone();
        let (tx, rx) = channel();
        // 冷启动预热：网易云 API 的首次搜索要 2s 左右（上游），提前打一次，
        // 用户输入关键词时就不用等这一下。结果丢弃，不影响 UI。
        {
            let api = NetEaseSource::clone(&api);
            let tx = tx.clone();
            thread::spawn(move || {
                // ncm-api 可能刚被启动器拉起来，给几次重试（后台，不挡首帧）
                for _ in 0..4 {
                    if api.search("a", 1, crate::source::SearchKind::Song).is_ok() {
                        break;
                    }
                    thread::sleep(Duration::from_millis(800));
                }
                drop(tx);
            });
        }
        let mut app = App {
            cfg,
            api,
            player,
            downloader,
            quality,
            failed_sources: std::collections::HashSet::new(),
            health: health::HealthTable::new(health::DEFAULT_TTL),
            probe_gen: 0,
            probe_total: 0,
            probe_done: 0,
            probe_started: None,
            agg_index: std::collections::HashMap::new(),
            agg_seen: std::collections::HashSet::new(),
            agg_failed: Vec::new(),
            search_gen: 0,
            show_all_sources: false,
            show_source_list: false,
            source_sel: 0,
            duration_asked: std::collections::HashSet::new(),
            agg_sources: std::collections::HashMap::new(),
            agg_candidates: std::collections::HashMap::new(),
            agg_artist_candidates: std::collections::HashMap::new(),
            agg_album_candidates: std::collections::HashMap::new(),
            agg_playlist_candidates: std::collections::HashMap::new(),
            bad_songs: std::collections::HashSet::new(),
            skip_streak: 0,
            last_mpv_error: 0,
            anim,
            fade_start: None,
            eq_analyzer: None,
            wave: crate::ui::spectrum::WaveRing::new(),
            eq_hold: None,
            eq_last_pos: None,
            eq_auto_gain: None,
            eq_auto_key: None,
            eq_seg: 0,
            eq_done_gen: None,
            eq_cache: std::collections::HashMap::new(),
            eq_cache_loaded: false,
            eq_fail: 0,
            eq_fallback: false,
            now_url: None,
            source_id: if !settings.source.is_empty()
                && (settings.source == AGGREGATE_ID || registry.get(&settings.source).is_some())
            {
                settings.source.clone()
            } else {
                "netease".to_string()
            },
            registry,
            favorites,
            tx,
            rx,
            screen: ScreenKind::Menu,
            nav: NavStack::default(),
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
            song_cfg_open: false,
            song_cfg_sel: 0,
            play_mode,
            rng: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x9E3779B97F4A7C15),
            pending_seek: None,

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
            settings,
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
        // 启动即后台探源：这里只起线程，不等待，界面照常进出
        self.start_probe();
        // 首帧之前积压的输入（多半是拉起程序的回车/误触）在开屏时一次性丢掉，
        // 之后不再按时间窗口吞键——否则「启动后立刻按 s」会没反应。
        while event::poll(Duration::ZERO)? {
            let _ = event::read()?;
        }
        let _ = self.started;
        loop {
            terminal.draw(|f| screens::render_all(f, self))?;
            if self.quit {
                break;
            }
            // 有任务在飞（搜索/加载/取歌单）时 16ms 一拍，异步结果最多等 16ms 就上屏；
            // 空闲或纯播放时回到 60ms 一拍，别为了这点延迟让空闲 CPU 翻三倍。
            // 「真在播放」= 有歌、mpv 没空闲、也没暂停；暂停后动画立刻停。
            let st = self.player.state();
                let playing = self.now_playing.is_some() && !st.idle && !st.paused;
            // 只有「正在播放页 + 真在播放 + 均衡器开着」才按均衡器帧率重绘；
            // 切页/暂停立刻回到空闲节奏（60ms），不因为均衡器让全程序按 15fps 跑。
            let tick_ms = if self.is_loading() {
                16
            } else if self.screen == ScreenKind::NowPlaying && playing && self.anim.equalizer_on() {
                crate::ui::anim::interval_ms(self.anim.equalizer_fps)
            } else {
                60
            };
            while event::poll(Duration::from_millis(tick_ms))? {
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
        let _ = writeln!(out, "\n[zmuse] 正在执行: {}", cmd);
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
        self.super_lines.push(crate::tr!(
            "正在准备: {} ({}) - 注意! 该设备将被格式化, 数据会全部丢失",
            "Preparing: {} ({}) - warning: this device will be formatted, all data will be lost",
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
                return (false, crate::tr!("UI 已退出", "UI has exited"));
            }
            match rrx.recv() {
                Ok(Ok(())) => (true, String::new()),
                Ok(Err(e)) => (false, e),
                Err(_) => (false, crate::tr!("未收到响应", "no response received")),
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
            crate::tr!(
                "下载位置: 本地 {}",
                "Download location: this computer {}",
                self.cfg.download_dir.display()
            ),
            Instant::now(),
        ));
    }

    fn select_usb_for_download(&mut self, idx: usize) {
        let Some(dev) = self.settings_usb.get(idx).cloned() else {
            if self.settings_usb.is_empty() && !self.settings_usb_loading {
                self.load_settings_usb();
            }
            self.toast = Some((crate::tr!("未检测到 U盘设备", "No USB drive detected"), Instant::now()));
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
                self.toast = Some((crate::tr!("下载位置: U盘 {}", "Download location: USB drive {}", m), Instant::now()));
            }
            _ => {
                self.toast = Some((crate::tr!("无法挂载 U盘 (请检查设备)", "Could not mount the USB drive (check the device)"), Instant::now()));
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

    /// 是否有后台任务在等结果：决定事件循环的轮询粒度（16ms vs 60ms）
    pub(crate) fn is_loading(&self) -> bool {
        self.search_loading
            || self.playlists_loading
            || self.songs_loading
            || self.albums_loading
            || self.lib_loading
            || self.super_loading
            || self.settings_usb_loading
    }

    fn tick(&mut self) {
        let st = self.player.state();
        self.sync_autogain();
        self.sync_eq(&st);
        // mpv 加载失败（403/失效链接/解码不了）必须让用户看见，并自动跳到下一首
        if st.error_seq != self.last_mpv_error {
            self.last_mpv_error = st.error_seq;
            if let Some(e) = st.error.clone() {
                self.skip_unplayable(Some(format!("mpv: {e}")));
            }
        }
        // 播完一首：按播放顺序决定下一首；顺序模式播到末尾就停住。
        // prev_eof 用来去重，避免 eof 一直为真时每帧都推进。
        // keep-open 下播完可能同时 paused=true，所以这里不能用 paused 当门禁；
        // 播到中途按暂停时 eof 为假，本来就不会触发。
        if st.eof && !self.prev_eof {
            if let Some(cur) = self.queue_idx {
                self.rng = self
                    .rng
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                if let Some(ni) =
                    playback::next_index(self.queue.len(), cur, self.play_mode, self.rng)
                {
                    self.queue_idx = Some(ni);
                    self.play_gen += 1;
                    self.play_current(true); // 自动连播：必须解暂停
                }
            }
        }
        self.prev_eof = st.eof;
        self.probe_timeout_check();
        if let Some((_, t)) = &self.toast {
            if t.elapsed() > Duration::from_secs(3) {
                self.toast = None;
            }
        }
    }

    /// 真频谱解码器的生命周期：只在「正在播放页 + 真在播放 + pcm 档」时存在，
    /// 切页/暂停/停止立刻 drop（Drop 里 kill 子进程、删 FIFO，不留孤儿）。
    /// 逐曲响度缓存文件：~/.cache/zmuse/eq_gain.json
    fn eq_cache_path() -> Option<std::path::PathBuf> {
        let base = dirs::cache_dir()?;
        let dir = base.join("zmuse");
        // 改名兼容：老的 ~/.cache/zmusic 抄一份过来（响度缓存没了也只是第一遍重估）
        crate::config::migrate_dir(&dir, &base.join(crate::config::LEGACY_DIR_NAME));
        Some(dir.join("eq_gain.json"))
    }

    fn ensure_eq_cache(&mut self) {
        if self.eq_cache_loaded {
            return;
        }
        self.eq_cache_loaded = true;
        let Some(path) = Self::eq_cache_path() else {
            return;
        };
        if let Ok(raw) = std::fs::read_to_string(&path) {
            if let Ok(map) = serde_json::from_str::<std::collections::HashMap<String, f32>>(&raw) {
                self.eq_cache = map;
            }
        }
    }

    fn save_eq_cache(&self) {
        let Some(path) = Self::eq_cache_path() else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(raw) = serde_json::to_string(&self.eq_cache) {
            let _ = std::fs::write(&path, raw);
        }
    }

    /// 当前这首歌的缓存 key：`源|曲目id`
    fn eq_cache_key(&mut self) -> Option<String> {
        self.ensure_eq_cache();
        self.now_playing
            .as_ref()
            .map(|s| format!("{}|{}", s.source, s.s_id))
    }

    /// 逐曲自动增益：整首一个值，歌内绝不改。
    ///
    /// - 缓存里听过（第二遍起）：直接用整曲标定，第一秒就到位、整首恒定
    /// - 首次播放：用固定增益（eq_gain 那套映射）——稳定但未必最优；
    ///   解码流顺手攒窗峰值，播到 80% 时算出整曲 p90 写进缓存，下一遍就准了
    ///
    /// 全程零额外下载：就是用波形那条本来就在跑的 1x 解码流。
    fn sync_autogain(&mut self) {
        if !self.anim.eq_autogain {
            self.eq_auto_gain = None;
            self.eq_auto_key = None;
            return;
        }
        let Some(key) = self.eq_cache_key() else {
            return;
        };
        if self.eq_auto_key.as_deref() != Some(key.as_str()) {
            self.eq_auto_key = Some(key.clone());
            self.ensure_eq_cache();
            self.eq_auto_gain = self
                .eq_cache
                .get(&key)
                .copied()
                .map(crate::ui::eq::autogain_from_p90);
        }
        self.absorb_eq_estimate();
    }

    /// 首次播放攒够 80% 后，把整曲 p90 写进缓存（同一首只写一次）
    fn absorb_eq_estimate(&mut self) {
        let Some(p90) = self.eq_analyzer.as_ref().and_then(|a| a.estimated_p90()) else {
            return;
        };
        let Some(key) = self.eq_cache_key() else {
            return;
        };
        if let Some(old) = self.eq_cache.get(&key) {
            if (old - p90).abs() < 0.5 {
                return;
            }
        }
        self.eq_cache.insert(key, p90);
        self.save_eq_cache();
    }

    fn sync_eq(&mut self, st: &crate::player::MpvState) {
        // 拖动判定：只有播放位置在**两拍之间**跳变超过 2.5 秒才算用户拖动。
        // 之前拿「解码器自报的 expected_pos」比，正常播放时解码器慢半拍就会被
        // 当成拖动，导致解码器每 7–20 秒自杀重建一次（波形和标定跟着抖）。
        let mut seeked = false;
        if let Some(pos) = st.time_pos {
            if let Some(prev) = self.eq_last_pos {
                let back = pos < prev - 0.2; // 倒退就是 seek
                let jump = (pos - prev) > 2.0; // 往前跳超过 2 秒也是
                if back || jump {
                    seeked = true;
                }
            }
            self.eq_last_pos = Some(pos);
        }
        // 歌放完了（eof 或已到结尾 5 秒内）：收掉解码器，并给这一「代」打完成标记。
        // 以前这里会重建一次、从 0 重拉整首（实测多下 11.6MB/首）。
        let near_end = match (st.time_pos, st.duration) {
            (Some(p), Some(d)) if d > 5.0 => p >= d - 5.0,
            _ => false,
        };
        if st.eof || near_end {
            self.eq_analyzer = None;
            self.eq_done_gen = Some(self.play_gen);
            return;
        }
        // 根本没歌在播（用户停了/清空了队列）：解码器该收就收
        if self.now_playing.is_none() {
            self.eq_analyzer = None;
            return;
        }
        // 换歌了：解码器还连着上一首的流，必须重建（否则波形画的是上一首）
        if let (Some(url), Some(a)) = (self.now_url.as_deref(), self.eq_analyzer.as_ref()) {
            if a.url() != url {
                self.eq_analyzer = None;
                self.eq_seg = 0; // 换歌：段号重新从 1 数
            }
        }
        if seeked {
            // 拖动后必须从新位置重开，并清掉「这一代已结束」的标记
            self.eq_analyzer = None;
            self.eq_done_gen = None;
            // mpv 的 time_pos 是异步更新的：等它稳定再从新位置重开
            self.eq_hold = Some(Instant::now() + Duration::from_millis(400));
        }
        // 只有在「正在播放页 + 真在播放」时才需要读流；其余情况**不丢解码器**，
        // 只关闸门（不读管道 → mpv 阻塞 → 不再下载）。丢了解码器再回来又要重拉整首。
        let active = self.anim.equalizer_on()
            && self.anim.equalizer_source == crate::config::EqSource::Pcm
            && !self.eq_fallback
            && self.screen == ScreenKind::NowPlaying
            && self.now_playing.is_some()
            && !st.idle
            && !st.paused;
        let mut dead = false;
        if let Some(a) = self.eq_analyzer.as_mut() {
            if let Some(pos) = st.time_pos {
                a.set_play_pos(pos);
            }
            a.set_gate(active);
            dead = !a.alive() && !a.finished();
        }
        if dead {
            // 解码进程意外死了：收掉，下一次 tick 允许重建一次（有连败计数兜底）
            self.eq_analyzer = None;
            self.eq_failed();
        }
        if self.eq_analyzer.is_some() {
            return;
        }
        if !active || self.eq_done_gen == Some(self.play_gen) {
            return;
        }
        if let Some(t) = self.eq_hold {
            if Instant::now() < t {
                return;
            }
            self.eq_hold = None;
        }
        let Some(url) = self.now_url.clone() else {
            return;
        };
        let wave_cfg = crate::ui::eq::WaveCfg {
            reference_dbfs: self.anim.eq_reference_dbfs,
            gain: self.anim.eq_gain,
            soft_clip: self.anim.eq_soft_clip,
            autogain: self.anim.eq_autogain,
        };
        // 整曲标定：缓存命中就直接带着走（第一秒就到位），否则固定增益起步
        let target = self.eq_auto_gain;
        // 整首时长：mpv 还没报就用搜索结果里的时长（判断「播到 80%」写缓存要用）
        let duration = st.duration.or_else(|| {
            self.now_playing
                .as_ref()
                .map(|s| s.dt as f64 / 1000.0)
                .filter(|d| *d > 1.0)
        });
        self.eq_seg += 1;
        match crate::ui::eq::PcmAnalyzer::start(
            &url,
            st.time_pos.unwrap_or(0.0),
            wave_cfg,
            target,
            duration,
            self.eq_seg,
        ) {
            Some(a) => self.eq_analyzer = Some(a),
            None => self.eq_failed(),
        }
    }

    /// 连续两次解码失败就降级成正弦波形，只 toast 一次，不死磕
    fn eq_failed(&mut self) {
        self.eq_fail += 1;
        if self.eq_fail >= 2 && !self.eq_fallback {
            self.eq_fallback = true;
            self.toast = Some((
                crate::tr!(
                    "频谱解码失败，已降级为正弦波形（[ui] equalizer_source=synth）",
                    "Waveform decoding failed, falling back to the synthetic placeholder ([ui] equalizer_source=synth)"
                ),
                Instant::now(),
            ));
        }
    }

    fn handle_msgs(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                AppMsg::Playlists(r) => {
                    self.playlists_loading = false;
                    match r {
                        Ok(v) => self.playlists = v,
                        Err(e) => {
                            self.toast = Some((crate::tr!("歌单推荐加载失败: {}", "Could not load playlist picks: {}", e), Instant::now()))
                        }
                    }
                }
                AppMsg::Songs(r) => {
                    self.songs_loading = false;
                    match r {
                        Ok(v) => {
                            self.songs = v;
                            self.songs_sel = 0;
                        }
                        Err(e) => {
                            self.toast = Some((crate::tr!("歌曲推荐加载失败: {}", "Could not load song picks: {}", e), Instant::now()))
                        }
                    }
                }
                AppMsg::Albums(r) => {
                    self.albums_loading = false;
                    match r {
                        Ok(v) => {
                            self.albums = v;
                            self.albums_sel = 0;
                        }
                        Err(e) => {
                            self.toast = Some((crate::tr!("专辑推荐加载失败: {}", "Could not load album picks: {}", e), Instant::now()))
                        }
                    }
                }
                AppMsg::Search(r) => {
                    self.search_loading = false;
                    match &r {
                        Ok(_) => {
                            self.failed_sources.remove(&self.source_id);
                        }
                        Err(e) => {
                            // 搜索失败就地更新健康状态：缺 key 归「需凭据」，其余归离线
                            self.failed_sources.insert(self.source_id.clone());
                            let st = health::classify_error(&e.to_string());
                            self.health.set(&self.source_id, st);
                        }
                    }
                    match r {
                        Ok(mut v) => {
                            // 没有时长的曲目不进列表（单源也一样）
                            crate::source::agg::retain_timed(&mut v.songs);
                            // 单源也跑同一套去重（同名歌手/专辑/歌单 + 同名歌曲）
                            let mut seen = std::collections::HashSet::new();
                            v.songs.retain(|s| seen.insert(health::song_key(s)));
                            let mut seen = std::collections::HashSet::new();
                            v.artists.retain(|a| seen.insert(health::artist_key(a)));
                            let mut seen = std::collections::HashSet::new();
                            v.playlists.retain(|p| seen.insert(health::playlist_key(p)));
                            let mut seen = std::collections::HashSet::new();
                            v.albums.retain(|a| seen.insert(health::album_key(a)));
                            self.search = v.songs;
                            self.search_playlists = v.playlists;
                            self.search_albums = v.albums;
                            self.search_artists = v.artists;
                            self.search_sel = 0;
                        }
                        Err(e) => self.toast = Some((crate::tr!("搜索失败: {}", "Search failed: {}", e), Instant::now())),
                    }
                }
                AppMsg::Probe { gen, id, state } => {
                    if gen == self.probe_gen {
                        self.health.set(&id, state);
                        self.probe_done += 1;
                        if self.probe_done >= self.probe_total {
                            self.probe_started = None;
                        }
                    }
                }
                AppMsg::ProbeDone { gen } => {
                    if gen == self.probe_gen {
                        self.probe_started = None;
                        self.probe_done = self.probe_total;
                    }
                }
                AppMsg::AggPart { gen, id, result } => {
                    if gen != self.search_gen {
                        continue; // 切词/切源后旧结果
                    }
                    self.search_loading = false;
                    match result {
                        Ok(mut v) => {
                            crate::source::agg::retain_timed(&mut v.songs);
                            if !v.songs.is_empty() {
                                for song in &v.songs {
                                    let key = health::song_key(song);
                                    let list = self.agg_sources.entry(key.clone()).or_default();
                                    if !list.contains(&song.source) {
                                        list.push(song.source.clone());
                                    }
                                    let cands = self.agg_candidates.entry(key).or_default();
                                    if !cands
                                        .iter()
                                        .any(|c| c.source == song.source && c.s_id == song.s_id)
                                    {
                                        cands.push(song.clone());
                                    }
                                }
                                // 代表源按优先级：后到的网易云会顶掉先到的咪咕
                                health::merge_songs_priority(
                                    &mut self.search,
                                    &mut self.agg_index,
                                    &v.songs,
                                );
                            }
                            // 歌手 / 歌单 / 专辑：按类型各自去重，同一份 agg_sources 记来源
                            for a in &v.artists {
                                let key = health::artist_key(a);
                                let list = self.agg_sources.entry(key.clone()).or_default();
                                if !list.contains(&id) {
                                    list.push(id.clone());
                                }
                                let cands = self
                                    .agg_artist_candidates
                                    .entry(health::artist_key(a))
                                    .or_default();
                                if !cands.iter().any(|(s, i)| s == &id && *i == a.id) {
                                    cands.push((id.clone(), a.id));
                                }
                                if self.agg_seen.insert(key) {
                                    self.search_artists.push(a.clone());
                                }
                            }
                            for pl in &v.playlists {
                                let key = health::playlist_key(pl);
                                let list = self.agg_sources.entry(key.clone()).or_default();
                                if !list.contains(&id) {
                                    list.push(id.clone());
                                }
                                let cands = self
                                    .agg_playlist_candidates
                                    .entry(health::playlist_key(pl))
                                    .or_default();
                                if !cands.iter().any(|(s, i)| s == &id && *i == pl.id) {
                                    cands.push((id.clone(), pl.id));
                                }
                                if self.agg_seen.insert(key) {
                                    self.search_playlists.push(pl.clone());
                                }
                            }
                            for al in &v.albums {
                                let key = health::album_key(al);
                                let list = self.agg_sources.entry(key.clone()).or_default();
                                if !list.contains(&id) {
                                    list.push(id.clone());
                                }
                                let cands = self
                                    .agg_album_candidates
                                    .entry(health::album_key(al))
                                    .or_default();
                                if !cands.iter().any(|(s, i)| s == &id && *i == al.id) {
                                    cands.push((id.clone(), al.id));
                                }
                                if self.agg_seen.insert(key) {
                                    self.search_albums.push(al.clone());
                                }
                            }
                            self.failed_sources.remove(&id);
                        }
                        Err(e) => {
                            if !self.agg_failed.iter().any(|f| f.starts_with(&id)) {
                                self.agg_failed.push(format!("{id}: {e}"));
                            }
                        }
                    }
                }
                AppMsg::AggDone { gen, failed } => {
                    if gen != self.search_gen {
                        continue;
                    }
                    self.search_loading = false;
                    // 全部到齐后按源优先级稳定排序（网易云在最上面）
                    health::sort_by_priority(&mut self.search);
                    let prio = |app: &Self, key: String| -> u8 {
                        app.agg_sources
                            .get(&key)
                            .and_then(|v| v.first())
                            .map(|s| crate::source::source_priority(s))
                            .unwrap_or(99)
                    };
                    let mut artists = std::mem::take(&mut self.search_artists);
                    artists.sort_by_key(|a| prio(self, health::artist_key(a)));
                    self.search_artists = artists;
                    let mut playlists = std::mem::take(&mut self.search_playlists);
                    playlists.sort_by_key(|p| prio(self, health::playlist_key(p)));
                    self.search_playlists = playlists;
                    let mut albums = std::mem::take(&mut self.search_albums);
                    albums.sort_by_key(|a| prio(self, health::album_key(a)));
                    self.search_albums = albums;
                    for f in failed {
                        self.agg_failed.push(f);
                    }
                    if self.search.is_empty() && !self.agg_failed.is_empty() {
                        let sep = if crate::i18n::en() { "; " } else { "；" };
                        let msg = self.agg_failed.join(sep);
                        self.toast = Some((
                            crate::tr!(
                                "聚合搜索全失败: {}",
                                "Aggregate search failed everywhere: {}",
                                msg
                            ),
                            Instant::now(),
                        ));
                    }
                }
                AppMsg::Duration { source, s_id, dt } => {
                    if dt > 0 {
                        let fix = |s: &mut Song| {
                            if s.source == source && s.s_id == s_id {
                                s.dt = dt;
                            }
                        };
                        for s in self.search.iter_mut() {
                            fix(s);
                        }
                        for s in self.queue.iter_mut() {
                            fix(s);
                        }
                        for s in self.lib.iter_mut() {
                            fix(s);
                        }
                        if let Some(s) = self.now_playing.as_mut() {
                            fix(s);
                        }
                    } else {
                        // 这次拿不到：允许换歌再试一次
                        self.duration_asked.remove(&format!("{source}|{s_id}"));
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
                        Err(e) => {
                            self.toast = Some((crate::tr!("歌单加载失败: {}", "Could not load the playlist: {}", e), Instant::now()))
                        }
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
                        Err(e) => {
                            self.toast = Some((crate::tr!("专辑加载失败: {}", "Could not load the album: {}", e), Instant::now()))
                        }
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
                        Err(e) => {
                            self.toast = Some((crate::tr!("歌手歌曲加载失败: {}", "Could not load the artist's songs: {}", e), Instant::now()))
                        }
                    }
                }
                AppMsg::SuperDevices(r) => {
                    self.super_loading = false;
                    match r {
                        Ok(v) => {
                            self.super_devices = v;
                            self.super_sel = 0;
                        }
                        Err(e) => {
                            self.toast = Some((crate::tr!("USB 设备检测失败: {}", "USB scan failed: {}", e), Instant::now()))
                        }
                    }
                }
                AppMsg::SettingsUsb(r) => {
                    self.settings_usb_loading = false;
                    match r {
                        Ok(v) => self.settings_usb = v,
                        Err(e) => {
                            self.toast = Some((crate::tr!("USB 设备检测失败: {}", "USB scan failed: {}", e), Instant::now()))
                        }
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
                        _ => crate::tr!("超级下载完成", "Super download finished"),
                    };
                    self.toast = Some((toast, Instant::now()));
                }
                AppMsg::RunSudo { cmd, resp } => {
                    let r = self.run_sudo(&cmd);
                    let _ = resp.send(r);
                }
                AppMsg::Enqueue(r) => match r {
                    Ok(v) => self.enqueue_songs(v),
                    Err(e) => {
                        self.toast = Some((crate::tr!("获取歌单曲目失败: {}", "Could not fetch the playlist tracks: {}", e), Instant::now()))
                    }
                },
                AppMsg::PlayUrl(gen, url, autoplay, proxy, used, failures) => {
                    if gen == self.play_gen {
                        // mpv 的 http-proxy 按曲目切：yt-dlp 系源走代理，其他源清空
                        self.player.set_http_proxy(proxy.as_deref());
                        match url {
                            Some(u) => {
                                self.skip_streak = 0;
                                // 换了一首（或同一首换了源）：重新做逐曲响度测速
                                if self.now_url.as_deref() != Some(u.as_str()) {
                                    self.eq_auto_gain = None;
                                    self.eq_auto_key = None;
                                }
                                self.now_url = Some(u.clone());
                                // 换源成功：把队列/正在播放这首歌的条目改成真正用的源，
                                // 让「源」列、i 弹窗、收藏都反映实际来源
                                if let Some((src, s_id)) = used {
                                    if let Some(cur) = self.now_playing.as_mut() {
                                        if cur.source != src {
                                            let mut fixed = cur.clone();
                                            fixed.source = src.clone();
                                            fixed.s_id = s_id.clone();
                                            *cur = fixed.clone();
                                            if let Some(i) = self.queue_idx {
                                                if let Some(q) = self.queue.get_mut(i) {
                                                    *q = fixed;
                                                }
                                            }
                                            self.toast = Some((
                                                crate::tr!(
                                                    "主源取不到流，已换 {} 播放",
                                                    "Primary source had no stream, playing from {}",
                                                    crate::source::source_display(&src)
                                                ),
                                                Instant::now(),
                                            ));
                                        }
                                    }
                                }
                                if autoplay {
                                    self.player.load_play(&u);
                                } else {
                                    self.player.load(&u);
                                }
                                // 切音质重取 URL 后，尽量回到原来的位置
                                if let Some(pos) = self.pending_seek.take() {
                                    self.player.seek_abs(pos);
                                }
                            }
                            None => {
                                // 所有备选源都失败：把每个源的原因带上，再跳过
                                let detail = if failures.is_empty() {
                                    None
                                } else {
                                    let items: Vec<String> = failures
                                        .iter()
                                        .take(4)
                                        .map(|(s, why)| format!("{s}: {why}"))
                                        .collect();
                                    Some(crate::tr!(
                        "所有源均取不到流（{}）",
                        "No source could produce a stream ({})",
                        items.join(" / ")
                    ))
                                };
                                self.skip_unplayable(detail);
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
        // 主菜单是根页：回菜单清栈，别把来源页压进去
        self.nav.go(self.screen, kind);
        self.screen = kind;
        self.selected.clear();
        self.ctx_open = false;
    }

    fn list_len(&self) -> usize {
        match self.screen {
            ScreenKind::Menu => menu_items().len(),
            ScreenKind::Playlists => self.playlists.len(),
            ScreenKind::Songs => self.songs.len(),
            ScreenKind::Albums => self.albums.len(),
            ScreenKind::Search if self.source_view_active() => self.registry.len(),
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
            ScreenKind::Menu
            | ScreenKind::Playlists
            | ScreenKind::Albums
            | ScreenKind::SuperDl
            | ScreenKind::Settings => 2,
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
                        if col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height {
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
        let Some(idx) = self.hit_index(row) else {
            return;
        };
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
            ScreenKind::Playlists => {
                self.open_playlist_ctx(self.playlists.get(idx).cloned(), col, row)
            }
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
            creator: crate::models::PlaylistCreator::default(),
        });
        self.ctx_song = None;
        self.ctx_sel = 0;
        self.ctx_x = x;
        self.ctx_y = y;
        self.ctx_open = self.ctx_playlist.is_some();
    }

    fn ctx_items(&self) -> Vec<String> {
        match self.ctx_kind {
            0 => vec![
                crate::tr!("1  下载", "1  Download"),
                crate::tr!("2  收藏", "2  Favorite"),
            ],
            1 => vec![
                crate::tr!("1  下载歌单", "1  Download playlist"),
                crate::tr!("2  收藏", "2  Favorite"),
            ],
            _ => vec![crate::tr!("1  下载专辑", "1  Download album")],
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
                            crate::tr!("已收藏: {}", "Added to favorites: {}", name)
                        } else {
                            crate::tr!("已在收藏中", "Already in favorites")
                        },
                        Instant::now(),
                    ));
                }
            }
            (1, 0) | (2, 0) => {
                if let Some(p) = pl {
                    self.toast = Some((crate::tr!("正在获取: {}...", "Fetching: {}...", p.name), Instant::now()));
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
                            crate::tr!("已收藏歌单: {}", "Playlist added to favorites: {}", p.name)
                        } else {
                            crate::tr!("歌单已在收藏中", "Playlist already in favorites")
                        },
                        Instant::now(),
                    ));
                }
            }
            _ => {}
        }
    }

    fn handle_key(&mut self, k: KeyEvent) {
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
        // 当前歌曲设置弹窗：模态，打开时其它按键一律不穿透
        if self.song_cfg_open {
            let n = self.song_cfg_items().len();
            match k.code {
                KeyCode::Esc => self.song_cfg_open = false,
                KeyCode::Up | KeyCode::Char('k') if n > 0 => {
                    self.song_cfg_sel = (self.song_cfg_sel + n - 1) % n;
                }
                KeyCode::Down | KeyCode::Char('j') if n > 0 => {
                    self.song_cfg_sel = (self.song_cfg_sel + 1) % n;
                }
                KeyCode::Enter => self.song_cfg_apply(),
                _ => {}
            }
            return;
        }
        // F5：搜索页等价于 S（换源并自动重搜），任何焦点状态都生效
        if k.code == KeyCode::F(5) && self.screen == ScreenKind::Search {
            self.cycle_source();
            return;
        }
        // 搜索页 R：随时重探（大写 R，避免和小写搜索词冲突）
        if k.code == KeyCode::Char('R') {
            self.reprobe_with_toast();
            return;
        }
        // 搜索页 V：随时切换「搜索结果 ↔ 源状态明细」，输入框聚焦时也生效
        if self.screen == ScreenKind::Search && k.code == KeyCode::Char('V') {
            self.show_source_list = !self.show_source_list;
            self.source_sel = 0;
            return;
        }
        // 搜索页焦点两态：输入栏 / 结果列表。Tab 只切焦点，不再兼做切类型。
        if self.screen == ScreenKind::Search {
            match search_keys::nav(&k, self.search_input_focus) {
                search_keys::Nav::FocusInput => {
                    self.search_input_focus = true;
                    return;
                }
                search_keys::Nav::FocusList => {
                    self.search_input_focus = false;
                    return;
                }
                search_keys::Nav::SetType(t) => {
                    self.set_search_type(t);
                    return;
                }
                search_keys::Nav::None => {}
            }
            // R 已在上面全局处理，这里只剩 V（切视图）
            // 列表聚焦时直接打字：回焦输入栏并插入，避免“打字丢字”
            if !self.search_input_focus {
                if let Some(c) = search_keys::printable_to_input(&k) {
                    self.search_input_focus = true;
                    self.search_push(c);
                    return;
                }
            }
        }
        if self.screen == ScreenKind::Search && self.search_input_focus {
            self.handle_search_input(k);
            return;
        }
        match k.code {
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => self.quit = true,
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Esc => {
                // 出栈一层；主菜单是根页、栈空时什么都不做（不退出、不跳菜单）
                if let Some(prev) = self.nav.back(self.screen) {
                    self.screen = prev;
                    self.ctx_open = false;
                }
            }
            KeyCode::Char('h') => self.goto(ScreenKind::Menu),
            KeyCode::Char('s') => {
                self.goto(ScreenKind::Search);
                self.search_input_focus = true;
            }
            KeyCode::Char('S') => self.cycle_source(),
            // 主菜单：0–6 直达对应项
            KeyCode::Char(c @ '0'..='6') if self.screen == ScreenKind::Menu => {
                self.menu_sel = (c as u8 - b'0') as usize;
                self.activate_menu();
            }
            KeyCode::Char('R') => self.reprobe_with_toast(),
            // 搜索页：S 循环的范围（只看可用 / 连离线的也放进来）
            KeyCode::Char('a') if self.screen == ScreenKind::Search => {
                self.toggle_show_all_sources();
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
            KeyCode::Char('i') => {
                if self.now_playing.is_some() {
                    self.song_cfg_open = true;
                    self.song_cfg_sel = 0;
                } else {
                    self.toast = Some((
                        crate::tr!("还没有在播放的歌曲", "Nothing is playing yet"),
                        Instant::now(),
                    ));
                }
            }
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
            KeyCode::Char(c) => self.search_push(c),
            KeyCode::Backspace => {
                self.search_input.pop();
            }
            KeyCode::Enter => {
                let q = self.search_input.trim().to_string();
                if !q.is_empty() {
                    self.do_search(q);
                }
            }
            KeyCode::Esc => {
                // 有内容先收焦点（还能继续看结果）；空串直接回上一页
                if self.search_input.is_empty() {
                    if let Some(prev) = self.nav.back(self.screen) {
                        self.screen = prev;
                        self.search_input_focus = false;
                    }
                } else {
                    self.search_input_focus = false;
                }
            }
            _ => {}
        }
    }

    const QUALITIES: [&'static str; 5] = ["standard", "higher", "exhigh", "lossless", "hires"];

    fn song_cfg_items(&self) -> Vec<String> {
        let Some(song) = &self.now_playing else {
            return vec![crate::tr!("当前没有播放中的歌曲", "Nothing is playing")];
        };
        let fav = if self.favorites.contains(song.id) {
            crate::tr!("已收藏", "favorited")
        } else {
            crate::tr!("未收藏", "not favorited")
        };
        vec![
            crate::tr!("收藏 / 取消收藏    {}", "Favorite / unfavorite    {}", fav),
            crate::tr!("下载这首", "Download this"),
            crate::tr!("音质    {}", "Quality    {}", self.cfg.quality),
            crate::tr!(
                "播放顺序    {}",
                "Play order    {}",
                self.play_mode.label()
            ),
            crate::tr!(
                "来源    {}",
                "Source    {}",
                crate::source::source_display(&song.source)
            ),
        ]
    }

    fn song_cfg_apply(&mut self) {
        let Some(song) = self.now_playing.clone() else {
            return;
        };
        match self.song_cfg_sel {
            0 => {
                if self.favorites.contains(song.id) {
                    let mut ids = HashSet::new();
                    ids.insert(song.id);
                    self.favorites.remove_ids(&ids);
                    self.favorites.save(&self.cfg.favorites_path);
                    self.toast = Some((
                    crate::tr!("已取消收藏 {}", "Removed from favorites: {}", song.name),
                    Instant::now(),
                ));
                } else {
                    self.favorites.add(std::slice::from_ref(&song));
                    self.favorites.save(&self.cfg.favorites_path);
                    self.toast = Some((
                        crate::tr!("已收藏 {}", "Added to favorites: {}", song.name),
                        Instant::now(),
                    ));
                }
            }
            1 => {
                let n = self.downloader.enqueue(vec![song]);
                self.toast = Some((
                    if n > 0 {
                        crate::tr!("已加入下载队列", "Added to the download queue")
                    } else {
                        crate::tr!("已在下载队列", "Already in the download queue")
                    },
                    Instant::now(),
                ));
            }
            2 => {
                let cur = Self::QUALITIES
                    .iter()
                    .position(|q| *q == self.cfg.quality)
                    .unwrap_or(2);
                let next = Self::QUALITIES[(cur + 1) % Self::QUALITIES.len()];
                self.cfg.quality = next.to_string();
                *self.quality.write().unwrap() = next.to_string();
                self.settings.quality = next.to_string();
                self.settings.save(&self.cfg.settings_path);
                self.toast = Some((
                    crate::tr!("音质切到 {}，重新取流", "Quality set to {}, fetching a new stream", next),
                    Instant::now(),
                ));
                self.reload_for_quality();
            }
            3 => {
                self.play_mode = self.play_mode.next();
                self.settings.play_mode = self.play_mode.as_str().to_string();
                self.settings.save(&self.cfg.settings_path);
                self.toast = Some((
                    crate::tr!("播放顺序: {}", "Play order: {}", self.play_mode.label()),
                    Instant::now(),
                ));
            }
            _ => {}
        }
    }

    /// 音质变了要重新取 URL；记下当前位置，新流加载后跳回去
    fn reload_for_quality(&mut self) {
        if self.queue_idx.is_none() {
            return;
        }
        self.pending_seek = self.player.state().time_pos;
        self.play_gen += 1;
        self.play_current(false); // 切音质：播放中就继续播，暂停中就保持暂停
    }

    /// 查询串长度上限：长按/粘贴不会把界面卡死，也不会把 API 请求撑爆
    const SEARCH_INPUT_MAX: usize = 80;

    fn search_push(&mut self, c: char) {
        if self.search_input.chars().count() < Self::SEARCH_INPUT_MAX {
            self.search_input.push(c);
        }
    }

    /// 当前源支持哪些搜索类型（UI 据此置灰/拒绝）
    fn source_caps(&self) -> Option<crate::source::Capabilities> {
        self.registry.get(&self.source_id).map(|s| s.capabilities())
    }

    fn set_search_type(&mut self, t: usize) {
        if t == self.search_type {
            return;
        }
        let kind = crate::source::SearchKind::from_index(t);
        if let Some(caps) = self.source_caps() {
            if !caps.supports(kind) {
                self.toast = Some((
                    crate::tr!(
                        "当前源不支持这种搜索，已保持原样",
                        "This source does not support that search kind, kept as is"
                    ),
                    Instant::now(),
                ));
                return;
            }
        }
        self.search_type = t;
        self.search_sel = 0;
        let q = self.search_input.trim().to_string();
        if !q.is_empty() {
            self.do_search(q);
        }
    }

    fn do_search(&mut self, q: String) {
        self.show_source_list = false;
        self.source_sel = 0;
        if self.source_id == AGGREGATE_ID {
            self.aggregate_search(q);
            return;
        }
        // 焦点保持原状：输入栏聚焦时继续可编辑，列表聚焦时保持列表操作
        self.search_loading = true;
        let kind = crate::source::SearchKind::from_index(self.search_type);
        let registry = self.registry.clone();
        let source = self.source_id.clone();
        let tx = self.tx.clone();
        spawn(tx, move || match registry.search(&source, &q, 60, kind) {
            Ok(v) => AppMsg::Search(Ok(v)),
            Err(e) => AppMsg::Search(Err(e.to_string())),
        });
    }

    /// 当前源的「组 · 名称」+ 是否可用
    fn current_source(&self) -> (String, bool) {
        if self.source_id == AGGREGATE_ID {
            let n = self.ok_source_ids().len();
            return (crate::tr!("聚合 · {} 个可用源", "Aggregate · {} sources available", n), n > 0);
        }
        match self.registry.get(&self.source_id) {
            // 不在渲染/按键路径里探活（会卡 UI）；可用性只读缓存 + 最近一次搜索结果
            Some(s) => {
                let st = self.health.state(s.id());
                let healthy = !matches!(st, SourceState::Down | SourceState::NeedsCreds);
                (
                    format!(
                        "{} · {}",
                        group_display(s.group()),
                        crate::source::source_display(s.id())
                    ),
                    healthy && !self.failed_sources.contains(&self.source_id),
                )
            }
            None => (crate::tr!("未知", "unknown"), false),
        }
    }

    /// S 键：在已注册的源之间循环；换了源就按当前关键词重搜
    fn cycle_source(&mut self) {
        let ids = self.source_cycle_ids();
        if ids.is_empty() {
            return;
        }
        let cur = ids.iter().position(|id| *id == self.source_id).unwrap_or(0);
        self.source_id = ids[(cur + 1) % ids.len()].clone();
        self.settings.source = self.source_id.clone();
        self.settings.save(&self.cfg.settings_path);
        if let Some(caps) = self.source_caps() {
            if !caps.supports(crate::source::SearchKind::from_index(self.search_type)) {
                self.search_type = 0;
            }
        }
        self.search_sel = 0;
        self.selected.clear();
        let (name, avail) = self.current_source();
        self.toast = Some((
            crate::tr!(
                "音乐源: {}{}",
                "Source: {}{}",
                name,
                if avail {
                    String::new()
                } else {
                    crate::tr!("（不可用）", " (unavailable)")
                }
            ),
            Instant::now(),
        ));
        let q = self.search_input.trim().to_string();
        if !q.is_empty() {
            self.do_search(q);
        }
    }

    /// S 循环用的源列表：聚合永远排第一，其余只放 `Ok` 的（V 键打开后连离线的也放）
    fn source_cycle_ids(&self) -> Vec<String> {
        let mut ids = vec![AGGREGATE_ID.to_string()];
        for src in self.registry.all() {
            let id = src.id();
            let ok = self.health.state(id).is_ok();
            if ok || self.show_all_sources || self.source_id == id {
                ids.push(id.to_string());
            }
        }
        ids
    }

    /// 当前可用的源（聚合只问这些）
    fn ok_source_ids(&self) -> Vec<String> {
        self.registry
            .all()
            .iter()
            .filter(|s| self.health.state(s.id()).is_ok())
            .map(|s| s.id().to_string())
            .collect()
    }

    fn toggle_show_all_sources(&mut self) {
        self.show_all_sources = !self.show_all_sources;
        let n = self.registry.len();
        let ok = self.ok_source_ids().len();
        self.toast = Some((
            if self.show_all_sources {
                crate::tr!(
                    "源列表: 全部 {} 个（{} 可用，{} 离线）",
                    "Sources: all {} ({} available, {} offline)",
                    n, ok, n - ok
                )
            } else {
                crate::tr!(
                    "源列表: 只用可用的 {} 个（V 看全部）",
                    "Sources: {} available only (V shows all)",
                    ok
                )
            },
            Instant::now(),
        ));
    }

    /// 启动/`R` 重探：所有源并发探活，结果通过 `AppMsg::Probe` 回到主线程
    /// 重新读 config.toml 的 `[sources] enabled` 并重建 registry。
    /// 运行中执行过 `zmuse source add/remove` 之后，按 R 就能生效，不用重启。
    fn reload_registry(&mut self) {
        let cfg = crate::config::Config::load();
        let ok = |id: &str| cfg.enabled_sources.iter().any(|e| e == id);
        if ok("netease") == self.registry.get("netease").is_some()
            && cfg.enabled_sources.len() == self.registry.len()
        {
            return; // 没变化就别白重建
        }
        let registry = crate::source::SourceRegistry::with_defaults(
            cfg.api_base.clone(),
            cfg.proxy.clone(),
            cfg.meting_url.clone(),
            cfg.ytdlp_binary.clone(),
            cfg.meting_cookie.clone(),
            cfg.jamendo_client_id.clone(),
            &cfg.enabled_sources,
        );
        self.registry = Arc::new(registry);
        // 当前源的 id 可能被停用了，退回聚合
        if self.source_id != AGGREGATE_ID && self.registry.get(&self.source_id).is_none() {
            self.source_id = AGGREGATE_ID.to_string();
        }
        self.health = health::HealthTable::new(health::DEFAULT_TTL);
        self.eq_last_pos = None;
    }

    fn start_probe(&mut self) {
        if self.health.probing_count() > 0 {
            return;
        }
        let srcs: Vec<(String, Arc<dyn crate::source::Source>)> = self
            .registry
            .all()
            .iter()
            .map(|s| (s.id().to_string(), s.clone()))
            .collect();
        if srcs.is_empty() {
            return;
        }
        // 重探同时清掉聚合结果缓存，避免拿旧结果糊弄
        crate::source::agg::clear_cache();
        self.probe_gen += 1;
        let gen = self.probe_gen;
        self.probe_total = srcs.len();
        self.probe_done = 0;
        self.probe_started = Some(Instant::now());
        for (id, _) in &srcs {
            self.health.set(id, SourceState::Probing);
        }
        let tx = self.tx.clone();
        thread::spawn(move || {
            health::probe_all_with(srcs, health::CONCURRENCY, |id, st| {
                let _ = tx.send(AppMsg::Probe { gen, id, state: st });
            });
            let _ = tx.send(AppMsg::ProbeDone { gen });
        });
    }

    /// 探测超时收尾：还没回来的源先按离线算，迟到的结果会覆盖它
    fn probe_timeout_check(&mut self) {
        let Some(started) = self.probe_started else {
            return;
        };
        if started.elapsed() < Duration::from_secs(8) {
            return;
        }
        let ids: Vec<String> = self
            .registry
            .all()
            .iter()
            .map(|s| s.id().to_string())
            .collect();
        for id in ids {
            if self.health.state(&id) == SourceState::Probing {
                self.health.set(&id, SourceState::Down);
            }
        }
        self.probe_done = self.probe_total;
        self.probe_started = None;
        self.toast = Some((
            crate::tr!(
                "源探测超时，已按离线处理（{} 个可用）",
                "Source probe timed out, treated as offline ({} available)",
                self.ok_source_ids().len()
            ),
            Instant::now(),
        ));
    }

    fn reprobe_with_toast(&mut self) {
        // 先按磁盘上的配置重建（用户可能刚在外面 `zmuse source add` 过）
        self.reload_registry();
        let before = self.probe_gen;
        self.start_probe();
        self.toast = Some((
            if self.probe_gen == before {
                crate::tr!("正在探测中…", "Already probing…")
            } else {
                crate::tr!(
                    "重新探测 {} 个源…",
                    "Re-probing {} sources…",
                    self.probe_total
                )
            },
            Instant::now(),
        ));
    }

    fn probe_status(&self) -> Option<String> {
        if self.probe_started.is_some() {
            Some(crate::tr!(
                "探测中 {}/{}",
                "probing {}/{}",
                self.probe_done,
                self.probe_total
            ))
        } else if self.health.ok_count() > 0 {
            Some(crate::tr!(
                "源 {}/{}",
                "sources {}/{}",
                self.health.ok_count(),
                self.registry.len()
            ))
        } else {
            None
        }
    }

    /// 聚合搜索：并发问所有 Ok 的源，谁先回来先显示；每源 5s、并发 4
    fn aggregate_search(&mut self, q: String) {
        self.search_gen += 1;
        let gen = self.search_gen;
        self.search_loading = true;
        self.search.clear();
        self.agg_index.clear();
        self.agg_seen.clear();
        self.agg_sources.clear();
        self.agg_candidates.clear();
        self.agg_artist_candidates.clear();
        self.agg_album_candidates.clear();
        self.agg_playlist_candidates.clear();
        self.agg_failed.clear();
        self.search_sel = 0;
        self.selected.clear();
        let kind = crate::source::SearchKind::from_index(self.search_type);
        let mut targets: Vec<(String, Arc<dyn crate::source::Source>)> = Vec::new();
        for src in self.registry.all() {
            let caps = src.capabilities();
            if !self.health.state(src.id()).is_ok() || !caps.supports(kind) {
                continue;
            }
            // 搜歌曲时只并能出流的源；歌手/歌单/专辑不需要 can_stream
            if kind == crate::source::SearchKind::Song && !caps.can_stream {
                continue;
            }
            targets.push((src.id().to_string(), src.clone()));
        }
        if targets.is_empty() {
            self.search_loading = false;
            let label = match kind {
                crate::source::SearchKind::Song => crate::tr!("歌曲", "songs"),
                crate::source::SearchKind::Artist => crate::tr!("歌手", "artists"),
                crate::source::SearchKind::Playlist => crate::tr!("歌单", "playlists"),
                crate::source::SearchKind::Album => crate::tr!("专辑", "albums"),
            };
            self.toast = Some((
                crate::tr!(
                    "没有可用源支持「{label}」搜索（R 重探或换源后再试）",
                    "No available source supports {label} search (press R to re-probe or pick another source)",
                    label = label
                ),
                Instant::now(),
            ));
            return;
        }
        let tx = self.tx.clone();
        let cfg = crate::source::agg::Config::default();
        thread::spawn(move || {
            let stats = crate::source::agg::run_kind(targets, &q, 20, kind, cfg, |p| {
                let _ = tx.send(AppMsg::AggPart {
                    gen,
                    id: p.id,
                    result: p.result,
                });
            });
            let mut failed: Vec<String> = stats.timed_out_ids.clone();
            failed.extend(
            stats
                .failed_ids
                .iter()
                .map(|id| crate::tr!("{}: 失败", "{}: failed", id))
        );
            let _ = tx.send(AppMsg::AggDone { gen, failed });
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
            ScreenKind::Search if self.source_view_active() => &mut self.source_sel,
            ScreenKind::Search => &mut self.search_sel,
            ScreenKind::Library => &mut self.lib_sel,
            ScreenKind::Favorites => &mut self.fav_sel,
            ScreenKind::SuperDl => &mut self.super_sel,
            ScreenKind::Settings => &mut self.settings_sel,
            _ => return,
        };
        *sel = (*sel as i32 + delta).rem_euclid(len as i32) as usize;
        if self.screen == ScreenKind::Search {
            self.lazy_duration_of_selected();
        }
    }

    /// 搜索页是否在「源状态明细」视图：手动 V 切过去，或者结果为空时默认看它
    pub(crate) fn source_view_active(&self) -> bool {
        self.show_source_list || (self.search_type == 0 && self.search.is_empty())
    }

    /// 每源一句话原因：只靠已缓存的探活状态与配置推断，不发网络请求
    pub(crate) fn source_reason(&self, it: &status::Item) -> String {
        let meting = it.id.starts_with("meting:");
        let zh = !crate::i18n::en();
        match it.state {
            SourceState::Ok if it.only_search => if zh {
                "仅搜索（无 cookie）"
            } else {
                "search only (no cookie)"
            }
            .to_string(),
            SourceState::Ok => "ok".to_string(),
            SourceState::Probing => if zh { "探测中…" } else { "probing…" }.to_string(),
            SourceState::Unknown => if zh { "未探测" } else { "not probed" }.to_string(),
            SourceState::NeedsCreds => {
                if it.id == "jamendo" {
                    if zh { "缺 client_id" } else { "missing client_id" }.to_string()
                } else if meting {
                    if zh {
                        "侧车未启动（或未配 cookie）"
                    } else {
                        "sidecar down (or no cookie)"
                    }
                    .to_string()
                } else {
                    if zh { "缺凭据" } else { "credentials missing" }.to_string()
                }
            }
            SourceState::Down => {
                if meting {
                    if zh { "侧车未启动" } else { "sidecar down" }.to_string()
                } else if matches!(it.id.as_str(), "youtube" | "soundcloud")
                    && self.cfg.proxy.is_none()
                {
                    if zh { "代理未配置" } else { "no proxy configured" }.to_string()
                } else {
                    if zh { "不可用" } else { "unavailable" }.to_string()
                }
            }
        }
    }

    /// 搜索页状态行的数据：按优先级（= 注册顺序）列源，只读探活缓存，不发请求
    pub(crate) fn source_status_items(&self) -> Vec<status::Item> {
        self.registry
            .all()
            .iter()
            .map(|s| {
                let id = s.id();
                let st = self.health.state(id);
                status::Item {
                    id: id.to_string(),
                    label: crate::source::source_display(id),
                    state: st,
                    current: self.source_id == id,
                    only_search: st.is_ok() && !s.capabilities().can_stream,
                }
            })
            .collect()
    }

    pub(crate) fn health_ok_count(&self) -> usize {
        self.health.ok_count()
    }

    /// 状态行末尾：探测中给进度，否则给可用计数
    pub(crate) fn source_status_suffix(&self) -> String {
        status::suffix(
            self.probe_started.is_some(),
            self.probe_done,
            self.probe_total,
            self.health.ok_count(),
        )
    }

    /// 列表里的来源短名 + 能不能出流（聚合模式的「源」列用）
    fn source_badge(&self, s: &Song) -> (String, bool) {
        let can = self.can_stream_song(s);
        if self.source_id == AGGREGATE_ID {
            if let Some(list) = self.agg_sources.get(&health::dedup_key(s)) {
                if !list.is_empty() {
                    // 同一首被多个源命中时全列出来，顺序也按优先级：「网易云+QQ(直连)」
                    let mut ids = list.clone();
                    ids.sort_by(|a, b| crate::source::by_source_priority(a, b));
                    let labels: Vec<String> = ids
                        .iter()
                        .map(|id| crate::source::source_display(id))
                        .collect();
                    let mut text = labels.join("+");
                    // 网易云付费曲目提前标出来，别等点了才知道：`网易云+QQ(直连)·会员`
                    if s.source == "netease" && s.fee != 0 {
                        text.push_str(if crate::i18n::en() { "·member" } else { "·会员" });
                    }
                    return (text, can || ids.len() > 1);
                }
            }
        }
        (crate::source::source_display(&s.source), can)
    }

    /// 打开专辑/歌单/歌手详情时用哪个 id：
    ///
    /// - 单源网易云：条目自己的 id 就能打开（**这才是唯一能打开的情况**，之前正好判反了）
    /// - 聚合：从候选里按优先级挑网易云的
    /// - 其它单源：这些源没有详情接口 → None，调用方提示换源
    fn detail_id(
        &self,
        own_id: i64,
        map: &std::collections::HashMap<String, Vec<(String, i64)>>,
        key: &str,
    ) -> Option<i64> {
        if self.source_id == "netease" {
            return Some(own_id);
        }
        if self.source_id == AGGREGATE_ID {
            return self.pick_candidate(map, key);
        }
        None
    }

    /// 候选里按源优先级挑第一个「能打开详情」的：目前只有网易云的详情接口
    fn pick_candidate(
        &self,
        map: &std::collections::HashMap<String, Vec<(String, i64)>>,
        key: &str,
    ) -> Option<i64> {
        if self.source_id != AGGREGATE_ID {
            return None;
        }
        let mut cands = map.get(key)?.clone();
        cands.sort_by(|a, b| crate::source::by_source_priority(&a.0, &b.0));
        cands
            .iter()
            .find(|(src, _)| src == "netease")
            .map(|(_, id)| *id)
    }

    /// 打不开详情时的提示：按当前源给出下一步，而不是笼统说「只有网易云能打开」
    fn only_netease_toast(&mut self, what: &str) {
        self.toast = Some((detail_unsupported_msg(&self.source_id, what), Instant::now()));
    }

    /// 某一行（歌/歌手/歌单/专辑）命中的源，按优先级并列；聚合模式才用
    pub(crate) fn item_sources(&self, key: &str) -> Option<String> {
        if self.source_id != AGGREGATE_ID {
            return None;
        }
        let list = self.agg_sources.get(key)?;
        if list.is_empty() {
            return None;
        }
        let mut ids = list.clone();
        ids.sort_by(|a, b| crate::source::by_source_priority(a, b));
        Some(
            ids.iter()
                .map(|id| crate::source::source_display(id))
                .collect::<Vec<_>>()
                .join("+"),
        )
    }

    /// 这首歌的源能不能出流（查 capability，不发网络请求）
    fn can_stream_song(&self, s: &Song) -> bool {
        self.registry
            .get(&s.source)
            .map(|src| src.capabilities().can_stream)
            .unwrap_or(false)
    }

    /// 播放地址拿不到：记下这首、跳过它、按播放顺序继续
    fn skip_unplayable(&mut self, why: Option<String>) {
        let Some(song) = self.now_playing.clone() else {
            return;
        };
        let key = format!("{}|{}", song.source, song.s_id);
        self.bad_songs.insert(key);
        if self.skip_streak >= 5 {
            self.skip_streak = 0;
            let tail = match &why {
                Some(w) => format!("；{w}"),
                None => String::new(),
            };
            self.toast = Some((
                crate::tr!(
                    "连续 5 首无法播放（最后一首: {}），已停下{}",
                    "5 tracks in a row were unplayable (last: {}), stopping{}",
                    song.name, tail
                ),
                Instant::now(),
            ));
            self.now_playing = None;
            self.queue_idx = None;
            return;
        }
        self.skip_streak += 1;
        let Some(idx) = self.queue_idx else { return };
        let n = self.queue.len();
        if n <= 1 {
            let reason = match &why {
                Some(w) => format!("：{w}"),
                None => String::new(),
            };
            self.toast = Some((
                crate::tr!(
                    "无法播放: {}（{}），队列里没有别的歌了{}",
                    "Cannot play: {} ({}), nothing else left in the queue{}",
                    song.name,
                    crate::source::source_display(&song.source),
                    reason
                ),
                Instant::now(),
            ));
            self.now_playing = None;
            self.queue_idx = None;
            return;
        }
        let blocked: Vec<bool> = self
            .queue
            .iter()
            .map(|s| {
                let k = format!("{}|{}", s.source, s.s_id);
                self.bad_songs.contains(&k) || !self.can_stream_song(s)
            })
            .collect();
        self.rng = self
            .rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let next_idx = playback::next_playable(n, idx, self.play_mode, &blocked, self.rng);
        match next_idx {
            Some(ni) => {
                let next_name = self.queue[ni].name.clone();
                let reason = match &why {
                    Some(w) => format!("：{w}"),
                    None => String::new(),
                };
                self.toast = Some((
                    format!(
                        "跳过无法播放: {}（{}）{reason} → 接着放 {}",
                        song.name,
                        crate::source::source_display(&song.source),
                        next_name
                    ),
                    Instant::now(),
                ));
                self.queue_idx = Some(ni);
                self.play_gen += 1;
                self.play_current(true);
            }
            None => {
                self.toast = Some((
                    crate::tr!(
                        "无法播放: {}（{}），队列里没有其它可播的曲目",
                        "Cannot play: {} ({}), no other playable track in the queue",
                        song.name,
                        crate::source::source_display(&song.source)
                    ),
                    Instant::now(),
                ));
                self.skip_streak = 0;
                self.now_playing = None;
                self.queue_idx = None;
            }
        }
    }

    /// 选中行（或正在播放那首）缺时长且源支持懒补时，问一次；只问一次
    fn lazy_duration_of_selected(&mut self) {
        let song = match self.screen {
            ScreenKind::Search => self.search.get(self.search_sel).cloned(),
            _ => self.now_playing.clone(),
        };
        let Some(song) = song else { return };
        self.lazy_duration_for(&song);
    }

    fn lazy_duration_for(&mut self, song: &Song) {
        if song.dt > 0 || !song.source.starts_with("meting:") {
            return;
        }
        let key = format!("{}|{}", song.source, song.s_id);
        if !self.duration_asked.insert(key) {
            return;
        }
        let registry = self.registry.clone();
        let tx = self.tx.clone();
        let source = song.source.clone();
        let s_id = song.s_id.clone();
        spawn(tx, move || {
            let dt = registry
                .duration_ms(&source, &crate::source::TrackId(s_id.clone()))
                .ok()
                .flatten()
                .unwrap_or(0);
            AppMsg::Duration {
                source: source.clone(),
                s_id,
                dt,
            }
        });
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
                    let key = health::album_key(a);
                    let own = a.id;
                    match self.detail_id(own, &self.agg_album_candidates, &key) {
                        Some(id) => self.open_album(id),
                        None => self.only_netease_toast("专辑"),
                    }
                }
            }
            ScreenKind::Playlists => {
                if let Some(pl) = self.playlists.get(self.playlists_sel) {
                    let key = health::playlist_key(pl);
                    let own = pl.id;
                    match self.detail_id(own, &self.agg_playlist_candidates, &key) {
                        Some(id) => self.open_playlist(id),
                        None => self.only_netease_toast("歌单"),
                    }
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
        // 源状态明细视图里 Enter 不做操作
        if self.source_view_active() {
            return;
        }
        match self.search_type {
            0 => {
                if self.search_sel < self.search.len() {
                    self.play_list(self.search.clone(), self.search_sel);
                }
            }
            1 => {
                if let Some(a) = self.search_artists.get(self.search_sel) {
                    let key = health::artist_key(a);
                    let own = a.id;
                    let name = a.name.clone();
                    match self.detail_id(own, &self.agg_artist_candidates, &key) {
                        Some(id) => self.open_artist(id, name),
                        None => self.only_netease_toast("歌手"),
                    }
                }
            }
            2 => {
                if let Some(p) = self.search_playlists.get(self.search_sel) {
                    let key = health::playlist_key(p);
                    let own = p.id;
                    match self.detail_id(own, &self.agg_playlist_candidates, &key) {
                        Some(id) => self.open_playlist(id),
                        None => self.only_netease_toast("歌单"),
                    }
                }
            }
            3 => {
                if let Some(a) = self.search_albums.get(self.search_sel) {
                    let key = health::album_key(a);
                    let own = a.id;
                    match self.detail_id(own, &self.agg_album_candidates, &key) {
                        Some(id) => self.open_album(id),
                        None => self.only_netease_toast("专辑"),
                    }
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
        spawn(tx, move || match api.album(id) {
            Ok((name, songs)) => AppMsg::AlbumDetail(Ok((name, songs))),
            Err(e) => AppMsg::AlbumDetail(Err(e.to_string())),
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

    fn play_list(&mut self, list: Vec<Song>, mut idx: usize) {
        if list.is_empty() {
            return;
        }
        let mut list = list;
        // 聚合列表里「仅搜索」的源不进播放队列，避免自动连播撞到取不到流的歌
        if self.source_id == AGGREGATE_ID {
            let cur = list.get(idx).cloned();
            list.retain(|s| self.can_stream_song(s));
            if list.is_empty() {
                self.toast = Some((
                    crate::tr!(
                        "这首来自「仅搜索」的源，取不到播放地址",
                        "This track comes from a search-only source, no playable URL"
                    ),
                    Instant::now(),
                ));
                return;
            }
            if let Some(c) = cur {
                idx = list
                    .iter()
                    .position(|s| s.source == c.source && s.s_id == c.s_id)
                    .unwrap_or(0);
            } else {
                idx = 0;
            }
        }
        self.queue = list;
        self.queue_idx = Some(idx);
        self.play_gen += 1;
        self.play_current(true); // 用户点播
        self.selected.clear();
        self.goto(ScreenKind::NowPlaying);
    }

    /// 这首歌在聚合里命中过的所有源条目，按优先级排序；非聚合模式就是它自己
    fn play_candidates(&self, song: &Song) -> Vec<Song> {
        let mut list: Vec<Song> = if self.source_id == AGGREGATE_ID {
            self.agg_candidates
                .get(&health::dedup_key(song))
                .cloned()
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        if list.is_empty() {
            list.push(song.clone());
        }
        crate::source::agg::order_candidates(&list)
    }

    /// `autoplay = true`：加载后强制播放（自动连播／用户点播）；
    /// `false`：只加载，保持当前暂停状态（手动切歌、切音质重取流）
    fn play_current(&mut self, autoplay: bool) {
        let Some(idx) = self.queue_idx else { return };
        let Some(song) = self.queue.get(idx).cloned() else {
            return;
        };
        self.now_playing = Some(song.clone());
        self.lyric = None;
        if self.anim.fade_on() {
            self.fade_start = Some(Instant::now());
        }
        let gen = self.play_gen;
        let tx = self.tx.clone();
        let quality = self.cfg.quality.clone();
        let id = song.id;
        self.lazy_duration_for(&song);
        // 聚合结果里同一首可能来自多个源：按优先级逐个试流，第一个成功的就用
        let candidates = self.play_candidates(&song);
        // 单源（不是聚合）时，主源取不到流要能跨源找同一首；聚合模式候选本来就有全网条目
        let single_source = self.source_id != AGGREGATE_ID;
        let fallback_query = format!("{} {}", song.name, crate::util::artists_of(&song.ar));
        let registry = self.registry.clone();
        let registry_probe = self.registry.clone();
        let proxy_cfg = self.cfg.proxy.clone();
        spawn(tx.clone(), move || {
            // 按优先级逐个试，逻辑在 agg::first_playable（有单测）
            let mut try_one = |cand: &crate::models::Song| -> Result<Option<String>, String> {
                let src = if cand.source.is_empty() {
                    "netease".to_string()
                } else {
                    cand.source.clone()
                };
                let track = crate::source::track_of(cand);
                registry
                    .stream_url(&src, &track, &quality)
                    .map_err(|e| e.to_string())
            };
            let (mut hit, mut failures) =
                crate::source::agg::first_playable(&candidates, &mut try_one);
            if hit.is_none() && single_source {
                // 主源失败：拿「歌名 + 歌手」去其它源补候选再试一轮
                let extra = crate::source::agg::fallback_candidates(
                    &registry_probe,
                    &candidates,
                    &fallback_query,
                );
                if !extra.is_empty() {
                    let (h2, mut f2) = crate::source::agg::first_playable(&extra, &mut try_one);
                    failures.append(&mut f2);
                    hit = h2;
                }
            }
            match hit {
                Some((u, used)) => {
                    let mpv_proxy =
                        if crate::source::needs_outbound_proxy(&used.source) {
                            crate::proxy::mpv_proxy(proxy_cfg.as_deref()).ok().flatten()
                        } else {
                            None
                        };
                    AppMsg::PlayUrl(
                        gen,
                        Some(u),
                        autoplay,
                        mpv_proxy,
                        Some((used.source.clone(), used.s_id.clone())),
                        failures,
                    )
                }
                None => AppMsg::PlayUrl(gen, None, autoplay, None, None, failures),
            }
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
        self.play_current(false); // 手动切歌：保持原暂停状态
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
            self.toast = Some((
                crate::tr!("已收藏 {} 首", "Added {} song(s) to favorites", n),
                Instant::now(),
            ));
        } else {
            self.toast = Some((
                crate::tr!("已在收藏中 (共 {} 首)", "Already in favorites ({} songs)", n),
                Instant::now(),
            ));
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
                self.toast = Some((
                crate::tr!("已取消收藏 {} 个歌单", "Removed {} playlist(s) from favorites", n),
                Instant::now(),
            ));
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
            self.toast = Some((
                crate::tr!("已从收藏移除 {} 首", "Removed {} song(s) from favorites", n),
                Instant::now(),
            ));
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
            self.toast = Some((
                crate::tr!("已加入 {} 首下载", "Added {} song(s) to downloads", added),
                Instant::now(),
            ));
        } else {
            self.toast = Some((
                crate::tr!("{} 首已在下载队列", "{} song(s) already in the download queue", n),
                Instant::now(),
            ));
        }
    }

    fn enqueue_songs(&mut self, list: Vec<Song>) {
        let n = self.downloader.enqueue(list);
        if n > 0 {
            self.toast = Some((
                crate::tr!("已加入 {} 首下载", "Added {} song(s) to downloads", n),
                Instant::now(),
            ));
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
                        self.toast = Some((crate::tr!("正在获取: {}...", "Fetching: {}...", p.name), Instant::now()));
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
                    self.toast =
                        Some((crate::tr!("正在获取歌单曲目: {}...", "Fetching playlist tracks: {}...", pl.name), Instant::now()));
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
                    self.toast = Some((crate::tr!("正在获取专辑曲目: {}...", "Fetching album tracks: {}...", a.name), Instant::now()));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_toast_points_to_the_next_action() {
        // 非网易云单源：告诉用户按 S 换源，而不是「只有网易云能打开」这种误导
        let m = detail_unsupported_msg("migu", "专辑");
        assert!(m.contains("暂不支持专辑详情"), "{m}");
        assert!(m.contains("按 S 换到网易云或聚合"), "{m}");
        // 聚合：说明是没有网易云条目，而不是源不支持
        let a = detail_unsupported_msg(AGGREGATE_ID, "歌单");
        assert!(a.contains("没有网易云的条目"), "{a}");
        assert!(!a.contains("暂不支持"), "{a}");
    }
}
