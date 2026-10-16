namespace PiWebKeeper;

/// <summary>
/// settings.json 的内容：描述唯一一个被托管的 WebUI 服务。
///
/// 只支持单个服务是有意为之 —— 程序与配置文件一对一，
/// 要托管另一个服务就复制一份目录、改配置文件。
///
/// 显示名不在这里：一个安装目录只有一套名字，写在窗口的 Title 上就够了。
/// </summary>
public sealed class KeeperConfig
{
    /// <summary>
    /// 服务监听主机，例如 "127.0.0.1"。
    /// </summary>
    public string Host { get; init; } = "127.0.0.1";

    /// <summary>
    /// 服务监听端口，用于判断就绪与否。
    /// </summary>
    public int Port { get; init; }

    /// <summary>
    /// Web 应用窗口的 AppUserModelID，用于精确关闭该应用的窗口。
    /// </summary>
    public string AppUserModelId { get; init; } = string.Empty;

    /// <summary>
    /// 启动服务后，等待端口就绪的最长秒数。
    /// </summary>
    public double WaitSeconds { get; init; } = 10;

    /// <summary>
    /// 端口就绪后额外等待的秒数（有些服务就绪后还要加载插件）。
    /// </summary>
    public double ReadyDelaySeconds { get; init; }

    /// <summary>
    /// 启动服务的命令行，例如 "pi-web --no-open"。
    /// </summary>
    public string StartCommand { get; init; } = string.Empty;
}
