using System.Windows;

namespace PiWebKeeper;

public partial class App : Application
{
    private SingleInstanceGuard? _instanceGuard;

    protected override void OnStartup(StartupEventArgs e)
    {
        base.OnStartup(e);

#pragma warning disable WPF0001 // ThemeMode 在 .NET 10 里还是实验性 API
        ThemeMode = ThemeMode.Dark; // 如果要跟随系统主题，就用 ThemeMode.System
#pragma warning restore WPF0001

        _instanceGuard = SingleInstanceGuard.TryAcquire();
        if (_instanceGuard is null)
        {
            /*
             * 已经有实例在跑：把那个窗口叫到前面，然后自己安静退出。
             * 直接退出而不提示，是因为"窗口已经弹到眼前"本身就是反馈，
             * 再弹一个对话框只是多一步点击。
             */
            SingleInstanceGuard.SignalExistingInstance();
            Shutdown();

            return;
        }

        var window = new MainWindow();
        MainWindow = window;

        // 唤醒信号来自后台线程，切回 UI 线程再碰窗口
        _instanceGuard.ListenForActivation(() => Dispatcher.BeginInvoke(new Action(() => ActivateWindow(window))));

        window.Show();
    }

    /// <summary>
    /// 把已经开着的窗口叫到前面。
    ///
    /// 最小化时要先还原：只调 Activate() 对一个最小化的窗口是没有效果的，
    /// 用户会以为程序没理他。
    /// </summary>
    private static void ActivateWindow(Window window)
    {
        if (window.WindowState == WindowState.Minimized)
        {
            window.WindowState = WindowState.Normal;
        }

        if (window.IsVisible)
        {
            window.Activate();
        }
        else
        {
            window.Show();
        }
    }

    protected override void OnExit(ExitEventArgs e)
    {
        _instanceGuard?.Dispose();
        _instanceGuard = null;

        base.OnExit(e);
    }
}
