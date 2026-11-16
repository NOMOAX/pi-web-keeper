//! 单实例门闸：同一个安装目录只允许一个启动器窗口。
//!
//! 名字与 C# 版完全一致（`Local\PiWebKeeper.Instance.<key>` 与 `...Activate.<key>`），
//! 所以新旧两个实现在同一个目录里能认出彼此 —— 换实现的那一天不会变成两个实例同时跑。
//!
//! 为什么按安装目录而不是全局唯一：一个安装目录托管一个服务，多个目录就是多个服务。
//! Pi Web 和别的服务各自一个目录，它们的启动器要能同时开着，所以互斥的名字必须带上目录。
//!
//! 第二个实例不会静默消失：它唤醒已经开着的那个窗口，再自己退出。

use sha2::{Digest, Sha256};
use windows::core::HSTRING;
use windows::Win32::Foundation::{
    CloseHandle, BOOL, HANDLE, WAIT_ABANDONED, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, OpenEventW, ReleaseMutex, SetEvent, WaitForSingleObject,
    EVENT_MODIFY_STATE, INFINITE,
};

/// 实例锁。
pub struct SingleInstance {
    mutex: HANDLE,
    activation_event: HANDLE,
}

impl SingleInstance {
    /// 尝试取得实例所有权。第一个实例返回门闸对象，后续实例返回 `None`。
    pub fn acquire() -> Option<Self> {
        let key = install_key();

        // SAFETY: 名字是合法的内核对象名；失败返回错误
        let mutex = unsafe {
            CreateMutexW(None, BOOL(0), &HSTRING::from(mutex_name(&key)))
        }
        .ok()?;

        // SAFETY: 同上。自动重置（第二个参数为假）—— 两次敲门合并成一次，正合需要
        let activation_event = unsafe {
            CreateEventW(None, BOOL(0), BOOL(0), &HSTRING::from(activation_name(&key)))
        }
        .ok()?;

        /*
         * 用 0 毫秒的等待而不是「创建即拥有」：
         * 前者能区分「我拿到了」和「锁被别人持有」，而废弃的锁（上一个实例被强杀）
         * 在 Windows 上会以 WAIT_ABANDONED 返回，同样算拿到。
         */
        // SAFETY: mutex 刚创建，句柄有效
        let waited = unsafe { WaitForSingleObject(mutex, 0) };

        if waited == WAIT_OBJECT_0 || waited == WAIT_ABANDONED {
            return Some(Self {
                mutex,
                activation_event,
            });
        }

        // SAFETY: 两个句柄都由本函数创建，尚未交给别人。
        // WAIT_TIMEOUT 说明已经有实例在跑；WAIT_FAILED 说明问不出来，都按「不启动」处理。
        debug_assert!(waited == WAIT_TIMEOUT || waited == WAIT_FAILED);

        unsafe {
            let _ = CloseHandle(mutex);
            let _ = CloseHandle(activation_event);
        }

        None
    }

    /// 在后台等第二个实例的唤醒信号，收到就回调。
    ///
    /// 回调在后台线程上，调用方自己切回 UI 线程。
    /// 这个线程不设退出条件：句柄归 `SingleInstance` 所有，程序结束时就一起没了。
    pub fn listen_for_activation<F>(&self, on_activated: F)
    where
        F: Fn() + Send + 'static,
    {
        let activation_event = self.activation_event;

        std::thread::Builder::new()
            .name("PiWebKeeper.SingleInstanceListener".to_owned())
            .spawn(move || loop {
                // SAFETY: 句柄归 SingleInstance 所有，它比这个线程活得久
                let waited = unsafe { WaitForSingleObject(activation_event, INFINITE) };

                if waited != WAIT_OBJECT_0 {
                    return;
                }

                on_activated();
            })
            .ok();
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        // SAFETY: 两个句柄都由 Self 独占，Drop 只跑一次
        unsafe {
            let _ = ReleaseMutex(self.mutex);
            let _ = CloseHandle(self.mutex);
            let _ = CloseHandle(self.activation_event);
        }
    }
}

/// 作为第二个实例，唤醒已经开着的那个窗口，然后自己退出。
pub fn signal_existing() {
    let name = activation_name(&install_key());

    // SAFETY: 名字合法；打不开说明对面已经没了，那就不叫
    let Ok(event) = (unsafe { OpenEventW(EVENT_MODIFY_STATE, BOOL(0), &HSTRING::from(name)) })
    else {
        return;
    };

    // SAFETY: 句柄刚取得且有效
    unsafe {
        let _ = SetEvent(event);
        let _ = CloseHandle(event);
    }
}

fn mutex_name(key: &str) -> String {
    format!("Local\\PiWebKeeper.Instance.{key}")
}

fn activation_name(key: &str) -> String {
    format!("Local\\PiWebKeeper.Activate.{key}")
}

/// 互斥名的一部分：取安装目录的哈希而不是路径本身。
///
/// 路径里有反斜杠、冒号、中文，直接拼进内核对象名不合法；
/// 哈希也顺带避免把用户目录结构暴露在对象名里。
///
/// 算法与 C# 版逐字节一致（小写化 → UTF-8 → SHA-256 → 前 8 字节 → 大写十六进制），
/// 这样新旧两个 exe 才会算出同一个名字。
fn install_key() -> String {
    let normalized = crate::config::install_directory().to_string_lossy().to_lowercase();

    let digest = Sha256::digest(normalized.as_bytes());

    digest[..8].iter().map(|byte| format!("{byte:02X}")).collect()
}
