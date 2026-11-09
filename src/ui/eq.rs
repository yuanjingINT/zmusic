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

/// 波形纵向标定：**窗内峰值 + 固定基准**，基准只以极慢速度微调。
///
/// 为什么不用「近 10 秒 p95 / 峰值保持」做归一化：那等于每首歌自己归一化，
/// 实测安静前奏 p50=0.40、副歌 p50=0.42（几乎一样），动态全被吃掉。
/// 固定基准（-9 dBFS，约等于流行乐副歌峰值）时实测安静 p50=0.09、副歌 p50=0.76，
/// 这才是「安静一条线、副歌占满」。基准只按 0.1 dB/s 往下漂（下限 -22 dBFS），
/// 一首歌的时间尺度内基本不动，所以不会出现「整排忽高忽低」。
struct WaveAgc {
    ref_db: f32,
}

/// 基准下漂速度（dB/秒）：一首歌内几乎不动
const WAVE_REF_DECAY_DB_PER_S: f32 = 0.1;
/// 基准下限（约 -22 dBFS）：再安静也不放大底噪
const WAVE_REF_FLOOR_DB: f32 = -22.0;
/// 基准上限：峰值不许把基准顶到满刻度以上
const WAVE_REF_CAP_DB: f32 = -3.0;
/// 冷启动基准：约等于流行乐副歌峰值（-9 dBFS）
const WAVE_REF_INIT_DB: f32 = -9.0;

impl WaveAgc {
    fn new() -> Self {
        WaveAgc {
            ref_db: WAVE_REF_INIT_DB,
        }
    }

    fn observe(&mut self, peak_db: f32, dt_s: f32) {
        if peak_db > self.ref_db {
            self.ref_db = peak_db.min(WAVE_REF_CAP_DB);
        } else {
            self.ref_db = (self.ref_db - WAVE_REF_DECAY_DB_PER_S * dt_s).max(WAVE_REF_FLOOR_DB);
        }
    }

    fn amp(&self, peak: f32) -> f32 {
        (peak / 10f32.powf(self.ref_db / 20.0))
            .clamp(0.0, 1.0)
            .powf(0.9)
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
    /// 读线程用的结束旗标（Drop 时置位，避免线程卡在阻塞读上）
    stop: Arc<Mutex<bool>>,
}

impl PcmAnalyzer {
    /// 起解码进程。`start_secs` 用来和播放器对齐（从当前播放位置开始解）。
    pub fn start(url: &str, start_secs: f64) -> Option<PcmAnalyzer> {
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
        let stop = Arc::new(Mutex::new(false));
        {
            let wave = wave.clone();
            let stop = stop.clone();
            let path = fifo.clone();
            std::thread::spawn(move || {
                let Ok(mut f) = std::fs::File::open(&path) else {
                    return;
                };
                // 已拿到 fd：立刻 unlink，靠 fd 继续读写。进程被强杀也不会留残留。
                let _ = std::fs::remove_file(&path);
                let mut buf = vec![0u8; WINDOW * 2];
                let mut wagc = WaveAgc::new();
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
                        let _ = writeln!(
                            f,
                            "{},{:.4},{:.2}",
                            now,
                            wagc.amp(peak),
                            20.0 * peak.max(1e-9).log10()
                        );
                    }
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
            stop,
        })
    }

    /// 最近 `max` 个波形样本 + 单调序号（界面据此只取新增的部分）
    pub fn wave(&self, max: usize) -> (Vec<f32>, u64) {
        let w = self.wave.lock().unwrap();
        let n = w.samples.len().min(max);
        (w.samples[w.samples.len() - n..].to_vec(), w.seq)
    }

    /// 按已运行时长推算的当前解码位置（秒）：和播放位置差太多就是用户拖过
    pub fn expected_pos(&self) -> f64 {
        self.start + self.started.elapsed().as_secs_f64()
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
