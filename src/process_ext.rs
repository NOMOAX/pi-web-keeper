//! 进程与作业对象（Job Object）的薄封装。
//!
//! 这个文件存在的唯一理由：让「终止整棵树」变成内核保证，而不是靠遍历父链猜。
//!
//! 按父链拼树再逐个杀靠不住：枚举所有进程、按父进程关系重建那棵树，对「已经被 reparent 掉的
//! 孙进程」无能为力，于是又得补一段「找占用端口的进程再杀一次」。
//!
//! 作业对象把这件事反过来做：进程一出生就在作业里，它 spawn 出去的一切也在作业里，
//! `TerminateJobObject` 一次收干净。而且 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` 让
//! 「启动器死掉 ⇒ 服务跟着死」由内核执行 —— 连启动器崩溃都覆盖到了。
//!
//! 两个不能省的细节：
//!   - 子进程以 `CREATE_SUSPENDED` 创建，先入作业再放行。否则从 `CreateProcess` 返回到
//!     `AssignProcessToJobObject` 之间有一小段窗口，`cmd.exe` 可能已经 spawn 出 `pwsh.exe`，
//!     那个孙进程就落在作业外面了。
//!   - 重启用的隐藏帮手**不**入作业。它是本进程的子进程，但它必须在启动器退出之后还活着，
//!     进了作业就会被 `KILL_ON_JOB_CLOSE` 一起带走。

use std::io;
use std::os::windows::io::AsRawHandle;
use std::process::{Child, Command};

use windows::Win32::Foundation::{CloseHandle, BOOL, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::System::Threading::{
    OpenThread, ResumeThread, CREATE_NO_WINDOW, CREATE_SUSPENDED, THREAD_SUSPEND_RESUME,
};

/// 一个作业对象。丢弃它就会终止里面的所有进程 —— 那是 `KILL_ON_JOB_CLOSE` 的效果。
pub struct JobObject {
    handle: HANDLE,
}

impl JobObject {
    /// 建一个「最后一个句柄关掉时杀掉全部成员」的作业。
    pub fn kill_on_close() -> io::Result<Self> {
        // SAFETY: 两个参数都允许为空（默认安全属性、匿名作业）
        let handle = unsafe { CreateJobObjectW(None, None) }
            .map_err(|error| io::Error::from_raw_os_error(error.code().0))?;

        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

        // SAFETY: limits 的大小与传入的长度一致，指向的是一份有效的值
        let configured = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };

        if let Err(error) = configured {
            // SAFETY: handle 刚由 CreateJobObjectW 取得，尚未交给别人
            unsafe {
                let _ = CloseHandle(handle);
            }

            return Err(io::Error::from_raw_os_error(error.code().0));
        }

        Ok(Self { handle })
    }

    /// 把一个尚未放行的进程收进作业。
    pub fn assign(&self, child: &Child) -> io::Result<()> {
        // SAFETY: child 的句柄由 std 持有，生命周期覆盖这次调用
        let assigned = unsafe { AssignProcessToJobObject(self.handle, HANDLE(child.as_raw_handle())) };

        assigned.map_err(|error| io::Error::from_raw_os_error(error.code().0))
    }

    /// 终止作业里的所有进程。
    pub fn terminate(&self) -> io::Result<()> {
        // SAFETY: handle 由 Self 独占持有
        let terminated = unsafe { TerminateJobObject(self.handle, 1) };

        terminated.map_err(|error| io::Error::from_raw_os_error(error.code().0))
    }
}

impl Drop for JobObject {
    fn drop(&mut self) {
        // SAFETY: 句柄只在 Self 里保存，Drop 只跑一次
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

/// 以挂起状态启动进程，等调用方把作业安排好了再 [`resume`]。
///
/// `CREATE_NO_WINDOW` 必须和 `CREATE_SUSPENDED` 一起在这里设：
/// `CommandExt::creation_flags` 是**赋值**而不是按位或，调用方先设一个、这里再设一个，
/// 第二个会把第一个覆盖掉。实测过被覆盖的后果 —— `cmd.exe /c pi-web` 带着控制台起来，
/// Windows Terminal 给它开了一个新标签页。
pub fn spawn_suspended(command: &mut Command) -> io::Result<Child> {
    use std::os::windows::process::CommandExt;

    command.creation_flags(CREATE_SUSPENDED.0 | CREATE_NO_WINDOW.0);
    command.spawn()
}

/// 放行一个挂起的进程。
///
/// `std::process::Child` 不给出主线程的句柄，所以这里按进程 ID 从线程快照里把它找回来。
/// 进程是挂起的，此刻它只有这一个线程，因此不存在认错的可能。
pub fn resume(child: &Child) -> io::Result<()> {
    let process_id = child.id();

    // SAFETY: 不带模块与堆参数，只取线程列表；返回的句柄下面由 RAII 包装关掉
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }
        .map_err(|error| io::Error::from_raw_os_error(error.code().0))?;

    let snapshot = HandleGuard(snapshot);

    let mut entry = THREADENTRY32 {
        dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };

    // SAFETY: entry.dwSize 已按 API 要求填好
    if let Err(error) = unsafe { Thread32First(snapshot.0, &mut entry) } {
        return Err(io::Error::from_raw_os_error(error.code().0));
    }

    loop {
        if entry.th32OwnerProcessID == process_id {
            // SAFETY: 线程 ID 来自快照，OpenThread 失败会返回错误而不是空句柄
            let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, BOOL(0), entry.th32ThreadID) }
                .map_err(|error| io::Error::from_raw_os_error(error.code().0))?;

            let thread = HandleGuard(thread);

            // SAFETY: 句柄有效，且这个线程确实是挂起的
            let previous = unsafe { ResumeThread(thread.0) };

            return if previous == u32::MAX {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            };
        }

        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;

        // SAFETY: 同 Thread32First；到头时返回错误
        if unsafe { Thread32Next(snapshot.0, &mut entry) }.is_err() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("找不到进程 {process_id} 的主线程"),
            ));
        }
    }
}

/// 只负责关句柄。
struct HandleGuard(HANDLE);

impl Drop for HandleGuard {
    fn drop(&mut self) {
        // 这个包装只会包住创建成功（非空）的句柄
        // SAFETY: 句柄由本结构独占
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
