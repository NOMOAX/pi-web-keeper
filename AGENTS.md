# Pi Web Keeper

A WPF (.NET 10, `net10.0-windows`) launcher that owns exactly one local WebUI service: it starts the service, waits for its port, opens the installed PWA in its own window, and sits in the notification area while it runs. It also carries a button that updates Pi itself.

The shipped artifact is a single self-contained file: `dotnet publish PiWebKeeper/PiWebKeeper.csproj -c Release -r win-x64 --self-contained true -p:PublishSingleFile=true -p:IncludeNativeLibrariesForSelfExtract=true -p:EnableCompressionInSingleFile=true -o publish/win-x64`. `IncludeNativeLibrariesForSelfExtract` is the flag that matters — without it WPF's native DLLs stay loose beside a 131 MB exe instead of bundling into ~64 MB. Publishing into `publish/win-x64` fails with `MSB4018` while an exe from that directory is still running, so close the launcher before publishing.

That directory is tracked, not ignored: `publish/win-x64/PiWebKeeper.exe` and its `settings.json` are committed and only `*.pdb` is ignored, so every republish adds another ~62 MB blob to the history of this repo and its private GitHub remote `NOMOAX/pi-web-keeper`.

## The lifecycle contract

The launcher's lifetime is the service's lifetime. While the launcher runs the service runs, and the one thing that ends both is `退出` in the tray menu: it stops the service and then exits. The window is not the launcher — minimizing it and closing it both only put it in the tray, and the service keeps running either way. Starting the exe means starting the service, with no window on the way (see [Launching](#launching)). Every change must hold this true.

A second launcher in the same install directory is refused: `SingleInstanceGuard` holds a mutex named from the install directory's hash, so a second launch signals the existing window to the foreground and exits without starting anything. The key is per-directory on purpose — two install directories are two services and must coexist. Note the window is created by `App.OnStartup`, not by `StartupUri`, because the guard has to run first. The name also carries a `PiWebKeeper` prefix, which is part of a running instance's identity: an old exe and a new one in one directory do not see each other, so close the old window before starting the new one.

Four mechanisms carry the contract, and each is fragile in a way that is not obvious from its signature. Each has a comment at the site explaining why — read it before changing the code, and keep it accurate after.

- `ServiceKeeper.StartAsync` — it stops whatever is running before it starts, so a stale process or a live port never stacks up into two services. The one service button is a toggle over it and `StopAsync`: `启动` while the port is free, `停止` while it is not. Restarting is stopping and starting, one click each. A start that succeeds also hides the launcher into the tray — what the user wants to see next is the app window, not the log; at launch there is nothing to hide, because the window was never shown — while a start that fails leaves the window up so the error stays in view, and a stop never minimizes, because that click is followed by another.
- `ServiceKeeper.StopAsync` — a killed process is not a released port; stop is only done once the port comes back.
- `ServiceKeeper.WaitUntilReadyAsync` — it ends either when the port opens or when the service process exits, whichever comes first. A process that is already gone will never open that port, and making the user watch the whole `WaitSeconds` before the log admits it died is the worse answer. The wait that loses the race is left running: it is bounded (by `WaitSeconds`, or by the process finally exiting) and never faults, because nothing there is cancelled.
- `ServiceKeeper.KillProcessTree` — the service is a `cmd → pwsh → node → node` chain, so a kill must reach the whole tree.
- `AppWindowCloser` — a web app window is identified by its own AUMID, because Edge co-locates several of them inside one process.

## Conventions

- Comments and XML docs are Chinese, matching the rest of the repo. XML docs carry the *why*; keep that split when you add any.
- Members of a type carry XML doc comments; `//` is for what happens inside a method body. A `<summary>` always spans three lines — tag, text, tag — while `<returns>` and `<remarks>` may sit on one.
- `settings.json` describes one service and is copied next to the exe. Hosting a second service means a second install directory, not a second entry here. The display name is not in it: it is the window `Title` in `MainWindow.xaml`.
- Await with `ConfigureAwait(false)` outside the UI thread; `MainWindow` is the only place that resumes on the dispatcher.
- Commit messages are Conventional Commits — an English subject (`docs:`, `feat(ui):`, `refactor(config):`, `style(ui):`, `chore:`) — followed by a body of prose saying why the change is wanted, not what it does.

## Launching

Starting the exe is starting the service: no window, no clicking `启动`, straight down the same path the button uses.

- `MainWindow.StartOnLaunch` runs `StartServiceAsync` and shows the window only when it says the start failed. Double-clicking a file has one obvious meaning, and "start the service" is it; an error nobody can see is not an error, so the failure case is the one that needs the window.
- Both callers share one start path and use its answer differently: the button ignores the result (the window is on screen either way), the launch flow acts on it. That is why `StartServiceAsync` returns `bool`.
- Nothing is shown on the success path, so `Loaded` never fires — `LoadKeeper` therefore moved out of that event and into the constructor.
- `SourceInitialized` comes from `EnsureHandle()` in that same constructor. The tray icon hangs off the window's handle (see [The tray icon](#the-tray-icon)), and without creating the handle up front there would be no tray icon until the first `Show()` — that is, no way into a program that looks like it never started.
- Creating the handle early also moves `WindowStartupLocation="CenterScreen"` from the first `Show()` to the handle's creation. Measured: still the centre of the work area of the monitor the cursor is on — and at that moment the cursor is still where the user double-clicked, so the documented centring behaviour is unchanged.

Both buttons — the service button (`启动`/`停止`) and the update button — sit on one row inside a `WrapPanel`, which is a fallback rather than the layout strategy: the window is pinned to 640x480 DIP (`Width`/`MinWidth` and `Height`/`MinHeight` in `MainWindow.xaml`). The row is far narrower than 640, so wrapping never happens; the 480 is what the log panel gets below the row. The width is a literal because the window used to be measured from the row, and once the row shrank to two buttons that measurement came out too narrow to read the log against. Both buttons are a fixed 80x40 DIP (`Width`/`Height`; no `MinWidth`, no `Padding`), a size the user chose by eye from screenshots — the literal is the decision, not a value to re-derive.

The log panel is the bare read-only `LogBox`: no container, no header, its own TextBox border being the edge of the panel, because an outer `GroupBox` only wrapped another ring of space around it. Its font is hardcoded `NSimSun` at `FontSize="12"` with `TextOptions.TextFormattingMode="Display"` rather than taken from the system theme: WPF draws outlines only and so ignores SimSun's embedded small-size bitmaps that GDI consumes, the user judged everything below 12 DIP blurry for this mixed Chinese and monospace text, and `FontSize` is DIP, so a smaller number does not help at high display scaling either.

`WindowStartupLocation` is `CenterScreen`, which is correct here precisely because nothing resizes the window after it is created. Measured on this machine: centered on the work area to within 0 DIP.

The maximize box is removed by clearing `WS_MAXIMIZEBOX` in `SourceInitialized`, then `SetWindowPos(..., SWP_FRAMECHANGED)` so the title bar is repainted. WPF has no "disable maximize but keep resizing" setting — the three `ResizeMode` values keep maximize or drop the resize border too — so this goes to the window style directly. `WS_THICKFRAME` and `WS_MINIMIZEBOX` stay.

## The tray icon

`TrayIcon` is a hand-written `Shell_NotifyIcon` wrapper. WinForms' `NotifyIcon` does the same job in a fraction of the code, and the first version of this feature used it — but switching on `UseWindowsForms` took the single-file executable from 65 MB to 75 MB, which is a lot of megabytes for one icon. The interop stays in that one file so the rest of the app remains WPF-only.

- The icon is the executable's own Win32 icon resource (id 32512, where `ApplicationIcon` puts it), loaded at the size the notification area asks for. There is no second copy of the `.ico` inside the assembly. If that resource cannot be loaded the icon falls back to the system default and says so in the log: a plain icon beats a window nobody can get back.
- Clicks arrive as a message on the main window (`WM_APP + 1`, see `TrayMessage`), so the icon is attached to a window handle in `SourceInitialized` (`TrayIcon.Attach`). Only the button-*up* messages are accepted: the shell sends down and up, and taking both would turn one click into two, which for a toggle reads as "nothing happened".
- Explorer restarting is routine and takes every tray icon with it, so the `TaskbarCreated` broadcast re-adds ours.
- The tooltip is the window's `Title`, so the display name is still written once.
- Left click toggles the window between screen and tray through `MainWindow.ShowFromTray`, which the single-instance wake-up signal also uses.
- Minimizing and closing the window both mean "into the tray": `Closing` is cancelled, and `StateChanged` catches the minimize however it was asked for — the button, the system menu, and the automatic hide after a successful start.
- Right click opens a WPF `ContextMenu`, so the menu follows the Fluent dark theme like the rest of the app. Its two action items call the very same handlers as the window's buttons and take their text and enablement from `RefreshActionState`; the one item that is not a button is `退出`, and it is greyed out while an update runs (see below).
- Whether the menu stays open after a click is a per-item decision, made by what that click is likely to be followed by: `更新` keeps it open (the update's state is shown right there), `停止` keeps it open (restarting is stop-then-start), while `启动` and `退出` close it. That is why the service item's flag is set in `RefreshActionState` — it is the item that changes meaning — instead of once at construction.
- The menu's own window is pushed to the foreground in `Opened`. That call is load-bearing rather than decoration: measured on this machine, without it clicking anywhere else leaves the menu sitting on screen.

## The update button

The button right of the service button is not part of the service contract and is not configurable — its commands are hardcoded in `MainWindow.RunUpdateAsync`. It updates Pi itself, not the service `settings.json` describes.
- One button runs both packages' own updaters, `pi update --all` then `pi-web update`, stopping at the first non-zero exit: what the user wants is Pi up to date, not a choice between two packages.
- An update never touches the service. It does not stop it to free the files npm is about to replace, and it does not restart anything afterwards.
- The update button is disabled while the port is listening. The running service is exactly what holds those files (npm reports `EBUSY rename` against them), so an update is only offered when the launcher does not have the service up. The tooltip states the reason. That is one layer, not the only one: `pi-web update` refuses on its own when it finds a running pi-web server in its run records. The two layers disagree about the evidence — here it is the port, there it is a record file — which is the point: each catches what the other misses.
- An update doubles as the version check: each updater reports for itself that it was already current.
- A running update also greys out `退出` in the tray menu, which is the only way out of the program: the updater cannot be cancelled, so the alternative would be quitting on top of a half-replaced npm package.
- `退出` stops the service and then closes the window, and it does nothing else: no waiting for, and no cancelling of, a start still in flight. The user is expected to let that finish, so the path needs no machinery at exit time.

## Verifying a change

There is no test project, and there cannot usefully be one for the process, port, and window behaviour — a real machine is the oracle. For pure visual changes, build (`dotnet build PiWebKeeper.slnx`) and republish, and the user runs `publish/win-x64/PiWebKeeper.exe` himself; do not build a UI-automation harness to check cosmetics. For anything touching the lifecycle contract or the update buttons, you must drive the real app and watch its log: service starts, port becomes ready, window opens, and `退出` in the tray menu leaves no listener on the port.

Three techniques that work here:

- Drive the real window with UI Automation: find the window by PID, `InvokePattern.Invoke()` a button by name, then read the log panel back through `ValuePattern`. This catches wiring bugs that reading the code does not.
- A window that has never been shown is invisible to both `Process.MainWindowHandle` (0) and the UIA tree, so its absence has to be probed with `EnumWindows`, and its presence there is the Win32 class name `HwndWrapper[PiWebKeeper;;{guid}]` — "Window" is what UIA reports, not what the OS knows. The process also owns several hidden helper windows of that same class, so match on the title as well. `IsWindowVisible` on that handle is the only way to tell "hidden" from "not created yet".
- Tray icons are not where one expects them: Windows 11 puts a new one in the hidden-icon overflow, so a probe has to click the `显示隐藏的图标` chevron first and then look inside the `TopLevelWindowForOverflowXamlIsland` window. The right-click menu is a top-level window of class `Popup` that belongs to the process — match on `ProcessId`, because searching the whole desktop for menu items also turns up the 系统 menu on every window's title bar. Greyed-out items report themselves through `IsEnabled`, so menu state can be asserted without reading pixels. Two installed copies put two identically named icons in the tray, and a probe cannot tell them apart — close the other instance first.
- Point `settings.json` at a throwaway command (`node` listening on a spare port) and copy the published exe somewhere disposable. That exercises start, readiness, and stop without touching the user's real service on port 30141. The update commands are hardcoded, so that path is real either way.

Set `"AppUserModelId": ""` in a test config to skip opening the PWA window.

The user's live install is `~/Tools/Pi Web Keeper`, outside the repo. Replacing the exe there needs the launcher closed — a running exe is locked — and its `settings.json` is the user's own and must be left alone.
