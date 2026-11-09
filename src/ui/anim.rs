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

/// 多行柱状图 + 峰值标记：`levels` 是当前档位，`peaks` 是峰值档位，
/// 峰值用 `▔` 画在对应高度上（0.5s 保持后由调用方匀速下落）。
pub fn bars_rows_with_peaks(levels: &[u8], peaks: &[u8], rows: usize) -> Vec<String> {
    let rows = rows.max(1);
    let n = levels.len();
    let mut out = vec![String::with_capacity(n); rows];
    for (r, line) in out.iter_mut().enumerate() {
        let row_from_bottom = rows - 1 - r;
        for i in 0..n {
            let lv = *levels.get(i).unwrap_or(&0) as usize;
            let pk = *peaks.get(i).unwrap_or(&0) as usize;
            let base = row_from_bottom * 7;
            let fill = (lv * rows).saturating_sub(base);
            let peak_fill = (pk * rows).saturating_sub(base);
            if fill >= 7 {
                line.push('█');
            } else if fill > 0 {
                line.push(['▁', '▂', '▃', '▄', '▅', '▆', '▇'][fill - 1]);
            } else if peak_fill > 0 {
                line.push('▔');
            } else {
                line.push(' ');
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
    fn stacked_bars_with_peaks_render_correctly() {
        let zero = bars_rows_with_peaks(&[0; 8], &[0; 8], 3);
        assert_eq!(zero.len(), 3);
        assert!(zero.iter().all(|l| l.chars().all(|c| c == ' ')));
        let full = bars_rows_with_peaks(&[7; 8], &[7; 8], 3);
        assert!(full.iter().all(|l| l.chars().all(|c| c == '█')));
        // 峰值高于当前档：应该留下 ▔ 标记
        let mix = bars_rows_with_peaks(&[0; 4], &[4; 4], 3);
        let marks: usize = mix
            .iter()
            .map(|l| l.chars().filter(|c| *c == '▔').count())
            .sum();
        assert!(marks > 0, "峰值应该用 ▔ 标出来: {mix:?}");
        // 档位越高越不空
        let mut prev = 0;
        for lv in 0..=7u8 {
            let rows = bars_rows_with_peaks(&[lv; 4], &[0; 4], 3);
            let filled: usize = rows
                .iter()
                .map(|l| l.chars().filter(|c| *c != ' ').count())
                .sum();
            assert!(filled >= prev);
            prev = filled;
        }
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
