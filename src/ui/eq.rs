//! 真实频谱：起一个隐藏的 mpv 解码子进程，从 FIFO 读 PCM，算 5 个频段的能量。
//!
//! 为什么再拉一次流：频谱必须来自真实音频样本。主播放器的音频在 mpv 里，
//! 拿不到 PCM，所以另起一个 `--ao=pcm` 的解码进程；它和播放器同时开始、按真实
//! 时间输出，所以柱子和耳朵听到的是同一段。代价是多一份带宽/解码，所以只在
//! 「正在播放页」且确实在播放时运行，切页/暂停立即杀掉。
//!
//! 纯函数（goertzel / band_energies / to_levels / smooth）都能单测；
//! 进程部分（`PcmAnalyzer`）负责 FIFO、读线程和清理孤儿。

// 频段那套（Goertzel / 16 段能量 / Agc 标定 / 峰值保持 / 合并）暂时不参与渲染：
// 现在的波形走「窗内峰值」通路。按 Chief 要求保留，换点阵面积或柱状样式时直接接回来，
// 所以这里允许 dead_code（它们都有单测覆盖）。
#![allow(dead_code)]

use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use crate::ui::spectrum::{HeightCfg, MAX_HEIGHT};

/// 频段中心频率：对数等距分布（60Hz–3.5kHz，8kHz 采样下不越奈奎斯特）
pub fn band_centers(n: usize) -> Vec<f64> {
    let (lo, hi) = (60.0f64, 3500.0f64);
    let n = n.max(1);
    if n == 1 {
        return vec![lo];
    }
    (0..n)
        .map(|i| lo * (hi / lo).powf(i as f64 / (n - 1) as f64))
        .collect()
}
/// 分析段数：固定算 16 段，UI 按终端宽度合并成 12 / 8
pub const BANDS: usize = 16;
/// 分析窗口：8kHz 下 256 点 = 32ms（≈31 窗/秒，和 30fps 对齐）
pub const WINDOW: usize = 256;
/// 每窗时长（微秒）：按这个节拍读 FIFO，给解码器限速（ao=pcm 本身是全速解码）
pub const WINDOW_MICROS: u64 = WINDOW as u64 * 1_000_000 / SAMPLE_RATE as u64;
/// 解码采样率：5 个频段到 2kHz 就够，8k 省带宽/CPU
pub const SAMPLE_RATE: f64 = 8000.0;

/// Goertzel：单频点能量，O(N)，比整段 FFT 便宜
pub fn goertzel(samples: &[i16], sr: f64, freq: f64) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let n = samples.len() as f64;
    let k = (freq / sr) * n;
    let w = 2.0 * std::f64::consts::PI * k / n;
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &x in samples {
        let s0 = x as f64 / 32768.0 + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    let mag = (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0).sqrt();
    (mag / n) as f32
}

/// N 个频段的能量（对数等距）
pub fn band_energies(samples: &[i16], sr: f64, n: usize) -> Vec<f32> {
    band_centers(n)
        .iter()
        .map(|f| goertzel(samples, sr, *f))
        .collect()
}

/// 自动增益：只标定「整首歌的响度上限」，映射本身是固定的 70dB 线性刻度。
///
/// 之前的失败教训：
/// 之前两种做法都失败过：缩到 p10–p95 会饱和（音乐各频段长期贴在 -20..-5dB）；
/// 逐段百分位等于自适应均衡，稳态信号永远骑在自己分布顶端，也饱和。
/// 所以只让 AGC 决定「天花板在哪」（最近 ~10 秒各频段 dB 的 p95 + 10dB 余量），
/// 再按固定 70dB 动态范围往下铺，安静段自然落到 1–3 档。
#[allow(dead_code)] // 保留：固定刻度映射的标定器，换样式时用来定 dB→高度
#[derive(Debug, Clone)]
pub struct Agc {
    /// 各频段 dB 的衰减直方图（16 × 80 bin）
    counts: Vec<[f32; AGC_BINS]>,
    total: f32,
    /// 当前天花板（dB）
    ceil: Option<f32>,
    last_update: u64,
    enabled: bool,
}

pub const AGC_BINS: usize = 80; // -80..0 dB
/// 映射动态范围（dB）：天花板往下铺这么多
pub const AGC_SPAN_DB: f32 = 70.0;
pub const AGC_STEP_MS: u64 = 1_000;

fn db_bin(db: f32) -> u8 {
    ((db + 80.0).clamp(0.0, 79.0)) as u8
}

impl Agc {
    pub fn new(enabled: bool) -> Self {
        Agc {
            counts: vec![[0.0; AGC_BINS]; BANDS],
            total: 0.0,
            ceil: None,
            last_update: 0,
            enabled,
        }
    }

    pub fn observe(&mut self, energies: &[f32], now_ms: u64) {
        if !self.enabled {
            return;
        }
        for (i, e) in energies.iter().enumerate().take(BANDS) {
            let bin = db_bin(20.0 * e.max(1e-9).log10()) as usize;
            for c in self.counts[i].iter_mut() {
                *c *= 0.995;
            }
            self.counts[i][bin] += 1.0;
        }
        self.total = self.total * 0.995 + 1.0;
        if now_ms.saturating_sub(self.last_update) >= AGC_STEP_MS && self.total > 20.0 {
            self.last_update = now_ms;
            // 全频段合计的 p95
            let mut counts = [0.0f32; AGC_BINS];
            for band in &self.counts {
                for (i, c) in band.iter().enumerate() {
                    counts[i] += c;
                }
            }
            let total: f32 = counts.iter().sum();
            if total < 100.0 {
                return;
            }
            let mut acc = 0.0f32;
            for (i, c) in counts.iter().enumerate() {
                acc += c;
                if acc >= total * 0.95 {
                    self.ceil = Some(i as f32 - 80.0 + 10.0); // p95 + 10dB 余量
                    break;
                }
            }
        }
    }

    /// dB → 高度（0..=7 连续，量化交给渲染器）：天花板由 AGC 定，关掉时用 -70..0
    pub fn height(&self, _band: usize, db: f32) -> f32 {
        HeightCfg {
            floor: self.floor(),
            span: AGC_SPAN_DB,
            gamma: 1.0,
        }
        .height(db)
    }

    /// 当前映射的下界（dB）
    pub fn floor(&self) -> f32 {
        match self.ceil {
            Some(c) if self.enabled => c - AGC_SPAN_DB,
            _ => -AGC_SPAN_DB,
        }
    }
}

/// attack 0.65 / release 0.12：起得快、落得慢，看着才像频谱
pub const ATTACK: f32 = 0.65;
pub const RELEASE: f32 = 0.12;

/// 峰值下落速度（高度/秒）：满格约 2 秒落完，匀速
pub const PEAK_FALL_PER_S: f32 = MAX_HEIGHT / 2.0;

/// 峰值保持：`hold_ms` 内不动，之后按时间匀速下落（与帧率无关）
#[allow(dead_code)] // 保留：峰值保持（点阵/柱状样式要用）
#[derive(Debug, Clone)]
pub struct Peaks {
    pub values: Vec<f32>,
    pub held_at: Vec<u64>,
    last_ms: u64,
}

impl Peaks {
    pub fn new(n: usize) -> Self {
        Peaks {
            values: vec![0.0; n],
            held_at: vec![0; n],
            last_ms: 0,
        }
    }
    /// `now_ms` 传单调时钟毫秒；`hold_ms` = 500
    pub fn update(&mut self, levels: &[f32], now_ms: u64, hold_ms: u64) -> Vec<f32> {
        if self.values.len() != levels.len() {
            *self = Peaks::new(levels.len());
        }
        let dt = now_ms.saturating_sub(self.last_ms) as f32 / 1000.0;
        self.last_ms = now_ms;
        for (i, lv) in levels.iter().enumerate() {
            if *lv >= self.values[i] {
                self.values[i] = *lv;
                self.held_at[i] = now_ms;
            } else if now_ms.saturating_sub(self.held_at[i]) >= hold_ms {
                self.values[i] = (self.values[i] - PEAK_FALL_PER_S * dt).max(0.0);
            }
        }
        self.values.clone()
    }
}

/// 一帧频谱：当前高度 + 峰值高度（0..=7 连续值，量化交给渲染器）。
///
/// 暂时不用：现在的波形走「窗内峰值」通路；换点阵面积/柱状样式时可以直接接回来。
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct EqFrame {
    pub heights: Vec<f32>,
    pub peaks: Vec<f32>,
}

/// FIFO 放 $XDG_RUNTIME_DIR/zmusic（没有就退回 /tmp），比 /tmp 干净、随登录会话回收
fn eq_dir() -> PathBuf {
    std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|d| !d.is_empty())
        .map(|d| PathBuf::from(d).join("zmusic"))
        .unwrap_or_else(|| std::env::temp_dir().join("zmusic"))
}

/// 启动时清掉自己前缀的残留 FIFO（上次被 SIGKILL 之类留下的）
fn sweep_stale(dir: &std::path::Path) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with("eq-") && name.ends_with(".pcm") {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// 波形样本（对称波形的每点列幅度，0..=1）与单调序号
#[derive(Debug, Default)]
struct WaveBuf {
    samples: Vec<f32>,
    seq: u64,
}

/// 自动增益：把这首歌的 p90 窗峰值映射到 8/9 点（留 1 点余量防削顶）
const AUTO_TARGET_FRAC: f32 = 8.0 / 9.0;
/// 扫描没回来前的中性基准（-12 dBFS），避免一开始就冲顶
pub const NEUTRAL_REF_DB: f32 = -12.0;
/// 从 ao=pcm 写出的 WAV 里读每窗峰值（16bit 单声道，8kHz）
fn wav_window_peaks(bytes: &[u8], window: usize) -> Vec<f32> {
    // 跳过 RIFF 头：找 "data" 块
    let mut off = 0usize;
    if bytes.len() > 12 && &bytes[0..4] == b"RIFF" {
        let mut i = 12;
        while i + 8 <= bytes.len() {
            let id = &bytes[i..i + 4];
            let size = u32::from_le_bytes([bytes[i + 4], bytes[i + 5], bytes[i + 6], bytes[i + 7]])
                as usize;
            if id == b"data" {
                off = i + 8;
                break;
            }
            i += 8 + size + (size & 1);
        }
    }
    let pcm = &bytes[off..];
    let mut out = Vec::new();
    let mut i = 0;
    while i + window * 2 <= pcm.len() {
        let mut peak = 0.0f32;
        for k in 0..window {
            let v = i16::from_le_bytes([pcm[i + k * 2], pcm[i + k * 2 + 1]]) as f32 / 32768.0;
            peak = peak.max(v.abs());
        }
        out.push(peak);
        i += window * 2;
    }
    out
}

/// 波形纵向标定：**窗内峰值 + 固定基准**，基准只以极慢速度微调。
///
/// 为什么不用「近 10 秒 p95 / 峰值保持」做归一化：那等于每首歌自己归一化，
/// 实测安静前奏 p50=0.40、副歌 p50=0.42（几乎一样），动态全被吃掉。
/// 固定基准（-9 dBFS，约等于流行乐副歌峰值）时实测安静 p50=0.09、副歌 p50=0.76，
/// 这才是「安静一条线、副歌占满」。基准只按 0.1 dB/s 往下漂（下限 -22 dBFS），
/// 一首歌的时间尺度内基本不动，所以不会出现「整排忽高忽低」。
/// 波形标定参数（来自 [ui] 段）
#[derive(Debug, Clone, Copy)]
pub struct WaveCfg {
    /// 手动满高基准（dBFS），`autogain` 关掉时用
    pub reference_dbfs: f32,
    /// 用户倍率（自动增益之上再乘一次）
    pub gain: f32,
    /// 软限幅：tanh 压缩，响的地方挤向顶部但不硬切
    pub soft_clip: bool,
    /// 逐曲自动增益（默认开）
    pub autogain: bool,
}

impl Default for WaveCfg {
    fn default() -> Self {
        WaveCfg {
            reference_dbfs: -6.0,
            gain: 1.0,
            soft_clip: true,
            autogain: true,
        }
    }
}

struct WaveAgc {
    ref_db: f32,
    gain: f32,
    soft_clip: bool,
    /// 自动模式：用已经在解码的这条流的峰值边放边估
    autogain: bool,
    /// 滚动窗口里的窗峰值（dB），约 15 秒
    peaks: Vec<f32>,
    /// 距离上次重算的秒数
    acc: f32,
    /// 缓存命中时不需要再估
    frozen: bool,
    /// 当前估计的 p90（dB），交给界面写进缓存
    done_p90: Arc<Mutex<Option<f32>>>,
}

/// 软限幅拐点：以下线性，以上 tanh
const SOFT_KNEE: f32 = 0.75;
/// 响度估计：攒够 1 秒才开始算（约 31 窗），攒满 15 秒冻结
const EST_MIN_WINDOWS: usize = 31;
const EST_WINDOWS: usize = 470;
/// 上行快（副歌来了立刻抬，别削顶）、下行慢（不呼吸）
const AUTO_UP_DB_PER_S: f32 = 1.5;
const AUTO_DOWN_DB_PER_S: f32 = 0.15;

/// 手动模式的基准下漂速度（dB/秒）
const WAVE_REF_DECAY_DB_PER_S: f32 = 0.1;
/// 手动模式的基准下限（约 -22 dBFS）：再安静也不放大底噪
const WAVE_REF_FLOOR_DB: f32 = -22.0;
/// 手动模式的基准上限
const WAVE_REF_CAP_DB: f32 = -3.0;
/// 冷启动基准（可被 [ui] eq_reference_dbfs 覆盖，默认 -6 dBFS）
const WAVE_REF_INIT_DB: f32 = -6.0;

impl WaveAgc {
    /// `p90_db`：缓存里这首歌的响度代表；None = 第一次听，边放边估
    fn new(cfg: WaveCfg, p90_db: Option<f32>, done_p90: Arc<Mutex<Option<f32>>>) -> Self {
        let auto_ref = p90_db.filter(|p| p.is_finite()).map(|p90| {
            // 让 p90 落在 8/9 点：硬切 x = 8/9；分段软限幅反解 0.75+0.25*tanh(t)=8/9
            let k = if cfg.soft_clip {
                SOFT_KNEE
                    + (1.0 - SOFT_KNEE)
                        * ((AUTO_TARGET_FRAC - SOFT_KNEE) / (1.0 - SOFT_KNEE)).atanh()
            } else {
                AUTO_TARGET_FRAC
            };
            (p90 + 20.0 * (cfg.gain.clamp(0.1, 8.0) / k).log10()).clamp(-60.0, 0.0)
        });
        WaveAgc {
            ref_db: match (cfg.autogain, auto_ref) {
                (true, Some(r)) => r,
                (true, None) => NEUTRAL_REF_DB,
                (false, _) => {
                    if cfg.reference_dbfs.is_finite() {
                        cfg.reference_dbfs.clamp(-60.0, -3.0)
                    } else {
                        WAVE_REF_INIT_DB
                    }
                }
            },
            gain: cfg.gain.clamp(0.1, 8.0),
            soft_clip: cfg.soft_clip,
            autogain: cfg.autogain,
            peaks: Vec::with_capacity(512),
            acc: 0.0,
            frozen: auto_ref.is_some(),
            done_p90,
        }
    }

    /// 自动模式：用**已经在解码的这条流**的窗峰值做滚动 15 秒 p90，边放边估。
    ///
    /// 上行快（1.5 dB/s）、下行很慢（0.15 dB/s）：安静前奏先把基准压低没关系，
    /// 副歌一响就快速抬上去（避免削顶）；反过来变安静时不会突然「呼吸」。
    /// 缓存命中（第二次听同一首）就直接用，不再估。
    fn observe(&mut self, peak_db: f32, dt_s: f32) {
        if self.autogain {
            if self.frozen {
                return;
            }
            self.peaks.push(peak_db);
            if self.peaks.len() > EST_WINDOWS {
                self.peaks.remove(0);
            }
            self.acc += dt_s;
            if self.peaks.len() < EST_MIN_WINDOWS || self.acc < 1.0 {
                return;
            }
            let elapsed = self.acc;
            self.acc = 0.0;
            let mut v = self.peaks.clone();
            v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let idx = ((v.len() as f32 * 0.9) as usize).min(v.len() - 1);
            let p90 = v[idx];
            let k = if self.soft_clip {
                SOFT_KNEE
                    + (1.0 - SOFT_KNEE)
                        * ((AUTO_TARGET_FRAC - SOFT_KNEE) / (1.0 - SOFT_KNEE)).atanh()
            } else {
                AUTO_TARGET_FRAC
            };
            let target = (p90 + 20.0 * (self.gain / k).log10()).clamp(-60.0, 0.0);
            // 目标比当前低（歌更响）→ 下行慢；目标更高（歌更轻）→ 上行快
            let rate = if target < self.ref_db {
                AUTO_DOWN_DB_PER_S
            } else {
                AUTO_UP_DB_PER_S
            };
            let step = rate * elapsed;
            self.ref_db = if self.ref_db < target {
                (self.ref_db + step).min(target)
            } else {
                (self.ref_db - step).max(target)
            };
            *self.done_p90.lock().unwrap() = Some(p90);
            return;
        }
        if peak_db > self.ref_db {
            self.ref_db = peak_db.min(WAVE_REF_CAP_DB);
        } else {
            self.ref_db = (self.ref_db - WAVE_REF_DECAY_DB_PER_S * dt_s).max(WAVE_REF_FLOOR_DB);
        }
    }

    /// 自动增益给这首歌加了多少 dB（相对中性基准）
    fn auto_gain_db(&self) -> f32 {
        self.ref_db - NEUTRAL_REF_DB
    }

    fn ref_db(&self) -> f32 {
        self.ref_db
    }

    /// 是否已经算出真正的标定（刚重启时是中性基准，别拿去显示）
    fn calibrated(&self) -> bool {
        (self.ref_db - NEUTRAL_REF_DB).abs() > 0.05
    }

    fn amp(&self, peak: f32) -> f32 {
        // 线性映射：增益直接作用在幅度上（之前多叠一个 powf(0.9) 会把峰值压低半档）
        let x = (peak / 10f32.powf(self.ref_db / 20.0)) * self.gain;
        if self.soft_clip {
            // 分段软限幅：拐点 0.75 以下保持线性（中段不被抬高，p50 才能落在 4-6/9），
            // 超过拐点用 tanh 挤向 1.0——响处不硬切成色块，平顶接近 0
            if x <= SOFT_KNEE {
                x.max(0.0)
            } else {
                let t = ((x - SOFT_KNEE) / (1.0 - SOFT_KNEE)).tanh();
                (SOFT_KNEE + (1.0 - SOFT_KNEE) * t).clamp(0.0, 1.0)
            }
        } else {
            x.clamp(0.0, 1.0)
        }
    }
}

pub struct PcmAnalyzer {
    child: Child,
    fifo: PathBuf,
    /// 起解码时的播放位置（秒）：界面据此发现「跳转/拖动」后重启解码
    start: f64,
    /// 解码开始时刻：据此推算它「现在该播到哪」，才能区分「正常前进」和「用户拖动」
    started: std::time::Instant,
    wave: Arc<Mutex<WaveBuf>>,
    /// 解码用的流地址（换歌要重建，不然波形还在画上一首）
    url: String,
    /// 当前自动增益（dB，相对中性基准），给界面显示
    auto_gain_db: Arc<Mutex<f32>>,
    /// 边放边估出来的 p90（收敛后才有值），界面据此写缓存
    est_p90: Arc<Mutex<Option<f32>>>,
    /// 读线程用的结束旗标（Drop 时置位，避免线程卡在阻塞读上）
    stop: Arc<Mutex<bool>>,
}

impl PcmAnalyzer {
    /// 起解码进程。`start_secs` 用来和播放器对齐（从当前播放位置开始解）。
    /// `p90_db`：这首歌的响度代表（App 层扫好传进来，拖动重开时传同一个值）
    pub fn start(
        url: &str,
        start_secs: f64,
        wave_cfg: WaveCfg,
        p90_db: Option<f32>,
    ) -> Option<PcmAnalyzer> {
        let dir = eq_dir();
        let _ = std::fs::create_dir_all(&dir);
        sweep_stale(&dir);
        let fifo = dir.join(format!("eq-{}.pcm", std::process::id()));
        let _ = std::fs::remove_file(&fifo);
        // 用 mkfifo 造管道：mpv 写、我们读，流式不落盘
        let ok = Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return None;
        }
        let child = Command::new("mpv")
            .args([
                "--no-video",
                "--no-terminal",
                "--really-quiet",
                "--ao=pcm",
                &format!("--ao-pcm-file={}", fifo.display()),
                "--audio-format=s16",
                &format!("--audio-samplerate={}", SAMPLE_RATE as u32),
                "--audio-channels=mono",
                &format!("--start={start_secs:.2}"),
                url,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;

        let wave = Arc::new(Mutex::new(WaveBuf::default()));
        let auto_gain_db = Arc::new(Mutex::new(0.0f32));
        let est_p90: Arc<Mutex<Option<f32>>> = Arc::new(Mutex::new(None));
        let stop = Arc::new(Mutex::new(false));
        {
            let wave = wave.clone();
            let auto_gain_cell = auto_gain_db.clone();
            let est_cell = est_p90.clone();
            let stop = stop.clone();
            let path = fifo.clone();
            std::thread::spawn(move || {
                let Ok(mut f) = std::fs::File::open(&path) else {
                    return;
                };
                // 已拿到 fd：立刻 unlink，靠 fd 继续读写。进程被强杀也不会留残留。
                let _ = std::fs::remove_file(&path);
                let mut buf = vec![0u8; WINDOW * 2];
                let mut wagc = WaveAgc::new(wave_cfg, p90_db, est_cell.clone());
                let scan_db_for_dump: Option<f32> = p90_db;
                let gain_cell = auto_gain_cell.clone();
                // 调试用：把每窗幅度写成 CSV（ms,amp,rms_db），默认不写
                let mut dump = std::env::var("ZMUSIC_WAVE_DUMP").ok().and_then(|p| {
                    std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(p)
                        .ok()
                });
                let t_start = std::time::Instant::now();
                let mut next = std::time::Instant::now();
                loop {
                    if *stop.lock().unwrap() {
                        break;
                    }
                    // 逐窗读满：mpv 写满管道就阻塞，解码节奏由我们的读取速度决定
                    if f.read_exact(&mut buf).is_err() {
                        break;
                    }
                    let mut samples = [0i16; WINDOW];
                    if let Some(chunks) = buf.as_chunks::<2>().0.get(..WINDOW) {
                        for (i, c) in chunks.iter().enumerate() {
                            samples[i] = i16::from_le_bytes(*c);
                        }
                    }
                    let now = t_start.elapsed().as_millis() as u64;
                    // 波形：这一窗的**峰值** → 0..=1 幅度，压进共享缓冲
                    // 用峰值不用 RMS：RMS 被整段平均，峰谷差被磨平，波形看起来像一条带
                    let peak = samples
                        .iter()
                        .map(|x| (*x as f32 / 32768.0).abs())
                        .fold(0.0f32, f32::max);
                    wagc.observe(
                        20.0 * peak.max(1e-9).log10(),
                        WINDOW as f32 / SAMPLE_RATE as f32,
                    );
                    if let Some(f) = dump.as_mut() {
                        use std::io::Write;
                        let scan = scan_db_for_dump;
                        let _ = writeln!(
                            f,
                            "{},{:.4},{:.2},{:.1},{:.1}",
                            now,
                            wagc.amp(peak),
                            20.0 * peak.max(1e-9).log10(),
                            wagc.auto_gain_db(),
                            scan.unwrap_or(-99.0)
                        );
                    }
                    *gain_cell.lock().unwrap() = wagc.auto_gain_db();
                    {
                        let mut w = wave.lock().unwrap();
                        w.samples.push(wagc.amp(peak));
                        w.seq += 1;
                        if w.samples.len() > 2048 {
                            w.samples.drain(..1024);
                        }
                    }
                    // 按实时节拍走：每窗 32ms
                    next += std::time::Duration::from_micros(WINDOW_MICROS);
                    let now = std::time::Instant::now();
                    if next > now {
                        std::thread::sleep(next - now);
                    } else {
                        next = now;
                    }
                }
            });
        }
        Some(PcmAnalyzer {
            child,
            fifo,
            start: start_secs,
            started: std::time::Instant::now(),
            wave,
            url: url.to_string(),
            auto_gain_db,
            est_p90,
            stop,
        })
    }

    /// 这个解码器用的流地址
    pub fn url(&self) -> &str {
        &self.url
    }

    /// 当前逐曲自动增益（dB，相对中性基准）：界面上给用户看「这首歌被调了多少」
    pub fn auto_gain_db(&self) -> f32 {
        *self.auto_gain_db.lock().unwrap()
    }

    /// 这首歌估出来的 p90（dB，收敛后才有）：界面拿去缓存，下次直接命中
    pub fn estimated_p90(&self) -> Option<f32> {
        *self.est_p90.lock().unwrap()
    }

    /// 是否已算出真正的标定（刚起解码器时还是中性基准）
    pub fn calibrated(&self) -> bool {
        self.auto_gain_db().abs() > 0.05
    }

    /// 当前满高基准（dBFS）：拖动重开解码器时沿用，不重新估
    pub fn ref_db(&self) -> f32 {
        self.auto_gain_db() + NEUTRAL_REF_DB
    }

    /// 最近 `max` 个波形样本 + 单调序号（界面据此只取新增的部分）
    pub fn wave(&self, max: usize) -> (Vec<f32>, u64) {
        let w = self.wave.lock().unwrap();
        let n = w.samples.len().min(max);
        (w.samples[w.samples.len() - n..].to_vec(), w.seq)
    }

    /// 解码进程是否还活着（死了要降级，不能一直读旧值）
    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for PcmAnalyzer {
    fn drop(&mut self) {
        *self.stop.lock().unwrap() = true;
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.fifo);
    }
}

/// 把 16 段按宽度合并成 `target` 段：每组取最大值，保证峰值不丢
#[allow(dead_code)] // 保留：多段样式合并用
pub fn merge_bands(src: &[f32], target: usize) -> Vec<f32> {
    let n = src.len();
    if target >= n || target == 0 {
        return src.to_vec();
    }
    let mut out = Vec::with_capacity(target);
    for i in 0..target {
        let lo = i * n / target;
        let hi = ((i + 1) * n / target).max(lo + 1).min(n);
        out.push(src[lo..hi].iter().copied().fold(0.0f32, f32::max));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::spectrum;

    fn sine(freq: f64, sr: f64, n: usize, amp: f64) -> Vec<i16> {
        (0..n)
            .map(|i| {
                (amp * (2.0 * std::f64::consts::PI * freq * i as f64 / sr).sin() * 32767.0) as i16
            })
            .collect()
    }

    #[test]
    fn wav_window_peaks_skips_header_and_reads_peaks() {
        // 44 字节 RIFF/WAVE/fmt 头 + "data" 块：两窗，第二窗更响
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 16]);
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&8u32.to_le_bytes()); // 4 个 i16
        for v in [1000i16, -2000, 16000, -32768] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        let peaks = wav_window_peaks(&bytes, 2);
        assert_eq!(peaks.len(), 2);
        assert!((peaks[0] - 2000.0 / 32768.0).abs() < 1e-6);
        assert!((peaks[1] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn autogain_maps_p90_to_eight_ninths() {
        let db = |x: f32| 10f32.powf(x / 20.0);
        // 软限幅（默认）：p90 落在 8/9 附近，更响的被压到接近但不越过 1
        let cfg = WaveCfg {
            autogain: true,
            soft_clip: true,
            gain: 1.0,
            reference_dbfs: -6.0,
        };
        let cell = Arc::new(Mutex::new(None));
        let agc = WaveAgc::new(cfg, Some(-20.0), cell.clone());
        let amp = agc.amp(db(-20.0));
        assert!((amp * 9.0 - 8.0).abs() < 0.2, "p90 应落在 8/9: {amp}");
        assert!(agc.amp(1.0) <= 1.0);
        assert!(agc.amp(db(-50.0)) < 0.15, "安静段要接近 0");

        // 硬切：p90 精确落在 8/9
        let hard = WaveAgc::new(
            WaveCfg {
                soft_clip: false,
                ..cfg
            },
            Some(-20.0),
            cell,
        );
        assert!((hard.amp(db(-20.0)) * 9.0 - 8.0).abs() < 0.05);
        // 更响的削到 1，不会超过
        assert_eq!(hard.amp(1.0), 1.0);
    }

    #[test]
    fn incremental_estimate_rises_fast_and_falls_slow() {
        let cfg = WaveCfg {
            autogain: true,
            soft_clip: true,
            gain: 1.0,
            reference_dbfs: -6.0,
        };
        let cell = Arc::new(Mutex::new(None));
        let mut agc = WaveAgc::new(cfg, None, cell.clone());
        assert!((agc.ref_db - NEUTRAL_REF_DB).abs() < 0.01, "第一次听先用中性基准");

        // 安静段：基准慢慢往下让（慢）
        let start = agc.ref_db;
        for _ in 0..470 {
            agc.observe(-30.0, 0.032);
        }
        let low = agc.ref_db;
        assert!(low < start, "安静段基准应该下移: {start} -> {low}");

        // 副歌来了：基准快速上抬，比下移快得多
        for _ in 0..470 {
            agc.observe(-3.0, 0.032);
        }
        assert!(agc.ref_db > low + 5.0, "响段要快速抬上去: {low} -> {}", agc.ref_db);
        assert!(cell.lock().unwrap().is_some(), "要回传估出来的 p90");
    }

    #[test]
    fn band_centers_are_log_spaced() {
        let c = band_centers(16);
        assert_eq!(c.len(), 16);
        assert!(c[0] >= 60.0 && *c.last().unwrap() <= 3500.0);
        let r1 = c[1] / c[0];
        let r2 = c[15] / c[14];
        assert!((r1 - r2).abs() < 0.01);
    }

    #[test]
    fn goertzel_peaks_at_the_right_frequency() {
        let s = sine(200.0, SAMPLE_RATE, WINDOW, 0.8);
        assert!(goertzel(&s, SAMPLE_RATE, 200.0) > goertzel(&s, SAMPLE_RATE, 900.0) * 5.0);
    }

    #[test]
    fn band_energies_follow_the_tone() {
        let low = band_energies(&sine(80.0, SAMPLE_RATE, WINDOW, 0.9), SAMPLE_RATE, 16);
        assert!(
            *low.first().unwrap() > *low.last().unwrap() * 3.0,
            "{low:?}"
        );
        let high = band_energies(&sine(2500.0, SAMPLE_RATE, WINDOW, 0.9), SAMPLE_RATE, 16);
        let peak = high
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i)
            .unwrap();
        assert!(peak >= 12, "{high:?}");
    }

    #[test]
    fn smooth_attacks_fast_and_releases_slow() {
        let up = spectrum::smooth(&[0.0; 4], &[7.0; 4], ATTACK, RELEASE);
        let down = spectrum::smooth(&[7.0; 4], &[0.0; 4], ATTACK, RELEASE);
        assert!((up[0] - 4.55).abs() < 0.6, "{up:?}");
        assert!((down[0] - 6.16).abs() < 0.6, "{down:?}");
    }

    #[test]
    fn peaks_hold_then_decay() {
        let mut p = Peaks::new(4);
        assert_eq!(p.update(&[3.0; 4], 0, 500), vec![3.0; 4]);
        // 保持期内不动
        assert_eq!(p.update(&[0.0; 4], 300, 500), vec![3.0; 4]);
        // 过期后按时间匀速下落（3.5/秒），不是按帧
        let a = p.update(&[0.0; 4], 600, 500)[0];
        assert!((a - 1.95).abs() < 0.2, "{a}");
        let b = p.update(&[0.0; 4], 700, 500)[0];
        assert!((a - b - 0.35).abs() < 0.05, "{a} {b}");
        // 一直掉到 0，不会变负
        let c = p.update(&[0.0; 4], 5000, 500)[0];
        assert_eq!(c, 0.0);
    }

    #[test]
    fn merge_bands_keeps_peaks() {
        let src: Vec<f32> = (0..16).map(|i| i as f32).collect();
        let m8 = merge_bands(&src, 8);
        assert_eq!(m8.len(), 8);
        assert_eq!(m8[0], 1.0);
        assert_eq!(*m8.last().unwrap(), 15.0);
        assert_eq!(merge_bands(&src, 32), src);
    }

    /// 逐段百分位：同一频段「低于常态」→ 低档，「高于常态」→ 高档
    #[test]
    fn agc_percentile_spreads_quiet_and_loud() {
        let mut agc = Agc::new(true);
        let quiet = 0.002f32;
        let loud = 0.4f32;
        // 交替喂 20 轮，形成宽分布（像真实音乐那样有强有弱）
        for _ in 0..20 {
            for _ in 0..10 {
                agc.observe(&[quiet; BANDS], 0);
            }
            for _ in 0..10 {
                agc.observe(&[loud; BANDS], 0);
            }
        }
        let q = agc.height(3, 20.0 * quiet.log10());
        let l = agc.height(3, 20.0 * loud.log10());
        assert!(q <= 3.0, "安静段应该低：{q}");
        assert!(l >= 5.0, "响段应该高：{l}");
        assert!(l - q >= 2.0, "差值太小：{q} vs {l}");
    }

    #[test]
    fn agc_disabled_uses_fixed_mapping() {
        let mut agc = Agc::new(false);
        let e = [0.05f32; BANDS];
        let mut t = 0;
        while t < 5_000 {
            agc.observe(&e, t);
            t += 32;
        }
        // 关掉 AGC：固定 -70..0，-70 → 0、0 → 7
        assert_eq!(agc.height(0, -70.0), 0.0);
        assert_eq!(agc.height(0, 0.0), 7.0);
    }
}
