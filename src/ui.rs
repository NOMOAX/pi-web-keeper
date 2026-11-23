//! 界面：一行四个按钮，下面是状态与只读日志。
//!
//! 界面是纯视图：所有阻塞动作都在控制器线程上（见 `controller.rs`），
//! 这里只读 [`Shared`]、只发 [`Command`]。所以点一下按钮不会让窗口定格。
//!
//! 与 XAML 版的对照：
//!   - 640×480、不可最大化、`CenterScreen` → `NativeOptions` / `ViewportBuilder`
//!   - 一行四个 80×40、间距 8 → `add_sized` + `item_spacing`
//!   - 只读多行 `TextBox` + 右键菜单（清空）→ `TextEdit` + `context_menu`
//!   - 点空白处清焦点 → egui 本来就不给面板焦点，这条不用写

use eframe::egui;

use crate::controller::{Command, Handle};
use crate::font;

/// 按钮尺寸与间距。XAML 版这两个数是用户按截图挑的，原样搬过来。
const BUTTON_SIZE: [f32; 2] = [80.0, 40.0];
const BUTTON_SPACING: f32 = 8.0;

/// 消息框里文字与框边的距离。横向用它，纵向只用一半 —— 文字行本身自带上方留白。
const BOX_PADDING: f32 = 8.0;

/// 主题按钮的尺寸。比四个大按钮窄、也矮一些 —— 它们是设置，不是主操作。
const THEME_BUTTON_SIZE: [f32; 2] = [44.0, 28.0];

/// 「跟随系统」时多久重读一次系统主题。
///
/// Windows 换主题不会通知应用，只能自己看。一秒一次足够：人手点完系统设置，
/// 再切回这个窗口，看到的已经是新颜色。
const SYSTEM_THEME_POLL: std::time::Duration = std::time::Duration::from_secs(1);

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
            .with_icon(std::sync::Arc::new(window_icon()))
            .with_title("Pi Web Keeper")
            /*
             * 创建时不显示，但这不是「启动期间不露面」——
             * eframe 画完第一帧就会把它显示出来，不管这里传什么。
             * 这一句只是压掉首帧之前那块白底。
             */
            .with_visible(false),
        centered: true,
        ..Default::default()
    };

    eframe::run_native(
        "Pi Web Keeper",
        options,
        Box::new(move |creation| {
            let context = &creation.egui_ctx;

            // 每次启动都回到「跟随系统」：这个选择只存在内存里（见 `theme.rs`）
            let initial_dark = crate::theme::apply(context, crate::theme::Preference::default());

            font::install(context);

            // 窗口已经建好但还没画过：改样式要趁现在，等到第一帧再改，
            // 系统已经把最大化按钮画上去了，清掉样式位也只会把它画成灰的
            prepare_window(&creation, context.clone());

            Ok(Box::new(KeeperApp::new(handle, start_on_launch, initial_dark)))
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

    /// 用户选的界面主题。
    theme: crate::theme::Preference,

    /// 窗口内容当前实际用的是不是深色。用来判断「要不要重装主题」。
    is_dark: bool,

    /// 「系统」那一档上次读到的应用模式。到点才刷新。
    system_is_dark: bool,

    /// 标题栏当前实际用的是不是深色。它跟 Windows 模式，与 `theme` 无关。
    title_bar_dark: bool,

    /// 上次向系统要主题的时间。`None` 表示下一帧就去看。
    system_theme_checked_at: Option<std::time::Instant>,
}

impl KeeperApp {
    fn new(handle: Handle, start_on_launch: bool, is_dark: bool) -> Self {
        let title_bar_dark = crate::theme::windows_mode_is_dark();

        // 标题栏是 DWM 画的，egui 不会替我们设，创建时就得对上
        crate::window::set_title_bar_dark(title_bar_dark);

        Self {
            handle,
            log_view: String::new(),
            log_revision: 0,
            start_on_launch,
            theme: crate::theme::Preference::default(),
            is_dark,
            system_is_dark: is_dark,
            title_bar_dark,
            system_theme_checked_at: None,
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
        let (quit, revision, listening, busy, updating, exiting, has_config, lines) = {
            let shared = self.handle.lock();

            (
                shared.quit,
                shared.log_revision,
                shared.listening,
                shared.busy,
                shared.updating,
                shared.exiting,
                shared.config_error.is_none(),
                shared.log.clone(),
            )
        };

        self.sync_log(&lines, revision);

        /*
         * 主题分两件事，彼此独立：
         *
         *   - 窗口内容：跟用户选的那一档；选「系统」时读 **应用模式**。
         *   - 标题栏与最小化/最大化/关闭按钮：永远跟 **Windows 模式**，
         *     与用户在这里选了什么无关（见 `theme.rs` 开头）。
         *
         * Windows 换主题不会通知应用，所以隔一秒去看一次 —— 人手点完系统设置、
         * 再切回这个窗口，看到的已经是新颜色。
         */
        let due = self
            .system_theme_checked_at
            .map_or(true, |checked| checked.elapsed() >= SYSTEM_THEME_POLL);

        if due {
            self.system_theme_checked_at = Some(std::time::Instant::now());

            let title_bar_dark = crate::theme::windows_mode_is_dark();

            if title_bar_dark != self.title_bar_dark {
                self.title_bar_dark = title_bar_dark;
                crate::window::set_title_bar_dark(title_bar_dark);

                context.request_repaint();
            }

            // 只有「系统」这一档需要重读应用模式；另两档是固定值
            if self.theme == crate::theme::Preference::System {
                self.system_is_dark = crate::theme::system_is_dark();
            }
        }

        let desired_dark = match self.theme {
            crate::theme::Preference::Dark => true,
            crate::theme::Preference::Light => false,
            crate::theme::Preference::System => self.system_is_dark,
        };

        if desired_dark != self.is_dark {
            self.is_dark = desired_dark;
            crate::theme::apply(context, self.theme);

            // 换主题要立刻看到，别等下一次 100 毫秒的刷新
            context.request_repaint();
        }

        if quit {
            context.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        /*
         * 启动时直接走一遍「启动」。
         *
         * 窗口在这段时间里是可见的（eframe 在第一帧之后把它显示出来，正好当进度看），
         * 启动成功之后由控制器收起来 —— 收的时机就是「要打开应用窗口了」。
         * 这里不做任何可见性判断，也不需要：每帧拉状态那种写法是上一次的遗留，
         * 那是在“窗口自始至终不露面”的前提下才需要的。
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
            crate::window::hide();
        }

        // 最小化也等于收进托盘：拦的是状态变化，所以按钮、系统菜单、自动收起都覆盖到了
        if context.input(|input| input.viewport().minimized).unwrap_or(false) {
            crate::window::hide();
        }

        /*
         * 更新期间文字不动，只有「启动」与「停止」两种：一次更新要跑好几分钟，
         * 把这一格写成「正在更新」既没说明服务现在是什么状态，又让按钮看起来在跳。
         * 置灰已经说明了「现在点不了」。
         */
        let service_label = if busy && !updating {
            "正在启动"
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
        let mut theme_changed = false;

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

                /*
                 * 占位：把主题按钮推到右端。宽度是「剩下的全部减去它要占的地方」，
                 * 窗口拉伸时跟着变，不会把它挤出去。
                 */
                ui.allocate_space(egui::vec2(
                    (ui.available_width() - THEME_BUTTON_SIZE[0] - BUTTON_SPACING).max(0.0),
                    0.0,
                ));

                /*
                 * 一个按钮，点一下换一档：系统 → 深色 → 浅色 → 系统。
                 * 按钮上的字就是当前这一档，所以它同时说明了「现在是什么」。
                 */
                if ui
                    .add_sized(
                        THEME_BUTTON_SIZE,
                        egui::Button::new(self.theme.label()),
                    )
                    .on_hover_text("主题：系统 / 深色 / 浅色，点一下换一档")
                    .clicked()
                {
                    self.theme = self.theme.next();

                    // 下一帧重新判断深浅；换档之后立刻去看系统设置
                    self.system_theme_checked_at = None;
                    theme_changed = true;
                }
            });

            /*
             * 消息框的框由这里自己画，不用 `TextEdit` 自带的那个。
             *
             * 原因：`TextEdit` 的高度只能是整数行（`desired_rows`），而面板剩下的高度几乎
             * 不会是行高的整数倍，差额就落成了多出来的下边距 —— 实测底部 24 像素、左右各
             * 16 像素。自己画框，框正好等于剩下的那一块，四边就都是 8。
             *
             * 里面的 `TextEdit` 因此关掉自带的框，只负责文字与滚动。
             */
            let box_size = ui.available_size();
            let (box_rect, _) = ui.allocate_exact_size(box_size, egui::Sense::hover());

            let inactive = ui.visuals().widgets.inactive.clone();

            ui.painter().rect(
                box_rect,
                inactive.rounding,
                ui.visuals().extreme_bg_color,
                inactive.bg_stroke,
            );

            let text_rect = box_rect.shrink2(egui::vec2(BOX_PADDING, BOX_PADDING / 2.0));

            let clear = ui
                .allocate_new_ui(egui::UiBuilder::new().max_rect(text_rect), |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("message-box")
                        .stick_to_bottom(true)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            /*
                             * 行数按这块的高度算：少了框里会留白，多了 `ScrollArea` 接管。
                             * 内容超过时也由它接管，新的一行进来会粘在底部。
                             */
                            let row_height =
                                ui.text_style_height(&egui::TextStyle::Monospace).max(1.0);
                            let rows =
                                ((ui.available_height() / row_height).floor() as usize).max(1);

                            let response = ui
                                .scope(|ui| {
                                    /*
                                     * 不要那根闪烁的输入光标。
                                     *
                                     * 这个框是可选的、不可改的，`interactive(false)` 会把选择也一起
                                     * 关掉（见 AGENTS.md），所以留着 `TextEdit`、只把光标画成零宽：
                                     * 光标是一条 0 宽线段，等于不画。`blink` 一并关掉 ——
                                     * 它每次闪烁都会请求重绘，而这个框没有东西需要刷新。
                                     */
                                    let cursor = &mut ui.visuals_mut().text_cursor;
                                    cursor.stroke = egui::Stroke::NONE;
                                    cursor.blink = false;

                                    ui.add(
                                        egui::TextEdit::multiline(&mut self.log_view)
                                            .frame(false)
                                            .font(egui::TextStyle::Monospace)
                                            .desired_width(f32::INFINITY)
                                            .desired_rows(rows),
                                    )
                                })
                                .inner;

                            let mut cleared = false;

                            response.context_menu(|ui| {
                                /*
                                 * 菜单项默认只留 4×1 的内边距，点上去就像在点文字本身。
                                 * 这里放大到 14×7：菜单是鼠标操作，目标得有面积。
                                 */
                                ui.spacing_mut().button_padding = egui::vec2(14.0, 7.0);

                                if ui.button("清空").clicked() {
                                    cleared = true;
                                    ui.close_menu();
                                }
                            });

                            cleared
                        })
                        .inner
                })
                .inner;

            if clear {
                // 只清共享日志；影子缓冲下一帧跟着 revision 一起重建，
                // 在这里顺手改它会被 sync_log 覆盖掉。
                self.handle.lock().clear_log();
            }
        });

        if let Some(command) = pressed {
            self.handle.send(command);
        }

        // 换了主题就别等下一次刷新，这一帧之后立刻重画
        if theme_changed {
            context.request_repaint();
        }

        // 控制器每 300 毫秒刷新一次状态；这里按帧轮询，比它稍快一点就够了
        context.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

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

/// 窗口图标。
///
/// 只能走这条路上，不能自己 `WM_SETICON`：eframe 在**第一帧之后**按 viewport 的设置
/// 再设一次图标，viewport 里没有图标就等于把图标清掉，于是标题栏退回系统那个通用方块。
/// 实测过 —— 创建窗口时手工 `WM_SETICON` 装上正确图标，第一帧一过就变回灰的；
/// 换成这里的 `with_icon` 立刻正常。
///
/// 数据由 `build.rs` 从 `assets/PiWebKeeper.ico` 里最大的一帧解出来（256×256 RGBA）：
/// 图标只有一份源文件，exe 资源（托盘、Explorer 用）与窗口用的是同一张图。
fn window_icon() -> egui::IconData {
    const SIZE: u32 = 256;

    egui::IconData {
        rgba: include_bytes!(concat!(env!("OUT_DIR"), "/window-icon.rgba")).to_vec(),
        width: SIZE,
        height: SIZE,
    }
}
