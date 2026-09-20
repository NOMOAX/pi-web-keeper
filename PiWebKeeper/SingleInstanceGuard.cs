using System.IO;
using System.Security.Cryptography;
using System.Text;

namespace PiWebKeeper;

/// <summary>
/// 单实例门闸：同一个安装目录只允许一个启动器窗口。
///
/// 为什么按安装目录而不是全局唯一：一个安装目录托管一个服务，多个目录就是多个服务
/// （见 AGENTS.md）。Pi Web 和 DeepSeek Harness 各自一个目录，它们的启动器要能同时开着，
/// 所以互斥的名字必须带上目录 —— 全局锁会把另一份安装也一起挡掉。
///
/// 第二个实例不会静默消失：它唤醒已经开着的那个窗口，再自己退出。
/// 这样用户双击第二次时看到的是"窗口被叫到前面"，而不是"什么都没发生"。
///
/// 内核对象名里的 PiWebKeeper 前缀是实例身份的一部分：改了前缀，同一个目录里
/// 新旧两个 exe 就各开各的。换版本的正确顺序是先关旧窗口、再启动新的。
/// </summary>
public sealed class SingleInstanceGuard : IDisposable
{
    private readonly Mutex _mutex;

    private readonly EventWaitHandle _activationEvent;

    private readonly CancellationTokenSource _listeningCancellation = new();

    private bool _ownsMutex;

    private SingleInstanceGuard(Mutex mutex, EventWaitHandle activationEvent)
    {
        _mutex           = mutex;
        _activationEvent = activationEvent;
    }

    /// <summary>
    /// 尝试取得实例所有权。第一个实例返回门闸对象，后续实例返回 null。
    /// </summary>
    public static SingleInstanceGuard? TryAcquire()
    {
        var key   = BuildKey();
        var mutex = new Mutex(initiallyOwned: false, name: $@"Local\PiWebKeeper.Instance.{key}");
        var activationEvent = new EventWaitHandle(
            initialState: false,
            mode: EventResetMode.AutoReset,
            name: $@"Local\PiWebKeeper.Activate.{key}"
        );

        try
        {
            /*
             * 用 WaitOne(0) 而不是构造函数的 initiallyOwned：
             * 前者能区分"我拿到了"和"锁是别人持有的"，后者拿不到时不会有任何提示。
             * 上一个实例被强杀时锁会处于废弃状态，这种情况算拿到。
             */
            var acquired = mutex.WaitOne(0);
            if (!acquired)
            {
                return null;
            }

            return new SingleInstanceGuard(mutex, activationEvent)
            {
                _ownsMutex = true
            };
        }
        catch (AbandonedMutexException)
        {
            // 上一个实例没有正常释放（被强杀）：这个锁现在归我们
            return new SingleInstanceGuard(mutex, activationEvent)
            {
                _ownsMutex = true
            };
        }
    }

    /// <summary>
    /// 作为第二个实例，唤醒已经开着的那个窗口，然后自己退出。
    /// </summary>
    public static void SignalExistingInstance()
    {
        if (EventWaitHandle.TryOpenExisting($@"Local\PiWebKeeper.Activate.{BuildKey()}", out var activationEvent))
        {
            using (activationEvent)
            {
                activationEvent.Set();
            }
        }
    }

    /// <summary>
    /// 在后台等第二个实例的唤醒信号，收到就回调（回调在后台线程上，调用方自己切回 UI 线程）。
    /// </summary>
    public void ListenForActivation(Action onActivated)
    {
        var thread = new Thread(() =>
            {
                var waitHandles = new[] { _activationEvent, _listeningCancellation.Token.WaitHandle };

                while (!_listeningCancellation.IsCancellationRequested)
                {
                    try
                    {
                        // 数组第一个是信号、第二个是取消：索引 0 表示真的有人来敲门
                        if (WaitHandle.WaitAny(waitHandles) == 0)
                        {
                            onActivated();
                        }
                    }
                    catch (ObjectDisposedException)
                    {
                        return;
                    }
                }
            }
        )
        {
            IsBackground = true,
            Name         = "PiWebKeeper.SingleInstanceListener"
        };

        thread.Start();
    }

    /// <summary>
    /// 互斥名的一部分。取安装目录的哈希而不是路径本身：
    /// 路径里有反斜杠、冒号、中文，直接拼进内核对象名不合法；
    /// 哈希也顺带避免把用户目录结构暴露在对象名里。
    /// </summary>
    private static string BuildKey()
    {
        var installDirectory = Path.GetDirectoryName(AppContext.BaseDirectory) ?? AppContext.BaseDirectory;

        var hash = SHA256.HashData(Encoding.UTF8.GetBytes(installDirectory.ToLowerInvariant()));

        return Convert.ToHexString(hash, 0, 8);
    }

    public void Dispose()
    {
        _listeningCancellation.Cancel();

        if (_ownsMutex)
        {
            try
            {
                _mutex.ReleaseMutex();
            }
            catch (ApplicationException)
            {
                // 已经释放过，忽略
            }

            _ownsMutex = false;
        }

        _listeningCancellation.Dispose();
        _activationEvent.Dispose();
        _mutex.Dispose();
    }
}
