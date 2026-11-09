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
    /// 自动增益（dB）：作用在「原始峰值 + gain」上，再用绝对刻度映射到 0..9
    gain_db: f32,
    /// 用户倍率（叠在归一化值上）
    user_gain: f32,
    soft_clip: bool,
    autogain: bool,
    /// 手动模式的天花板（dBFS）
    ceiling_db: f32,
    /// 全曲累计的窗峰值（dB）
    peaks: Vec<f32>,
    /// 距离上次重算的秒数
    acc: f32,
    /// 这首歌已经播了多少秒（置信度爬坡用）
    played: f32,
    /// 平滑后的 p90（dB）
    p90_est: f32,
    /// 缓存命中时不需要再估
    frozen: bool,
    /// 当前估计的 p90（dB），交给界面写缓存
    done_p90: Arc<Mutex<Option<f32>>>,
}

/// 软限幅拐点：以下线性，以上 tanh
const SOFT_KNEE: f32 = 0.75;
/// 映射窗口（dB）：天花板往下 24 dB 是 0 档
const SPAN_DB: f32 = 24.0;
/// 自动增益的目标：整曲 p90 落在 6.5/9 档（留 2.5 档给歌内动态）
const AUTO_TARGET_LEVEL: f32 = 6.0;
/// 目标 p90 对应到「相对天花板的 dB」：p90 + gain 要落在 -6.67 dB
/// （(x+24)/24 = 6.5/9 → x = -6.67）
const AUTO_TARGET_POST_DB: f32 = -SPAN_DB + SPAN_DB * AUTO_TARGET_LEVEL / 9.0;
/// p90 的精细段平滑速度：差距缩到 1 dB 以内后慢慢贴合，避免听感抖动
const P90_SLEW_FINE_DB_PER_S: f32 = 0.3;
/// p90 的纠正段速度：差得多时允许更快，否则整曲 p90 在副歌后抬起来，
/// 增益要几十秒才降得下来，一首歌里波动十几 dB
const P90_SLEW_FAST_DB_PER_S: f32 = 0.6;
/// 差距超过这个值走纠正段速度
const P90_FAST_GAP_DB: f32 = 3.0;
/// 差距缩到这个值以内走精细段速度
const P90_FINE_GAP_DB: f32 = 1.0;

/// 按当前差距挑平滑速度：>3 dB 快（0.6），≤1 dB 慢（0.3），中间线性过渡
fn p90_slew_rate(gap: f32) -> f32 {
    let g = gap.abs();
    if g >= P90_FAST_GAP_DB {
        P90_SLEW_FAST_DB_PER_S
    } else if g <= P90_FINE_GAP_DB {
        P90_SLEW_FINE_DB_PER_S
    } else {
        let t = (g - P90_FINE_GAP_DB) / (P90_FAST_GAP_DB - P90_FINE_GAP_DB);
        P90_SLEW_FINE_DB_PER_S + (P90_SLEW_FAST_DB_PER_S - P90_SLEW_FINE_DB_PER_S) * t
    }
}
/// 增益的置信度爬坡：前 15 秒最多 ±2 dB，15–45 秒 ±4 dB，之后放开到硬钳位
fn gain_limits(played: f32) -> (f32, f32) {
    if played < 15.0 {
        (-2.0, 2.0)
    } else if played < 45.0 {
        (-4.0, 4.0)
    } else {
        (GAIN_MIN_DB, GAIN_MAX_DB)
    }
}

/// 增益硬钳位：热母带要能压下去（-12），冷母带 +6 够用
const GAIN_MIN_DB: f32 = -12.0;
const GAIN_MAX_DB: f32 = 6.0;

/// 攒够 1 秒才开始估
const EST_MIN_WINDOWS: usize = 31;
/// 全曲累计上限（约 10 分钟，防长播内存涨）
const EST_MAX_WINDOWS: usize = 20000;

/// 手动模式的基准下漂速度（dB/秒）
const WAVE_REF_DECAY_DB_PER_S: f32 = 0.1;
/// 手动模式的基准下限（约 -22 dBFS）
const WAVE_REF_FLOOR_DB: f32 = -22.0;
/// 手动模式的基准上限
const WAVE_REF_CAP_DB: f32 = -3.0;
/// 默认天花板（可被 [ui] eq_reference_dbfs 覆盖）
const WAVE_REF_INIT_DB: f32 = -6.0;

fn approach(cur: f32, target: f32, step: f32) -> f32 {
    if cur < target {
        (cur + step).min(target)
    } else {
        (cur - step).max(target)
    }
}

impl WaveAgc {
    /// `p90_db`：缓存里这首歌的响度代表；None = 第一次听，边放边估
    fn new(cfg: WaveCfg, p90_db: Option<f32>, done_p90: Arc<Mutex<Option<f32>>>) -> Self {
        let cached = p90_db.filter(|p| p.is_finite());
        let ceiling = if cfg.reference_dbfs.is_finite() {
            cfg.reference_dbfs.clamp(-60.0, -3.0)
        } else {
            WAVE_REF_INIT_DB
        };
        WaveAgc {
            gain_db: cached
                .map(|p| (AUTO_TARGET_POST_DB - p).clamp(GAIN_MIN_DB, GAIN_MAX_DB))
                .unwrap_or(0.0),
            user_gain: cfg.gain.clamp(0.1, 8.0),
            soft_clip: cfg.soft_clip,
            autogain: cfg.autogain,
            ceiling_db: if cfg.autogain { 0.0 } else { ceiling },
            peaks: Vec::with_capacity(2048),
            acc: 0.0,
            played: 0.0,
            p90_est: cached.unwrap_or(NEUTRAL_REF_DB),
            frozen: cached.is_some(),
            done_p90,
        }
    }

    /// 自动模式：**整曲累计**的窗峰值取 p90，双速率对称平滑。
    ///
    /// 之前是「只往更响方向更新 + 15 秒滑动窗」，等于 p90 取历史最大值：
    /// 一首歌里只要响过一次，增益就再也回不来，于是晴天一路爬了 145 秒。
    /// 改成全曲累计 + 对称平滑后，晴天副歌 127 秒才来、p90 那时才抬起来，
    /// 0.3 dB/s 回落太慢，145 秒还没收敛完（一首里波动十几 dB）。
    /// 现在按差距挑速度：差 >3 dB 走 0.6 dB/s，≤1 dB 回 0.3 dB/s 精修。
    fn observe(&mut self, peak_db: f32, dt_s: f32) {
        if self.autogain {
            if self.frozen {
                return;
            }
            if self.peaks.len() < EST_MAX_WINDOWS {
                self.peaks.push(peak_db);
            }
            self.played += dt_s;
            self.acc += dt_s;
            if self.peaks.len() < EST_MIN_WINDOWS {
                return;
            }
            let elapsed = self.acc;
            if elapsed < 1.0 {
                return;
            }
            self.acc = 0.0;
            let mut v = self.peaks.clone();
            v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let idx = ((v.len() as f32 * 0.9) as usize).min(v.len() - 1);
            let p90 = v[idx];
            // 双速率限速：差得多时 0.6 dB/s 纠正，贴合后 0.3 dB/s 精修
            let slew = p90_slew_rate(p90 - self.p90_est) * elapsed;
            self.p90_est = approach(self.p90_est, p90, slew);
            // 置信度爬坡 + 硬钳位：早期样本少就别拿 10 秒的估计去推整首
            let (lo, hi) = gain_limits(self.played);
            self.gain_db = (AUTO_TARGET_POST_DB - self.p90_est).clamp(lo, hi);
            *self.done_p90.lock().unwrap() = Some(self.p90_est);
            return;
        }
        // 手动模式：天花板跟着峰值慢漂
        if peak_db > self.ceiling_db {
            self.ceiling_db = peak_db.min(WAVE_REF_CAP_DB);
        } else {
            self.ceiling_db =
                (self.ceiling_db - WAVE_REF_DECAY_DB_PER_S * dt_s).max(WAVE_REF_FLOOR_DB);
        }
    }

    /// 自动增益给这首歌加了多少 dB
    fn auto_gain_db(&self) -> f32 {
        if self.autogain {
            self.gain_db
        } else {
            0.0
        }
    }

    /// 当前 p90 估计（还没估出来时用增益反推，永远有值）
    fn p90_db(&self) -> f32 {
        if self.autogain {
            AUTO_TARGET_POST_DB - self.gain_db
        } else {
            self.p90_est
        }
    }

    /// 是否已经算出真正的标定（刚重启时是中性状态）
    fn calibrated(&self) -> bool {
        self.autogain && self.p90_est != NEUTRAL_REF_DB
    }

    /// 峰值（线性）在增益后的 dBFS
    fn post_db(&self, peak: f32) -> f32 {
        20.0 * peak.max(1e-9).log10() + self.auto_gain_db()
    }

    /// 归一化前的倍数（1.0 = 正好顶到天花板）：平顶标志与断言都用它
    fn norm_pre(&self, peak: f32) -> f32 {
        10f32.powf((self.post_db(peak) - self.ceiling_db) / 20.0)
    }

    fn amp(&self, peak: f32) -> f32 {
        let post = self.post_db(peak);
        let x = ((post - self.ceiling_db + SPAN_DB) / SPAN_DB).clamp(0.0, 1.0) * self.user_gain;
        if self.soft_clip && x > SOFT_KNEE {
            // 超出拐点用 tanh 挤向 1：响处不硬切成一条线
            let t = ((x - SOFT_KNEE) / (1.0 - SOFT_KNEE)).tanh();
            (SOFT_KNEE + (1.0 - SOFT_KNEE) * t).clamp(0.0, 1.0)
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
        seg_no: usize,
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
        // 段号由界面给：同一首歌内从 1 开始，重启一次 +1（换歌重新从 1 数）
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
                        // 列：1 秒 2 原始dB 3 level(0..9) 4 自动增益dB 5 p90dB 6 段号 7 平顶
                        let level = wagc.amp(peak) * 9.0;
                        let p90 = est_cell.lock().unwrap().unwrap_or_else(|| wagc.p90_db());
                        // 平顶 = 归一化前的倍数 ≥0.995（顶到/超过天花板），
                        // 和 level 用同一个 post-gain 口径，不再和绝对 dBFS 混用
                        let flat = if wagc.norm_pre(peak) >= 0.995 {
                            1
                        } else {
                            0
                        };
                        let _ = writeln!(
                            f,
                            "{:.3},{:.2},{:.2},{:.1},{:.2},{},{}",
                            now as f64 / 1000.0,
                            20.0 * peak.max(1e-9).log10(),
                            level,
                            wagc.auto_gain_db(),
                            p90,
                            seg_no,
                            flat
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

    /// 当前满高基准（dBFS）：p90 反推出来，拖动重开时沿用
    pub fn ref_db(&self) -> f32 {
        self.auto_gain_db() + NEUTRAL_REF_DB
    }

    /// 当前 p90 估计（dBFS）
    pub fn p90_db(&self) -> f32 {
        self.est_p90.lock().unwrap().unwrap_or(0.0)
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
    fn autogain_maps_p90_to_target_level() {
        let db = |x: f32| 10f32.powf(x / 20.0);
        // 软限幅（默认）：p90 落在 8/9 附近，更响的被压到接近但不越过 1
        let cfg = WaveCfg {
            autogain: true,
            soft_clip: true,
            gain: 1.0,
            reference_dbfs: -6.0,
        };
        let cell = Arc::new(Mutex::new(None));
        // 缓存 p90=-14 → 需要的增益正好 +6（钳位边界内），p90 应落在 6/9
        let agc = WaveAgc::new(cfg, Some(-14.0), cell.clone());
        let amp = agc.amp(db(-14.0));
        assert!((amp * 9.0 - 6.0).abs() < 0.1, "p90 应落在 6/9: {amp}");
        assert!(agc.amp(1.0) <= 1.0);
        assert!(agc.amp(db(-50.0)) < 0.15, "安静段要接近 0");

        // 硬切：p90 精确落在 8/9
        let hard = WaveAgc::new(
            WaveCfg {
                soft_clip: false,
                ..cfg
            },
            Some(-14.0),
            cell,
        );
        assert!((hard.amp(db(-14.0)) * 9.0 - 6.0).abs() < 0.05);
        // 更响的削到 1，不会超过
        assert_eq!(hard.amp(1.0), 1.0);
    }

    #[test]
    fn autogain_is_symmetric_not_a_ratchet() {
        let cfg = WaveCfg {
            autogain: true,
            soft_clip: true,
            gain: 1.0,
            reference_dbfs: -6.0,
        };
        let cell = Arc::new(Mutex::new(None));
        let mut agc = WaveAgc::new(cfg, None, cell.clone());
        // 响段：p90 上抬 → 增益变小（波形压低）
        for _ in 0..620 {
            agc.observe(-10.0, 0.032);
        }
        let loud = agc.auto_gain_db();
        // 之后一大段安静（占多数）：整曲累计的 p90 会被拉下来，增益随之回升，
        // 证明它不再是「只升不降」的棘轮
        for _ in 0..7000 {
            agc.observe(-35.0, 0.032);
        }
        let quiet = agc.auto_gain_db();
        assert!(quiet > loud + 3.0, "安静段增益要能升回来: {loud} -> {quiet}");
    }

    #[test]
    fn flat_flag_matches_norm_and_level() {
        let cfg = WaveCfg {
            autogain: true,
            soft_clip: true,
            gain: 1.0,
            reference_dbfs: -6.0,
        };
        let cell = Arc::new(Mutex::new(None));
        let agc = WaveAgc::new(cfg, Some(-20.0), cell);
        // 找到「刚好顶到天花板」的线性峰值：norm_pre ≈ 1
        let dbv = |x: f32| 10f32.powf(x / 20.0);
        let mut peak = dbv(-20.0);
        for db in (-20..0).rev() {
            if agc.norm_pre(dbv(db as f32)) >= 0.995 {
                peak = dbv(db as f32);
                break;
            }
        }
        assert!(agc.norm_pre(peak) >= 0.995, "应能找到顶到天花板的窗");
        // 自洽性：flat=1 时 post-gain 已经顶到天花板，level 也在高位
        // （软限幅下 level 渐近 9、不会等于 9，所以这里判 ≥8）
        assert!(agc.amp(peak) * 9.0 >= 8.0, "level {}", agc.amp(peak) * 9.0);
        assert!(agc.norm_pre(peak) >= 0.995);
        // Chief 要求的自洽性：flat=1 ⇒ post-gain 已经到 0 dBFS 附近
        assert!(
            agc.post_db(peak) >= -0.5,
            "flat 窗的 post-gain 必须 ≥ -0.5 dBFS: {}",
            agc.post_db(peak)
        );
        // 安静窗：两个量都要小
        assert!(agc.norm_pre(dbv(-50.0)) < 0.05);
        assert!(agc.amp(dbv(-50.0)) * 9.0 < 0.5);
    }

    #[test]
    fn autogain_ramps_confidence_and_clamps() {
        assert_eq!(gain_limits(5.0), (-2.0, 2.0));
        assert_eq!(gain_limits(30.0), (-4.0, 4.0));
        assert_eq!(gain_limits(60.0), (-12.0, 6.0));

        // 前 15 秒即使歌很轻，也不许把增益推到 +12
        let cfg = WaveCfg {
            autogain: true,
            soft_clip: true,
            gain: 1.0,
            reference_dbfs: -6.0,
        };
        let cell = Arc::new(Mutex::new(None));
        let mut agc = WaveAgc::new(cfg, None, cell);
        for _ in 0..400 {
            agc.observe(-45.0, 0.032);
        }
        assert!(agc.auto_gain_db() <= 2.01, "前 15 秒最多 +2: {}", agc.auto_gain_db());
        // 45 秒后放开到硬钳位 [-12, +6]
        for _ in 0..1200 {
            agc.observe(-45.0, 0.032);
        }
        let g = agc.auto_gain_db();
        assert!((-12.01..=6.01).contains(&g), "增益必须在 [-12, +6] 内: {g}");
        // 极轻的歌：整曲 p90 约 -45，目标 -8，要不出来 +37 但被 +6 钳住
        assert!(g <= 6.01, "{g}");
    }

    #[test]
    fn p90_slew_is_fast_on_big_gap_and_fine_near_target() {
        assert_eq!(p90_slew_rate(5.0), P90_SLEW_FAST_DB_PER_S);
        assert_eq!(p90_slew_rate(-5.0), P90_SLEW_FAST_DB_PER_S);
        assert_eq!(p90_slew_rate(0.4), P90_SLEW_FINE_DB_PER_S);
        assert_eq!(p90_slew_rate(0.0), P90_SLEW_FINE_DB_PER_S);
        // 中间线性过渡
        let mid = p90_slew_rate(2.0);
        assert!(mid > P90_SLEW_FINE_DB_PER_S && mid < P90_SLEW_FAST_DB_PER_S, "{mid}");

        // 一次 12 dB 的跳变：纠正段能在 ~20 秒内走完，而不是 40 秒
        let mut est = -12.0f32;
        let mut t = 0.0f32;
        while (est - 0.0).abs() > 0.05 && t < 60.0 {
            est = approach(est, 0.0, p90_slew_rate(0.0 - est) * 0.1);
            t += 0.1;
        }
        assert!(t < 25.0, "12 dB 纠正在 {t} 秒完成");
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
