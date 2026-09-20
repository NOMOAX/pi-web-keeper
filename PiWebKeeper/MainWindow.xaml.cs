using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
using System.Windows;
using System.Windows.Interop;

namespace PiWebKeeper;

/// <summary>
/// 主窗口：一个按钮控制服务的启动 / 停止，右侧是更新按钮，下方是只读日志。
///
/// 关闭窗口即停止服务 —— 窗口与服务同生命周期，这正是它存在的意义。
/// </summary>
public partial class MainWindow
{
    private ServiceKeeper? _keeper;

    private bool _isClosingAfterStop;

    private bool _isBusy;

    private bool _isUpdating;

    /// 正在进行的那次停止。只给按钮文字用：它区分“正在停止”和“正在启动”。
    private Task? _stopTask;

    /// <summary>
    /// 按钮之间的间距，水平与垂直用同一个值。
    /// 水平方向是同行相邻按钮的间隔；垂直方向是按钮自身的外边距，
    /// 所以它决定了按钮行底部到日志面板的距离，以及（万一折行时）两行之间的间隔。
    /// </summary>
    private const double ButtonSpacing = 8;

    public MainWindow()
    {
        InitializeComponent();

        ApplyButtonDetails();

        SourceInitialized += OnSourceInitialized;
        Loaded            += OnLoaded;
        Closing           += OnClosing;
    }

    /// <summary>
    /// 两个按钮的外边距与提示语。
    ///
    /// 间距不写在 XAML 里，是为了服务按钮和更新按钮用同一个值：两边各写一份的话，
    /// 改一边就会漏掉另一边。窗口宽度是固定的，行尾多出来的那点边距不会撑宽窗口。
    ///
    /// 提示语也放在这里：同一句话还要写进日志，两份文本就是两个会各自变陈旧的版本。
    /// </summary>
    private void ApplyButtonDetails()
    {
        var margin = new Thickness(0, 0, ButtonSpacing, ButtonSpacing);

        ServiceButton.Margin = margin;
        UpdateButton.Margin  = margin;
        UpdateButton.ToolTip = ServiceRunningUpdateHint;
    }

    /// <summary>
    /// 取掉窗口的最大化按钮。
    ///
    /// 放在这里（这时窗口已经建好、但还没显示）是因为改动不会被用户看到。
    /// </summary>
    private void OnSourceInitialized(object? sender, EventArgs e)
    {
        var handle = new WindowInteropHelper(this).Handle;
        if (handle == IntPtr.Zero)
        {
            return;
        }

        RemoveMaximizeBox(handle);
    }

    /// <summary>
    /// 清掉 WS_MAXIMIZEBOX 位。
    ///
    /// WPF 没有“只禁用最大化、保留缩放”的开关：ResizeMode 的三个取值要么把最大化留着，
    /// 要么连边框拖拽一起去掉。这里要的正是中间那种，所以直接改窗口样式的位。
    /// 缩放与最小化照旧；双击标题栏最大化也会一并失效，它走的也是这个位。
    /// </summary>
    private static void RemoveMaximizeBox(IntPtr handle)
    {
        var style = GetWindowLongPtr(handle, GWL_STYLE);
        if (style == IntPtr.Zero)
        {
            return;
        }

        SetWindowLongPtr(handle, GWL_STYLE, style.ToInt64() & ~WS_MAXIMIZEBOX);

        // 改完样式要告诉系统重算非客户区，否则标题栏上的按钮不会被重画
        SetWindowPos(handle, IntPtr.Zero, 0, 0, 0, 0, SWP_FLAGS);
    }

    private const int GWL_STYLE = -16;

    private const long WS_MAXIMIZEBOX = 0x00010000L;

    private const uint SWP_NOSIZE = 0x0001;

    private const uint SWP_NOMOVE = 0x0002;

    private const uint SWP_NOZORDER = 0x0004;

    private const uint SWP_NOACTIVATE = 0x0010;

    private const uint SWP_FRAMECHANGED = 0x0020;

    private const uint SWP_FLAGS = SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED;

    [DllImport("user32.dll", EntryPoint = "GetWindowLongPtrW")]
    private static extern IntPtr GetWindowLongPtr(IntPtr hWnd, int index);

    [DllImport("user32.dll", EntryPoint = "SetWindowLongPtrW")]
    private static extern IntPtr SetWindowLongPtr(IntPtr hWnd, int index, long value);

    [DllImport("user32.dll")]
    private static extern bool SetWindowPos(
        IntPtr hWnd,
        IntPtr insertAfter,
        int    x,
        int    y,
        int    width,
        int    height,
        uint   flags);

    private void OnLoaded(object sender, RoutedEventArgs e)
    {
        KeeperConfig config;
        try
        {
            config = KeeperConfigLoader.Load();
        }
        catch (Exception ex)
        {
            ServiceButton.IsEnabled = false;
            AppendLog("配置加载失败：" + ex.Message);
            return;
        }

        _keeper              =  new ServiceKeeper(config);
        _keeper.LogReceived  += OnKeeperLog;
        _keeper.StateChanged += OnKeeperState;

        AppendLog($"已加载配置：{KeeperConfigLoader.ConfigPath}");
        AppendLog($"服务地址：{config.Host}:{config.Port}");
        AppendLog($"命令：{config.StartCommand}");

        RefreshButtonState();
    }

    /// <summary>
    /// 更新按钮上为什么会被置灰的说明。
    /// </summary>
    private const string ServiceRunningUpdateHint = "服务运行中不执行更新：更新不会停掉正在运行的服务，请先点“停止”，或直接关闭本窗口。";

    private void RefreshButtonState()
    {
        if (_isBusy)
        {
            ServiceButton.Content = _isUpdating
                                        ? "正在更新"
                                        : _stopTask is { IsCompleted: false }
                                            ? "正在停止"
                                            : "正在启动";
            ServiceButton.IsEnabled = false;
        }
        else if (_keeper is null)
        {
            ServiceButton.Content   = "启动";
            ServiceButton.IsEnabled = false;
        }
        else
        {
            ServiceButton.Content   = _keeper.IsPortListening ? "停止" : "启动";
            ServiceButton.IsEnabled = true;
        }

        /*
         * 更新按钮只在服务没在跑的时候可用。
         *
         * 更新会重写已安装的包，而正在运行的服务就锁着那些文件（实测：服务跑着时
         * npm 会报 EBUSY rename）；但这个程序不会为了更新去把服务停掉 —— 那等于用一个
         * 按钮杀死用户正在用的服务。所以选择是不更新：服务在跑就不动手，
         * 把按钮置灰，并把原因写在提示里。
         */
        var canUpdate = !_isBusy && !_isUpdating && _keeper is { IsPortListening: false };
        UpdateButton.IsEnabled = canUpdate;
    }

    private void OnKeeperLog(string line)
    {
        Dispatcher.BeginInvoke(WindowAppendLog, this, line);
    }

    private static readonly Action<MainWindow, string> WindowAppendLog = (window, text) => window.AppendLog(text);

    private void OnKeeperState(string state)
    {
        Dispatcher.BeginInvoke(WindowApplyState, this, state);
    }

    private static readonly Action<MainWindow, string> WindowApplyState = (window, text) =>
    {
        window.AppendLog(text);
        window.RefreshButtonState();
    };

    /// <summary>
    /// 同一个按钮的两个方向：服务没在监听就是“启动”，在监听就是“停止”。
    ///
    /// 没有单独的“重新启动”：想重启就点一次“停止”再点一次“启动”——
    /// 少一条把“停”和“启”串在一起、而两步各自都可能失败的路径。
    /// </summary>
    private async void OnServiceButtonClicked(object sender, RoutedEventArgs e)
    {
        try
        {
            if (_keeper is null || _isBusy)
            {
                return;
            }

            if (_keeper.IsPortListening)
            {
                await StopServiceAsync();
                return;
            }

            await StartServiceAsync();
        }
        catch (Exception)
        {
            // 两条路径各自都把错误写进日志了
        }
    }

    private async Task StartServiceAsync()
    {
        _isBusy = true;
        RefreshButtonState();

        try
        {
            await _keeper!.StartAsync();

            // 服务真的起来了再收起启动器：用户接下来要看的是应用窗口，不是日志。
            // 位置放在这里而不是点击处，是为了让启动失败时窗口还留在屏幕上 —— 报错得看得见。
            WindowState = WindowState.Minimized;
        }
        catch (Exception ex)
        {
            AppendLog("启动失败：" + ex.Message);
        }
        finally
        {
            _isBusy = false;
            RefreshButtonState();
        }
    }

    /// <summary>
    /// 按钮上的“停止”：只停服务，窗口继续开着。
    ///
    /// 这次停记在 _stopTask 上，按钮文字靠它显示“正在停止”。
    /// </summary>
    private async Task StopServiceAsync()
    {
        _isBusy = true;
        RefreshButtonState();

        var stopping = StopCurrentAsync();
        _stopTask = stopping;

        try
        {
            await stopping;
            AppendLog("服务已停止。");
        }
        finally
        {
            _isBusy = false;
            RefreshButtonState();
        }
    }

    /// <summary>
    /// 执行一次更新。
    ///
    /// 不碰正在运行的服务：它就是被更新的那些文件的占用者。服务在跑就不执行，
    /// 直接告知原因（按钮已经置灰，这里只是把双击等旁路也堵上）。
    /// 也不停服务、不重启服务 —— 更新只改磁盘上的包，服务的生命周期不因它改变。
    ///
    /// 更新命令自己还有一层同类保护：pi-web update 会先看有没有在跑的 pi-web 服务，
    /// 有就拒绝执行。两边都拦，是因为两边的“在跑”判断依据不同（这里是端口，
    /// 它那里是启动记录），谁都比对方多认得出一种情况。
    /// </summary>
    private async void OnUpdateButtonClicked(object sender, RoutedEventArgs e)
    {
        try
        {
            if (_isBusy || _isUpdating || _keeper is null)
            {
                return;
            }

            if (_keeper.IsPortListening)
            {
                AppendLog("服务正在运行，不执行更新。" + ServiceRunningUpdateHint);
                return;
            }

            await RunUpdateAsync();
        }
        catch (Exception)
        {
            // RunUpdateAsync 自己已经把错误写进日志了
        }
    }

    /// <summary>
    /// 把下面两条命令按顺序跑完，输出实时写进日志。
    ///
    /// 整段过程不碰服务：既不先停它，也不在结束后拉起来。
    /// 也不提供中途取消：半途杀掉 npm 会留下一个替换了一半的包。
    ///
    /// 一条命令退出码非零就停下：后面的命令更新的是另一份包，
    /// 在前一步已经出错的情况下继续跑，只会把失败的事搅成一团。
    /// </summary>
    private async Task RunUpdateAsync()
    {
        /*
         * 命令写死在这里，不读配置：更新的是 Pi 本体，与 settings.json 描述的那个
         * 被托管服务无关。两项更新的顺序是先 Pi 本体、后 Pi Web。
         *
         * 也不去调用户手边那些 .cmd 快捷方式：它们是给人点的，带 -NoExit，
         * 会一直占着控制台不退出，当按钮的命令就是永远转不完。
         */
        string[] updateCommands = ["pi update --all", "pi-web update"];

        _isBusy     = true;
        _isUpdating = true;
        RefreshButtonState();

        try
        {
            foreach (var command in updateCommands)
            {
                AppendLog($"执行：{command}");

                // 输出回调在后台线程上：必须丢回 UI 线程，AppendLog 会碰到控件
                var exitCode = await UpdateRunner.RunStreamingAsync(
                                                     command,
                                                     line => Dispatcher.BeginInvoke(WindowAppendLog, this, line)
                                                 )
                                                 .ConfigureAwait(true);

                if (exitCode != 0)
                {
                    AppendLog($"更新中止：{command} 退出码 {exitCode}，后面的命令不再执行。");
                    return;
                }
            }

            AppendLog("更新完成。");
        }
        catch (Exception ex)
        {
            AppendLog("更新失败：" + ex.Message);
        }
        finally
        {
            _isUpdating = false;
            _isBusy     = false;
            RefreshButtonState();
        }
    }

    /// <summary>
    /// 停止服务：只停服务，窗口留着。
    /// </summary>
    private async Task StopCurrentAsync()
    {
        if (_keeper is null)
        {
            return;
        }

        using var cancellation = new CancellationTokenSource(TimeSpan.FromSeconds(20));
        try
        {
            await _keeper.StopAsync(cancellation.Token);
        }
        catch (Exception ex) when (ex is OperationCanceledException or TimeoutException)
        {
            AppendLog("停止服务超时。");
        }
    }

    private async void OnClosing(object? sender, CancelEventArgs e)
    {
        try
        {
            if (_isClosingAfterStop)
            {
                return;
            }

            /*
             * 关窗只管一件事：把它拉起来的服务停掉。
             * 启动或更新还在跑时关窗不做任何兜底（不等、不取消）—— 用户自己会让它们跑完，
             * 这两条路上再堆一层等待与取消，只是多一套要跟着维护的路径。
             */

            // 没有 _keeper（配置加载失败）时无服务可停，直接关闭
            if (_keeper is null || !_keeper.IsPortListening)
            {
                return;
            }

            e.Cancel = true;
            _isBusy   = true;
            RefreshButtonState();

            /*
             * 按钮上可能已经在停了：这里再停一次也无妨 ——
             * StopAsync 第一次就把进程交出去了，第二次只会等端口释放。
             */
            _stopTask = StopCurrentAsync();

            try
            {
                await _stopTask;
                AppendLog("服务已停止，关闭窗口。");
            }
            catch (Exception ex)
            {
                AppendLog("停止服务时出错：" + ex.Message);
            }

            _isClosingAfterStop = true;
            _                   = Dispatcher.BeginInvoke(new Action(Close));
        }
        catch (Exception)
        {
            // ignored
        }
    }

    private void AppendLog(string line)
    {
        var builder = new StringBuilder();
        if (LogBox.Text.Length > 0)
        {
            builder.AppendLine();
        }

        builder.Append(line);
        LogBox.AppendText(builder.ToString());
        LogBox.ScrollToEnd();
    }
}
