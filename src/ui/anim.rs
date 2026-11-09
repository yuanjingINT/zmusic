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
