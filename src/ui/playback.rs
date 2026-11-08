//! 播放顺序与"下一首"的选择逻辑。纯函数，方便单测。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayMode {
    /// 顺序播放：播到列表末尾就停，不循环
    Sequential,
    /// 随机
    Shuffle,
    /// 单曲循环
    RepeatOne,
}

impl PlayMode {
    #[cfg(test)]
    pub const ALL: [PlayMode; 3] = [PlayMode::Sequential, PlayMode::Shuffle, PlayMode::RepeatOne];

    pub fn label(&self) -> &'static str {
        match self {
            PlayMode::Sequential => "顺序",
            PlayMode::Shuffle => "随机",
            PlayMode::RepeatOne => "单曲循环",
        }
    }

    pub fn parse(s: &str) -> PlayMode {
        match s {
            "shuffle" => PlayMode::Shuffle,
            "repeat_one" => PlayMode::RepeatOne,
            _ => PlayMode::Sequential,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            PlayMode::Sequential => "sequential",
            PlayMode::Shuffle => "shuffle",
            PlayMode::RepeatOne => "repeat_one",
        }
    }

    pub fn next(&self) -> PlayMode {
        match self {
            PlayMode::Sequential => PlayMode::Shuffle,
            PlayMode::Shuffle => PlayMode::RepeatOne,
            PlayMode::RepeatOne => PlayMode::Sequential,
        }
    }
}

/// 播完一首后该放哪首。
///
/// - `Sequential`：下一首；已经是最后一首就返回 None（停住，不循环）
/// - `RepeatOne`：当前这首（重播）
/// - `Shuffle`：用 `seed` 取一个"非当前"的位置（纯函数，方便测试）
pub fn next_index(len: usize, idx: usize, mode: PlayMode, seed: u64) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let idx = idx.min(len - 1);
    match mode {
        PlayMode::Sequential => {
            if idx + 1 < len {
                Some(idx + 1)
            } else {
                None
            }
        }
        PlayMode::RepeatOne => Some(idx),
        PlayMode::Shuffle => {
            if len == 1 {
                Some(0)
            } else {
                let offset = 1 + (seed % (len as u64 - 1)) as usize;
                Some((idx + offset) % len)
            }
        }
    }
}

/// 取流失败后找下一首：跳过 `blocked[i] == true` 的曲目（取不到流 / 仅搜索的源）。
///
/// - `Sequential`：往后扫，扫到末尾就停（不回绕，保持"播完就停"的语义）
/// - `Shuffle`：用 `seed` 只抽一个候选，抽到坏的继续抽，最多 `len-1` 次
/// - `RepeatOne`：不跳（单曲循环遇到取不到流就该停，不能偷偷换歌）
pub fn next_playable(
    len: usize,
    idx: usize,
    mode: PlayMode,
    blocked: &[bool],
    seed: u64,
) -> Option<usize> {
    if len == 0 || blocked.len() != len || mode == PlayMode::RepeatOne {
        return None;
    }
    let idx = idx.min(len - 1);
    match mode {
        PlayMode::Sequential => (idx + 1..len).find(|i| !blocked[*i]),
        PlayMode::Shuffle => {
            let mut state = seed;
            for _ in 0..len.saturating_sub(1) {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let offset = 1 + (state % (len as u64 - 1)) as usize;
                let cand = (idx + offset) % len;
                if !blocked[cand] {
                    return Some(cand);
                }
            }
            None
        }
        PlayMode::RepeatOne => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_cycle_and_round_trip_through_strings() {
        assert_eq!(PlayMode::Sequential.next(), PlayMode::Shuffle);
        assert_eq!(PlayMode::Shuffle.next(), PlayMode::RepeatOne);
        assert_eq!(PlayMode::RepeatOne.next(), PlayMode::Sequential);
        for m in PlayMode::ALL {
            assert_eq!(PlayMode::parse(m.as_str()), m);
        }
        assert_eq!(PlayMode::parse("nonsense"), PlayMode::Sequential);
    }

    #[test]
    fn next_index_on_empty_queue_is_none() {
        for m in [PlayMode::Sequential, PlayMode::Shuffle, PlayMode::RepeatOne] {
            assert_eq!(next_index(0, 0, m, 1), None);
        }
    }

    #[test]
    fn sequential_stops_at_end() {
        assert_eq!(next_index(3, 0, PlayMode::Sequential, 0), Some(1));
        assert_eq!(next_index(3, 1, PlayMode::Sequential, 0), Some(2));
        assert_eq!(next_index(3, 2, PlayMode::Sequential, 0), None);
    }

    #[test]
    fn repeat_one_stays_on_current() {
        assert_eq!(next_index(3, 1, PlayMode::RepeatOne, 0), Some(1));
        assert_eq!(next_index(1, 0, PlayMode::RepeatOne, 7), Some(0));
    }

    #[test]
    fn shuffle_never_picks_current_and_covers_others() {
        for seed in 0..50u64 {
            let n = next_index(5, 2, PlayMode::Shuffle, seed).unwrap();
            assert_ne!(n, 2);
            assert!(n < 5);
        }
        // 两首时只有"另一首"
        assert_eq!(next_index(2, 0, PlayMode::Shuffle, 0), Some(1));
        assert_eq!(next_index(2, 1, PlayMode::Shuffle, 0), Some(0));
        // 单曲随机 = 它自己
        assert_eq!(next_index(1, 0, PlayMode::Shuffle, 3), Some(0));
    }

    #[test]
    fn single_song_sequential_stops() {
        assert_eq!(next_index(1, 0, PlayMode::Sequential, 0), None);
    }

    #[test]
    fn out_of_range_index_is_clamped() {
        assert_eq!(next_index(2, 99, PlayMode::Sequential, 0), None);
        assert_eq!(next_index(2, 99, PlayMode::RepeatOne, 0), Some(1));
    }

    #[test]
    fn every_mode_has_a_label() {
        for m in PlayMode::ALL {
            assert!(!m.label().is_empty());
        }
    }
}

#[cfg(test)]
mod playable_tests {
    use super::*;

    #[test]
    fn sequential_skips_blocked_and_stops_at_end() {
        let blocked = [false, false, true, false];
        assert_eq!(
            next_playable(4, 0, PlayMode::Sequential, &blocked, 0),
            Some(1)
        );
        // 从 1 往后：2 被挡 → 3
        assert_eq!(
            next_playable(4, 1, PlayMode::Sequential, &blocked, 0),
            Some(3)
        );
        // 3 是最后一首 → 停
        assert_eq!(next_playable(4, 3, PlayMode::Sequential, &blocked, 0), None);
        // 末尾被挡也不回绕
        let blocked = [false, true, false, true];
        assert_eq!(next_playable(4, 2, PlayMode::Sequential, &blocked, 0), None);
    }

    #[test]
    fn all_blocked_yields_none() {
        let blocked = [true, true, true];
        for mode in [PlayMode::Sequential, PlayMode::Shuffle] {
            assert_eq!(next_playable(3, 0, mode, &blocked, 7), None);
        }
    }

    #[test]
    fn repeat_one_never_switches_song_even_if_blocked() {
        let blocked = [true, false];
        assert_eq!(next_playable(2, 0, PlayMode::RepeatOne, &blocked, 0), None);
    }

    #[test]
    fn shuffle_never_returns_blocked_or_current() {
        let blocked = [false, true, false, false, true];
        for seed in 0..40u64 {
            if let Some(i) = next_playable(5, 0, PlayMode::Shuffle, &blocked, seed) {
                assert!(!blocked[i]);
                assert_ne!(i, 0);
            }
        }
        // 只剩当前可播时没有别的可跳
        let blocked = [false, true, true];
        assert_eq!(next_playable(3, 0, PlayMode::Shuffle, &blocked, 1), None);
    }

    #[test]
    fn bad_blocked_len_is_rejected() {
        assert_eq!(next_playable(3, 0, PlayMode::Sequential, &[false], 0), None);
    }
}
