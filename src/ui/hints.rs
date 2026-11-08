//! 底部提示栏：只列当前页面真能用的键。
//!
//! 之前是一条写死的全量提示，每页都一样，既长又误导。这里做成纯映射表，方便单测。

use super::ScreenKind;

/// 按页面 + 搜索页焦点状态返回提示文本（不含"已选 N 首"这类动态内容）。
pub fn hints(screen: ScreenKind, search_input_focus: bool) -> &'static str {
    match screen {
        ScreenKind::Menu => "j/k 选择  Enter 进入  s 搜索  g 下载  q 退出",
        ScreenKind::Search => {
            if search_input_focus {
                "输入关键词  Enter 搜索  Tab 到列表  F1-F4 换类型  Esc 返回"
            } else {
                "j/k 选择  Enter 播放  d 下载  l 收藏  t 换类型  S 换源  Tab 回到输入  Esc 返回"
            }
        }
        ScreenKind::Songs => "j/k 选择  Enter 播放  d 下载  l 收藏  Alt+Enter 多选  Esc 返回",
        ScreenKind::Albums => "j/k 选择  Enter 打开专辑  Esc 返回",
        ScreenKind::Playlists => "j/k 选择  Enter 打开歌单  Esc 返回",
        ScreenKind::Library => "j/k 选择  Enter 播放  d 下载  l 收藏  Esc 返回",
        ScreenKind::NowPlaying => "空格 暂停  n/p 切歌  -/= 音量  i 歌曲设置  Esc 返回",
        ScreenKind::Downloads => "j/k 选择  Esc 返回",
        ScreenKind::Favorites => "j/k 选择  Enter 播放  t 切视图  x 移除  Esc 返回",
        ScreenKind::SuperDl => "Esc 返回  q 退出",
        ScreenKind::Settings => "j/k 选择  ←/→ 切换  Esc 返回",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [ScreenKind; 11] = [
        ScreenKind::Menu,
        ScreenKind::Search,
        ScreenKind::Songs,
        ScreenKind::Albums,
        ScreenKind::Playlists,
        ScreenKind::Library,
        ScreenKind::NowPlaying,
        ScreenKind::Downloads,
        ScreenKind::Favorites,
        ScreenKind::SuperDl,
        ScreenKind::Settings,
    ];

    #[test]
    fn every_screen_has_a_hint() {
        for s in ALL {
            for focus in [true, false] {
                let h = hints(s, focus);
                assert!(!h.is_empty(), "{s:?} 无提示");
            }
        }
    }

    #[test]
    fn hints_stay_short_enough_for_one_line() {
        for s in ALL {
            for focus in [true, false] {
                let n = hints(s, focus).chars().count();
                assert!(n <= 90, "{s:?} 提示过长: {n} 字符");
            }
        }
    }

    #[test]
    fn search_hint_follows_focus() {
        assert!(hints(ScreenKind::Search, true).contains("Tab 到列表"));
        assert!(hints(ScreenKind::Search, false).contains("Tab 回到输入"));
        assert!(hints(ScreenKind::Search, true).contains("F1-F4"));
    }

    #[test]
    fn only_now_playing_advertises_the_song_modal() {
        assert!(hints(ScreenKind::NowPlaying, false).contains("i 歌曲设置"));
        for s in [ScreenKind::Menu, ScreenKind::Songs, ScreenKind::Library] {
            assert!(!hints(s, false).contains("i 歌曲设置"), "{s:?} 不该提示 i");
        }
    }

    #[test]
    fn menu_hint_does_not_advertise_playback_keys() {
        let h = hints(ScreenKind::Menu, false);
        for k in ["空格", "n/p", "i 歌曲"] {
            assert!(!h.contains(k), "菜单不该出现 {k}");
        }
    }
}
