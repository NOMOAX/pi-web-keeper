//! 托管一个 WebUI 服务进程：启动、等待就绪、打开窗口、停止。
//!
//! 生命周期契约：启动器存活期间服务就存活，启动器关闭时服务必须被终止。
//! 「关闭启动器就是停止服务」是整个程序的核心语义。
//!
//! 与 C# 版的结构差别：这里没有 async。C# 版用 `Task` 是因为 UI 线程不能被阻塞；
//! 这里所有阻塞操作都跑在后台线程上，UI 只读状态、只发命令，所以同步代码就够了 ——
//! 少一套 `ConfigureAwait` 与「输掉的那条 Task 没人 await」的心智负担。

use std::os::windows::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

use crate::app_window;
use crate::command;
use crate::config::KeeperConfig;
use crate::port;
use crate::process_ext::{self, JobObject};
/// 停止时等端口释放的上限。
const PORT_RELEASE_TIMEOUT: Duration = Duration::from_secs(10);

/// 等进程退出的上限。
const PROCESS_EXIT_TIMEOUT: Duration = Duration::from_secs(5);

/// 后台线程发给界面的东西。
#[derive(Debug, Clone)]
pub enum Event {
    /// 一行日志。
    Message(String),
}

/// 托管一个服务。
pub struct ServiceKeeper {
    config: KeeperConfig,

    /// 当前这一代服务进程。每次启动加一，用来分辨「旧进程的退出通知」。
    generation: AtomicU64,

    inner: Mutex<Inner>,

    events: Sender<Event>,
}

/// 受互斥保护的那一小块状态。
#[derive(Default)]
struct Inner {
    /// 作业对象：丢弃它就会终止里面的所有进程。
    job: Option<JobObject>,

    /// 服务进程。为 `None` 表示没有在跑。
    child: Option<Child>,
}

impl ServiceKeeper {
    /// 建一个托管者。`events` 是发给界面的通道。
    pub fn new(config: KeeperConfig, events: Sender<Event>) -> Self {
        Self {
            config,
            generation: AtomicU64::new(0),
            inner: Mutex::new(Inner::default()),
            events,
        }
    }

    /// 端口上是否有服务在监听。
    ///
    /// 与「进程是否由我们持有」不同：服务可能是启动器之外拉起来的，
    /// 此时进程不由我们持有，但端口是活的。
    pub fn is_port_listening(&self) -> bool {
        port::is_listening(self.config.port)
    }

    /// 启动服务。
    ///
    /// 启动前先把已有的一切停干净（进程 + 窗口），再从干净状态起来：
    /// 端口上已经有人在听、或者上一次的进程还没退干净，都不会堆成两份服务。
    /// 重新启动就是「停止」再「启动」各点一次，走的就是这里和 [`Self::stop`]。
    pub fn start(&self) -> Result<(), String> {
        self.emit_message("正在停止已有服务…");
        self.stop(None);

        self.emit_message("正在启动服务…");
        self.spawn_process()?;
        self.wait_until_ready()?;

        if self.config.ready_delay_seconds > 0.0 {
            self.emit_message(format!(
                "端口已就绪，额外等待 {:.1} 秒（部分服务就绪后还要加载插件）。",
                self.config.ready_delay_seconds
            ));
            std::thread::sleep(Duration::from_secs_f64(self.config.ready_delay_seconds));
        }

        self.open_app_window();

        Ok(())
    }

    /// 停止服务：关闭应用窗口，终止服务进程树，并确认端口真的释放。
    ///
    /// 最后一步（端口校验）不能省：进程被杀不代表端口立刻回收，实测存在延迟；
    /// 而只要端口还在监听，下次启动就会撞车。
    pub fn stop(&self, timeout: Option<Duration>) {
        self.close_app_window_if_any();
        self.kill_process_tree();

        if !port::is_listening(self.config.port) {
            return;
        }

        self.emit_message("等待端口释放…");

        let budget = timeout.unwrap_or(PORT_RELEASE_TIMEOUT);

        if port::wait_for_state(self.config.port, false, budget) {
            return;
        }

        // 兜底：树杀没有覆盖到的进程（例如服务是启动器之外拉起来的，或残留的孤儿子进程）
        self.emit_message(format!(
            "端口 {} 仍被占用，尝试终止占用进程。",
            self.config.port
        ));

        for process_id in port::owning_process_ids(self.config.port) {
            self.kill_process_by_id(process_id);
        }

        if !port::wait_for_state(self.config.port, false, budget) {
            self.emit_message(format!(
                "警告：端口 {} 仍未释放，下次启动可能会失败。",
                self.config.port
            ));
        }
    }

    /// 起进程、入作业、放行，并把输出接到日志上。
    fn spawn_process(&self) -> Result<(), String> {
        let mut command = Command::new("cmd.exe");
        command
            /*
             * `raw_arg` 而不是 `arg`：`arg` 会按 Rust 自己的规则给参数加引号、转义里面的引号，
             * 而 `cmd.exe /c` 收到的应该是**一整段命令文本**。
             * C# 版的 `ProcessStartInfo("cmd.exe", $"/c {config.StartCommand}")`
             * 就是那样的：整串原样传过去，没有第二层引号。
             * 换成 `arg` 的话，`node -e "..."` 这种命令会被搞坏。
             *
             * 创建标志不在这里设：`creation_flags` 是赋值而不是按位或，
             * 早点设的会被 `spawn_suspended` 覆盖掉（见那里）。
             */
            .raw_arg(format!("/c {}", self.config.start_command))
            .current_dir(command::working_directory())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = process_ext::spawn_suspended(&mut command)
            .map_err(|error| format!("启动服务进程失败：{error}"))?;

        /*
         * 先入作业再放行：从 CreateProcess 返回到 AssignProcessToJobObject 之间，
         * 挂起的进程一步也走不了，所以 cmd.exe 不可能抢在这之前 spawn 出 pwsh.exe。
         */
        let job = JobObject::kill_on_close()
            .map_err(|error| format!("创建作业对象失败：{error}"))?;

        job.assign(&child)
            .map_err(|error| format!("把服务进程收进作业失败：{error}"))?;

        process_ext::resume(&child).map_err(|error| format!("放行服务进程失败：{error}"))?;

        let process_id = child.id();

        if let Some(stdout) = child.stdout.take() {
            self.pipe_to_log(stdout, "");
        }

        if let Some(stderr) = child.stderr.take() {
            self.pipe_to_log(stderr, "[stderr] ");
        }

        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;

        {
            let mut inner = self.inner.lock().map_err(|_| "内部状态锁已损坏。".to_owned())?;
            inner.job = Some(job);
            inner.child = Some(child);
        }

        self.emit_message(format!(
            "服务已启动（pid {process_id}）：{}",
            self.config.start_command
        ));

        // 这一代号只用于日志；退出由 `poll_exit` 按帧检查
        let _ = generation;

        Ok(())
    }

    /// 把一块管道接到日志上。
    fn pipe_to_log(&self, pipe: impl std::io::Read + Send + 'static, prefix: &'static str) {
        let events = self.events.clone();

        std::thread::spawn(move || {
            use std::io::BufRead;

            for line in std::io::BufReader::new(pipe).lines() {
                let Ok(line) = line else { break };

                let _ = events.send(Event::Message(format!("{prefix}{line}")));
            }
        });
    }

    /// 检查服务进程是否已经退出，退出就更新状态。
    ///
    /// 由界面线程每帧调用一次。放在这里而不是后台线程，是因为「当前进程」这个判断
    /// 需要读 `inner`，而 `inner` 的生命周期跟着 `ServiceKeeper` 走。
    pub fn poll_exit(&self) {
        let mut inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(_) => return,
        };

        let Some(child) = inner.child.as_mut() else {
            return;
        };

        match child.try_wait() {
            Ok(Some(status)) => {
                let code = status.code().unwrap_or(-1);

                inner.child = None;
                inner.job = None;

                drop(inner);

                self.emit_message(format!("服务进程已退出（退出码 {code}）。"));
            }
            Ok(None) => {}
            Err(error) => {
                inner.child = None;
                inner.job = None;

                drop(inner);

                self.emit_message(format!("无法查询服务进程状态：{error}"));
            }
        }
    }

    /// 等端口就绪，或者等来一个坏消息。
    ///
    /// 两种情况都算结束：端口开了，或者进程自己退出了。
    /// 第二种必须算结束而不是继续等 —— 进程都没了，端口不会自己长出来，
    /// 而用户要对着「超时」那句等满整个 WaitSeconds，日志里明明早就写着服务退出了。
    fn wait_until_ready(&self) -> Result<(), String> {
        self.emit_message(format!(
            "等待服务就绪（最多 {:.1} 秒）…",
            self.config.wait_seconds
        ));

        let deadline = Instant::now() + Duration::from_secs_f64(self.config.wait_seconds.max(0.0));

        loop {
            if port::is_listening(self.config.port) {
                self.emit_message(format!("端口 {} 已就绪。", self.config.port));
                return Ok(());
            }

            if let Some(code) = self.take_exit_code() {
                return Err(format!(
                    "服务进程启动后立刻退出了（退出码 {code}），端口 {} 没有打开。上面的输出就是它退出前说的话。",
                    self.config.port
                ));
            }

            if Instant::now() >= deadline {
                return Err(format!(
                    "等待 {}:{} 就绪超时（{:.1} 秒）。请查看日志确认服务是否启动失败。",
                    self.config.host, self.config.port, self.config.wait_seconds
                ));
            }

            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// 进程若已退出，返回退出码并清掉它。
    fn take_exit_code(&self) -> Option<i32> {
        let mut inner = self.inner.lock().ok()?;
        let child = inner.child.as_mut()?;

        match child.try_wait() {
            Ok(Some(status)) => {
                let code = status.code().unwrap_or(-1);

                inner.child = None;
                inner.job = None;

                Some(code)
            }
            _ => None,
        }
    }

    /// 用 explorer 拉起已安装的 PWA，这样才会得到独立的 Web 应用窗口。
    fn open_app_window(&self) {
        if self.config.app_user_model_id.trim().is_empty() {
            self.emit_message("未配置 AppUserModelId，跳过打开应用窗口。");
            return;
        }

        let target = format!("shell:AppsFolder\\{}", self.config.app_user_model_id);

        match Command::new("explorer.exe")
            .arg(target)
            .creation_flags(CREATE_NO_WINDOW.0)
            .spawn()
        {
            Ok(_) => self.emit_message("已打开应用窗口。"),
            Err(error) => self.emit_message(format!("打开应用窗口失败：{error}")),
        }
    }

    /// 关掉属于配置里那个 AUMID 的窗口。
    fn close_app_window_if_any(&self) {
        if self.config.app_user_model_id.trim().is_empty() {
            return;
        }

        let closed = app_window::close_windows(&self.config.app_user_model_id);

        if closed > 0 {
            self.emit_message(format!("已关闭 {closed} 个应用窗口。"));
        }
    }

    /// 终止服务进程树。
    ///
    /// 必须整树终止：服务本身是 `cmd.exe -> pwsh -> node -> node` 这样的链条，
    /// 只杀第一层会留下一堆还在监听端口的 node 孤儿进程。
    fn kill_process_tree(&self) {
        let (job, mut child) = {
            let Ok(mut inner) = self.inner.lock() else {
                return;
            };

            (inner.job.take(), inner.child.take())
        };

        if let Some(job) = job {
            if let Err(error) = job.terminate() {
                self.emit_message(format!("终止服务进程失败：{error}"));
            }
        }

        if let Some(child) = child.as_mut() {
            let deadline = Instant::now() + PROCESS_EXIT_TIMEOUT;

            while Instant::now() < deadline {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                    Err(_) => break,
                }
            }

            self.emit_message("服务进程已终止。");
        }
    }

    /// 按进程 ID 终止整棵树。用于树杀失败后的兜底清理。
    fn kill_process_by_id(&self, process_id: u32) {
        let result = Command::new("taskkill.exe")
            .args(["/PID", &process_id.to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW.0)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        match result {
            Ok(status) if status.success() => {
                self.emit_message(format!("已终止占用端口的进程（pid {process_id}）。"));
            }
            Ok(_) => {
                // 进程可能已经自己退出了
                self.emit_message(format!("终止进程 {process_id} 时未成功。"));
            }
            Err(error) => {
                self.emit_message(format!("终止进程 {process_id} 时未成功：{error}"));
            }
        }
    }

    fn emit_message(&self, message: impl Into<String>) {
        let _ = self.events.send(Event::Message(format!(
            "{} {}",
            timestamp(),
            message.into()
        )));
    }

}

/// `[HH:mm:ss]`，取系统本地时间。
fn timestamp() -> String {
    // SAFETY: 绑定库包好了缓冲区，返回值就是结构体本身
    let now: SYSTEMTIME = unsafe { GetLocalTime() };

    format!("[{:02}:{:02}:{:02}]", now.wHour, now.wMinute, now.wSecond)
}
