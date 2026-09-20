using System.ComponentModel;
using System.Diagnostics;
using System.Net.NetworkInformation;

namespace PiWebKeeper;

/// <summary>
/// 端口探测：判断服务是否就绪、是否真的已经停止。
/// </summary>
public static class PortProbe
{
    public static bool IsListening(int port)
    {
        try
        {
            foreach (var endpoint in IPGlobalProperties.GetIPGlobalProperties().GetActiveTcpListeners())
            {
                if (endpoint.Port == port)
                {
                    return true;
                }
            }
        }
        catch (NetworkInformationException)
        {
            // 查询失败时按"未监听"处理，让调用方走等待/重试
        }

        return false;
    }

    /// <summary>
    /// 等待端口进入指定状态。关掉服务后端口回收可能有延迟（实测可达数秒），
    /// 所以停止流程必须用带超时的轮询，而不能杀完就认为已停止。
    /// </summary>
    public static async Task<bool> WaitForStateAsync(
        int               port,
        bool              shouldBeListening,
        TimeSpan          timeout,
        CancellationToken cancellationToken)
    {
        var deadline = DateTime.UtcNow + timeout;
        while (DateTime.UtcNow < deadline)
        {
            cancellationToken.ThrowIfCancellationRequested();
            if (IsListening(port) == shouldBeListening)
            {
                return true;
            }

            await Task.Delay(100, cancellationToken).ConfigureAwait(false);
        }

        return IsListening(port) == shouldBeListening;
    }

    /// <summary>
    /// 找出占用指定端口的进程 ID。用于树杀失败后的兜底清理。
    /// </summary>
    public static List<int> FindOwningProcessIds(int port)
    {
        var processIds = new List<int>();

        try
        {
            var psi = new ProcessStartInfo("netstat.exe", "-ano -p TCP")
            {
                UseShellExecute        = false,
                CreateNoWindow         = true,
                RedirectStandardOutput = true
            };

            using var process = Process.Start(psi);
            if (process is null)
            {
                return processIds;
            }

            var output = process.StandardOutput.ReadToEnd();
            process.WaitForExit(5000);

            // 只认 LISTENING 行，避免把处于 TIME_WAIT / 已建立连接的进程一起算进来
            var suffix = ":" + port;
            foreach (var rawLine in output.Split('\n'))
            {
                var line = rawLine.Trim();
                if (!line.StartsWith("TCP", StringComparison.OrdinalIgnoreCase))
                {
                    continue;
                }

                if (!line.Contains("LISTENING", StringComparison.OrdinalIgnoreCase))
                {
                    continue;
                }

                var columns = line.Split(' ', StringSplitOptions.RemoveEmptyEntries);
                if (columns.Length < 5)
                {
                    continue;
                }

                if (!columns[1].EndsWith(suffix, StringComparison.Ordinal))
                {
                    continue;
                }

                if (int.TryParse(columns[4], out var processId) && processId > 0 && !processIds.Contains(processId))
                {
                    processIds.Add(processId);
                }
            }
        }
        catch (Exception ex) when (ex is Win32Exception or InvalidOperationException)
        {
            // 拿不到占用者时返回空列表，由调用方决定如何处理
        }

        return processIds;
    }
}
