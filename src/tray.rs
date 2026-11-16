//! 通知区域里的那一个托盘图标。
//!
//! 与 C# 版一样，图标用的是 exe 自己的图标资源（`build.rs` 把 `assets/PiWebKeeper.ico`
//! 编进去的那一份），不另外再嵌一份。
//!
//! 与 C# 版的结构差别：这里托盘跑在自己的线程上，并且自己泵消息。
//! 原因是 egui 的窗口一旦收进托盘就不再重绘，而托盘点击必须有人处理 ——
//! 把托盘挂在界面线程上，就等于「窗口藏起来之后点托盘没反应」。
//! 托盘线程只做两件事：把点击翻译成 [`Command`]，把菜单文字与可用性同步成当前状态。

use std::time::Duration;

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
};

use crate::controller::{Command, Handle};

/// 起托盘线程。
pub fn spawn(handle: Handle) {
    std::thread::Builder::new()
        .name("PiWebKeeper.Tray".to_owned())
        .spawn(move || run(handle))
        .expect("托盘线程起不来");
}

/// 托盘线程的主体。
fn run(handle: Handle) {
    let menu = Menu::new();

    let service_item = MenuItem::new("启动", true, None);
    let update_item = MenuItem::new("更新", true, None);
    let restart_item = MenuItem::new("重启程序", true, None);
    let exit_item = MenuItem::new("退出", true, None);

    let separator = PredefinedMenuItem::separator();

    let _ = menu.append_items(&[
        &service_item,
        &update_item,
        &separator,
        &restart_item,
        &exit_item,
    ]);

    let Ok(tray) = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("Pi Web Keeper")
        .with_icon(load_icon())
        .build()
    else {
        // 托盘起不来就没有入口了，但窗口还在，程序仍然可用
        handle
            .lock()
            .push_line("托盘图标登记失败：通知区域没有接受它。".to_owned());

        return;
    };

    // 托盘必须活到线程结束：Drop 会摘掉图标
    let _keep_alive = tray;

    loop {
        pump_messages();

        while let Ok(event) = MenuEvent::receiver().try_recv() {
            let command = if &event.id == service_item.id() {
                Some(Command::ToggleService)
            } else if &event.id == update_item.id() {
                Some(Command::Update)
            } else if &event.id == restart_item.id() {
                Some(Command::RestartProgram)
            } else if &event.id == exit_item.id() {
                Some(Command::Exit)
            } else {
                None
            };

            if let Some(command) = command {
                handle.send(command);
            }
        }

        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            /*
             * 只认按下后的那一下（Up）。Shell 把 Down 和 Up 都发过来，
             * 两个都认等于一次点击做两遍 —— 左键会变成「显示又立刻收起」，
             * 看上去就是点了没反应。
             */
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                handle.send(Command::ShowWindow);
            }
        }

        sync_menu(&handle, &service_item, &update_item, &restart_item, &exit_item);

        std::thread::sleep(Duration::from_millis(20));
    }
}

/// 把菜单的文字与可用性同步成当前状态。
///
/// 判断只做一次、只写一处：窗口上的按钮和菜单里的行读的是同一份 [`Shared`]，
/// 所以不会出现「按钮是停止、菜单是启动」这种自相矛盾的样子。
fn sync_menu(
    handle: &Handle,
    service_item: &MenuItem,
    update_item: &MenuItem,
    restart_item: &MenuItem,
    exit_item: &MenuItem,
) {
    let shared = handle.lock();

    let service_text = if shared.busy {
        if shared.updating {
            "正在更新"
        } else {
            "正在启动"
        }
    } else if shared.config_error.is_some() {
        "启动"
    } else if shared.listening {
        "停止"
    } else {
        "启动"
    };

    service_item.set_text(service_text);
    service_item.set_enabled(!shared.busy && shared.config_error.is_none());

    update_item.set_text(if shared.updating { "正在更新" } else { "更新" });
    update_item.set_enabled(!shared.busy && !shared.updating && !shared.listening);

    // 更新期间一律置灰：更新没有中途取消这条路，半途走掉会留下一个替换了一半的包
    let can_leave = !shared.updating && !shared.exiting;

    restart_item.set_enabled(can_leave);
    exit_item.set_enabled(can_leave);
}

/// 取一帧图标给通知区域。
///
/// 优先用 exe 自己的图标资源；取不到就退回一个纯色方块 ——
/// 窗口收起来之后托盘是唯一入口，难看一点的图标总好过一个进不去的窗口。
fn load_icon() -> Icon {
    // 资源号 1：build.rs 用 winresource 把 .ico 编在 1 号上
    if let Ok(icon) = Icon::from_resource(1, Some((32, 32))) {
        return icon;
    }

    fallback_icon()
}

/// 32×32 的纯色图标，只在图标资源取不到时用。
fn fallback_icon() -> Icon {
    const SIZE: u32 = 32;

    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);

    for y in 0..SIZE {
        for x in 0..SIZE {
            // 中间留一个浅色的圆，至少能看出是个图标而不是一块污渍
            let dx = x as f32 - 15.5;
            let dy = y as f32 - 15.5;
            let inside = dx * dx + dy * dy < 100.0;

            let (r, g, b) = if inside { (240, 240, 240) } else { (60, 60, 60) };

            rgba.extend_from_slice(&[r, g, b, 255]);
        }
    }

    Icon::from_rgba(rgba, SIZE, SIZE).expect("32×32 的 RGBA 一定合法")
}

/// 把本线程消息队列里的消息派发掉。
///
/// 用 `PeekMessage` 而不是 `GetMessage`：这个线程还要轮询通道，
/// 不能阻塞在消息队列上。
fn pump_messages() {
    let mut message = MSG::default();

    // SAFETY: message 是本函数的局部变量，长度由类型保证
    while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
        // SAFETY: message 由 PeekMessageW 填好
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}
