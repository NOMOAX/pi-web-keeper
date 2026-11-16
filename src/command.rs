//! 跑外部命令：流式输出、一次性退出码，以及重启用的「继任者交接」。
//!
//! 这两件事在 C# 版里分别是 `UpdateRunner` 和 `MainWindow.TryStartRestartHelper`。
//!
//! 帮手那一段与 C# 版有一个实质区别：C# 版把「等旧进程退出、再拉起新进程」写进一段
//! PowerShell 脚本，于是就出现了 AGENTS.md 里那个「引号前面那道反斜杠不能删」的坑 ——
//! 安装路径里的空格会被命令行解析撕成两半。
//!
//! 这里不要 PowerShell：本程序自己带一个隐藏模式（`--take-over <pid>`），
//! 由它做同样的事。少一层命令行转义，就少一整类 bug。

use std::io::{BufRead, BufReader};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use windows::Win32::Foundation::{BOOL, CloseHandle};
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

/// 隐藏模式：等指定进程退出，再把本程序重新拉起来。
pub const TAKE_OVER_FLAG: &str = "--take-over";

/// 只显示界面、不自动启动服务。
pub const NO_AUTOSTART_FLAG: &str = "--no-autostart";

/// 继任者等待旧进程退出的上限。
///
/// 到了上限也不再等：宁可多开一个实例（它会被单实例门闸当成「敲门」），
/// 也不要一个永远不启动的新进程。
const TAKE_OVER_WAIT: Duration = Duration::from_secs(30);

/// 一条命令的工作目录：用户主目录，与原先 .cmd 快捷方式的行为一致。
pub fn working_directory() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .unwrap_or_else(|| std::env::current_exe().ok().and_then(|path| path.parent().map(PathBuf::from)).unwrap_or_default())
}

/// 跑一条命令，把输出逐行交给回调，返回退出码。
///
/// 一律通过 `cmd.exe` 执行：npm / pi 都是 `.cmd` shim，直接 `CreateProcess` 找不到它们。
/// stdin 接空设备：命令若企图等待输入，会立刻读到 EOF 而不是把窗口挂死。
pub fn run_streaming(
    command_line: &str,
    on_line: &(dyn Fn(String) + Sync),
) -> std::io::Result<i32> {
    let mut child = Command::new("cmd.exe")
        .arg("/c")
        .arg(command_line)
        .current_dir(working_directory())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW.0)
        .spawn()?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // 两条管道各一个线程：只用一个线程轮流读，会在一块慢下来的管道上把另一块堵死
    std::thread::scope(|scope| {
        if let Some(stdout) = stdout {
            scope.spawn(|| pump(stdout, "", on_line));
        }

        if let Some(stderr) = stderr {
            scope.spawn(|| pump(stderr, "[stderr] ", on_line));
        }
    });

    let status = child.wait()?;

    Ok(status.code().unwrap_or(-1))
}

/// 把一块管道读干，逐行回调。
fn pump(pipe: impl std::io::Read, prefix: &str, on_line: &(dyn Fn(String) + Sync)) {
    for line in BufReader::new(pipe).lines() {
        match line {
            Ok(line) => on_line(format!("{prefix}{line}")),
            Err(_) => break,
        }
    }
}

/// 安排继任者：拉起一个隐藏模式的自己，它会在本进程退出之后启动新的实例。
///
/// 为什么必须放到进程外面：新实例要等旧实例真的退出才能起来 ——
/// 单实例门闸拿的是按安装目录命名的互斥，撞上锁的那个实例只会「敲一下已有实例然后自己退出」。
/// 而「等待」这件事本进程做不了，它马上就不在了。
///
/// 失败不致命：用户再双击一次 exe 就行。这是接这个方案时就认下的代价。
pub fn spawn_successor() -> std::io::Result<()> {
    let executable = std::env::current_exe()?;

    Command::new(executable)
        .arg(TAKE_OVER_FLAG)
        .arg(std::process::id().to_string())
        .creation_flags(CREATE_NO_WINDOW.0)
        .spawn()?;

    Ok(())
}

/// 隐藏模式的主体：等 `process_id` 退出，再启动一个没有参数的自己。
///
/// 等不到也算等到 —— 目标进程可能早就死透了，那正是我们要的状态。
pub fn take_over(process_id: u32) -> std::io::Result<()> {
    let deadline = std::time::Instant::now() + TAKE_OVER_WAIT;

    while std::time::Instant::now() < deadline && is_alive(process_id) {
        std::thread::sleep(Duration::from_millis(50));
    }

    let executable = std::env::current_exe()?;
    Command::new(executable).creation_flags(CREATE_NO_WINDOW.0).spawn()?;

    Ok(())
}

/// 进程是否还活着。
fn is_alive(process_id: u32) -> bool {
    // 拿不到句柄就是已经没了；拿得到再看是否仍然 signaled
    let Ok(process) = open_for_wait(process_id) else {
        return false;
    };

    // SAFETY: 句柄刚取得且有效
    let waited = unsafe { windows::Win32::System::Threading::WaitForSingleObject(process, 0) };

    // SAFETY: 句柄用完就还回去，否则每 50 毫秒泄一个
    unsafe {
        let _ = CloseHandle(process);
    }

    // WAIT_TIMEOUT (258) 表示还活着
    waited.0 == 258
}

/// 取一个用于等待的进程句柄。
fn open_for_wait(process_id: u32) -> windows::core::Result<windows::Win32::Foundation::HANDLE> {
    // SAFETY: pid 由调用方给出；失败返回错误而不是空句柄
    unsafe {
        windows::Win32::System::Threading::OpenProcess(
            windows::Win32::System::Threading::SYNCHRONIZE,
            BOOL(0),
            process_id,
        )
    }
}
