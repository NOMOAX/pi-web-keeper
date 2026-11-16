//! 窗口的显示、隐藏与关闭，走 Win32 而不是 egui。
//!
//! 为什么不能交给 egui：窗口收进托盘之后就不再重绘，而 `eframe::App::update` 只在重绘时被调用。
//! 于是「托盘左键把窗口叫出来」这条路径会死在里面 —— 标志置上了，但没有人来读它。
//! 实测确认过：服务启动成功、窗口藏起来之后，第二个实例的唤醒信号确实到达，
//! 窗口却再也不出来。
//!
//! 所以这三个动作直接调 `ShowWindow` / `PostMessage`，让窗口状态不依赖界面帧。
//! 句柄与 `egui::Context` 在窗口创建时记下来，托盘线程和单实例监听线程都能用。

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::OnceLock;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow, SW_HIDE, SW_RESTORE, SW_SHOW,
};

/// 主窗口句柄。窗口创建时写入一次。
static WINDOW: AtomicIsize = AtomicIsize::new(0);

/// egui 的上下文，用来在显示之后请求一次重绘。
static CONTEXT: OnceLock<egui::Context> = OnceLock::new();

/// 记住主窗口与它的 egui 上下文。
pub fn remember(window: HWND, context: egui::Context) {
    WINDOW.store(window.0 as isize, Ordering::SeqCst);
    let _ = CONTEXT.set(context);
}

/// 把窗口从托盘里拿出来、叫到前面。
///
/// 最小化时要先还原：只调 `SetForegroundWindow` 对最小化的窗口没有效果，
/// 用户看到的就是「程序没理他」。
pub fn show() {
    let Some(window) = handle() else {
        return;
    };

    // SAFETY: 句柄在窗口存活期间有效；下面的调用对失效句柄只会失败
    unsafe {
        if IsIconic(window).as_bool() {
            let _ = ShowWindow(window, SW_RESTORE);
        } else {
            let _ = ShowWindow(window, SW_SHOW);
        }

        let _ = SetForegroundWindow(window);
    }

    // 窗口重新可见之后要有人画第一帧，否则它是一块没有内容的矩形
    if let Some(context) = CONTEXT.get() {
        context.request_repaint();
    }
}

/// 收进托盘。服务继续跑。
pub fn hide() {
    let Some(window) = handle() else {
        return;
    };

    // SAFETY: 同上
    unsafe {
        let _ = ShowWindow(window, SW_HIDE);
    }
}

/// 窗口现在是否显示在屏幕上。
///
/// 托盘左键是个开关，方向靠它决定。
pub fn is_visible() -> bool {
    let Some(window) = handle() else {
        return false;
    };

    // SAFETY: 句柄在窗口存活期间有效
    unsafe { IsWindowVisible(window).as_bool() }
}

fn handle() -> Option<HWND> {
    let raw = WINDOW.load(Ordering::SeqCst);

    if raw == 0 {
        None
    } else {
        Some(HWND(raw as *mut core::ffi::c_void))
    }
}
