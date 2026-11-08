//! 导航返回栈。
//!
//! 原来是单槽 `back: Option<ScreenKind>`，进「正在播放」还绕过 `goto` 直接赋值，
//! 于是 Esc 会回到一个陈旧的页面（表现为“直接回菜单”），多层下钻也只能回一层。
//! 这里换成真栈：每次前进压入来源页，Esc 弹出一层，栈空则什么都不做。

use super::ScreenKind;

#[derive(Default)]
pub struct NavStack {
    stack: Vec<ScreenKind>,
}

impl NavStack {
    /// 离开 `from` 进入新页面：压入来源页；连续重复的不重复压。
    pub fn enter(&mut self, from: ScreenKind) {
        if self.stack.last() != Some(&from) {
            self.stack.push(from);
        }
    }

    /// 返回上一页；栈空返回 None（调用方应什么都不做）。
    pub fn leave(&mut self) -> Option<ScreenKind> {
        self.stack.pop()
    }

    /// 回根页（主菜单）：清空栈，不压栈。否则 `h` 回菜单后按 Esc 会回到刚才那页。
    pub fn reset(&mut self) {
        self.stack.clear();
    }

    /// 前进：目标是根页就清栈，否则压入来源页。
    pub fn go(&mut self, from: ScreenKind, to: ScreenKind) {
        if to == ScreenKind::Menu {
            self.reset();
        } else {
            self.enter(from);
        }
    }

    /// 后退：主菜单是根页，Esc 一律 no-op（双保险，就算栈里有残留也不动）。
    pub fn back(&mut self, current: ScreenKind) -> Option<ScreenKind> {
        if current == ScreenKind::Menu {
            return None;
        }
        self.leave()
    }

    #[cfg(test)]
    pub fn depth(&self) -> usize {
        self.stack.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drill_down_pops_back_in_reverse_order() {
        let mut nav = NavStack::default();
        nav.enter(ScreenKind::Menu);
        nav.enter(ScreenKind::Search);
        nav.enter(ScreenKind::Albums);
        assert_eq!(nav.depth(), 3);
        assert_eq!(nav.leave(), Some(ScreenKind::Albums));
        assert_eq!(nav.leave(), Some(ScreenKind::Search));
        assert_eq!(nav.leave(), Some(ScreenKind::Menu));
        assert_eq!(nav.leave(), None);
    }

    #[test]
    fn repeated_enter_of_same_page_does_not_grow_stack() {
        let mut nav = NavStack::default();
        nav.enter(ScreenKind::Menu);
        nav.enter(ScreenKind::Menu);
        nav.enter(ScreenKind::Menu);
        assert_eq!(nav.depth(), 1);
        assert_eq!(nav.leave(), Some(ScreenKind::Menu));
        assert_eq!(nav.leave(), None);
    }

    #[test]
    fn leave_on_empty_stack_is_a_noop() {
        let mut nav = NavStack::default();
        assert_eq!(nav.leave(), None);
        assert_eq!(nav.leave(), None);
        assert_eq!(nav.depth(), 0);
    }

    #[test]
    fn going_home_clears_the_stack() {
        let mut nav = NavStack::default();
        nav.go(ScreenKind::Menu, ScreenKind::Search);
        nav.go(ScreenKind::Search, ScreenKind::Albums);
        assert_eq!(nav.depth(), 2);
        nav.go(ScreenKind::Albums, ScreenKind::Menu);
        assert_eq!(nav.depth(), 0);
    }

    #[test]
    fn esc_on_menu_after_going_home_from_search_does_nothing() {
        let mut nav = NavStack::default();
        nav.go(ScreenKind::Menu, ScreenKind::Search);
        nav.go(ScreenKind::Search, ScreenKind::Menu); // h 回菜单
        assert_eq!(nav.back(ScreenKind::Menu), None);
    }

    #[test]
    fn esc_on_menu_after_drill_then_home_does_nothing() {
        let mut nav = NavStack::default();
        nav.go(ScreenKind::Menu, ScreenKind::Search);
        nav.go(ScreenKind::Search, ScreenKind::Albums);
        nav.go(ScreenKind::Albums, ScreenKind::Menu);
        assert_eq!(nav.back(ScreenKind::Menu), None);
        assert_eq!(nav.depth(), 0);
    }

    #[test]
    fn esc_on_menu_is_noop_even_with_leftover_entries() {
        let mut nav = NavStack::default();
        nav.enter(ScreenKind::Search);
        assert_eq!(nav.back(ScreenKind::Menu), None);
    }

    #[test]
    fn now_playing_returns_to_the_page_it_started_from() {
        let mut nav = NavStack::default();
        nav.enter(ScreenKind::Songs);
        nav.enter(ScreenKind::NowPlaying);
        assert_eq!(nav.leave(), Some(ScreenKind::NowPlaying));
        assert_eq!(nav.leave(), Some(ScreenKind::Songs));
    }
}
