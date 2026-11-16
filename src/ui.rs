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
            .with_inner_size([640.0, 480.0])
            .with_min_inner_size([640.0, 480.0])
            .with_maximizable(false)
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
        let (quit, show, hide, revision, state, listening, busy, updating, exiting, has_config, lines) = {
            let mut shared = self.handle.lock();

            (
                shared.quit,
                std::mem::take(&mut shared.show_window),
                std::mem::take(&mut shared.hide_window),
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

        if show {
            context.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            context.send_viewport_cmd(egui::ViewportCommand::Focus);
        }

        if hide {
            context.send_viewport_cmd(egui::ViewportCommand::Visible(false));
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
            context.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        // 最小化也等于收进托盘：拦的是状态变化，所以按钮、系统菜单、自动收起都覆盖到了
        if context.input(|input| input.viewport().minimized).unwrap_or(false) {
            context.send_viewport_cmd(egui::ViewportCommand::Visible(false));
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

            let clear = egui::ScrollArea::vertical()
                .id_salt("message-box")
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let response = ui.add(
                        egui::TextEdit::multiline(&mut self.log_view)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY)
                            .desired_rows(20),
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
