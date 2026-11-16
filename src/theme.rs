//! 配色。
//!
//! 这些数值不是挑出来的，是从上一版窗口的截图上量出来的：窗口底 `#202020`、
//! 控件面 `#2D2D30`、面板边线 `#343437`、文字纯白、强调色 `#0078D4`。
//! 量的是仓库里那张窗口截图（`0e7d34d:docs/launcher-window.png`，之后随重写一起
//! 从工作树里删掉了，需要时从 git 历史里取）：
//!
//! | 位置 | 量到的值 |
//! |---|---|
//! | 窗口底色 | `#201F22` |
//! | 按钮面 | `#2C2D30` / `#2D2D30` |
//! | 按钮、消息框边线 | `#312F32` 上下 |
//! | 消息框面 | `#2E2C2F` |
//! | 按钮文字、日志文字 | `#FFFFFF` |
//! | 禁用按钮面 / 文字 | `#2A2A2C` / `#9E9E9F` |
//!
//! 之前用的是 `egui::Visuals::dark()` —— egui 自带的深色主题，一个数值都没改过，
//! 于是窗口底是 `#1B1B1B`（egui 的 gray(27)）、按钮面 `#3C3C3C`（gray(60)）、
//! 消息框 `#0A0A0A`（gray(10)）、文字 `#B4B4B4`（gray(180)）：
//! 比上一版整体更暗、文字更灰、按钮反而更亮，看着就是另一套皮肤。
//!
//! 禁用态不单独配：egui 把控件画好之后整体往底色调淡，量到的 `#2A2A2C` / `#9E9E9F`
//! 正好是这个淡化在白字、`#2D2D30` 面上落在的位置。

use eframe::egui;
use egui::{Color32, Rounding, Stroke};

/// 窗口与面板底色。
const WINDOW_BACKGROUND: Color32 = Color32::from_rgb(0x20, 0x20, 0x20);

/// 按钮、消息框的面。
const CONTROL_BACKGROUND: Color32 = Color32::from_rgb(0x2D, 0x2D, 0x30);

/// 按钮、消息框的 1 像素边线。比面色亮一点点，亮多了就变成框。
const CONTROL_BORDER: Color32 = Color32::from_rgb(0x31, 0x31, 0x34);

/// 文字。上一版这里是纯白，不是深色主题常见的淡灰。
const TEXT: Color32 = Color32::WHITE;

/// 强调色：消息框获得焦点时的边线、选区的底色。
const ACCENT: Color32 = Color32::from_rgb(0x00, 0x78, 0xD4);

/// 圆角。按钮与消息框都是 4，与上一版一致。
const CORNER: f32 = 4.0;

/// 边线宽度。上一版是 1 DIP。
const BORDER_WIDTH: f32 = 1.0;

/// 装上这套配色。
pub fn install(context: &egui::Context) {
    let mut visuals = egui::Visuals::dark();

    visuals.window_fill = WINDOW_BACKGROUND;
    visuals.panel_fill = WINDOW_BACKGROUND;
    visuals.window_stroke = Stroke::new(BORDER_WIDTH, CONTROL_BORDER);

    // 消息框的底色走这个字段（`egui::TextEdit` 用它填自己的框）
    visuals.extreme_bg_color = CONTROL_BACKGROUND;
    visuals.code_bg_color = CONTROL_BACKGROUND;

    visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(0x00, 0x78, 0xD4, 120);
    visuals.selection.stroke = Stroke::new(BORDER_WIDTH, ACCENT);

    /*
     * 按钮、消息框在常态下都读 `inactive`：面、1 像素边线、文字颜色、圆角。
     * 消息框没有焦点时画的也是这条边线（`TextEdit` 取 `interact().bg_stroke`）。
     */
    visuals.widgets.inactive.weak_bg_fill = CONTROL_BACKGROUND;
    visuals.widgets.inactive.bg_fill = CONTROL_BACKGROUND;
    visuals.widgets.inactive.bg_stroke = Stroke::new(BORDER_WIDTH, CONTROL_BORDER);
    visuals.widgets.inactive.fg_stroke = Stroke::new(BORDER_WIDTH, TEXT);
    visuals.widgets.inactive.rounding = Rounding::same(CORNER);

    // 悬停：面色提一级、边线提一级，文字保持纯白（上一版也是纯白，不随状态变色）
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x33, 0x33, 0x36);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(0x33, 0x33, 0x36);
    visuals.widgets.hovered.bg_stroke = Stroke::new(BORDER_WIDTH, Color32::from_rgb(0x3D, 0x3D, 0x40));
    visuals.widgets.hovered.fg_stroke = Stroke::new(BORDER_WIDTH, TEXT);
    visuals.widgets.hovered.rounding = Rounding::same(CORNER);
    visuals.widgets.hovered.expansion = 0.0;

    // 按下：比常态再暗一点，像被按进去
    visuals.widgets.active.weak_bg_fill = Color32::from_rgb(0x26, 0x26, 0x29);
    visuals.widgets.active.bg_fill = Color32::from_rgb(0x26, 0x26, 0x29);
    visuals.widgets.active.bg_stroke = Stroke::new(BORDER_WIDTH, CONTROL_BORDER);
    visuals.widgets.active.fg_stroke = Stroke::new(BORDER_WIDTH, TEXT);
    visuals.widgets.active.rounding = Rounding::same(CORNER);
    visuals.widgets.active.expansion = 0.0;

    visuals.widgets.open.weak_bg_fill = CONTROL_BACKGROUND;
    visuals.widgets.open.bg_stroke = Stroke::new(BORDER_WIDTH, CONTROL_BORDER);
    visuals.widgets.open.fg_stroke = Stroke::new(BORDER_WIDTH, TEXT);
    visuals.widgets.open.rounding = Rounding::same(CORNER);

    // 面板、分隔线一类不属于任何控件的东西
    visuals.widgets.noninteractive.bg_fill = WINDOW_BACKGROUND;
    visuals.widgets.noninteractive.weak_bg_fill = WINDOW_BACKGROUND;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(BORDER_WIDTH, CONTROL_BORDER);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(BORDER_WIDTH, TEXT);

    context.set_visuals(visuals);
}
