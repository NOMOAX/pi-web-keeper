//! 界面主题：跟随系统、深色、浅色。
//!
//! 三档都在 egui 自己的主题上做：`Visuals::dark()` 与 `Visuals::light()`，
//! 不另配调色板 —— 界面上的颜色本来就是这个程序唯一的那份「皮肤」。
//!
//! 「跟随系统」读的是 Windows 的**应用**模式，不是 Windows 模式：
//! 前者管应用窗口的深浅，后者管标题栏、开始菜单与任务栏。
//! 对应注册表值 `AppsUseLightTheme`（1 = 浅色，0 = 深色）。
//!
//! 这个选择只存在内存里：程序每次启动都回到「跟随系统」，
//! 因为一个小启动器没有值得写进磁盘的偏好。

use egui::Context;

/// 用户在界面上选的主题。
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Preference {
    /// 跟随 Windows 的应用模式。
    #[default]
    System,

    /// 固定深色。
    Dark,

    /// 固定浅色。
    Light,
}

impl Preference {
    /// 三档的顺序就是按钮从左到右的顺序。
    pub const ALL: [Self; 3] = [Self::System, Self::Dark, Self::Light];

    /// 按钮上的字。
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "系统",
            Self::Dark => "深色",
            Self::Light => "浅色",
        }
    }
}

/// 装上主题，返回这次用的是不是深色。
pub fn apply(context: &Context, preference: Preference) -> bool {
    let dark = match preference {
        Preference::System => system_is_dark(),
        Preference::Dark => true,
        Preference::Light => false,
    };

    /*
     * 每次都从 `Visuals::dark()` / `light()` 重新开始，而不是在上一份上改：
     * 这两份是 egui 的完整预设，来回切换时才不会留下上一档的痕迹。
     */
    context.set_visuals(if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    });

    dark
}

/// Windows 的**应用**模式当前是不是深色。
///
/// 读不到就按深色算：本程序一直以来的样子就是深色，退回浅色反而像是新行为。
pub fn system_is_dark() -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

    // 注册表 API 要的是以 0 结尾的宽字符串，且不接收长度参数
    let subkey: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0"
        .encode_utf16()
        .collect();
    let value: Vec<u16> = "AppsUseLightTheme\0".encode_utf16().collect();

    let mut data: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;

    // SAFETY: 两个字符串都以 0 结尾；缓冲区与声明的长度一致
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut data as *mut u32 as *mut core::ffi::c_void),
            Some(&mut size),
        )
    };

    if status != ERROR_SUCCESS {
        return true;
    }

    data == 0
}
