//! 界面字体：**内嵌**的微软雅黑。
//!
//! 字体在构建期从 `%WINDIR%\Fonts\msyh.ttc` 取，`build.rs` 把它复制到 `OUT_DIR`，
//! 这里用 `include_bytes!` 编进 exe —— 运行时不再读系统字体，也不去系统里挑候选。
//!
//! 几个决定是显式定下来的：
//!   - 只有一款字体。候选表（黑体 → 等线 → 楷体 → 仿宋 → 隶书）会让同一份程序在不同机器上
//!     长成不同的样子，而字体是外观的一部分，不该由机器决定。
//!   - 微软雅黑而不是 WPF 版用的 NSimSun：egui 走 `ab_glyph`，只画轮廓，
//!     宋体那套小字号点阵用不上，12 号以下会发糊；雅黑的字形本来就是为屏显做的。
//!
//! 代价写在这里，免得下一个人以为是免费的：`msyh.ttc` 是 18.8 MB，
//! 也就是说这个 exe 有大半体积是它。真要压，办法是对字表做子集化（日志里的文本是动态的，
//! 子集要按 GBK 常用字来划，别只按界面文案划）。
//!
//! 还有一条不是技术问题：微软雅黑随 Windows 授权使用，把它编进对外分发的 exe
//! 等于再分发这款字体，那需要微软的许可。自己用没关系，发 release 之前要留意。
//!
//! `msyh.ttc` 是字体集合（YaHei / YaHei UI / YaHei Light 三家），所以要指定 `index`；
//! `epaint` 把它传给 `ab_glyph::FontVec::try_from_vec_and_index`，索引 0 是 YaHei 本体。

use egui::{Context, FontData, FontDefinitions, FontFamily};

/// 构建期由 `build.rs` 放进 `OUT_DIR` 的字体。
const FONT_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/msyh.ttc"));

/// 字体集合里的第几张：0 = Microsoft YaHei。
const FONT_INDEX: u32 = 0;

/// 字体在 egui 里的键名。
const FONT_KEY: &str = "PiWebKeeper.Ui";

/// 装上内嵌字体，返回结果描述（给日志用）。
pub fn install(context: &Context) -> String {
    let mut fonts = FontDefinitions::default();

    let mut data = FontData::from_static(FONT_BYTES);
    data.index = FONT_INDEX;

    fonts.font_data.insert(FONT_KEY.to_owned(), data);

    /*
     * 两个字族都排在第一位：比例字族给按钮，等宽字族给日志。
     * egui 自带的字体退到后面当兜底 —— 雅黑里没有的字（某些符号、emoji）
     * 才不会变成方框。
     */
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, FONT_KEY.to_owned());
    }

    context.set_fonts(fonts);

    format!("微软雅黑（内嵌，{} KB）", FONT_BYTES.len() / 1024)
}
