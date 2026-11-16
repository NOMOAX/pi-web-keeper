//! `settings.json` 的内容：描述唯一一个被托管的 WebUI 服务。
//!
//! 只支持单个服务是有意为之 —— 程序与配置文件一对一，
//! 要托管另一个服务就复制一份目录、改配置文件。
//!
//! 显示名不在这里：一个安装目录只有一套名字，写在窗口标题上就够了。

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// 配置文件与 exe 同目录。
pub const FILE_NAME: &str = "settings.json";

/// 一个被托管的服务。
///
/// 字段名沿用 C# 版的 PascalCase（大小写不敏感地读，见 [`read_json`]），
/// 所以 main 分支上那份 `settings.json` 不用改一个字节。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct KeeperConfig {
    /// 服务监听主机，例如 `127.0.0.1`。
    #[serde(default = "default_host", alias = "host")]
    pub host: String,

    /// 服务监听端口，用于判断就绪与否。
    #[serde(alias = "port")]
    pub port: u16,

    /// Web 应用窗口的 AppUserModelID，用于精确打开与关闭该应用的窗口。
    #[serde(default, alias = "appUserModelId", alias = "appusermodelid")]
    pub app_user_model_id: String,

    /// 启动服务后，等待端口就绪的最长秒数。
    #[serde(default = "default_wait_seconds", alias = "waitSeconds")]
    pub wait_seconds: f64,

    /// 端口就绪后额外等待的秒数（有些服务就绪后还要加载插件）。
    #[serde(default, alias = "readyDelaySeconds")]
    pub ready_delay_seconds: f64,

    /// 启动服务的命令行，例如 `pi-web --no-open`。
    #[serde(default, alias = "startCommand")]
    pub start_command: String,
}

fn default_host() -> String {
    "127.0.0.1".to_owned()
}

fn default_wait_seconds() -> f64 {
    10.0
}

/// 加载配置时出的错。每一种都对应一句给用户看的话。
#[derive(Debug)]
pub enum ConfigError {
    /// 找不到配置文件。
    NotFound(PathBuf),

    /// 文件读得出来，但内容不是合法的 JSON。
    Malformed(String),

    /// 缺少必填项。
    MissingStartCommand,

    /// 端口不可用。
    InvalidPort,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(path) => write!(formatter, "找不到配置文件：{}", path.display()),
            Self::Malformed(reason) => write!(formatter, "配置文件内容为空或格式不正确：{reason}"),
            Self::MissingStartCommand => write!(formatter, "配置文件缺少 startCommand。"),
            Self::InvalidPort => write!(formatter, "port 取值无效：0 不表示具体端口，无法用于探测就绪状态。"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// 配置文件路径：exe 所在目录。
pub fn config_path() -> PathBuf {
    install_directory().join(FILE_NAME)
}

/// exe 所在目录。
///
/// 与 C# 版的 `AppContext.BaseDirectory` 同一个意思：单文件发布时 exe 就在那里，
/// 不会因为工作目录变化而找不到配置。
pub fn install_directory() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 读配置。
pub fn load() -> Result<KeeperConfig, ConfigError> {
    let path = config_path();

    if !path.exists() {
        return Err(ConfigError::NotFound(path));
    }

    let text = std::fs::read_to_string(&path)
        .map_err(|error| ConfigError::Malformed(error.to_string()))?;

    let config: KeeperConfig =
        json5::from_str(&text).map_err(|error| ConfigError::Malformed(error.to_string()))?;

    if config.start_command.trim().is_empty() {
        return Err(ConfigError::MissingStartCommand);
    }

    /*
     * 端口 0 表示“由系统分配”，这种端口无法用于探测就绪状态，所以同样视为无效。
     * 上限由 u16 本身保证。
     */
    if config.port == 0 {
        return Err(ConfigError::InvalidPort);
    }

    Ok(config)
}
