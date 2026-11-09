//! 动效的纯函数层：帧只由输入决定，方便单测，也保证「关掉动画」时零渲染。
//!
//! 所有动效都受 `[ui] animations` 总开关 + 各自开关控制；关掉后返回 `None`/空，
//! 渲染函数走静态分支，与没有动画的版本行为一致。

/// braille 转圈：10 帧，10fps（100ms 一帧）
pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
pub const SPINNER_FRAME_MS: u64 = 100;

/// 加载转圈：`elapsed_ms` 用进程启动至今的毫秒数（不额外计时）
pub fn spinner(on: bool, loading: bool, elapsed_ms: u64) -> Option<&'static str> {
    if !on || !loading {
        return None;
    }
    Some(SPINNER[((elapsed_ms / SPINNER_FRAME_MS) as usize) % SPINNER.len()])
}

/// synth 兜底：极慢的「呼吸」，不是蠕动。
///
/// 12 段之间相位连续（整排一起缓缓起伏），周期 3.5 秒，振幅只有 1–2 档；
/// 宁可不明显，也不要像虫子一样乱跳。
pub fn breathing_levels(t_secs: f64, bars: usize) -> Vec<u8> {
    if bars == 0 {
        return Vec::new();
    }
    let phase = t_secs * std::f64::consts::TAU / 3.5;
    (0..bars)
        .map(|i| {
            let kont = i as f64 / bars as f64; // 0..1，整排略微错相，仍连续
            let v = (phase + kont * 0.6).sin();
            (1.5 + v).round().clamp(0.0, 3.0) as u8
        })
        .collect()
}

/// 一段频谱的布局：段数、每段宽度、段间距（宽度不够时先缩段宽，再减段数）
pub fn layout(avail: usize, want: usize) -> (usize, usize, usize) {
    // 优先 2 格宽 + 1 格间距
    for (bands, w) in [(want, 2), (want, 1), (16, 1), (12, 1), (8, 1), (4, 1)] {
        let bands = bands.min(want.max(1));
        let need = bands * (w + 1) - 1;
        if need <= avail && bands > 0 {
            return (bands, w, 1);
        }
    }
    (1, 1, 1)
}

/// 一行里的一个格子：字符 + 它距离底部的高度（用来按高度取色）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    /// 0 = 最底行，rows-1 = 最顶行
    pub from_bottom: usize,
}

/// 生成频谱的每一行（含段宽与间距）。
///
/// - 柱体按 8 档块字符堆叠，宽度 `band_w`、间距 `gap`
/// - 峰值 `▔` 单独画在**柱顶上方那一格**，不与实心柱重叠
pub fn spectrum_rows(
    levels: &[u8],
    peaks: &[u8],
    rows: usize,
    band_w: usize,
    gap: usize,
) -> Vec<Vec<Cell>> {
    let rows = rows.max(1);
    let n = levels.len();
    let mut out: Vec<Vec<Cell>> = (0..rows)
        .map(|_| Vec::with_capacity(n * (band_w + gap)))
        .collect();
    for (r, line) in out.iter_mut().enumerate() {
        let row_from_bottom = rows - 1 - r;
        for i in 0..n {
            let lv = *levels.get(i).unwrap_or(&0) as usize;
            let pk = *peaks.get(i).unwrap_or(&0) as usize;
            let eighths = lv * rows;
            let base = row_from_bottom * 7;
            let fill = eighths.saturating_sub(base).min(7);
            // 峰值只画在「柱顶上方那一格」，不与实心柱重叠
            let filled_rows = eighths.div_ceil(7);
            let marker_here = pk > lv && fill == 0 && row_from_bottom == filled_rows;
            let ch = if fill == 7 {
                '█'
            } else if fill > 0 {
                ['▁', '▂', '▃', '▄', '▅', '▆', '▇'][fill - 1]
            } else if marker_here {
                '▔'
            } else {
                ' '
            };
            for _ in 0..band_w {
                line.push(Cell {
                    ch,
                    from_bottom: row_from_bottom,
                });
            }
            for _ in 0..gap {
                if i + 1 < n {
                    line.push(Cell {
                        ch: ' ',
                        from_bottom: row_from_bottom,
                    });
                }
            }
        }
    }
    out
}

/// 某个动效帧率对应的重绘间隔（毫秒），夹在 5–60fps
pub fn interval_ms(fps: u8) -> u64 {
    1000 / fps.clamp(5, 60) as u64
}

/// 切歌/切页淡入：3 档明暗，每档 90ms，一次性结束（不循环）。
/// 返回 `None` 表示「没有淡入」或已经结束，按正常样式渲染。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FadeStep {
    /// 最暗（次级再压一档）
    Dim,
    /// 中间档（正常次级色）
    Half,
    /// 最亮（正常样式）
    Full,
}

pub const FADE_STEP_MS: u64 = 90;

pub fn fade_step(on: bool, elapsed_ms: Option<u64>) -> Option<FadeStep> {
    if !on {
        return None;
    }
    let ms = elapsed_ms?;
    if ms < FADE_STEP_MS {
        Some(FadeStep::Dim)
    } else if ms < FADE_STEP_MS * 2 {
        Some(FadeStep::Half)
    } else if ms < FADE_STEP_MS * 3 {
        Some(FadeStep::Full)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spinner_is_static_when_disabled_or_idle() {
        assert_eq!(spinner(false, true, 500), None);
        assert_eq!(spinner(true, false, 500), None);
    }

    #[test]
    fn spinner_cycles_ten_frames_once_per_100ms() {
        let frames: std::collections::HashSet<_> = (0..10)
            .map(|i| spinner(true, true, i * SPINNER_FRAME_MS).unwrap())
            .collect();
        assert_eq!(frames.len(), 10);
        assert_eq!(spinner(true, true, 100), spinner(true, true, 199));
        assert_eq!(spinner(true, true, 0), spinner(true, true, 1000));
    }

    /// synth 兜底必须是「慢呼吸」：周期 3.5s、幅度 0–3 档、相邻段相位接近
    #[test]
    fn breathing_is_slow_and_gentle() {
        let a = breathing_levels(0.0, 12);
        assert_eq!(a.len(), 12);
        assert!(a.iter().all(|l| *l <= 3), "幅度不该超过 3 档: {a:?}");
        // 12 段之间几乎等差：整排一起呼吸，不是各自乱跳
        let spread = a.iter().max().unwrap() - a.iter().min().unwrap();
        assert!(spread <= 2, "同帧内各段落差应该很小: {a:?}");
        // 半个周期（1.75s）后整体反向
        let b = breathing_levels(1.75, 12);
        assert_ne!(a, b, "半个周期后应该有变化");
        // 相邻帧（1/30s）变化极小
        let c = breathing_levels(1.0 / 30.0, 12);
        let diff: i32 = a
            .iter()
            .zip(c.iter())
            .map(|(x, y)| (*x as i32 - *y as i32).abs())
            .sum();
        assert!(diff <= 12, "1/30 秒内不该明显变化: {diff}");
    }

    #[test]
    fn spectrum_rows_render_bars_with_peak_marker() {
        // 全零：只有空格
        let zero = spectrum_rows(&[0; 8], &[0; 8], 3, 2, 1);
        assert_eq!(zero.len(), 3);
        assert!(zero.iter().all(|r| r.iter().all(|c| c.ch == ' ')));
        // 满档：柱子全实心（间距仍是空格）
        let full = spectrum_rows(&[7; 8], &[7; 8], 3, 2, 1);
        let solid: usize = full
            .iter()
            .map(|r| r.iter().filter(|c| c.ch == '█').count())
            .sum();
        assert_eq!(solid, 8 * 2 * 3, "8 段 × 2 格 × 3 行");
        // 峰值高于柱体：柱顶上方那格应该出现 ▔
        let mix = spectrum_rows(&[2; 4], &[6; 4], 3, 2, 1);
        let marks: usize = mix
            .iter()
            .map(|r| r.iter().filter(|c| c.ch == '▔').count())
            .sum();
        assert!(marks > 0, "峰值应有 ▔ 标记: {mix:?}");
        // 每段 2 格宽 + 1 格间距：4 段 = 2*4+3 = 11 格
        assert_eq!(mix[0].len(), 11);
        // 2 格宽的柱子：同一段的两格字符相同
        let row = &mix[2];
        assert_eq!(row[0].ch, row[1].ch);
        assert_eq!(row[2].ch, ' ');
    }

    #[test]
    fn interval_is_clamped_between_5_and_60_fps() {
        assert_eq!(interval_ms(30), 33);
        assert_eq!(interval_ms(5), 200);
        assert_eq!(interval_ms(60), 16);
        assert_eq!(interval_ms(1), 200);
        assert_eq!(interval_ms(120), 16);
    }

    #[test]
    fn fade_is_off_when_disabled_and_steps_through_three_frames() {
        assert_eq!(fade_step(false, Some(0)), None);
        assert_eq!(fade_step(true, None), None);
        assert_eq!(fade_step(true, Some(0)), Some(FadeStep::Dim));
        assert_eq!(fade_step(true, Some(FADE_STEP_MS)), Some(FadeStep::Half));
        assert_eq!(
            fade_step(true, Some(FADE_STEP_MS * 2)),
            Some(FadeStep::Full)
        );
        assert_eq!(fade_step(true, Some(FADE_STEP_MS * 3)), None);
    }
}
