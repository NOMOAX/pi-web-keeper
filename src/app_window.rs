//! 按窗口级 AppUserModelID 关闭 Web 应用窗口。
//!
//! 为什么不用进程或命令行匹配：Edge 会把多个 Web 应用窗口共置在同一进程里，
//! 按进程杀会误伤普通浏览器窗口；只有窗口自己的 AUMID 才是稳定身份。
//!
//! COM 里最琐碎的一块，注意 `GetValue` 对**存在**的属性返回 `S_FALSE` 而不是 `S_OK`。

use windows::core::{GUID, PROPVARIANT, PWSTR};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, WPARAM};
use windows::Win32::UI::Shell::PropertiesSystem::{
    IPropertyStore, PROPERTYKEY, SHGetPropertyStoreForWindow,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, IsWindowVisible, PostMessageW, WM_CLOSE,
};

/// PROPVARIANT 的 vt 取值：VT_LPWSTR(31)，表示联合体里是一个 UTF-16 字符串指针。
///
/// 绑定库里没有导出这个常量，按头文件的值写死。
const VARIANT_TYPE_UTF16_STRING: u16 = 31;

/// Chromium 的顶层窗口类。
///
/// 限定它，否则会误伤普通 Edge 窗口 —— 那些窗口也有 AUMID（就是 Edge 自己的）。
const CHROMIUM_WINDOW_CLASS: &str = "Chrome_WidgetWin_1";

/// PKEY_AppUserModel_ID：{9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3}, 5。
const PKEY_APP_USER_MODEL_ID: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9F4C_2855_9F79_4B39_A8D0_E1D4_2DE1_D5F3),
    pid:   5,
};

/// 关闭属于指定 AUMID 的所有可见窗口，返回关闭的窗口数。
pub fn close_windows(app_user_model_id: &str) -> usize {
    if app_user_model_id.trim().is_empty() {
        return 0;
    }

    let mut targets: Vec<HWND> = Vec::new();
    let mut search = WindowSearch {
        wanted: app_user_model_id,
        found:  &mut targets,
    };

    // SAFETY: 回调只在这个调用期间被调用；search 活得比它长
    let _ = unsafe {
        EnumWindows(
            Some(collect_window),
            LPARAM((&mut search as *mut WindowSearch) as isize),
        )
    };

    for window in &targets {
        // SAFETY: 窗口句柄由 EnumWindows 给出，即使已经失效也只是投递失败
        unsafe {
            let _ = PostMessageW(*window, WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }

    targets.len()
}

/// EnumWindows 回调之间传递的查找条件。
struct WindowSearch<'a> {
    wanted: &'a str,
    found:  &'a mut Vec<HWND>,
}

/// EnumWindows 的回调：挑出类名与 AUMID 都对得上的可见窗口。
///
/// 返回 `true` 表示继续枚举 —— 这里永远继续，失效的窗口跳过就好。
unsafe extern "system" fn collect_window(window: HWND, l_param: LPARAM) -> BOOL {
    // SAFETY: l_param 是调用方传进来的 &mut WindowSearch
    let search = unsafe { &mut *(l_param.0 as *mut WindowSearch) };

    // SAFETY: window 来自系统枚举
    if !unsafe { IsWindowVisible(window) }.as_bool() {
        return BOOL(1);
    }

    let mut class_name = [0u16; 64];

    // SAFETY: 缓冲区长度与传入的长度一致
    let copied = unsafe { GetClassNameW(window, &mut class_name) };

    if copied <= 0 {
        return BOOL(1);
    }

    let class_name = String::from_utf16_lossy(&class_name[..copied as usize]);

    // 限定 Chromium 窗口类，否则会误伤普通 Edge 窗口
    if class_name != CHROMIUM_WINDOW_CLASS {
        return BOOL(1);
    }

    // SAFETY: window 来自系统枚举
    let Ok(actual) = (unsafe { window_app_user_model_id(window) }) else {
        return BOOL(1);
    };

    if actual.as_deref() == Some(search.wanted) {
        search.found.push(window);
    }

    BOOL(1)
}

/// 读取窗口的 AUMID；窗口没有设置 AUMID 时返回 `None`。
unsafe fn window_app_user_model_id(window: HWND) -> windows::core::Result<Option<String>> {
    // SAFETY: window 有效；要哪个接口由泛型参数指定
    let store: IPropertyStore = unsafe { SHGetPropertyStoreForWindow(window) }?;

    /*
     * 注意：GetValue 对存在的属性返回 S_FALSE(0x1) 而不是 S_OK。
     * 绑定库把 S_FALSE 当成功处理，所以能走到这里 ——
     * 但也不能因此认为「拿到了字符串」，下面的 vt 判断才是真正的依据。
     */
    // SAFETY: store 有效，key 指向一个静态常量
    let value: PROPVARIANT = unsafe { store.GetValue(&PKEY_APP_USER_MODEL_ID) }?;

    let raw = value.as_raw();

    // SAFETY: 读联合体里那个已经写好的 vt 字段
    let variant_type: u16 = unsafe { raw.Anonymous.Anonymous.vt };

    if variant_type != VARIANT_TYPE_UTF16_STRING {
        return Ok(None);
    }

    // SAFETY: vt 说明联合体里就是一个 UTF-16 字符串指针
    let pointer = unsafe { raw.Anonymous.Anonymous.Anonymous.pwszVal };

    if pointer.is_null() {
        return Ok(None);
    }

    /*
     * 字符串不在这里释放：PROPVARIANT 的 Drop 会调 PropVariantClear，
     * 而它归我们所有。
     */
    let text = unsafe { PWSTR::from_raw(pointer).to_string() }?;

    Ok(Some(text))
}
