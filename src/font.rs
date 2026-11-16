//! 字体：内嵌优先，系统字体兜底。
//!
//! WPF 那条注释说的是「WPF 只画轮廓，所以用不上宋体自带的小字号点阵，12 DIP 以下发糊」。
//! egui 走的是 `ab_glyph` + `swash`，同样只画轮廓，所以那个问题在这里以同样的形式存在 ——
//! 换语言并不会让它变好，能改善的只有「换一款在小字号下更耐看的字体」。
//!
//! 两种来源：
//!   1. `assets/fonts/` 里放了 `.ttf`/`.otf`：`build.rs` 会把它编进二进制。
//!      这是内嵌路径 —— 用哪款字体由你决定，代码不用改。选开源字体时注意许可，
//!      思源黑体 / 更纱黑体 / 霞鹜文楷 / Maple Mono 都是 SIL OFL，可以随程序分发；
//!      `SimSun` / `NSimSun` 是微软和方正的字，**不能**随程序分发。
//!   2. 没有内嵌文件：从系统里找一款单面 TTF。
//!
//! 系统候选里刻意只有单面 TTF：`simsun.ttc` 与 `msyh.ttc` 是字体集合（TTC），
//! `ab_glyph` 解析不了，塞进去只会让整个字体加载失败。

use std::path::PathBuf;

use egui::{Context, FontData, FontDefinitions, FontFamily};

// build.rs 生成的：EMBEDDED_FONT: Option<(&str, &[u8])>
include!(concat!(env!("OUT_DIR"), "/font_data.rs"));

/// 内嵌字体若存在，它叫什么。
pub const EMBEDDED_NAME: &str = "PiWebKeeper.Embedded";

/// 系统字体候选，按顺序取第一个能读到的。
///
/// 全是单面 TTF：黑体、等线、楷体、仿宋、隶书。
const SYSTEM_CANDIDATES: [&str; 5] = [
    "simhei.ttf",
    "Deng.ttf",
    "simkai.ttf",
    "simfang.ttf",
    "SIMLI.TTF",
];

/// 装上字体，返回实际用上的那一个的名字（给日志用）。
pub fn install(context: &Context) -> String {
    let mut fonts = FontDefinitions::default();

    if let Some((name, bytes)) = EMBEDDED_FONT {
        let key = EMBEDDED_NAME.to_owned();

        fonts
            .font_data
            .insert(key.clone(), FontData::from_static(bytes));

        prefer(&mut fonts, &key);
        context.set_fonts(fonts);

        return format!("内嵌字体（{name}，{} KB）", bytes.len() / 1024);
    }

    if let Some((path, bytes)) = load_first_system_font() {
        let key = "PiWebKeeper.System".to_owned();

        fonts
            .font_data
            .insert(key.clone(), FontData::from_owned(bytes));

        prefer(&mut fonts, &key);
        context.set_fonts(fonts);

        return format!("系统字体（{}）", path.display());
    }

    context.set_fonts(fonts);

    "没有可用的中文字体，界面上的汉字会变成方框。把一款开源字体放进 assets/fonts/ 再重新编译。"
        .to_owned()
}

/// 把某个字体排到两个字族的最前面：比例字族给按钮，等宽字族给日志。
fn prefer(fonts: &mut FontDefinitions, key: &str) {
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, key.to_owned());
    }
}

/// 从系统字体目录里取第一款读得出来的单面 TTF。
fn load_first_system_font() -> Option<(PathBuf, Vec<u8>)> {
    let windows_dir = std::env::var_os("WINDIR").map(PathBuf::from)?;
    let fonts_dir = windows_dir.join("Fonts");

    for candidate in SYSTEM_CANDIDATES {
        let path = fonts_dir.join(candidate);

        if let Ok(bytes) = std::fs::read(&path) {
            return Some((path, bytes));
        }
    }

    None
}
