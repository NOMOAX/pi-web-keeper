//! 界面主题：跟随系统、深色、浅色。
//!
//! 三档都在 egui 自己的主题上做：`Visuals::dark()` 与 `Visuals::light()`，
//! 不另配调色板 —— 界面上的颜色本来就是这个程序唯一的那份「皮肤」。
//!
//! 这里有两个彼此独立的概念，别混：
//!
//!   - **应用模式**（`AppsUseLightTheme`）决定*窗口内容*的深浅 —— 三个按钮管的就是它；
//!   - **Windows 模式**（`SystemUsesLightTheme`）决定*标题栏与最小化/最大化/关闭按钮*
//!     的深浅 —— 它不看用户在这里选了什么，永远跟系统（见 `window::set_title_bar_dark`）。
//!
//! 两者在系统设置里是可以各选各的，所以它们是两条独立的读法，不是一条。
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
    /// 下一次点击之后该是哪一档：系统 → 深色 → 浅色 → 系统。
    pub fn next(self) -> Self {
        match self {
            Self::System => Self::Dark,
            Self::Dark => Self::Light,
            Self::Light => Self::System,
        }
    }

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

/// Windows 的**应用**模式当前是不是深色。窗口内容跟它。
///
/// 读不到就按深色算：本程序一直以来的样子就是深色，退回浅色反而像是新行为。
pub fn system_is_dark() -> bool {
    theme_value_is_dark("AppsUseLightTheme")
}

/// Windows 的 **Windows 模式**当前是不是深色。标题栏与三个窗口按钮跟它。
pub fn windows_mode_is_dark() -> bool {
    theme_value_is_dark("SystemUsesLightTheme")
}

/// 读 `Personalize` 下某个深浅色开关：1 = 浅色，0 = 深色。
///
/// 读不到按深色算：两个调用方都习惯深色，而读失败的常见原因恰恰是注册表结构与预期不同，
/// 那时猜哪一边都不对，索性保持本程序原本的样子。
fn theme_value_is_dark(name: &str) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

    // 注册表 API 要的是以 0 结尾的宽字符串，且不接收长度参数
    let subkey: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0"
        .encode_utf16()
        .collect();
    let value: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();

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
