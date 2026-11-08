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
    fn every_mode_has_a_label() {
        for m in PlayMode::ALL {
            assert!(!m.label().is_empty());
        }
    }
}
