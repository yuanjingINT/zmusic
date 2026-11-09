//! 真实频谱：起一个隐藏的 mpv 解码子进程，从 FIFO 读 PCM，算 5 个频段的能量。
//!
//! 为什么再拉一次流：频谱必须来自真实音频样本。主播放器的音频在 mpv 里，
//! 拿不到 PCM，所以另起一个 `--ao=pcm` 的解码进程；它和播放器同时开始、按真实
//! 时间输出，所以柱子和耳朵听到的是同一段。代价是多一份带宽/解码，所以只在
//! 「正在播放页」且确实在播放时运行，切页/暂停立即杀掉。
//!
//! 纯函数（goertzel / band_energies / to_levels / smooth）都能单测；
//! 进程部分（`PcmAnalyzer`）负责 FIFO、读线程和清理孤儿。

use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

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

/// 能量 → 0..7 档：log10 映射（低频不会一响就顶满，高频也不会永远平）
pub fn to_levels(energies: &[f32], floor_db: f32, ceil_db: f32) -> Vec<u8> {
    energies
        .iter()
        .map(|e| {
            let db = 20.0 * e.max(1e-9).log10();
            let t = ((db - floor_db) / (ceil_db - floor_db)).clamp(0.0, 1.0);
            (t * 7.0).round() as u8
        })
        .collect()
}

/// attack 0.65 / release 0.12：起得快、落得慢，看着才像频谱
pub const ATTACK: f32 = 0.65;
pub const RELEASE: f32 = 0.12;

pub fn smooth(prev: &[u8], target: &[u8], attack: f32, release: f32) -> Vec<u8> {
    prev.iter()
        .zip(target.iter())
        .map(|(p, t)| {
            let (p, t) = (*p as f32, *t as f32);
            let k = if t >= p { attack } else { release };
            (p + (t - p) * k).round().clamp(0.0, 7.0) as u8
        })
        .collect()
}

/// 峰值保持：0.5 秒内不动，之后每帧掉一档（匀速下落）
#[derive(Debug, Clone)]
pub struct Peaks {
    pub values: Vec<u8>,
    pub held_at: Vec<u64>,
}

impl Peaks {
    pub fn new(n: usize) -> Self {
        Peaks {
            values: vec![0; n],
            held_at: vec![0; n],
        }
    }
    /// `now_ms` 传单调时钟毫秒；`hold_ms` = 500
    pub fn update(&mut self, levels: &[u8], now_ms: u64, hold_ms: u64) -> Vec<u8> {
        if self.values.len() != levels.len() {
            *self = Peaks::new(levels.len());
        }
        for (i, lv) in levels.iter().enumerate() {
            if *lv >= self.values[i] {
                self.values[i] = *lv;
                self.held_at[i] = now_ms;
            } else if now_ms.saturating_sub(self.held_at[i]) >= hold_ms {
                self.values[i] = self.values[i].saturating_sub(1);
                self.held_at[i] = now_ms;
            }
        }
        self.values.clone()
    }
}

/// 一帧频谱：当前档位 + 峰值保持档位
#[derive(Debug, Clone, Default)]
pub struct EqFrame {
    pub levels: Vec<u8>,
    pub peaks: Vec<u8>,
}

pub struct PcmAnalyzer {
    child: Child,
    fifo: PathBuf,
    frame: Arc<Mutex<EqFrame>>,
    /// 读线程用的结束旗标（Drop 时置位，避免线程卡在阻塞读上）
    stop: Arc<Mutex<bool>>,
}

impl PcmAnalyzer {
    /// 起解码进程。`start_secs` 用来和播放器对齐（从当前播放位置开始解）。
    pub fn start(url: &str, start_secs: f64) -> Option<PcmAnalyzer> {
        let fifo = std::env::temp_dir().join(format!("zmusic-eq-{}.pcm", std::process::id()));
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

        let frame = Arc::new(Mutex::new(EqFrame::default()));
        let stop = Arc::new(Mutex::new(false));
        {
            let frame = frame.clone();
            let stop = stop.clone();
            let path = fifo.clone();
            std::thread::spawn(move || {
                let Ok(mut f) = std::fs::File::open(&path) else {
                    return;
                };
                let mut buf = vec![0u8; WINDOW * 2];
                let mut prev: Vec<u8> = vec![0; BANDS];
                let mut peaks = Peaks::new(BANDS);
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
                    let target =
                        to_levels(&band_energies(&samples, SAMPLE_RATE, BANDS), -60.0, -10.0);
                    prev = smooth(&prev, &target, ATTACK, RELEASE);
                    let peaks_now = peaks.update(&prev, t_start.elapsed().as_millis() as u64, 500);
                    *frame.lock().unwrap() = EqFrame {
                        levels: prev.clone(),
                        peaks: peaks_now,
                    };
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
            frame,
            stop,
        })
    }

    pub fn frame(&self) -> EqFrame {
        self.frame.lock().unwrap().clone()
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
pub fn merge_bands(src: &[u8], target: usize) -> Vec<u8> {
    let n = src.len();
    if target >= n || target == 0 {
        return src.to_vec();
    }
    let mut out = Vec::with_capacity(target);
    for i in 0..target {
        let lo = i * n / target;
        let hi = ((i + 1) * n / target).max(lo + 1).min(n);
        out.push(src[lo..hi].iter().copied().max().unwrap_or(0));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f64, sr: f64, n: usize, amp: f64) -> Vec<i16> {
        (0..n)
            .map(|i| {
                (amp * (2.0 * std::f64::consts::PI * freq * i as f64 / sr).sin() * 32767.0) as i16
            })
            .collect()
    }

    /// 频段中心频率对数等距，且不越 8kHz 的奈奎斯特
    #[test]
    fn band_centers_are_log_spaced() {
        let c = band_centers(16);
        assert_eq!(c.len(), 16);
        assert!(c[0] >= 60.0 && *c.last().unwrap() <= 3500.0);
        // 相邻比值恒定（对数等距）
        let r1 = c[1] / c[0];
        let r2 = c[15] / c[14];
        assert!((r1 - r2).abs() < 0.01, "对数等距: {r1} vs {r2}");
    }

    #[test]
    fn goertzel_peaks_at_the_right_frequency() {
        let s = sine(200.0, SAMPLE_RATE, WINDOW, 0.8);
        let at = goertzel(&s, SAMPLE_RATE, 200.0);
        let off = goertzel(&s, SAMPLE_RATE, 900.0);
        assert!(at > off * 5.0, "200Hz 能量应远高于 900Hz：{at} vs {off}");
    }

    /// 低频正弦 → 低段亮；高频正弦 → 高段亮
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
        assert!(
            peak >= 12,
            "2.5kHz 应落在高段（下标 ≥12），实际 {peak}: {high:?}"
        );
    }

    #[test]
    fn levels_map_quiet_and_loud() {
        assert!(to_levels(&[1e-6; 8], -60.0, -10.0).iter().all(|l| *l == 0));
        assert!(to_levels(&[0.9; 8], -60.0, -10.0).iter().all(|l| *l == 7));
    }

    /// attack 0.65 快、release 0.12 慢
    #[test]
    fn smooth_attacks_fast_and_releases_slow() {
        let up = smooth(&[0; 4], &[7; 4], ATTACK, RELEASE);
        let down = smooth(&[7; 4], &[0; 4], ATTACK, RELEASE);
        assert!((up[0] as i32 - 5).abs() <= 1, "上升应到 4-5 档: {}", up[0]);
        assert!(
            (down[0] as i32 - 6).abs() <= 1,
            "下降一帧只掉一点: {}",
            down[0]
        );
    }

    /// 峰值保持 0.5s 后匀速下落
    #[test]
    fn peaks_hold_then_decay() {
        let mut p = Peaks::new(4);
        assert_eq!(p.update(&[3, 3, 3, 3], 0, 500), vec![3, 3, 3, 3]);
        // 保持期内不掉
        assert_eq!(p.update(&[0, 0, 0, 0], 300, 500), vec![3, 3, 3, 3]);
        // 超过 0.5s 后每帧掉一档
        assert_eq!(p.update(&[0, 0, 0, 0], 600, 500), vec![2, 2, 2, 2]);
        // 掉落之后重新计时：下一次要再等 500ms 才继续掉
        assert_eq!(p.update(&[0, 0, 0, 0], 700, 500), vec![2, 2, 2, 2]);
        assert_eq!(p.update(&[0, 0, 0, 0], 1200, 500), vec![1, 1, 1, 1]);
    }

    /// 16 段按宽度合并：组内取最大，不丢峰值
    #[test]
    fn merge_bands_keeps_peaks() {
        let src: Vec<u8> = (0..16).map(|i| i as u8).collect();
        let m12 = merge_bands(&src, 12);
        assert_eq!(m12.len(), 12);
        let m8 = merge_bands(&src, 8);
        assert_eq!(m8.len(), 8);
        assert_eq!(m8[0], 1, "0..2 组取最大");
        assert_eq!(*m8.last().unwrap(), 15);
        assert_eq!(merge_bands(&src, 16), src);
        assert_eq!(merge_bands(&src, 32), src, "目标比源大时原样返回");
    }
}

#[cfg(test)]
mod live_probe {
    use super::*;
    /// 手动跑：cargo test --release -- --ignored live_probe --nocapture
    #[test]
    #[ignore]
    fn live_probe() {
        let mut a = PcmAnalyzer::start("/tmp/dyn.wav", 0.0).expect("start");
        for i in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let f = a.frame();
            println!(
                "t={:>4}ms levels={:?} peaks={:?} alive={}",
                i * 250,
                f.levels,
                f.peaks,
                a.alive()
            );
        }
    }
}
