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
