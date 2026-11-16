//! 界面字体：**内嵌**的思源黑体（Source Han Sans SC Regular）。
//!
//! 字体文件就是 `assets/fonts/NotoSansSC-Regular.otf`，随仓库走、编译进来，
//! 运行时既不读系统字体，也不去系统里挑候选。
//!
//! 几个决定是显式定下来的：
//!   - 只有一款字体。之前那张候选表（黑体 → 等线 → 楷体 → 仿宋 → 隶书）会让同一份程序
//!     在不同机器上长成不同的样子，而字体是外观的一部分，不该由机器决定。
//!   - 无衬线而不是 WPF 版用的 NSimSun：egui 走 `ab_glyph`，只画轮廓，
//!     宋体那套小字号点阵用不上，12 号以下会发糊。
//!   - 开源字体而不是微软雅黑：雅黑随 Windows 授权使用，编进对外分发的 exe
//!     等于再分发它。思源黑体是 SIL OFL 1.1，可以随程序分发 ——
//!     条件是**许可全文要跟着一起发**（见 `assets/fonts/LICENSE.txt`，发布包里要带上）。
//!
//! `NotoSansSC-Regular.otf` 是 Noto CJK 项目里「SubsetOTF / SC」那一档：
//! 思源黑体的简体中文子集，单一字重、单面 OTF，8 MB。
//! 全量思源黑体带日韩字形，要大一倍多，而这个程序的界面只有简体中文和拉丁字符。

use egui::{Context, FontData, FontDefinitions, FontFamily};

/// 构建期编进来的字体。
const FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/NotoSansSC-Regular.otf");

/// 单面 OTF，不需要选字面。
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
     * egui 自带的字体退到后面当兜底 —— 思源黑体里没有的字（某些符号、emoji）
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

    format!("思源黑体 SC Regular（内嵌，{} KB）", FONT_BYTES.len() / 1024)
}
