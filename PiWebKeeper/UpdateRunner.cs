using System.Diagnostics;
using System.IO;
using System.Text;

namespace PiWebKeeper;

/// <summary>
/// 运行一条外部命令，把输出逐行带回，并给出退出码。
///
/// 与 <see cref="ServiceKeeper"/> 的区别是生命周期：这里跑的命令是一次性的，
/// 结束即结束，不持有进程、不占用端口。
/// </summary>
public static class UpdateRunner
{
    /// <summary>
    /// 运行一条命令，输出逐行回调。
    ///
    /// 一律通过 cmd.exe 执行：npm / pi 都是 .cmd shim，直接 CreateProcess 找不到它们。
    /// stdin 主动关闭：命令若企图等待输入，会立刻读到 EOF 而不是把窗口挂死。
    /// </summary>
    /// <returns>进程退出码。</returns>
    public static async Task<int> RunStreamingAsync(
        string         command,
        Action<string> onOutput)
    {
        using var process = CreateProcess(command);

        process.OutputDataReceived += (_, e) =>
        {
            if (e.Data is not null)
            {
                onOutput(e.Data);
            }
        };
        process.ErrorDataReceived += (_, e) =>
        {
            if (e.Data is not null)
            {
                onOutput("[stderr] " + e.Data);
            }
        };

        process.Start();
        process.BeginOutputReadLine();
        process.BeginErrorReadLine();
        process.StandardInput.Close();

        await process.WaitForExitAsync().ConfigureAwait(false);

        return process.ExitCode;
    }

    private static Process CreateProcess(string command)
    {
        var startInfo = new ProcessStartInfo("cmd.exe", $"/c {command}")
        {
            UseShellExecute        = false,
            CreateNoWindow         = true,
            RedirectStandardOutput = true,
            RedirectStandardError  = true,
            RedirectStandardInput  = true,
            StandardOutputEncoding = Encoding.UTF8,
            StandardErrorEncoding  = Encoding.UTF8,
            WorkingDirectory       = ResolveWorkingDirectory()
        };

        return new Process
        {
            StartInfo           = startInfo,
            EnableRaisingEvents = true
        };
    }

    /// <summary>
    /// 命令的工作目录，与启动服务保持一致：用户主目录。
    /// </summary>
    private static string ResolveWorkingDirectory()
    {
        var profileDirectory = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);

        return Directory.Exists(profileDirectory) ? profileDirectory : AppContext.BaseDirectory;
    }
}
