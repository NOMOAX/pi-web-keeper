> [!WARNING]
> **此项目只支持 Windows 系统。**

# Pi Web Keeper

Pi Web Keeper 是一个负责托管 **一个**本地 Pi Web 服务的 Windows 桌面启动器。

| 项目     | 内容                                         |
|----------|----------------------------------------------|
| 语言     | C#                                           |
| 运行时   | .NET 10（`net10.0-windows`）                 |
| 界面框架 | WPF                                          |
| 发布形式 | 单文件、自包含，目标机器无需安装 .NET 运行时 |

它做的事情可以概括为三件：

- 启动服务，等待服务端口开始监听，然后把已经安装好的 PWA 打开在它自己的窗口里；
- 关闭启动器窗口时停掉服务。窗口存在，服务就在运行，两者生命周期相同；
- 另外提供一个更新按钮，用来把 Pi 本身更新到最新版本，依次执行 `pi update --all` 与 `pi-web update`。

仓库中的 `publish/win-x64/PiWebKeeper.exe` 就是一份可以直接运行的程序。

## 快速开始

开始之前只需要准备两样东西：一台 Windows 电脑，以及已经安装好的 Node.js（要求 `npm` 可以在 `PATH` 中找到）。下面的命令都在 PowerShell 中执行。

1. 安装 `pi-web`

    ```powershell
    npm install -g @agegr/pi-web
    ```

2. 安装 `pi`

    - 这一步是可选的，但是，如果你想要正常使用界面上的 `更新` 按钮，就必须安装它。

    ```powershell
    npm install -g --ignore-scripts @earendil-works/pi-coding-agent
    ```

3. 手动启动服务

    ```powershell
    pi-web
    ```

    - 启动一次，确认服务可以正常运行。
    - 这条命令会启动服务，并用默认浏览器打开 Pi Web 页面。下一步需要在 Edge 中把页面安装为应用，所以默认浏览器最好是 Edge。
    - 控制台中出现 `Local: http://127.0.0.1:30141` （`pi-web` 的默认端口号是 `30141`）就说明服务正常。此时 Edge 中的页面已经打开，不再需要这个终端，直接关闭终端窗口即可，已经加载好的页面不会因为服务停止而立刻发生变化。保留它同样没有问题，因为启动器每次启动服务之前，都会先停掉已经在运行的服务。

4. 在 Edge 中安装为应用

    - （从刚才打开的 Edge 中的 Pi Web 网页窗口继续这一步）点击 Edge 地址栏内部右侧的“安装为应用”按钮，并确认。
    - 安装完成之后，开始菜单中会多出一项，名称通常是 `Pi Web`。启动器要打开和关闭的，正是这一个应用窗口。
    - 现在可以把这个网页窗口关了。

5. 查询 AppUserModelId

    ```powershell
    Get-StartApps | Where-Object Name -like '*Pi Web*'
    ```

    ```text
    Name     AppID
    ----     -----
    Pi Web   127.0.0.1-34C534B4_x57ssgwjdvp94!App
    ```

    - （输出类似上面的内容）`AppID` 这一列就是这个应用的 AppUserModelId，请把这一列显示的内容完整记录下来，末尾的 `!App` 也包括在内。如果查询不到，或者名称对不上，请参考[下一节](#如何获取-appusermodelid)中给出的另一种查询方法。

6. 准备安装目录

    - 新建一个目录（放在哪里都可以），把下面两个文件粘贴进去：
    - `PiWebKeeper.exe`：可以使用仓库中 `publish/win-x64/PiWebKeeper.exe` 这一份，也可以自行构建；
    - `settings.json`：可以从仓库中复制一份，然后把 `AppUserModelId` 换成上一步查到的值；`StartCommand` 保持默认的 `pi-web --no-open` 即可，不需要修改。
    - 一个目录对应一个被托管的服务。如果要托管第二个服务，请另外新建一个目录。

7. 启动

    - 运行 `PiWebKeeper.exe`，然后点击界面上的 `启动` 按钮。
    - 服务启动后端口开始监听，PWA 窗口随之打开，启动器自己会最小化到任务栏。再次从任务栏打开启动器窗口时，按钮已经变成 `停止`。

## settings.json

```json
{
  "Host": "127.0.0.1",
  "Port": 30141,
  "AppUserModelId": "127.0.0.1-34C534B4_x57ssgwjdvp94!App",
  "WaitSeconds": 5,
  "ReadyDelaySeconds": 0,
  "StartCommand": "pi-web --no-open"
}
```

| 字段                | 说明                                                                |
|---------------------|---------------------------------------------------------------------|
| `Host` / `Port`     | 服务监听的地址与端口。启动器通过探测这个端口来判断服务是否已经就绪  |
| `AppUserModelId`    | 需要打开和关闭的 PWA 的标识，取值方法见下一节；留空时不打开应用窗口 |
| `WaitSeconds`       | 等待端口就绪的最长时间，单位为秒                                    |
| `ReadyDelaySeconds` | 端口就绪之后额外等待的时间，单位为秒                                |
| `StartCommand`      | 启动服务所使用的命令，由 `cmd.exe /c` 执行                          |

配置文件必须与可执行文件放在同一个目录中，文件名固定为 `settings.json`。

## 如何获取 AppUserModelId

**前提条件**：已经完成[快速开始](#快速开始)中的第 3 步和第 4 步。

安装完成之后，用 PowerShell 中的一条命令就能查到它（`Get-StartApps` 是 Windows 自带的命令）：

```powershell
Get-StartApps | Where-Object Name -like '*Pi Web*'
```

```text
Name     AppID
----     -----
Pi Web   127.0.0.1-34C534B4_x57ssgwjdvp94!App
```

**`AppID` 这一列就是 `settings.json` 中的 `AppUserModelId`**，请完整复制它所显示的内容，末尾的 `!App` 也要包括在内。

如果 `Get-StartApps` 的输出无法判断是哪一条（例如存在重名，或者你的 Windows 上没有这个命令），可以改用下面的方法。它列出开始菜单中的全部应用，PWA 同样在其中：

```powershell
$shell = New-Object -ComObject Shell.Application
$shell.NameSpace('shell:AppsFolder').Items() |
    Select-Object Name, Path |
    Where-Object Name -like '*Pi Web*'
```

这条命令输出的 `Path` 列就是 AppID，与上面 `AppID` 列的值相同。

以下几件事情值得了解：

- AppUserModelId 由 Edge 在安装 PWA 时生成，格式为 `<站点标识>-<哈希>!App`。从 `localhost` 或 `127.0.0.1` 安装的应用显示为 `127.0.0.1-xxxx!App`，从公网站点安装的应用显示为 `https://example.com-xxxx!App`，此外还有 `Microsoft.WindowsCalculator_8wekyb3d8bbwe!App` 这种来自应用商店的写法。
- **卸载并重新安装 PWA 之后，其中的哈希会发生变化**，原来的 AppUserModelId 随即失效，需要重新查询并填写。
- 把 `AppUserModelId` 留空（`""`）时，启动器只启动和停止服务，既不打开应用窗口，也不关闭应用窗口，适合纯命令行的使用场景。
- 启动器与 Windows 的交互只有两处：打开应用时执行 `explorer.exe shell:AppsFolder\<AppUserModelId>`，关闭应用时按 AppUserModelId 查找窗口。之所以按 AppUserModelId 查找而不按进程查找，是因为 Edge 会把多个 PWA 的窗口放在同一个 `msedge.exe` 进程里。

## 从源码构建

```powershell
dotnet publish PiWebKeeper/PiWebKeeper.csproj -c Release -r win-x64 --self-contained true -p:PublishSingleFile=true -p:IncludeNativeLibrariesForSelfExtract=true -p:EnableCompressionInSingleFile=true -o publish/win-x64
```

`IncludeNativeLibrariesForSelfExtract` 是其中最关键的一个参数：缺少它，WPF 需要的原生 DLL 会散落在可执行文件旁边，整个目录约为 131 MB；加上它之后，得到的才是单个约 62 MB 的可执行文件。

`publish/win-x64/` 目录中只跟踪可执行文件与 `settings.json`，`.pdb` 调试符号文件由该目录下的 `.gitignore` 排除。
