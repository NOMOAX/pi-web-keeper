//! 界面与后台之间的那一层：命令上行、状态下行。
//!
//! 为什么要有这一层：生命周期契约是「启动器活着服务就活着」，而启动器的主体是那个窗口。
//! 如果服务由界面线程驱动，那么「窗口收进托盘、egui 不再重绘」的时候就没人看着服务了 ——
//! 那正是 C# 版靠 WPF 的 Dispatcher 恰好避开的坑。
//!
//! 所以这里把职责切干净：
//!   - 控制器线程独占 [`ServiceKeeper`]，负责一切阻塞操作（启停、更新、离开）；
//!   - 托盘线程只负责图标与菜单，点一下就把命令丢进通道；
//!   - 界面线程只读 [`Shared`]、只发命令，从不阻塞。
//!
//! 于是窗口画不画，与服务跑不跑，互不相干。

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::command;
use crate::config::KeeperConfig;
use crate::service::{Event, ServiceKeeper};

/// 日志最多留这么多行。再多就丢最旧的。
///
/// 这个上限的作用不是省内存，是让「追加一行」的代价保持恒定 ——
/// 界面每帧都要排版这一整块文本，行数增长就是每帧代价线性增长。
const LOG_LINE_LIMIT: usize = 2000;

/// 界面能做、托盘也能做的四件事，加上两个单向动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// 启动服务。启动时自动走这一条 —— 它不先看端口，因为「启动」的语义就是启动。
    StartService,

    /// 启动／停止服务（同一件事的两个方向，与按钮一致）。
    ToggleService,

    /// 跑更新。
    Update,

    /// 安排好继任者然后退出。
    RestartProgram,

    /// 停服务然后退出。
    Exit,

    /// 把窗口从托盘里叫出来。
    ShowWindow,
}

/// 后台与界面共享的状态。两边都只做「短锁 + 拷贝」。
#[derive(Default)]
pub struct Shared {
    /// 日志行。
    pub log: Vec<String>,

    /// 日志版本号。每追加一行加一，界面靠它判断要不要重建影子缓冲。
    pub log_revision: u64,

    /// 状态一句话，显示在按钮下方。
    pub state: String,

    /// 正在启停服务：按钮变灰，文字换成「正在启动 / 正在停止」。
    pub busy: bool,

    /// 正在更新：服务类动作与离开类动作一起变灰。
    pub updating: bool,

    /// 端口上是否有服务在监听。按钮文字与可用性靠它。
    pub listening: bool,

    /// 正在离开：重入保护。
    pub exiting: bool,

    /// 配置读失败时的原因。有它就没有服务可管，只剩托盘里的「退出」。
    pub config_error: Option<String>,

    /// 一次性标志：界面读完就关窗。
    pub quit: bool,
}

impl Shared {
    /// 追加一行日志，并把行数压到上限之内。
    pub fn push_line(&mut self, line: String) {
        self.log.push(line);
        self.log_revision = self.log_revision.wrapping_add(1);

        if self.log.len() > LOG_LINE_LIMIT {
            let overflow = self.log.len() - LOG_LINE_LIMIT;
            self.log.drain(..overflow);
        }
    }

    /// 清空日志。
    pub fn clear_log(&mut self) {
        self.log.clear();
        self.log_revision = self.log_revision.wrapping_add(1);
    }
}

/// 供界面与托盘使用的句柄。
#[derive(Clone)]
pub struct Handle {
    /// 共享状态。
    pub shared: Arc<Mutex<Shared>>,

    /// 上行命令。
    pub commands: Sender<Command>,
}

impl Handle {
    /// 读一眼共享状态。锁坏了就当空。
    pub fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        // 锁中毒只可能来自别的线程 panic，而这里的状态是可以重建的，直接忽略
        self.shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 发一条命令。失败说明控制器已经走了，忽略即可。
    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
}

/// 起控制器线程。
///
/// `config` 为 `None` 表示配置读失败：那时没有服务可管，
/// 但托盘里的「退出」仍然得能走通。
pub fn spawn(handle: Handle, config: Option<KeeperConfig>, inbox: Receiver<Command>) {
    std::thread::Builder::new()
        .name("PiWebKeeper.Controller".to_owned())
        .spawn(move || run(handle, config, inbox))
        .expect("控制器线程起不来");
}

/// 控制器的循环。
fn run(handle: Handle, config: Option<KeeperConfig>, inbox: Receiver<Command>) {
    // 服务对象要一个自己的事件通道，这里就地转进 Shared
    let (event_tx, event_rx) = std::sync::mpsc::channel::<Event>();

    let keeper = config.map(|config| ServiceKeeper::new(config, event_tx));

    if keeper.is_none() {
        handle.lock().state = "没有可管理的服务".to_owned();
    } else {
        // 按钮的初始文字靠这一句：它说的是「现在没有服务在跑」，而不是空白
        handle.lock().state = "未启动".to_owned();
    }

    loop {
        // 先等命令；超时正好用来做周期性的状态刷新
        match inbox.recv_timeout(Duration::from_millis(300)) {
            Ok(Command::Exit) => {
                leave(&handle, keeper.as_ref(), Kind::Exit);
                return;
            }
            Ok(Command::RestartProgram) => {
                leave(&handle, keeper.as_ref(), Kind::Restart);
                return;
            }
            Ok(Command::ToggleService) => {
                toggle_service(&handle, keeper.as_ref());
            }
            Ok(Command::StartService) => {
                start_service(&handle, keeper.as_ref());
            }
            Ok(Command::Update) => {
                run_update(&handle, keeper.as_ref());
            }
            Ok(Command::ShowWindow) => {
                // 托盘左键与「第二个实例来敲门」走同一条路。
                // 直接调 Win32：窗口藏起来之后界面不再重绘，置标志没人读（见 `window.rs`）。
                crate::window::show();
            }
            Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }

        drain_events(&handle, &event_rx);

        if let Some(keeper) = keeper.as_ref() {
            keeper.poll_exit();

            let listening = keeper.is_port_listening();

            handle.lock().listening = listening;
        }
    }
}

/// 把服务那边的事件搬进共享状态。
fn drain_events(handle: &Handle, events: &Receiver<Event>) {
    let mut guard = None;

    loop {
        match events.try_recv() {
            Ok(Event::Message(line)) => {
                guard.get_or_insert_with(|| handle.lock()).push_line(line);
            }
            Ok(Event::State(state)) => {
                guard.get_or_insert_with(|| handle.lock()).state = state;
            }
            Err(_) => break,
        }
    }
}

/// 服务按钮：服务没在监听就启动，在监听就停止。
fn toggle_service(handle: &Handle, keeper: Option<&ServiceKeeper>) {
    let Some(keeper) = keeper else {
        return;
    };

    if keeper.is_port_listening() {
        stop_service(handle, keeper);
    } else {
        start_service(handle, Some(keeper));
    }
}

/// 启动服务。
///
/// 成功就收起启动器（用户接下来要看的是应用窗口，不是日志），
/// 失败就把它留在屏幕上（报错得看得见）—— 启动时走的就是同一条路，
/// 只是那时窗口还没显示过，收起来等于什么都没发生。
fn start_service(handle: &Handle, keeper: Option<&ServiceKeeper>) {
    let Some(keeper) = keeper else {
        return;
    };

    if !begin_busy(handle) {
        return;
    }

    // 服务那边出 panic 也不能把控制器线程带走：界面还得能继续用
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| keeper.start()));

    {
        let mut shared = handle.lock();

        shared.busy = false;
        shared.listening = keeper.is_port_listening();

        match &outcome {
            Ok(Ok(())) => {}
            Ok(Err(reason)) => shared.push_line(format!("启动失败：{reason}")),
            Err(_) => shared.push_line("启动过程中出现未预期错误。".to_owned()),
        }
    }

    /*
     * 成功就收起启动器（用户接下来要看的是应用窗口，不是日志），
     * 失败就把它留在屏幕上（报错得看得见）—— 启动时走的就是同一条路，
     * 只是那时窗口还没显示过，收起来等于什么都没发生。
     */
    if matches!(outcome, Ok(Ok(()))) {
        crate::window::hide();
    } else {
        crate::window::show();
    }
}

/// 停止服务：只停服务，窗口留着。
fn stop_service(handle: &Handle, keeper: &ServiceKeeper) {
    if !begin_busy(handle) {
        return;
    }

    keeper.stop(Some(Duration::from_secs(20)));

    let mut shared = handle.lock();

    shared.busy = false;
    shared.listening = keeper.is_port_listening();
    shared.push_line("服务已停止。".to_owned());
}

/// 置忙。已经在忙、在更新、在离开时返回 `false`。
fn begin_busy(handle: &Handle) -> bool {
    let mut shared = handle.lock();

    if shared.busy || shared.updating || shared.exiting {
        return false;
    }

    shared.busy = true;
    true
}

/// 两种离开方式。
#[derive(Clone, Copy)]
enum Kind {
    /// 停服务、关窗口。
    Exit,

    /// 先安排继任者，再停服务、关窗口。
    Restart,
}

/// 离开程序。
///
/// 两者只差「有没有继任者」这一件事，而且都先把服务停掉：这样差值是零 ——
/// 服务在旧实例手里停完，继任者接着把它拉起来。
/// 把停服务留给继任者做的话，中间会多出「服务在跑、启动器不在」的一段。
fn leave(handle: &Handle, keeper: Option<&ServiceKeeper>, kind: Kind) {
    {
        let mut shared = handle.lock();

        if shared.exiting {
            return;
        }

        shared.exiting = true;
    }

    /*
     * 继任者先安排：它起不来就哪儿也别去 —— 留在这里，日志里有原因，
     * 服务也还没动，用户点一下「启动」就能恢复。
     */
    if matches!(kind, Kind::Restart) {
        match command::spawn_successor() {
            Ok(()) => handle
                .lock()
                .push_line("正在重启：本程序退出后它会再把它拉起来。".to_owned()),
            Err(error) => {
                let mut shared = handle.lock();
                shared.push_line(format!("重启失败：{error}"));
                shared.exiting = false;
                return;
            }
        }
    }

    if let Some(keeper) = keeper {
        keeper.stop(Some(Duration::from_secs(20)));
    }

    let mut shared = handle.lock();
    shared.push_line("服务已停止，关闭窗口。".to_owned());
    shared.busy = false;
    shared.quit = true;
    drop(shared);

    /*
     * 不能靠 egui 的关闭流程：窗口很可能正藏在托盘里，而藏着的窗口不再重绘，
     * `App::update` 也就不再被调用 —— 实测：只发 `WM_CLOSE` 的话进程会留在后台，
     * 服务停了、窗口没了，进程还在。
     *
     * 所以先让托盘线程把图标摘掉，再直接退进程。
     */
    crate::tray::request_shutdown();
    std::thread::sleep(Duration::from_millis(400));
    std::process::exit(0);
}

/// 跑一次更新。
///
/// 不碰正在运行的服务：它就是被更新的那些文件的占用者。服务在跑就不执行，
/// 直接告知原因（按钮已经置灰，这里只是把旁路也堵上）。
///
/// 命令写死在这里，不读配置：更新的是 Pi 本体，与 `settings.json` 描述的那个服务无关。
///
/// 也不提供中途取消：半途杀掉 npm 会留下一个替换了一半的包。
fn run_update(handle: &Handle, keeper: Option<&ServiceKeeper>) {
    {
        let mut shared = handle.lock();

        if shared.busy || shared.updating || shared.exiting {
            return;
        }

        let listening = keeper.map(|keeper| keeper.is_port_listening()).unwrap_or(false);

        if listening {
            shared.push_line(
                "服务正在运行，不执行更新。服务运行中不执行更新：更新不会停掉正在运行的服务，请先点「停止」。"
                    .to_owned(),
            );
            return;
        }

        shared.busy = true;
        shared.updating = true;
    }

    /*
     * 一条命令退出码非零就停下：后面的命令更新的是另一份包，
     * 在前一步已经出错的情况下继续跑，只会把失败的事搅成一团。
     */
    for command_line in ["pi update --all", "pi-web update"] {
        handle.lock().push_line(format!("执行：{command_line}"));

        let sink = handle.clone();

        let outcome = command::run_streaming(command_line, &move |line| {
            sink.lock().push_line(line);
        });

        match outcome {
            Ok(0) => {}
            Ok(code) => {
                handle.lock().push_line(format!(
                    "更新中止：{command_line} 退出码 {code}，后面的命令不再执行。"
                ));

                break;
            }
            Err(error) => {
                handle.lock().push_line(format!("更新失败：{error}"));
                break;
            }
        }
    }

    let mut shared = handle.lock();

    shared.updating = false;
    shared.busy = false;
}
