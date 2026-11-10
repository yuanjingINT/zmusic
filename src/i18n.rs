//! 极小的界面文案表：只覆盖主界面骨架（菜单、标题、快捷键提示、设置页）。
//!
//! 设计取舍：先让「语言」这个选项真的起作用，而不是先做一套完整的 i18n 框架。
//! 深层提示（toast、抓歌失败的原文）暂时仍是中文，遇到时再逐条补。

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Zh,
    En,
}

static LANG: AtomicU8 = AtomicU8::new(0); // 0 = 中文，1 = English

/// 可以随时切：首次向导里一按方向键就要立刻换语言。
pub fn set_lang(l: Lang) {
    LANG.store(
        match l {
            Lang::Zh => 0,
            Lang::En => 1,
        },
        Ordering::Relaxed,
    );
}

pub fn lang() -> Lang {
    if LANG.load(Ordering::Relaxed) == 1 {
        Lang::En
    } else {
        Lang::Zh
    }
}

/// 当前是不是英文模式
pub fn en() -> bool {
    lang() == Lang::En
}

/// 一次性文案（toast、报错）直接给中英两版，按当前语言挑一个。
///
/// 骨架文案用上面的 `t(key)` 表；这里给零散的动态句子用，省得为每句话都起个键。
pub fn pick(zh: impl Into<String>, en_text: impl Into<String>) -> String {
    if en() {
        en_text.into()
    } else {
        zh.into()
    }
}

/// `tr!("中文 {}","English {}", x)` → 按当前语言格式化
#[macro_export]
macro_rules! tr {
    ($zh:literal, $en:literal) => {
        $crate::i18n::pick($zh, $en)
    };
    ($zh:literal, $en:literal, $($arg:tt)*) => {
        $crate::i18n::pick(format!($zh, $($arg)*), format!($en, $($arg)*))
    };
}

/// 按当前语言取一条文案。键在下面这张表里，缺键就回退到中文原文。
pub fn t(key: &str) -> &'static str {
    let en = lang() == Lang::En;
    match (key, en) {
        // 主菜单
        ("menu.search", false) => "搜索音乐",
        ("menu.search", true) => "Search",
        ("menu.songs", false) => "歌曲推荐",
        ("menu.songs", true) => "Songs",
        ("menu.albums", false) => "专辑推荐",
        ("menu.albums", true) => "Albums",
        ("menu.playlists", false) => "歌单推荐",
        ("menu.playlists", true) => "Playlists",
        ("menu.downloads", false) => "我的下载",
        ("menu.downloads", true) => "Downloads",
        ("menu.favorites", false) => "我的收藏",
        ("menu.favorites", true) => "Favorites",
        ("menu.superdl", false) => "超级下载",
        ("menu.superdl", true) => "Super download",
        ("menu.title", false) => "功能模块",
        ("menu.title", true) => "Main menu",
        // 屏幕标题
        ("title.search", false) => "搜索音乐",
        ("title.search", true) => "Search",
        ("title.songs", false) => "歌曲推荐",
        ("title.songs", true) => "Songs",
        ("title.albums", false) => "专辑推荐",
        ("title.albums", true) => "Albums",
        ("title.playlists", false) => "歌单推荐",
        ("title.playlists", true) => "Playlists",
        ("title.library", false) => "曲目列表",
        ("title.library", true) => "Tracks",
        ("title.nowplaying", false) => "正在播放",
        ("title.nowplaying", true) => "Now playing",
        ("title.downloads", false) => "我的下载",
        ("title.downloads", true) => "Downloads",
        ("title.favorites", false) => "我的收藏",
        ("title.favorites", true) => "Favorites",
        ("title.superdl", false) => "超级下载",
        ("title.superdl", true) => "Super download",
        ("title.settings", false) => "设置",
        ("title.settings", true) => "Settings",
        ("title.not_playing", false) => "未在播放",
        ("title.not_playing", true) => "Nothing playing",
        ("label.source", false) => "源",
        ("label.source", true) => "Source",
        ("label.quality", false) => "音质",
        ("label.quality", true) => "Quality",
        ("label.auto_gain", false) => "自动",
        ("label.auto_gain", true) => "auto",
        // 设置向导 / 设置页
        ("wizard.title", false) => "首次使用设置",
        ("wizard.title", true) => "First-run setup",
        ("wizard.lang", false) => "界面语言",
        ("wizard.lang", true) => "Language",
        ("wizard.theme", false) => "主题",
        ("wizard.theme", true) => "Theme",
        ("wizard.wave", false) => "播放时显示波形",
        ("wizard.wave", true) => "Show waveform while playing",
        ("wizard.dir", false) => "下载目录",
        ("wizard.dir", true) => "Download folder",
        ("wizard.quality", false) => "音质",
        ("wizard.quality", true) => "Audio quality",
        ("wizard.next", false) => "Enter 下一步   Esc 用默认值跳过",
        ("wizard.next", true) => "Enter next   Esc skip with defaults",
        ("wizard.done", false) => "完成！按 Enter 进入 zmuse",
        ("wizard.done", true) => "Done! Press Enter to start",
        // 设置页
        ("set.title", false) => "设置",
        ("set.title", true) => "Settings",
        ("set.theme", false) => "外观 · 主题",
        ("set.theme", true) => "Appearance · theme",
        ("set.wave", false) => "外观 · 波形",
        ("set.wave", true) => "Appearance · waveform",
        ("set.spacing", false) => "外观 · 波形间距",
        ("set.spacing", true) => "Appearance · waveform spacing",
        ("set.pad", false) => "外观 · 波形上下留白",
        ("set.pad", true) => "Appearance · waveform padding",
        ("set.anim", false) => "外观 · 总动画",
        ("set.anim", true) => "Appearance · animations",
        ("set.spinner", false) => "外观 · 加载转圈",
        ("set.spinner", true) => "Appearance · loading spinner",
        ("set.download", false) => "下载 · 目录",
        ("set.download", true) => "Downloads · folder",
        ("set.superdl", false) => "超级下载 · 目标目录",
        ("set.superdl", true) => "Super download · folder",
        ("set.quality", false) => "音质",
        ("set.quality", true) => "Quality",
        ("set.lang", false) => "界面语言",
        ("set.lang", true) => "Language",
        ("set.help", false) => "j/k ↑↓ 选择   h/l ←→ 切换   Enter 编辑/切换   s 保存   q 保存并退出",
        ("set.help", true) => "j/k ↑↓ move  h/l ←→ change  Enter edit  s save  q save & quit",
        // 通用
        ("common.on", false) => "开",
        ("common.on", true) => "on",
        ("common.off", false) => "关",
        ("common.off", true) => "off",
        ("common.rows", false) => "行",
        ("common.rows", true) => "rows",
        (_, false) => "",
        (_, true) => "",
    }
}
