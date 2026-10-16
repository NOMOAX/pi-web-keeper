using System.Windows;

namespace PiWebKeeper;

public partial class App
{
    private SingleInstanceGuard? _instanceGuard;

    protected override void OnStartup(StartupEventArgs e)
    {
        base.OnStartup(e);

#pragma warning disable WPF0001     // ThemeMode 在 .NET 10 里还是实验性 API
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

        // 唤醒信号来自后台线程，切回 UI 线程再碰窗口（走的是托盘的展示逻辑，
        // 因为窗口可能正收在托盘里）
        _instanceGuard.ListenForActivation(() => Dispatcher.BeginInvoke(new Action(window.ShowFromTray)));

        /*
         * 不显示窗口就开跑：启动流程成功的话，这个窗口自始至终没露过面，
         * 用户要的是服务和应用窗口；失败时它自己会出来（见 StartOnLaunch）。
         */
        window.StartOnLaunch();
    }

    protected override void OnExit(ExitEventArgs e)
    {
        _instanceGuard?.Dispose();
        _instanceGuard = null;

        base.OnExit(e);
    }
}
