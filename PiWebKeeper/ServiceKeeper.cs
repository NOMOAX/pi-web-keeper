using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Text;

namespace PiWebKeeper;

/// <summary>
/// 托管一个 WebUI 服务进程：启动、等待就绪、打开窗口、停止。
///
/// 生命周期契约：启动器存活期间服务就存活，启动器关闭时服务必须被终止。
/// 「关闭启动器就是停止服务」是整个程序的核心语义。
/// </summary>
public sealed class ServiceKeeper
{
    private static readonly TimeSpan PortReleaseTimeout = TimeSpan.FromSeconds(10);

    private static readonly TimeSpan ProcessExitTimeout = TimeSpan.FromSeconds(5);

    private readonly KeeperConfig _config;

    private readonly Lock _gate = new();

    private Process? _process;

    public ServiceKeeper(KeeperConfig config)
    {
        _config = config;
    }

    /// <summary>输出一行日志（供界面显示）。</summary>
    public event Action<string>? LogReceived;

    /// <summary>运行状态变化（供界面显示）。</summary>
    public event Action<string>? StateChanged;

    public bool IsRunning
    {
        get
        {
            lock (_gate)
            {
                return _process is { HasExited: false };
            }
        }
    }

    /// <summary>
    /// 端口上是否有服务在监听。与 <see cref="IsRunning"/> 不同：
    /// 服务可能是启动器之外拉起来的，此时进程不由我们持有，但端口是活的。
    /// </summary>
    public bool IsPortListening => PortProbe.IsListening(_config.Port);

    /// <summary>
    /// 启动服务。
    ///
    /// 启动前先把已有的一切停干净（进程 + 窗口），再从干净状态起来：
    /// 端口上已经有人在听、或者上一次的进程还没退干净，都不会堆成两份服务。
    /// 重新启动就是“停止”再“启动”各点一次，走的就是这里和 <see cref="StopAsync"/>。
    /// </summary>
    public async Task StartAsync()
    {
        SetState("正在停止已有服务…");
        // 起始这一停不带超时 token：它自己的收尾时限在 StopAsync 内部按需要给
        await StopAsync(CancellationToken.None).ConfigureAwait(false);

        SetState("正在启动服务…");
        StartProcess();
        await WaitUntilReadyAsync(CancellationToken.None).ConfigureAwait(false);

        if (_config.ReadyDelaySeconds > 0)
        {
            SetState($"等待服务加载（{_config.ReadyDelaySeconds:0.#} 秒）…");
            Log($"端口已就绪，额外等待 {_config.ReadyDelaySeconds:0.#} 秒（部分服务就绪后还要加载插件）。");
            await Task.Delay(TimeSpan.FromSeconds(_config.ReadyDelaySeconds)).ConfigureAwait(false);
        }

        OpenAppWindow();
        SetState($"运行中（{_config.Host}:{_config.Port}）");
    }

    /// <summary>
    /// 停止服务：关闭应用窗口，终止服务进程树，并确认端口真的释放。
    ///
    /// 最后一步（端口校验）不能省：进程被杀不代表端口立刻回收，实测存在延迟；
    /// 而只要端口还在监听，下次启动就会撞车。
    /// </summary>
    public async Task StopAsync(CancellationToken cancellationToken)
    {
        CloseAppWindowIfAny();
        KillProcessTree();

        if (!PortProbe.IsListening(_config.Port))
        {
            return;
        }

        SetState("等待端口释放…");
        var released = await PortProbe.WaitForStateAsync(
                                          _config.Port,
                                          shouldBeListening: false,
                                          PortReleaseTimeout,
                                          cancellationToken
                                      )
                                      .ConfigureAwait(false);

        if (released)
        {
            return;
        }

        // 兜底：树杀没有覆盖到的进程（例如服务是启动器之外拉起来的，或残留的孤儿子进程）
        Log($"端口 {_config.Port} 仍被占用，尝试终止占用进程。");
        foreach (var processId in PortProbe.FindOwningProcessIds(_config.Port))
        {
            TryKillProcessById(processId, killTree: true);
        }

        released = await PortProbe.WaitForStateAsync(
                                      _config.Port,
                                      shouldBeListening: false,
                                      PortReleaseTimeout,
                                      cancellationToken
                                  )
                                  .ConfigureAwait(false);

        if (!released)
        {
            Log($"警告：端口 {_config.Port} 仍未释放，下次启动可能会失败。");
        }
    }

    private void StartProcess()
    {
        // 通过 cmd.exe 执行，才能解析 PATH 上的 npm 全局命令（pi-web / dsh 都是 .cmd shim）
        var startInfo = new ProcessStartInfo("cmd.exe", $"/c {_config.StartCommand}")
        {
            UseShellExecute        = false,
            CreateNoWindow         = true,
            RedirectStandardOutput = true,
            RedirectStandardError  = true,
            StandardOutputEncoding = Encoding.UTF8,
            StandardErrorEncoding  = Encoding.UTF8,
            WorkingDirectory       = ResolveWorkingDirectory()
        };

        var process = new Process
        {
            StartInfo           = startInfo,
            EnableRaisingEvents = true
        };

        process.OutputDataReceived += (_, e) =>
        {
            if (e.Data is not null)
            {
                Log(e.Data);
            }
        };
        process.ErrorDataReceived += (_, e) =>
        {
            if (e.Data is not null)
            {
                Log("[stderr] " + e.Data);
            }
        };
        process.Exited += (_, _) =>
        {
            var exitCode = TryGetExitCode(process);
            Log($"服务进程已退出（退出码 {exitCode}）。");

            // Exited 是异步回调：重启流程里旧进程的退出通知可能晚于新进程的启动，
            // 只有确认退出的就是当前进程时才更新状态，否则会把"正在启动"覆盖成"已停止"。
            lock (_gate)
            {
                if (!ReferenceEquals(_process, process))
                {
                    return;
                }
            }

            SetState("已停止");
        };

        process.Start();
        process.BeginOutputReadLine();
        process.BeginErrorReadLine();

        lock (_gate)
        {
            _process = process;
        }

        Log($"服务已启动（pid {process.Id}）：{_config.StartCommand}");
    }

    /// <summary>
    /// 服务进程的工作目录，固定为用户主目录，与原先 .cmd 快捷方式的行为一致。
    /// </summary>
    private static string ResolveWorkingDirectory()
    {
        var profileDirectory = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);

        return Directory.Exists(profileDirectory) ? profileDirectory : AppContext.BaseDirectory;
    }

    private async Task WaitUntilReadyAsync(CancellationToken cancellationToken)
    {
        SetState($"等待服务就绪（最多 {_config.WaitSeconds:0.#} 秒）…");

        var ready = await PortProbe.WaitForStateAsync(
                                       _config.Port,
                                       shouldBeListening: true,
                                       TimeSpan.FromSeconds(_config.WaitSeconds),
                                       cancellationToken
                                   )
                                   .ConfigureAwait(false);

        if (!ready)
        {
            throw new TimeoutException(
                $"等待 {_config.Host}:{_config.Port} 就绪超时（{_config.WaitSeconds:0.#} 秒）。请查看日志确认服务是否启动失败。"
            );
        }

        Log($"端口 {_config.Port} 已就绪。");
    }

    private void OpenAppWindow()
    {
        if (string.IsNullOrWhiteSpace(_config.AppUserModelId))
        {
            Log("未配置 AppUserModelId，跳过打开应用窗口。");
            return;
        }

        try
        {
            // 用 explorer 拉起已安装的 PWA，这样才会得到独立的 Web 应用窗口
            Process.Start(
                new ProcessStartInfo("explorer.exe", $@"shell:AppsFolder\{_config.AppUserModelId}")
                {
                    UseShellExecute = true
                }
            );
            Log("已打开应用窗口。");
        }
        catch (Exception ex) when (ex is Win32Exception or InvalidOperationException)
        {
            Log($"打开应用窗口失败：{ex.Message}");
        }
    }

    private void CloseAppWindowIfAny()
    {
        if (string.IsNullOrWhiteSpace(_config.AppUserModelId))
        {
            return;
        }

        var closed = AppWindowCloser.CloseWindows(_config.AppUserModelId);
        if (closed > 0)
        {
            Log($"已关闭 {closed} 个应用窗口。");
        }
    }

    /// <summary>
    /// 终止服务进程树。
    ///
    /// 必须整树终止：服务本身是 cmd.exe -> pwsh -> node -> node 这样的链条，
    /// 只杀第一层会留下一堆还在监听端口的 node 孤儿进程。
    /// 实测：普通 Kill 会留下 detached 孙进程，Kill(entireProcessTree) 才能清干净。
    /// </summary>
    private void KillProcessTree()
    {
        Process? process;
        lock (_gate)
        {
            process  = _process;
            _process = null;
        }

        if (process is null)
        {
            return;
        }

        try
        {
            if (!process.HasExited)
            {
                process.Kill(entireProcessTree: true);
                process.WaitForExit((int)ProcessExitTimeout.TotalMilliseconds);
                Log("服务进程已终止。");
            }
        }
        catch (Exception ex) when (ex is InvalidOperationException or Win32Exception or NotSupportedException)
        {
            Log($"终止服务进程失败：{ex.Message}");
        }
        finally
        {
            process.Dispose();
        }
    }

    private void TryKillProcessById(int processId, bool killTree)
    {
        try
        {
            using var process = Process.GetProcessById(processId);
            process.Kill(entireProcessTree: killTree);
            process.WaitForExit((int)ProcessExitTimeout.TotalMilliseconds);
            Log($"已终止占用端口的进程（pid {processId}）。");
        }
        catch (Exception ex) when (ex is ArgumentException or InvalidOperationException or Win32Exception)
        {
            // 进程可能已经自己退出了
            Log($"终止进程 {processId} 时未成功：{ex.Message}");
        }
    }

    private static int TryGetExitCode(Process process)
    {
        try
        {
            return process.ExitCode;
        }
        catch (InvalidOperationException)
        {
            return -1;
        }
    }

    private void Log(string message)
    {
        LogReceived?.Invoke($"[{DateTime.Now:HH:mm:ss}] {message}");
    }

    private void SetState(string state)
    {
        StateChanged?.Invoke(state);
    }
}
