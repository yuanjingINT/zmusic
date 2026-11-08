//! 搜索页的按键分类：焦点在两态（输入栏 / 结果列表）之间切换。
//!
//! 背景：原来 `Tab` 同时兼做“切搜索类型”，而切类型会触发重新搜索、
//! `do_search` 又把焦点置假，于是每次切类型都跳出输入框，后续输入全落进全局快捷键
//! （`d` 触发下载、`q` 直接退出）。这里把决定“焦点/类型”的纯逻辑单独拎出来，方便单测。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// 列表聚焦时仍然保留的全局快捷键字符；其余可打印字符一律回到输入栏插入。
/// 注意 `s` 不在表里：搜索页上 `s` 本来只是重进搜索页，没有意义，当成正常输入更好。
pub const LIST_SHORTCUTS: &[char] = &[
    'q', 'h', 'g', 'v', 'd', 'D', 'l', 'x', 'A', 't', 'n', 'p', 'k', 'j', 'S', '0', '1', '2', '3',
    '[', ']', '+', '=', '-', ' ',
];

#[derive(Debug, PartialEq, Eq)]
pub enum Nav {
    /// 不改变焦点/类型，交给后面的通用按键处理
    None,
    FocusInput,
    FocusList,
    SetType(usize),
}

/// 决定 Tab / 功能键 / Alt(Ctrl)+数字 的语义。与焦点无关的部分（F1-F4、Alt+数字）
/// 在两种焦点下都生效，保证输入栏聚焦时也能换类型。
pub fn nav(k: &KeyEvent, input_focused: bool) -> Nav {
    match k.code {
        KeyCode::Tab => {
            if input_focused {
                Nav::FocusList
            } else {
                Nav::FocusInput
            }
        }
        KeyCode::F(n) if (1..=4).contains(&n) => Nav::SetType((n - 1) as usize),
        KeyCode::Char(c) if c.is_ascii_digit() => {
            let idx = (c as u8 - b'0') as usize;
            let mods = k.modifiers;
            if idx <= 3
                && (mods.contains(KeyModifiers::ALT) || mods.contains(KeyModifiers::CONTROL))
            {
                Nav::SetType(idx)
            } else {
                Nav::None
            }
        }
        _ => Nav::None,
    }
}

/// 列表聚焦时，这个按键是不是“普通可打印字符”。
/// 是的话调用方应把焦点切回输入栏并把字符插进查询串，避免打字丢字。
pub fn printable_to_input(k: &KeyEvent) -> Option<char> {
    if !k.modifiers.is_empty() {
        return None;
    }
    match k.code {
        KeyCode::Char(c) if !c.is_control() && !LIST_SHORTCUTS.contains(&c) => Some(c),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }
    fn alt(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)
    }
    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn tab_toggles_focus_instead_of_switching_type() {
        assert_eq!(
            nav(&KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), true),
            Nav::FocusList
        );
        assert_eq!(
            nav(&KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), false),
            Nav::FocusInput
        );
    }

    #[test]
    fn function_keys_set_type_in_both_focus_states() {
        for focused in [true, false] {
            for n in 1..=4u8 {
                assert_eq!(
                    nav(&KeyEvent::new(KeyCode::F(n), KeyModifiers::NONE), focused),
                    Nav::SetType((n - 1) as usize)
                );
            }
        }
    }

    #[test]
    fn alt_and_ctrl_digits_set_type() {
        assert_eq!(nav(&alt('0'), true), Nav::SetType(0));
        assert_eq!(nav(&alt('3'), false), Nav::SetType(3));
        assert_eq!(nav(&ctrl('2'), true), Nav::SetType(2));
        // 只有 0-3 是类型；其他数字不是
        assert_eq!(nav(&alt('4'), true), Nav::None);
        assert_eq!(nav(&alt('9'), false), Nav::None);
    }

    #[test]
    fn plain_digits_are_not_a_type_switch() {
        assert_eq!(nav(&ch('1'), true), Nav::None);
        // 列表聚焦时数字仍走全局快捷键（SetType），但分类层不拦
        assert_eq!(nav(&ch('1'), false), Nav::None);
    }

    #[test]
    fn typing_in_list_focus_refocuses_input() {
        assert_eq!(printable_to_input(&ch('周')), Some('周'));
        assert_eq!(printable_to_input(&ch('e')), Some('e'));
        // 搜索页上 s 无意义，当普通输入
        assert_eq!(printable_to_input(&ch('s')), Some('s'));
    }

    #[test]
    fn shortcuts_keep_working_in_list_focus() {
        for c in [
            'd', 'D', 'l', 'x', 'A', 't', 'n', 'p', 'j', 'k', 'q', 'h', 'g', 'v', ' ', '0', '1',
            '2', '3',
        ] {
            assert_eq!(printable_to_input(&ch(c)), None, "{c} 应保留为快捷键");
        }
    }
}
