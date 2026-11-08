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
    fn now_playing_returns_to_the_page_it_started_from() {
        let mut nav = NavStack::default();
        nav.enter(ScreenKind::Songs);
        nav.enter(ScreenKind::NowPlaying);
        assert_eq!(nav.leave(), Some(ScreenKind::NowPlaying));
        assert_eq!(nav.leave(), Some(ScreenKind::Songs));
    }
}
