//! 界面：一行四个按钮，下面是状态与只读日志。
//!
//! 界面是纯视图：所有阻塞动作都在控制器线程上（见 `controller.rs`），
//! 这里只读 [`Shared`]、只发 [`Command`]。所以点一下按钮不会让窗口定格。
//!
//! 与 XAML 版的对照：
//!   - 640×480、不可最大化、`CenterScreen` → `NativeOptions` / `ViewportBuilder`
//!   - 一行四个 80×40、间距 8 → `add_sized` + `item_spacing`
//!   - 只读多行 `TextBox` + 右键菜单（复制／清空）→ `TextEdit` + `context_menu`
//!   - 点空白处清焦点 → egui 本来就不给面板焦点，这条不用写

use eframe::egui;

use crate::controller::{Command, Handle};
use crate::font;

/// 按钮尺寸与间距。XAML 版这两个数是用户按截图挑的，原样搬过来。
const BUTTON_SIZE: [f32; 2] = [80.0, 40.0];
const BUTTON_SPACING: f32 = 8.0;

/// 跑界面主循环。`start_on_launch` 为假表示这次是「以普通方式启动」。
pub fn run(handle: Handle, start_on_launch: bool) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            /*
             * XAML 里那个 640x480 是**含边框的外框**尺寸（`Window.Width/Height`），
             * 而这里的 `inner_size` 是客户区。实测本机非客户区占 16x39，减掉才能得到
             * 一样大的窗口。逻辑点与物理像素都按 DPI 缩放，所以这个差在两边的比例下都成立。
             */
            .with_inner_size([624.0, 441.0])
            .with_min_inner_size([624.0, 441.0])
            // 只去掉最大化按钮，窗口仍可缩放 —— 与 XAML 版清 WS_MAXIMIZEBOX 是同一件事
            .with_maximize_button(false)
            .with_title("Pi Web Keeper")
            // 启动流程跑完之前不露面：成功的话这个窗口自始至终没见过，
            // 启动时窗口本来就没显示过，失败时它自己会出来
            .with_visible(false),
        centered: true,
        ..Default::default()
    };

    eframe::run_native(
        "Pi Web Keeper",
        options,
        Box::new(move |creation| {
            let context = &creation.egui_ctx;

            context.set_visuals(egui::Visuals::dark());

            let font_note = font::install(context);

            // 窗口已经建好但还没画过：改样式要趁现在，等到第一帧再改，
            // 系统已经把最大化按钮画上去了，清掉样式位也只会把它画成灰的
            prepare_window(&creation, context.clone());

            Ok(Box::new(KeeperApp::new(handle, start_on_launch, font_note)))
        }),
    )
}

/// 界面状态。
struct KeeperApp {
    handle: Handle,

    /// 日志的影子缓冲：`TextEdit` 要 `&mut String`。
    ///
    /// 用户在里面敲字会被下一帧的同步覆盖掉 —— 这是「只读文本框」在 egui 里的常见做法，
    /// egui 没有 `IsReadOnly` 这个开关。
    log_view: String,

    /// 上次同步时的日志版本号。
    log_revision: u64,

    /// 还没执行过的「启动时自动启动服务」。
    start_on_launch: bool,

    /// 字体实际用上了哪个，写进日志的第一行。
    font_note: String,

    /// 字体说明是否已写进日志。
    font_noted: bool,
}

impl KeeperApp {
    fn new(handle: Handle, start_on_launch: bool, font_note: String) -> Self {
        Self {
            handle,
            log_view: String::new(),
            log_revision: 0,
            start_on_launch,
            font_note,
            font_noted: false,
        }
    }

    /// 把共享状态里的日志同步到影子缓冲。
    fn sync_log(&mut self, lines: &[String], revision: u64) {
        if revision == self.log_revision {
            return;
        }

        self.log_revision = revision;
        self.log_view.clear();

        for line in lines {
            self.log_view.push_str(line);
            self.log_view.push('\n');
        }
    }
}

impl eframe::App for KeeperApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        let (quit, visible, revision, state, listening, busy, updating, exiting, has_config, lines) = {
            let shared = self.handle.lock();

            (
                shared.quit,
                shared.visible,
                shared.log_revision,
                shared.state.clone(),
                shared.listening,
                shared.busy,
                shared.updating,
                shared.exiting,
                shared.config_error.is_none(),
                shared.log.clone(),
            )
        };

        if !self.font_noted {
            self.font_noted = true;
            self.handle.lock().push_line(format!("字体：{}", self.font_note));
        }

        self.sync_log(&lines, revision);

        if quit {
            context.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        /*
         * 每帧把实际可见状态拉回「该不该」。
         *
         * eframe 在第一帧画完之后会无条件把窗口显示出来（`window::remember` 那边有说明），
         * 而启动成功的那一下要把窗口收起来——如果不在这里拉回来，用户就会看到
         * 「先闪出来一下，然后隐藏」。
         *
         * `request_repaint` 不能省：藏完之后窗口不再重绘，不主动要一帧的话，
         * 再下帧要等 100 毫秒，那 100 毫秒就是一个真的看得见的窗口。
         */
        if visible {
            // 在屏幕外就把它展回来（否则它一直隐在 -32000 那个位置）
            if crate::window::is_parked() {
                crate::window::show();
            }
        } else if crate::window::is_visible() {
            crate::window::hide();
        }

        if !visible {
            context.request_repaint();
        }

        /*
         * 启动时直接走一遍「启动」，中间不显示窗口。
         * 这一条命令与按钮发的是同一个，只是不等结果。
         */
        if self.start_on_launch {
            self.start_on_launch = false;
            self.handle.send(Command::StartService);
        }

        /*
         * 关闭按钮不关掉程序，只是把窗口收进托盘 —— 服务继续跑。
         * 真正退出走「退出」，它先把服务停掉再关窗口。
         */
        if context.input(|input| input.viewport().close_requested()) && !exiting {
            context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.handle.lock().visible = false;
            crate::window::hide();
        }

        // 最小化也等于收进托盘：拦的是状态变化，所以按钮、系统菜单、自动收起都覆盖到了
        if context.input(|input| input.viewport().minimized).unwrap_or(false) {
            self.handle.lock().visible = false;
            crate::window::hide();
        }

        let service_label = if busy {
            if updating {
                "正在更新"
            } else {
                "正在启动"
            }
        } else if !has_config {
            "启动"
        } else if listening {
            "停止"
        } else {
            "启动"
        };

        let can_update = !busy && !updating && !listening && has_config;

        // 更新期间一律置灰：更新没有中途取消这条路
        let can_leave = !updating && !exiting;

        let mut pressed: Option<Command> = None;

        egui::CentralPanel::default().show(context, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(BUTTON_SPACING, BUTTON_SPACING);

            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !busy && has_config,
                        egui::Button::new(service_label)
                            .min_size(egui::vec2(BUTTON_SIZE[0], BUTTON_SIZE[1])),
                    )
                    .clicked()
                {
                    pressed = Some(Command::ToggleService);
                }

                if ui
                    .add_enabled(
                        can_update,
                        egui::Button::new("更新")
                            .min_size(egui::vec2(BUTTON_SIZE[0], BUTTON_SIZE[1])),
                    )
                    .on_hover_text(
                        "服务运行中不执行更新：更新不会停掉正在运行的服务，请先点「停止」。",
                    )
                    .clicked()
                {
                    pressed = Some(Command::Update);
                }

                if ui
                    .add_enabled(
                        can_leave,
                        egui::Button::new("重启程序")
                            .min_size(egui::vec2(BUTTON_SIZE[0], BUTTON_SIZE[1])),
                    )
                    .clicked()
                {
                    pressed = Some(Command::RestartProgram);
                }

                if ui
                    .add_enabled(
                        can_leave,
                        egui::Button::new("退出")
                            .min_size(egui::vec2(BUTTON_SIZE[0], BUTTON_SIZE[1])),
                    )
                    .clicked()
                {
                    pressed = Some(Command::Exit);
                }
            });

            ui.label(&state);

            // 留给消息框的高度：面板里剩下的全部（在外面量，见下面的注释）
            let log_height = ui.available_height();

            let clear = egui::ScrollArea::vertical()
                .id_salt("message-box")
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    /*
                     * 消息框要填满面板剩下的那一块，而不是固定多少行然后下面留一片空白。
                     * 行数按进 ScrollArea 之前的高度算 —— 进去之后 available_height
                     * 就变成内容高度了，量到的是它自己。
                     * 内容超过时由 ScrollArea 接管，新的一行进来会粘在底部。
                     */
                    let row_height = ui.text_style_height(&egui::TextStyle::Monospace).max(1.0);
                    let rows = ((log_height / row_height).floor() as usize).max(1);

                    let response = ui.add(
                        egui::TextEdit::multiline(&mut self.log_view)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY)
                            .desired_rows(rows),
                    );

                    let mut cleared = false;

                    response.context_menu(|ui| {
                        if ui.button("复制").clicked() {
                            ui.ctx().copy_text(self.log_view.clone());
                            ui.close_menu();
                        }

                        if ui.button("清空").clicked() {
                            cleared = true;
                            ui.close_menu();
                        }
                    });

                    cleared
                })
                .inner;

            if clear {
                self.handle.lock().clear_log();
                self.log_view = CLEARED_PLACEHOLDER.to_owned();
            }
        });

        if let Some(command) = pressed {
            self.handle.send(command);
        }

        // 控制器每 300 毫秒刷新一次状态；这里按帧轮询，比它稍快一点就够了
        context.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

/// 清空消息框后留在框里的那句话。
///
/// 不留下空白：空白分不清「程序还没写过日志」与「日志刚被清掉」，而留一句话就能分清。
const CLEARED_PLACEHOLDER: &str = "----- Message cleared. -----";

/// 取掉窗口的最大化按钮。
///
/// eframe 的 `with_maximize_button(false)` 一路传到了 winit，但实测这一帧里它还是画出来了，
/// 所以照 XAML 版的做法直接改窗口样式位：清掉 `WS_MAXIMIZEBOX`，再让系统重算非客户区。
/// 缩放与最小化照旧；双击标题栏最大化也会一并失效，它走的也是这个位。
/// 窗口建好之后的收尾：取掉最大化按钮，并把句柄与上下文记下来。
///
/// 记句柄是为了让托盘线程和单实例监听线程能直接显示／隐藏窗口 ——
/// 窗口藏起来之后界面不再重绘，任何靠界面帧去做的动作都会停在那里（见 `window.rs`）。
fn prepare_window(source: &impl raw_window_handle::HasWindowHandle, context: egui::Context) {
    use raw_window_handle::RawWindowHandle;

    let Ok(handle) = source.window_handle() else {
        return;
    };

    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };

    let window = windows::Win32::Foundation::HWND(win32.hwnd.get() as *mut core::ffi::c_void);

    crate::window::remember(window, context);

    /*
     * 先把窗口挪到屏幕外：eframe 会在第一帧画完之后无条件把它显示出来，
     * 而启动成功时它自始至终不该露面。藏在屏幕外，那一下就没任何人看到。
     */
    crate::window::park();

    // SAFETY: 句柄来自 winit，在窗口存活期间有效
    unsafe {
        let style = windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(
            window,
            windows::Win32::UI::WindowsAndMessaging::GWL_STYLE,
        );

        windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(
            window,
            windows::Win32::UI::WindowsAndMessaging::GWL_STYLE,
            style & !(windows::Win32::UI::WindowsAndMessaging::WS_MAXIMIZEBOX.0 as isize),
        );

        // 改完样式要告诉系统重算非客户区，否则标题栏上的按钮不会被重画
        let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowPos(
            window,
            None,
            0,
            0,
            0,
            0,
            windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                | windows::Win32::UI::WindowsAndMessaging::SWP_NOSIZE
                | windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER
                | windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE
                | windows::Win32::UI::WindowsAndMessaging::SWP_FRAMECHANGED,
        );
    }
}
