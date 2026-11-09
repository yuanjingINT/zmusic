//! 底部提示栏：只列当前页面真能用的键。
//!
//! 之前是一条写死的全量提示，每页都一样，既长又误导。这里做成纯映射表，方便单测。

use super::ScreenKind;

/// 按页面 + 搜索页焦点状态返回提示文本（不含"已选 N 首"这类动态内容）。
pub fn hints(screen: ScreenKind, search_input_focus: bool) -> &'static str {
    if crate::i18n::en() {
        hints_en(screen, search_input_focus)
    } else {
        hints_zh(screen, search_input_focus)
    }
}

fn hints_zh(screen: ScreenKind, search_input_focus: bool) -> &'static str {
    match screen {
        ScreenKind::Menu => "0-6 直达  j/k 选择  Enter 进入  s 搜索  g 下载  q 退出",
        ScreenKind::Search => {
            if search_input_focus {
                "输入关键词  Enter 搜索  Tab 到列表  F1-F4 换类型  Esc 返回"
            } else {
                "j/k 选择  Enter 播放  d 下载  S/F5 换源  R 重探  V 源状态  Tab 回到输入  Esc 返回"
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

fn hints_en(screen: ScreenKind, search_input_focus: bool) -> &'static str {
    match screen {
        ScreenKind::Menu => "0-6 jump  j/k move  Enter open  s search  g downloads  q quit",
        ScreenKind::Search => {
            if search_input_focus {
                "type keywords  Enter search  Tab to list  F1-F4 kind  Esc back"
            } else {
                "j/k move  Enter play  d download  S/F5 source  R re-probe  V source status  Tab to input  Esc back"
            }
        }
        ScreenKind::Songs => "j/k move  Enter play  d download  l favorite  Alt+Enter multi  Esc back",
        ScreenKind::Albums => "j/k move  Enter open album  Esc back",
        ScreenKind::Playlists => "j/k move  Enter open playlist  Esc back",
        ScreenKind::Library => "j/k move  Enter play  d download  l favorite  Esc back",
        ScreenKind::NowPlaying => "Space pause  n/p track  -/= volume  i song options  Esc back",
        ScreenKind::Downloads => "j/k move  Esc back",
        ScreenKind::Favorites => "j/k move  Enter play  t switch view  x remove  Esc back",
        ScreenKind::SuperDl => "Esc back  q quit",
        ScreenKind::Settings => "j/k move  ←/→ change  Esc back",
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

    /// 中英两版都得有，且英文版不许出现汉字（英文模式下不混排）
    #[test]
    fn every_screen_has_a_hint_in_both_languages() {
        for s in ALL {
            for focus in [true, false] {
                assert!(!hints_zh(s, focus).is_empty(), "{s:?} 无中文提示");
                let en = hints_en(s, focus);
                assert!(!en.is_empty(), "{s:?} 无英文提示");
                assert!(
                    !en.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)),
                    "{s:?} 英文提示里混了中文: {en}"
                );
            }
        }
    }

    #[test]
    fn hints_stay_short_enough_for_one_line() {
        for s in ALL {
            for focus in [true, false] {
                let n = hints_zh(s, focus).chars().count();
                assert!(n <= 90, "{s:?} 提示过长: {n} 字符");
            }
        }
    }

    #[test]
    fn search_hint_follows_focus() {
        assert!(hints_zh(ScreenKind::Search, true).contains("Tab 到列表"));
        assert!(hints_zh(ScreenKind::Search, false).contains("Tab 回到输入"));
        assert!(hints_zh(ScreenKind::Search, true).contains("F1-F4"));
        assert!(hints_en(ScreenKind::Search, true).contains("Tab to list"));
        assert!(hints_en(ScreenKind::Search, false).contains("Tab to input"));
    }

    #[test]
    fn only_now_playing_advertises_the_song_modal() {
        assert!(hints_zh(ScreenKind::NowPlaying, false).contains("i 歌曲设置"));
        assert!(hints_en(ScreenKind::NowPlaying, false).contains("i song options"));
        for s in [ScreenKind::Menu, ScreenKind::Songs, ScreenKind::Library] {
            assert!(!hints_zh(s, false).contains("i 歌曲设置"), "{s:?} 不该提示 i");
            assert!(!hints_en(s, false).contains("i song options"), "{s:?} 不该提示 i");
        }
    }

    #[test]
    fn menu_hint_does_not_advertise_playback_keys() {
        let h = hints_zh(ScreenKind::Menu, false);
        for k in ["空格", "n/p", "i 歌曲"] {
            assert!(!h.contains(k), "菜单不该出现 {k}");
        }
        let e = hints_en(ScreenKind::Menu, false);
        for k in ["Space", "n/p", "i song"] {
            assert!(!e.contains(k), "menu must not advertise {k}");
        }
    }
}
