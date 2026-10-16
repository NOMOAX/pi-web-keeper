using System.Runtime.InteropServices;
using System.Windows.Interop;

namespace PiWebKeeper;

/// <summary>
/// 通知区域里的那一个托盘图标：显示应用名，把左键 / 右键点击交回主窗口，
/// 并在 Explorer 重启之后把自己重新登记回去。
///
/// 直接调 Shell_NotifyIcon，没有走 WinForms 的 NotifyIcon：托盘图标是这里唯一需要
/// WinForms 的东西，为它引进来会让自包含的单文件可执行文件从 64 MB 涨到 75 MB，
/// 代价远大于这一百多行互操作。
///
/// 点击要由一个窗口来收：Shell 把鼠标动作 post 成一条自定义消息（<see cref="TrayMessage"/>）
/// 发到登记的窗口上，所以图标得挂在主窗口的 handle 上（<see cref="Attach"/>）。
/// 窗口收进托盘也不影响 —— 隐藏的窗口照样收消息，它只是不再显示。
/// </summary>
public sealed class TrayIcon(string tooltip) : IDisposable
{
    /// <summary>
    /// Shell 把点击发到主窗口时用的消息号。取 WM_APP 后的第一格，不碰系统消息。
    /// </summary>
    private const int TrayMessage = WmApp + 1;

    private const int WmApp = 0x8000;

    /// <summary>
    /// 认的就是按下后的那一下（Up）。
    ///
    /// Shell 把 Down 和 Up 都发过来，两个都认等于一次点击做两遍 ——
    /// 左键会变成"显示又立刻收起"，看上去就是点了没反应。
    /// </summary>
    private const int WmLeftButtonUp = 0x0202;

    private const int WmRightButtonUp = 0x0205;

    /// <summary>
    /// 键盘操作托盘图标时 Shell 发的是这条（Vista 起的版本里右键点击也归到它）。
    /// </summary>
    private const int WmContextMenu = 0x007B;

    /// <summary>
    /// 程序自己那份图标的资源号：.NET SDK 的 ApplicationIcon 就嵌在 32512 上
    /// （这是 Windows 留给“程序主图标”的号，也是 IDI_APPLICATION）。
    /// </summary>
    private const int ApplicationIconResourceId = 32512;

    /// <summary>
    /// 系统默认的程序图标，图标资源取不到时顶上。
    /// </summary>
    private const int DefaultApplicationIconResourceId = 0x7F00;

    /// <summary>
    /// 通知区域的提示语最长 63 个字符，超了旧版 Shell 会截断得很难看。
    /// </summary>
    private const int TooltipLengthLimit = 63;

    private const int ImageIcon = 1;

    private const int SystemMetricSmallIconWidth = 49;

    private const int SystemMetricSmallIconHeight = 50;

    private const uint LrShared = 0x00008000;

    private const int NifMessage = 0x00000001;

    private const int NifIcon = 0x00000002;

    private const int NifTip = 0x00000004;

    private const int NimAdd = 0x00000000;

    private const int NimDelete = 0x00000002;

    private readonly string _tooltip = Truncate(tooltip, TooltipLengthLimit);

    /// <summary>
    /// Explorer 重启之后广播的消息号。收到就得重新交一次图标，否则托盘里会空出一格。
    /// </summary>
    /// <remarks>消息号每个进程都不一样，得在登记图标之前问出来。</remarks>
    private readonly int _taskbarCreatedMessage = RegisterWindowMessage("TaskbarCreated");

    private readonly int _dataSize = Marshal.SizeOf<NotifyIconData>();

    private IntPtr _windowHandle;

    private HwndSource? _messageSource;

    private bool _isAdded;

    /// <summary>
    /// 输出一行日志（供界面显示）。
    /// </summary>
    public event Action<string>? LogReceived;

    /// <summary>
    /// 左键单击：让主窗口在屏幕与托盘之间来回。
    /// </summary>
    public event Action? LeftClicked;

    /// <summary>
    /// 右键单击：弹出右键菜单（菜单本身由主窗口构造）。
    /// </summary>
    public event Action? MenuRequested;

    /// <summary>
    /// 把图标登记到通知区域，点击消息落到 windowHandle 上。
    ///
    /// 对象先在构造函数里建好、这里才挂窗口，是因为点击靠窗口来收，
    /// 而窗口的 handle 要等 WPF 把它建出来才有（见 MainWindow.OnSourceInitialized）。
    /// </summary>
    public void Attach(IntPtr windowHandle)
    {
        _windowHandle  = windowHandle;
        _messageSource = HwndSource.FromHwnd(windowHandle);
        _messageSource?.AddHook(OnWindowMessage);

        AddIcon();
    }

    private void AddIcon()
    {
        if (_windowHandle == IntPtr.Zero)
        {
            return;
        }

        var data = CreateData(LoadIconHandle());
        data.Tooltip = _tooltip;
        data.Flags   = NifMessage | NifIcon | NifTip;

        _isAdded = Shell_NotifyIcon(NimAdd, ref data);

        if (!_isAdded)
        {
            LogReceived?.Invoke("托盘图标登记失败：通知区域没有接受它。");
        }
    }

    /// <summary>
    /// 取一帧图标给通知区域。
    ///
    /// 用的是 exe 自己的图标资源（ApplicationIcon 嵌进去的那一份），不另外再嵌一份 .ico：
    /// 就是同一个文件，托管资源里存第二遍没有意义。
    /// 尺寸按通知区域当前需要的大小取，100% 缩放下是 16 —— 交给 Shell 缩放会糊。
    /// </summary>
    private IntPtr LoadIconHandle()
    {
        var width  = GetSystemMetrics(SystemMetricSmallIconWidth);
        var height = GetSystemMetrics(SystemMetricSmallIconHeight);

        /*
         * LR_SHARED：图标归系统管，两件事都省了 —— 进程里不用自己 DestroyIcon，
         * 而 Shell 拿到的这个 handle 在进程活着的时候一直有效，不必担心被回收。
         */
        var icon = LoadImage(GetModuleHandle(null), ApplicationIconResourceId, ImageIcon, width, height, LrShared);

        if (icon != IntPtr.Zero)
        {
            return icon;
        }

        // 图标资源没取到也照样要有托盘图标：窗口收起来之后它是唯一的入口，
        // 难看一点的图标总好过一个进不去的窗口
        LogReceived?.Invoke("取不到程序图标，托盘改用系统默认图标。");

        return LoadIcon(IntPtr.Zero, DefaultApplicationIconResourceId);
    }

    /// <summary>
    /// 主窗口的消息钩子：Shell 发来的点击与 Explorer 的重启广播都从这里进。
    /// </summary>
    private IntPtr OnWindowMessage(IntPtr hwnd, int message, IntPtr wParam, IntPtr lParam, ref bool handled)
    {
        if (message == _taskbarCreatedMessage)
        {
            AddIcon();
            return IntPtr.Zero;
        }

        if (message != TrayMessage)
        {
            return IntPtr.Zero;
        }

        // 具体是哪个鼠标动作写在 lParam 里
        switch (lParam.ToInt32())
        {
            case WmLeftButtonUp:
                handled = true;
                LeftClicked?.Invoke();
                break;

            case WmRightButtonUp:
            case WmContextMenu:
                handled = true;
                MenuRequested?.Invoke();
                break;
        }

        return IntPtr.Zero;
    }

    private NotifyIconData CreateData(IntPtr iconHandle) =>
        new()
        {
            Size            = _dataSize,
            WindowHandle    = _windowHandle,
            Id              = 1,
            CallbackMessage = TrayMessage,
            IconHandle      = iconHandle
        };

    private static string Truncate(string text, int limit) => text.Length <= limit ? text : text[..limit];

    /// <summary>
    /// 摘掉托盘图标。
    ///
    /// 少了这一步，图标会以幽灵的形式留在通知区域里，要等鼠标划过去才消失。
    /// 图标本身不用销毁：它是 LR_SHARED 取来的，归系统管。
    /// </summary>
    public void Dispose()
    {
        if (_isAdded)
        {
            var data = CreateData(IntPtr.Zero);
            Shell_NotifyIcon(NimDelete, ref data);
            _isAdded = false;
        }

        _messageSource?.RemoveHook(OnWindowMessage);
        _messageSource = null;
    }

    /// <summary>
    /// NOTIFYICONDATA 的 Vista 版（x64 上是 976 字节）。
    ///
    /// 用完整尺寸而不是前几个字段的旧版：字段顺序与对齐跟原生结构一一对应，
    /// 而 cbSize 写的就是这个结构的大小 —— Shell 靠它判断该按哪一版解释。
    /// </summary>
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct NotifyIconData
    {
        public int Size;

        public IntPtr WindowHandle;

        public int Id;

        public int Flags;

        public int CallbackMessage;

        public IntPtr IconHandle;

        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)]
        public string Tooltip;

        public int State;

        public int StateMask;

        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 256)]
        public string Info;

        public int VersionOrTimeout;

        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 64)]
        public string InfoTitle;

        public int InfoFlags;

        public Guid ItemGuid;

        public IntPtr BalloonIconHandle;
    }

    [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
    private static extern bool Shell_NotifyIcon(int message, ref NotifyIconData data);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int RegisterWindowMessage(string message);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
    private static extern IntPtr GetModuleHandle(string? moduleName);

    [DllImport("user32.dll")]
    private static extern IntPtr LoadImage(IntPtr instance, int name, int type, int width, int height, uint flags);

    [DllImport("user32.dll")]
    private static extern IntPtr LoadIcon(IntPtr instance, int name);

    [DllImport("user32.dll")]
    private static extern int GetSystemMetrics(int index);
}
