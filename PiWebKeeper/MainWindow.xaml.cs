using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Interop;

namespace PiWebKeeper;

/// <summary>
/// 主窗口：一个按钮控制服务的启动 / 停止，右侧是更新按钮，下方是只读日志。
///
/// 窗口与服务同生命周期：窗口收进托盘的时候服务继续跑，只有托盘菜单的“退出”
/// 才会连服务一起收走。最小化与关闭对用户是同一个意思 —— 回到托盘里去。
/// </summary>
public partial class MainWindow
{
    private ServiceKeeper? _keeper;

    /// <summary>
    /// 托盘图标。窗口收起来之后，它是这个程序唯一的入口。
    /// </summary>
    private readonly TrayIcon _trayIcon;

    /// <summary>
    /// 托盘图标的右键菜单。
    /// </summary>
    private readonly ContextMenu _trayMenu;

    /// <summary>
    /// 菜单里的服务项与更新项，文字与可用性跟着窗口上的按钮走（见 RefreshActionState）。
    /// </summary>
    private readonly MenuItem _trayServiceItem = new()
    {
        Header = "启动"
    };

    private readonly MenuItem _trayUpdateItem = new()
    {
        Header = "更新"
    };

    /// <summary>
    /// 菜单里的“退出”。更新期间会置灰（见 RefreshActionState）。
    /// </summary>
    private readonly MenuItem _trayExitItem = new()
    {
        Header = "退出"
    };

    private bool _isClosingAfterStop;

    /// <summary>
    /// 是否正在退出：窗口的“关闭”只是收进托盘，只有它为真时才真的关窗。
    /// </summary>
    private bool _isExiting;

    private bool _isBusy;

    private bool _isUpdating;

    /// <summary>
    /// 正在进行的那次停止。只给按钮文字用：它区分“正在停止”和“正在启动”。
    /// </summary>
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

        /*
         * 托盘图标在构造函数里就建好，不等配置加载：配置读失败时窗口上的按钮会被禁用，
         * 而关闭窗口又只是收进托盘，那时托盘菜单的“退出”就是唯一的出路，它得先存在。
         */
        _trayMenu = BuildTrayMenu();

        _trayIcon               =  new TrayIcon(Title);
        _trayIcon.LogReceived   += OnKeeperLog;
        _trayIcon.LeftClicked   += OnTrayIconLeftClicked;
        _trayIcon.MenuRequested += ShowTrayMenu;
        _trayMenu.Opened        += OnTrayMenuOpened;

        LoadKeeper();

        SourceInitialized += OnSourceInitialized;
        Closing           += OnClosing;
        Closed            += OnClosed;
        StateChanged      += OnWindowStateChanged;

        /*
         * 先把窗口的 handle 造出来，但不要显示它。
         *
         * 两件事都指望着它：托盘图标挂在主窗口的 handle 上（见 TrayIcon），
         * 而启动流程要在窗口不露面的情况下跑完（见 StartOnLaunch）。
         * 不在这里造出来的话，handle 要等到第一次 Show() 才有，
         * 那时托盘图标和窗口一起出现，即使用户根本不需要窗口。
         */
        new WindowInteropHelper(this).EnsureHandle();
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
    /// 取掉窗口的最大化按钮，并把托盘图标挂到窗口上。
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

        // 托盘的点击由这个窗口接收（见 TrayIcon），所以图标要等 handle 有了才能登记
        _trayIcon.Attach(handle);
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
        var style = GetWindowLongPtr(handle, WindowStyleIndex);
        if (style == IntPtr.Zero)
        {
            return;
        }

        SetWindowLongPtr(handle, WindowStyleIndex, style.ToInt64() & ~MaximizeBoxStyle);

        // 改完样式要告诉系统重算非客户区，否则标题栏上的按钮不会被重画
        SetWindowPos(handle, IntPtr.Zero, 0, 0, 0, 0, StyleChangeOnly);
    }

    /// <summary>
    /// GetWindowLongPtr 的索引：窗口样式（GWL_STYLE）。
    /// </summary>
    private const int WindowStyleIndex = -16;

    /// <summary>
    /// 窗口样式里的“最大化按钮”那一位（WS_MAXIMIZEBOX）。
    /// </summary>
    private const long MaximizeBoxStyle = 0x00010000L;

    /// <summary>
    /// SetWindowPos 的 SWP_NOSIZE：不改窗口大小。
    /// </summary>
    private const uint PreserveSize = 0x0001;

    /// <summary>
    /// SetWindowPos 的 SWP_NOMOVE：不移动窗口。
    /// </summary>
    private const uint PreservePosition = 0x0002;

    /// <summary>
    /// SetWindowPos 的 SWP_NOZORDER：不改窗口的 Z 序。
    /// </summary>
    private const uint PreserveZOrder = 0x0004;

    /// <summary>
    /// SetWindowPos 的 SWP_NOACTIVATE：不改激活状态。
    /// </summary>
    private const uint PreserveActivation = 0x0010;

    /// <summary>
    /// SetWindowPos 的 SWP_FRAMECHANGED：让系统按新样式重算非客户区。
    /// </summary>
    private const uint RedrawFrame = 0x0020;

    /// <summary>
    /// 上面五个标志合起来：这次调用只改样式，位置、大小、Z 序、激活状态都保持原样。
    /// </summary>
    private const uint StyleChangeOnly =
        PreservePosition | PreserveSize | PreserveZOrder | PreserveActivation | RedrawFrame;

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

    /// <summary>
    /// 读配置、建服务对象。
    ///
    /// 放在构造函数里而不是窗口的 Loaded 事件上：启动流程要在窗口从未显示的情况下跑完，
    /// 那时 Loaded 不会触发（见 StartOnLaunch）。
    ///
    /// 读失败时两个按钮都得置灰：加载器已经在日志里写清楚了原因。
    /// </summary>
    private void LoadKeeper()
    {
        KeeperConfig config;
        try
        {
            config = KeeperConfigLoader.Load();
        }
        catch (Exception ex)
        {
            AppendLog("配置加载失败：" + ex.Message);
            RefreshActionState();
            return;
        }

        _keeper              =  new ServiceKeeper(config);
        _keeper.LogReceived  += OnKeeperLog;
        _keeper.StateChanged += OnKeeperState;

        AppendLog($"已加载配置：{KeeperConfigLoader.ConfigPath}");
        AppendLog($"服务地址：{config.Host}:{config.Port}");
        AppendLog($"命令：{config.StartCommand}");

        RefreshActionState();
    }

    /// <summary>
    /// 更新按钮上为什么会被置灰的说明。
    /// </summary>
    private const string ServiceRunningUpdateHint = "服务运行中不执行更新：更新不会停掉正在运行的服务，请先点“停止”，或直接关闭本窗口。";

    /// <summary>
    /// 这个启动器能做三件事：启动 / 停止服务、更新、退出。这里把每件事现在叫什么、
    /// 能不能做，一次写到它出现的两个地方 —— 窗口上的两个按钮，和托盘菜单里的三行。
    ///
    /// 判断只在这里做一次：同一件事有两个入口，两处各判断一次，迟早会有一处变陈旧，
    /// 变成“按钮是停止、菜单是启动”这种自相矛盾的样子。
    /// </summary>
    private void RefreshActionState()
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

        /*
         * 菜单那三行的可用性完全取自按钮。
         *
         * 文字没有完全跟着按钮：窗口上只有那一个按钮要兼作“正在启动 / 正在更新”的提示位，
         * 菜单里这两句话分别由两个菜单项自己说（第一项说启动或停止，第二项说更新或正在更新），
         * 照抄按钮的话，忙碌时两行会显示同一句话。
         */
        _trayServiceItem.Header    = _keeper is { IsPortListening: true } ? "停止" : "启动";
        _trayServiceItem.IsEnabled = ServiceButton.IsEnabled;

        /*
         * 这一项点击后菜单要不要留着，取决于它当下是什么动作：
         * 点“停止”多半后面还跟着一次“启动”（重启就是这两下），收起来等于逼用户再右键一次；
         * 点“启动”则相反 —— 启动成功窗口就收起来了，菜单挂在屏幕上只会碍事。
         */
        _trayServiceItem.StaysOpenOnClick = _keeper is { IsPortListening: true };

        _trayUpdateItem.Header    = _isUpdating ? "正在更新" : "更新";
        _trayUpdateItem.IsEnabled = UpdateButton.IsEnabled;

        /*
         * 更新期间连退出也置灰。
         * 更新没有中途取消这条路（半途杀掉 npm 会留下一个替换了一半的包），
         * 于是“能不能退出”就变成了“要不要留一个坏掉的安装”——
         * 这几十秒不值得用一份坏包去换。
         */
        _trayExitItem.IsEnabled = !_isUpdating;
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
        window.RefreshActionState();
    };

    /// <summary>
    /// 程序启动时的第一件事：直接走一遍“启动”，中间不显示窗口。
    ///
    /// 成功的话这个窗口从头到尾没露过面 —— 用户要的是服务和应用窗口，不是这块日志；
    /// 失败（配置读不出来，或者服务没起来）才把它拿出来：错误得看得见。
    /// 这正好也是窗口上那个按钮的语义：成功就收起来，失败就留在眼前。
    /// </summary>
    public void StartOnLaunch()
    {
        _ = StartOnLaunchAsync();
    }

    private async Task StartOnLaunchAsync()
    {
        // 配置读不出来时 _keeper 是 null：窗口必须出来，原因就写在日志里
        if (_keeper is null || !await StartServiceAsync())
        {
            ShowFromTray();
        }
    }

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

            // 成功与否这里不用管：成功就收进托盘，失败时窗口本来就留在屏幕上
            await StartServiceAsync();
        }
        catch (Exception)
        {
            // 两条路径各自都把错误写进日志了
        }
    }

    /// <summary>
    /// 启动服务。
    ///
    /// 成功就收起启动器（用户接下来要看的是应用窗口，不是日志），
    /// 失败就把它留在屏幕上（报错得看得见）—— 启动时走的就是同一条路，
    /// 只是那时窗口还没显示过，收起来等于什么都没发生。
    /// </summary>
    /// <returns>服务是否起来了。</returns>
    private async Task<bool> StartServiceAsync()
    {
        _isBusy = true;
        RefreshActionState();

        try
        {
            await _keeper!.StartAsync();

            // 收进托盘而不是摊在任务栏上 —— 要它回来点托盘图标就行
            HideToTray();
            return true;
        }
        catch (Exception ex)
        {
            AppendLog("启动失败：" + ex.Message);
            return false;
        }
        finally
        {
            _isBusy = false;
            RefreshActionState();
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
        RefreshActionState();

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
            RefreshActionState();
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
        RefreshActionState();

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
            RefreshActionState();
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

    /// <summary>
    /// 关闭按钮不关掉程序，只是把窗口收进托盘 —— 服务继续跑。
    ///
    /// 真正退出走托盘菜单的“退出”（见 OnExitMenuItemClicked），它先把服务停掉
    /// 再关窗口；所以走到这里而 _isExiting 为真的，只剩那一条路。
    /// </summary>
    private async void OnClosing(object? sender, CancelEventArgs e)
    {
        try
        {
            if (!_isExiting)
            {
                e.Cancel = true;
                HideToTray();
                return;
            }

            if (_isClosingAfterStop)
            {
                return;
            }

            /*
             * 走到这里只有一件事要做：确认服务真的停了（退出那条路已经停过一次，
             * 端口还活着说明那次超时了）。
             * 启动或更新还在跑时关窗不做任何兜底（不等、不取消）—— 用户自己会让它们跑完，
             * 这两条路上再堆一层等待与取消，只是多一套要跟着维护的路径。
             */

            // 没有 _keeper（配置加载失败）时无服务可停，直接关闭
            if (_keeper is null || !_keeper.IsPortListening)
            {
                return;
            }

            e.Cancel = true;
            _isBusy  = true;
            RefreshActionState();

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

    /// <summary>
    /// 托盘图标的右键菜单：启动 / 停止，更新，分隔线，退出。
    ///
    /// 前两项直接挂在窗口按钮的处理函数上：菜单项和按钮要做的是同一件事，
    /// 各写一个处理函数，就等于同一件事有两条会各自走偏的路径。
    ///
    /// 退出只在更新期间置灰（见 RefreshActionState）：
    /// 那是唯一一个“现在走会留下烂摊子”的时刻。
    /// </summary>
    private ContextMenu BuildTrayMenu()
    {
        _trayServiceItem.Click += OnServiceButtonClicked;
        _trayUpdateItem.Click  += OnUpdateButtonClicked;
        _trayExitItem.Click    += OnExitMenuItemClicked;

        /*
         * 更新这一项点击后菜单不收起。
         * 更新是这里唯一一个“点完还得看着”的动作：那两行会当场变成“正在更新”
         * 和灰掉的“退出”（见 RefreshActionState），菜单先收起来再让用户右键一次才能看到，
         * 等于让用户白点一次。
         *
         * 服务那一项的同一个开关是跟着它的动作走的（停止留着、启动收起）；
         * 退出则一定收起 —— 点完它程序就没了。
         */
        _trayUpdateItem.StaysOpenOnClick = true;

        return new ContextMenu
        {
            Items =
            {
                _trayServiceItem,
                _trayUpdateItem,
                new Separator(),
                _trayExitItem
            }
        };
    }

    /// <summary>
    /// 托盘左键：窗口在屏幕上就收进托盘，已经收着就拿出来。
    ///
    /// 双击就是两次单击：先收起来再拿出来，用户看到的是窗口闪一下。
    /// 想让它变成一次单击，就得把第一次单击压后等一等，为一个没人会用的手势
    /// 让单击也慢半拍，不划算。
    /// </summary>
    private void OnTrayIconLeftClicked()
    {
        if (IsVisible && WindowState != WindowState.Minimized)
        {
            HideToTray();
            return;
        }

        ShowFromTray();
    }

    /// <summary>
    /// 弹出右键菜单，位置取鼠标处，也就是托盘图标那里。
    /// </summary>
    private void ShowTrayMenu()
    {
        _trayMenu.Placement = PlacementMode.MousePoint;
        _trayMenu.IsOpen    = true;
    }

    /// <summary>
    /// 菜单弹出之后，把菜单自己的窗口推到前台。
    ///
    /// 实测：少了这一步，点菜单以外的地方菜单不会收起来。
    /// 菜单的消失靠的是它自己那把鼠标捕获被夺走，而进程手里没有前台窗口时，
    /// 那一下收不到；托盘图标不在任何窗口上，菜单就挂在那里了。
    /// </summary>
    private void OnTrayMenuOpened(object? sender, RoutedEventArgs e)
    {
        if (sender is ContextMenu menu && PresentationSource.FromVisual(menu) is HwndSource source)
        {
            SetForegroundWindow(source.Handle);
        }
    }

    /// <summary>
    /// 把窗口收进托盘。
    ///
    /// 先还原成 Normal 再隐藏：一个隐藏着的窗口若状态还是 Minimized，
    /// 下次 Show() 出来的是个最小化的空壳。
    /// </summary>
    private void HideToTray()
    {
        WindowState = WindowState.Normal;
        Hide();
    }

    /// <summary>
    /// 把窗口从托盘里拿出来、叫到前面。托盘左键与“第二个实例来敲门”都走这里。
    ///
    /// 最小化时要先还原：只调 Activate() 对最小化的窗口没有效果，
    /// 用户看到的就是“程序没理他”。
    /// </summary>
    public void ShowFromTray()
    {
        if (WindowState == WindowState.Minimized)
        {
            WindowState = WindowState.Normal;
        }

        if (!IsVisible)
        {
            Show();
        }

        Activate();
    }

    /// <summary>
    /// 最小化按钮不再往任务栏上放窗口，而是收到托盘里去。
    ///
    /// 拦的是状态变化而不是按钮点击：Alt+空格菜单里的“最小化”、
    /// 任务栏右键菜单、以及启动成功后自己收起的那一下，走的都是这里。
    /// </summary>
    private void OnWindowStateChanged(object? sender, EventArgs e)
    {
        if (WindowState == WindowState.Minimized)
        {
            HideToTray();
        }
    }

    /// <summary>
    /// 托盘菜单的“退出”：这才是真正的退出。
    ///
    /// 关窗口只是收进托盘（见 OnClosing），所以退出得自己把服务停掉 ——
    /// “启动器活着服务就活着，服务停了才轮到启动器走”这条契约，
    /// 不能因为多了一个托盘图标就断掉。
    /// 停服务走按钮那条路：按钮文字、超时处理只有一份。
    /// </summary>
    private async void OnExitMenuItemClicked(object sender, RoutedEventArgs e)
    {
        try
        {
            if (_isExiting)
            {
                return;
            }

            _isExiting = true;

            await StopServiceAsync();

            // 关掉这个窗口就是关掉最后一个窗口，程序随之退出
            Close();
        }
        catch (Exception)
        {
            // ignored
        }
    }

    private void OnClosed(object? sender, EventArgs e)
    {
        // 不摘掉托盘图标，它会变成通知区域里的一块幽灵，要等鼠标划过去才消失
        _trayIcon.Dispose();
    }

    [DllImport("user32.dll")]
    private static extern bool SetForegroundWindow(IntPtr hWnd);

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
