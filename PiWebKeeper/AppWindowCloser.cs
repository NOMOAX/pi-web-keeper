using System.Runtime.InteropServices;
using System.Text;

namespace PiWebKeeper;

/// <summary>
/// 按窗口级 AppUserModelID 关闭 Web 应用窗口。
///
/// 为什么不用进程或命令行匹配：Edge 会把多个 Web 应用窗口共置在同一进程里，
/// 按进程杀会误伤普通浏览器窗口；只有窗口自己的 AUMID 才是稳定身份。
/// </summary>
public static class AppWindowCloser
{
    private const uint WindowMessageClose = 0x0010;

    /// <summary>
    /// PROPVARIANT 的 vt 取值：VT_LPWSTR(31)，表示联合体里是一个 UTF-16 字符串指针。
    /// </summary>
    private const ushort VariantTypeUtf16String = 31;

    private const string ChromiumWindowClass = "Chrome_WidgetWin_1";

    private const string InterfaceIdPropertyStoreString = "886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99";

    private static readonly Guid InterfaceIdPropertyStore = new(InterfaceIdPropertyStoreString);

    private static readonly Guid FormatIdAppUserModelId = new("9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3");

    private const uint PropertyIdAppUserModelId = 5;

    [StructLayout(LayoutKind.Sequential)]
    private struct PropertyKey
    {
        public Guid FormatId;

        public uint PropertyId;
    }

    /// <summary>
    /// PROPVARIANT 在 x64 上是 24 字节：vt 在偏移 0，联合体在偏移 8。
    /// </summary>
    [StructLayout(LayoutKind.Explicit)]
    private struct PropVariant
    {
        [FieldOffset(0)]
        public ushort VariantType;

        [FieldOffset(8)]
        public nint PointerValue;

        [FieldOffset(16)]
        public nint Reserved;
    }

    [ComImport]
    [Guid(InterfaceIdPropertyStoreString)]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IPropertyStore
    {
        void GetCount(out uint count);

        void GetAt(uint propertyIndex, out PropertyKey key);

        void GetValue(ref PropertyKey key, out PropVariant value);

        void SetValue(ref PropertyKey key, ref PropVariant value);

        void Commit();
    }

    private delegate bool EnumWindowsProc(nint windowHandle, nint lParam);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool EnumWindows(EnumWindowsProc callback, nint lParam);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool IsWindowVisible(nint windowHandle);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetClassName(nint windowHandle, StringBuilder className, int maxCount);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool PostMessage(nint windowHandle, uint message, nint wParam, nint lParam);

    [DllImport("shell32.dll")]
    private static extern int SHGetPropertyStoreForWindow(
        nint                windowHandle,
        ref Guid            interfaceId,
        out IPropertyStore? propertyStore);

    /// <summary>
    /// 读取窗口的 AUMID；窗口没有设置 AUMID 时返回 null。
    /// </summary>
    private static string? TryGetWindowAppUserModelId(nint windowHandle)
    {
        var             interfaceId   = InterfaceIdPropertyStore;
        IPropertyStore? propertyStore = null;

        try
        {
            if (SHGetPropertyStoreForWindow(windowHandle, ref interfaceId, out propertyStore) != 0 ||
                propertyStore is null)
            {
                return null;
            }

            var key = new PropertyKey
            {
                FormatId   = FormatIdAppUserModelId,
                PropertyId = PropertyIdAppUserModelId
            };
            propertyStore.GetValue(ref key, out var value);

            // 注意：GetValue 对存在的属性会返回 S_FALSE(0x1) 而不是 S_OK，
            // 所以只能靠 VariantType 判断是否拿到了字符串，不能检查 HRESULT。
            if (value.VariantType != VariantTypeUtf16String || value.PointerValue == 0)
            {
                return null;
            }

            var aumid = Marshal.PtrToStringUni(value.PointerValue);
            Marshal.FreeCoTaskMem(value.PointerValue);
            return aumid;
        }
        catch (COMException)
        {
            return null;
        }
        finally
        {
            if (propertyStore is not null)
            {
                Marshal.ReleaseComObject(propertyStore);
            }
        }
    }

    /// <summary>
    /// 关闭属于指定 AUMID 的所有可见窗口，返回关闭的窗口数。
    /// </summary>
    public static int CloseWindows(string aumid)
    {
        if (string.IsNullOrWhiteSpace(aumid))
        {
            return 0;
        }

        var windowHandles   = new List<nint>();
        var classNameBuffer = new StringBuilder(64);

        _ = EnumWindows(
            (windowHandle, _) =>
            {
                if (!IsWindowVisible(windowHandle))
                {
                    return true;
                }

                classNameBuffer.Clear();
                if (GetClassName(windowHandle, classNameBuffer, classNameBuffer.Capacity) <= 0)
                {
                    return true;
                }

                // 限定 Chromium 窗口类，否则会误伤普通 Edge 窗口
                if (classNameBuffer.ToString() != ChromiumWindowClass)
                {
                    return true;
                }

                if (string.Equals(TryGetWindowAppUserModelId(windowHandle), aumid, StringComparison.Ordinal))
                {
                    windowHandles.Add(windowHandle);
                }

                return true;
            },
            0
        );

        foreach (var windowHandle in windowHandles)
        {
            _ = PostMessage(windowHandle, WindowMessageClose, 0, 0);
        }

        return windowHandles.Count;
    }
}
