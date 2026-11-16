//! 窗口的显示、隐藏与关闭，走 Win32 而不是 egui。
//!
//! 为什么不能交给 egui：窗口收进托盘之后就不再重绘，而 `eframe::App::update` 只在重绘时被调用。
//! 于是「托盘左键把窗口叫出来」这条路径会死在里面 —— 标志置上了，但没有人来读它。
//! 实测确认过：服务启动成功、窗口藏起来之后，第二个实例的唤醒信号确实到达，
//! 窗口却再也不出来。
//!
//! 所以这三个动作直接调 `ShowWindow` / `PostMessage`，让窗口状态不依赖界面帧。
//! 句柄与 `egui::Context` 在窗口创建时记下来，托盘线程和单实例监听线程都能用。

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Mutex, OnceLock};

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, IsIconic, IsWindowVisible, SetForegroundWindow, SetWindowPos, ShowWindow, SW_HIDE,
    SWP_NOACTIVATE, SWP_NOZORDER, SWP_NOSIZE, SW_RESTORE, SW_SHOW,
};

/// 主窗口句柄。窗口创建时写入一次。
static WINDOW: AtomicIsize = AtomicIsize::new(0);

/// 窗口在屏幕上的位置与大小，`park` 之前记下来。
static RESTORE_POSITION: Mutex<Option<(i32, i32, i32, i32)>> = Mutex::new(None);

/// 窗口现在是不是被挪到了屏幕外。
static PARKED: AtomicBool = AtomicBool::new(false);

/// 放到屏幕外的坐标。当于「不在任何显示器上」。
const PARKED_POSITION: i32 = -32_000;

/// egui 的上下文，用来在显示之后请求一次重绘。
static CONTEXT: OnceLock<egui::Context> = OnceLock::new();

/// 记住主窗口与它的 egui 上下文。
pub fn remember(window: HWND, context: egui::Context) {
    WINDOW.store(window.0 as isize, Ordering::SeqCst);
    let _ = CONTEXT.set(context);
}

/// 把窗口挪到屏幕外，并记住原来的位置。
///
/// 为什么需要这个：eframe 在第一帧画完之后会无条件显示窗口（见 [`remember`]），
/// 而启动成功时这个窗口自始至终不该露面。单靠「下一帧再藏回去」仍会剩两帧左右的可见内烁，
/// 先把窗口放屏幕外，那两帧就落在没人看得到的地方。
pub fn park() {
    let Some(window) = handle() else {
        return;
    };

    // SAFETY: 句柄在窗口存活期间有效
    unsafe {
        let mut rect = RECT::default();

        if GetWindowRect(window, &mut rect).is_ok() {
            if let Ok(mut saved) = RESTORE_POSITION.lock() {
                *saved = Some((
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                ));
            }
        }

        let _ = SetWindowPos(
            window,
            None,
            PARKED_POSITION,
            PARKED_POSITION,
            0,
            0,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOSIZE,
        );
    }

    PARKED.store(true, Ordering::SeqCst);
}

/// 窗口是否还在屏幕外。
pub fn is_parked() -> bool {
    PARKED.load(Ordering::SeqCst)
}

/// 把窗口从托盘里拿出来、叫到前面。
///
/// 最小化时要先还原：只调 `SetForegroundWindow` 对最小化的窗口没有效果，
/// 用户看到的就是「程序没理他」。
pub fn show() {
    let Some(window) = handle() else {
        return;
    };

    let saved = RESTORE_POSITION
        .lock()
        .ok()
        .and_then(|saved| *saved);

    // SAFETY: 句柄在窗口存活期间有效；下面的调用对失效句柄只会失败
    unsafe {
        // 先从屏幕外挪回来，再显示 —— 反过来会先看到它出现在屏幕外那个位置再跳过来
        if let Some((x, y, width, height)) = saved {
            let _ = SetWindowPos(
                window,
                None,
                x,
                y,
                width,
                height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }

        if IsIconic(window).as_bool() {
            let _ = ShowWindow(window, SW_RESTORE);
        } else {
            let _ = ShowWindow(window, SW_SHOW);
        }

        let _ = SetForegroundWindow(window);
    }

    PARKED.store(false, Ordering::SeqCst);

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
