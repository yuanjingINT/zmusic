//! 动效的纯函数层：帧只由输入决定，方便单测，也保证「关掉动画」时零渲染。
//!
//! 所有动效都受 `[ui] animations` 总开关 + 各自开关控制；关掉后返回 `None`/空，
//! 渲染函数走静态分支，与没有动画的版本行为一致。

/// braille 转圈：10 帧，10fps（100ms 一帧）
pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
pub const SPINNER_FRAME_MS: u64 = 100;

/// 均衡器柱子：8 级高度
pub const EQ_LEVELS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

/// 加载转圈：`elapsed_ms` 用进程启动至今的毫秒数（不额外计时）
pub fn spinner(on: bool, loading: bool, elapsed_ms: u64) -> Option<&'static str> {
    if !on || !loading {
        return None;
    }
    Some(SPINNER[((elapsed_ms / SPINNER_FRAME_MS) as usize) % SPINNER.len()])
}

/// 均衡器：`bars` 根柱子，**单调时钟驱动**，15fps 下相邻帧高度只差 1–2 档，看起来像在呼吸。
///
/// 波形不是随机数：每根柱子是两条正弦叠加（频率比 1 : 1.7，相位错开），
/// 再量化到 8 档。同一时刻结果确定，重绘不会抖。
pub fn eq_bars(on: bool, playing: bool, t_secs: f64, bars: usize) -> Vec<&'static str> {
    if !on || !playing || bars == 0 {
        return Vec::new();
    }
    use std::f64::consts::TAU;
    let mut out = Vec::with_capacity(bars);
    for i in 0..bars {
        let phase = i as f64 * 0.9;
        let a = (t_secs * TAU * 0.8 + phase).sin();
        let b = (t_secs * TAU * 1.37 + phase * 1.7).sin() * 0.5;
        let v = (a + b) / 1.5; // -1..1
        let level = (((v + 1.0) / 2.0) * 7.0).round().clamp(0.0, 7.0) as usize;
        out.push(EQ_LEVELS[level]);
    }
    out
}

/// 某个动效帧率对应的重绘间隔（毫秒），夹在 5–30fps
pub fn interval_ms(fps: u8) -> u64 {
    1000 / fps.clamp(5, 30) as u64
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
    use std::collections::HashSet;

    #[test]
    fn spinner_is_static_when_disabled_or_idle() {
        assert_eq!(spinner(false, true, 500), None, "总开关关掉不该出帧");
        assert_eq!(spinner(true, false, 500), None, "没有加载时不该动");
    }

    #[test]
    fn spinner_cycles_ten_frames_once_per_100ms() {
        let frames: HashSet<_> = (0..10)
            .map(|i| spinner(true, true, i * SPINNER_FRAME_MS).unwrap())
            .collect();
        assert_eq!(frames.len(), 10, "10 帧都应出现");
        // 同一 100ms 窗口内多次重绘必须是同一帧（不会抖）
        assert_eq!(
            spinner(true, true, 100),
            spinner(true, true, 199),
            "同一帧窗口内要稳定"
        );
        // 绕回
        assert_eq!(spinner(true, true, 0), spinner(true, true, 1000));
    }

    #[test]
    fn equalizer_is_empty_when_disabled_or_paused() {
        assert!(eq_bars(false, true, 7.0, 5).is_empty());
        assert!(eq_bars(true, false, 7.0, 5).is_empty());
    }

    #[test]
    fn equalizer_is_deterministic_for_the_same_instant() {
        let a = eq_bars(true, true, 42.25, 5);
        let b = eq_bars(true, true, 42.25, 5);
        assert_eq!(a, b, "同一时刻必须一样，否则重绘会抖");
        assert_eq!(a.len(), 5);
        assert!(a.iter().all(|c| EQ_LEVELS.contains(c)));
    }

    /// 15fps 下相邻帧每根柱子最多差 2 档：这是「流畅」的量化标准，
    /// 不是三档乱跳（旧实现 1fps + 伪随机就是这么跳的）。
    #[test]
    fn equalizer_moves_smoothly_at_15fps() {
        let fps = 15.0;
        let mut prev = eq_bars(true, true, 0.0, 5);
        let mut max_jump = 0i32;
        for step in 1..=90 {
            let t = step as f64 / fps;
            let cur = eq_bars(true, true, t, 5);
            assert_eq!(cur.len(), 5);
            for (x, y) in prev.iter().zip(cur.iter()) {
                let lx = EQ_LEVELS.iter().position(|c| c == x).unwrap() as i32;
                let ly = EQ_LEVELS.iter().position(|c| c == y).unwrap() as i32;
                max_jump = max_jump.max((lx - ly).abs());
            }
            prev = cur;
        }
        assert!(max_jump <= 2, "相邻帧最多差 2 档，实际 {max_jump}");
    }

    /// 连续 5 秒里柱子必须真的在动（不能是常量）
    #[test]
    fn equalizer_actually_moves_over_time() {
        let frames: Vec<Vec<&str>> = (0..75)
            .map(|i| eq_bars(true, true, i as f64 / 15.0, 5))
            .collect();
        assert!(
            frames.windows(2).filter(|w| w[0] != w[1]).count() > 50,
            "大多数相邻帧都应该有变化"
        );
    }

    #[test]
    fn interval_is_clamped_between_5_and_30_fps() {
        assert_eq!(interval_ms(15), 66);
        assert_eq!(interval_ms(5), 200);
        assert_eq!(interval_ms(30), 33);
        assert_eq!(interval_ms(1), 200, "低于 5fps 按 5fps 算");
        assert_eq!(interval_ms(60), 33, "高于 30fps 按 30fps 算");
    }

    #[test]
    fn fade_is_off_when_disabled_and_steps_through_three_frames() {
        assert_eq!(fade_step(false, Some(0)), None);
        assert_eq!(fade_step(true, None), None, "没有触发时不淡入");
        assert_eq!(fade_step(true, Some(0)), Some(FadeStep::Dim));
        assert_eq!(fade_step(true, Some(FADE_STEP_MS)), Some(FadeStep::Half));
        assert_eq!(
            fade_step(true, Some(FADE_STEP_MS * 2)),
            Some(FadeStep::Full)
        );
        assert_eq!(fade_step(true, Some(FADE_STEP_MS * 3)), None, "只播一次");
    }
}
