//! Pi Web Keeper：托管唯一一个本地 Pi Web 服务的 Windows 启动器。
//!
//! 生命周期契约：启动器存活期间服务就存活。这一条不因实现语言而改变，
//! 只是换成了由内核执行 —— 服务进程活在一个「最后句柄关掉就全部杀掉」的作业对象里
//! （见 `process_ext.rs`），所以启动器崩溃也会把服务带走。
//!
//! 启动路径只有一条：双击 exe 就等于「启动服务」，不显示窗口。
//! 唯一例外是 `--no-autostart`（跳转列表里的「以普通方式启动」）：显示界面、不动服务。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_window;
mod command;
mod config;
mod controller;
mod font;
mod jump_list;
mod port;
mod process_ext;
mod service;
mod single_instance;
mod tray;
mod ui;
mod window;

use std::sync::{mpsc, Arc, Mutex};

use controller::{Command, Handle, Shared};

fn main() {
    let arguments: Vec<String> = std::env::args().collect();

    /*
     * 隐藏模式：等旧进程退出，再把自己重新拉起来。
     *
     * 这一段必须在单实例门闸之前 —— 它的全部意义就是「新实例要等旧实例真的退出」。
     * 用本程序自己的隐藏模式而不是外部脚本：同样是等一个 pid，
     * 但没有命令行转义，「路径里的空格把命令撕成两半」那个坑不存在。
     */
    if let Some(position) = arguments.iter().position(|argument| argument == command::TAKE_OVER_FLAG) {
        let process_id = arguments
            .get(position + 1)
            .and_then(|raw| raw.parse::<u32>().ok())
            .unwrap_or(0);

        let _ = command::take_over(process_id);
        return;
    }

    let start_on_launch = !arguments.iter().any(|argument| argument == command::NO_AUTOSTART_FLAG);

    /*
     * 单实例：同一个安装目录只允许一个启动器。
     * 第二个实例不会静默消失 —— 它唤醒已经开着的那个窗口，再自己退出。
     */
    let Some(instance) = single_instance::SingleInstance::acquire() else {
        single_instance::signal_existing();
        return;
    };

    let shared = Arc::new(Mutex::new(Shared::default()));

    let (command_tx, command_rx) = mpsc::channel::<Command>();

    let handle = Handle {
        shared: Arc::clone(&shared),
        commands: command_tx,
    };

    /*
     * 跳转列表：登记失败不致命，只记一行日志。
     * 它必须在创建任何窗口之前登记 —— AppUserModelID 一旦设下，进程身份就定了。
     */
    let jump_list_error = jump_list::install().err();

    // 读配置。失败就没有服务可管，但托盘里的「退出」仍然得能走通。
    let config = match config::load() {
        Ok(config) => Some(config),
        Err(error) => {
            let mut guard = handle.lock();
            guard.config_error = Some(error.to_string());
            guard.push_line(format!("配置加载失败：{error}"));
            None
        }
    };

    if let Some(config) = config.as_ref() {
        let mut guard = handle.lock();
        guard.push_line(format!("已加载配置：{}", config::config_path().display()));
        guard.push_line(format!("服务地址：{}:{}", config.host, config.port));
        guard.push_line(format!("命令：{}", config.start_command));
    }

    if let Some(error) = jump_list_error {
        handle.lock().push_line(format!("跳转列表登记失败：{error}"));
    }

    controller::spawn(handle.clone(), config, command_rx);
    tray::spawn(handle.clone());

    /*
     * 第二个实例来敲门：把窗口叫出来。
     * 回调在后台线程上，而且可能发生在窗口已经藏进托盘、界面不再重绘的时候，
     * 所以这条命令由控制器线程直接调 Win32，不经过界面帧（见 `window.rs`）。
     */
    instance.listen_for_activation({
        let handle = handle.clone();

        move || handle.send(Command::ShowWindow)
    });

    if let Err(error) = ui::run(handle, start_on_launch) {
        // 窗口起不来就没有界面了；这行在 windows_subsystem 下看不见，但至少不会静默
        eprintln!("界面启动失败：{error}");
    }

    drop(instance);
}
