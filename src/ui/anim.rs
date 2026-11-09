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

/// 均衡器：`bars` 根柱子，**1 秒只变一次**（用播放进度秒数驱动，不另起高频计时器）。
/// 同一秒内是确定性的（同一秒重绘多次结果一样，不会闪）。
pub fn eq_bars(on: bool, playing: bool, secs: u64, bars: usize) -> Vec<&'static str> {
    if !on || !playing || bars == 0 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(bars);
    for i in 0..bars {
        // 线性同余 + 位移混合：确定性、相邻秒之间跳得足够明显
        let mut x = secs
            .wrapping_mul(6364136223846793005)
            .wrapping_add((i as u64 + 1).wrapping_mul(1442695040888963407));
        x ^= x >> 33;
        x = x.wrapping_mul(0xff51afd7ed558ccd);
        x ^= x >> 33;
        out.push(EQ_LEVELS[(x % EQ_LEVELS.len() as u64) as usize]);
    }
    out
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
        assert!(eq_bars(false, true, 7, 5).is_empty());
        assert!(eq_bars(true, false, 7, 5).is_empty());
    }

    #[test]
    fn equalizer_is_deterministic_within_a_second() {
        let a = eq_bars(true, true, 42, 5);
        let b = eq_bars(true, true, 42, 5);
        assert_eq!(a, b, "同一秒内必须一样，否则会闪");
        assert_eq!(a.len(), 5);
        assert!(a.iter().all(|c| EQ_LEVELS.contains(c)));
    }

    #[test]
    fn equalizer_changes_between_seconds() {
        let secs: Vec<Vec<&str>> = (0..6).map(|s| eq_bars(true, true, s, 5)).collect();
        assert!(
            secs.windows(2).any(|w| w[0] != w[1]),
            "不同秒至少要有变化，否则看起来是静止的"
        );
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
