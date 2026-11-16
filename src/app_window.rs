//! 按窗口级 AppUserModelID 关闭 Web 应用窗口。
//!
//! 为什么不用进程或命令行匹配：Edge 会把多个 Web 应用窗口共置在同一进程里，
//! 按进程杀会误伤普通浏览器窗口；只有窗口自己的 AUMID 才是稳定身份。
//!
//! 这是整个重写里 COM 最琐碎的一块，做法与 C# 版逐处对齐 —— 包括那条
//! 「GetValue 对存在的属性返回 S_FALSE 而不是 S_OK」的注意事项。

use windows::core::{Interface, GUID, PCWSTR};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{CoTaskMemFree, StructuredStorage::PROPVARIANT};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, PROPERTYKEY};
use windows::Win32::UI::Shell::SHGetPropertyStoreForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, IsWindowVisible, PostMessageW, WM_CLOSE,
};

/// PROPVARIANT 的 vt 取值：VT_LPWSTR(31)，表示联合体里是一个 UTF-16 字符串指针。
const VARIANT_TYPE_UTF16_STRING: u16 = 31;

/// PROPVARIANT 在 x64 上的大小。
const PROPVARIANT_SIZE: usize = 24;

/// 联合体在 PROPVARIANT 里的偏移（x64）。
const PROPVARIANT_UNION_OFFSET: usize = 8;

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
    // SAFETY: window 有效；接口标识由 IPropertyStore 自带
    let store: IPropertyStore = unsafe {
        let mut raw: *mut std::ffi::c_void = std::ptr::null_mut();

        SHGetPropertyStoreForWindow(window, &IPropertyStore::IID, &mut raw)?;

        IPropertyStore::from_raw(raw)
    };

    /*
     * PROPVARIANT 按字节读，不依赖绑定库给这个联合体起的名字。
     * 布局是固定的：vt 在偏移 0，联合体在偏移 8，x64 上整个结构 24 字节。
     */
    let mut raw_value = [0u8; PROPVARIANT_SIZE];

    // SAFETY: GetValue 写一个 PROPVARIANT；raw_value 足够大且按 8 字节对齐
    let read = unsafe {
        store.GetValue(
            &PKEY_APP_USER_MODEL_ID,
            raw_value.as_mut_ptr().cast::<PROPVARIANT>(),
        )
    };

    /*
     * 注意：GetValue 对存在的属性返回 S_FALSE(0x1) 而不是 S_OK，
     * 所以不能靠 HRESULT 判断是否拿到了字符串 —— 只能看 vt。
     * windows-rs 把 S_FALSE 当作成功返回 Ok，正合需要。
     */
    if read.is_err() {
        return Ok(None);
    }

    let variant_type = u16::from_ne_bytes([raw_value[0], raw_value[1]]);

    if variant_type != VARIANT_TYPE_UTF16_STRING {
        return Ok(None);
    }

    let pointer_bytes: [u8; 8] = raw_value[PROPVARIANT_UNION_OFFSET..PROPVARIANT_UNION_OFFSET + 8]
        .try_into()
        .expect("切片长度固定为 8");

    // SAFETY: 低 8 字节原样还原成指针
    let pointer = unsafe { std::ptr::read_unaligned(pointer_bytes.as_ptr().cast::<*mut u16>()) };

    if pointer.is_null() {
        return Ok(None);
    }

    // SAFETY: vt 说明联合体里就是这个字符串；它归调用方所有，读完整份
    let text = unsafe { PCWSTR::from_raw(pointer).to_string() };

    // SAFETY: 这个指针是 shell 用 CoTaskMemAlloc 分配的，读完必须还回去
    unsafe {
        CoTaskMemFree(Some(pointer.cast()));
    }

    Ok(Some(text))
}
