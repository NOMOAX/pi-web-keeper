using System.IO;
using System.Net;
using System.Text.Json;

namespace PiWebKeeper;

public static class KeeperConfigLoader
{
    private const string FileName = "settings.json";

    /// <summary>配置文件与 exe 同目录。</summary>
    public static string ConfigPath => Path.Combine(AppContext.BaseDirectory, FileName);

    private static readonly JsonSerializerOptions Options = new()
    {
        PropertyNameCaseInsensitive = true,
        ReadCommentHandling         = JsonCommentHandling.Skip,
        AllowTrailingCommas         = true
    };

    public static KeeperConfig Load()
    {
        if (!File.Exists(ConfigPath))
        {
            throw new FileNotFoundException($"找不到配置文件：{ConfigPath}", ConfigPath);
        }

        var json = File.ReadAllText(ConfigPath);
        var config = JsonSerializer.Deserialize<KeeperConfig>(json, Options) ??
                     throw new InvalidDataException("配置文件内容为空或格式不正确。");

        if (string.IsNullOrWhiteSpace(config.StartCommand))
        {
            throw new InvalidDataException("配置文件缺少 startCommand。");
        }

        /*
         * 用 <= MinPort 而不是 < MinPort：端口 0 表示“由系统分配”，
         * 这种端口无法用于探测就绪状态，所以同样视为无效。
         */
        if (config.Port is <= IPEndPoint.MinPort or > IPEndPoint.MaxPort)
        {
            throw new InvalidDataException($"port 取值无效：{config.Port}");
        }

        return config;
    }
}
