# Pi Web Keeper

A WPF (.NET 10, `net10.0-windows`) launcher that owns exactly one local WebUI service: it starts the service, waits for its port, opens the installed PWA in its own window, and takes everything down when the launcher window closes. It also carries a button that updates Pi itself.

The shipped artifact is a single self-contained file: `dotnet publish PiWebKeeper/PiWebKeeper.csproj -c Release -r win-x64 --self-contained true -p:PublishSingleFile=true -p:IncludeNativeLibrariesForSelfExtract=true -p:EnableCompressionInSingleFile=true -o publish/win-x64`. `IncludeNativeLibrariesForSelfExtract` is the flag that matters — without it WPF's native DLLs stay loose beside a 131 MB exe instead of bundling into ~64 MB. Publishing into `publish/win-x64` fails with `MSB4018` while an exe from that directory is still running, so close the launcher before publishing.

That directory is tracked, not ignored: `publish/win-x64/PiWebKeeper.exe` and its `settings.json` are committed and only `*.pdb` is ignored, so every republish adds another ~62 MB blob to the history of this repo and its private GitHub remote `NOMOAX/pi-web-keeper`.

## The lifecycle contract

The launcher's lifetime is the service's lifetime. While the window is open the service runs; closing the window stops it. Every change must hold this true.

A second launcher in the same install directory is refused: `SingleInstanceGuard` holds a mutex named from the install directory's hash, so a second launch signals the existing window to the foreground and exits without starting anything. The key is per-directory on purpose — two install directories are two services and must coexist. Note the window is created by `App.OnStartup`, not by `StartupUri`, because the guard has to run first. The name also carries a `PiWebKeeper` prefix, which is part of a running instance's identity: an old exe and a new one in one directory do not see each other, so close the old window before starting the new one.

Four mechanisms carry the contract, and each is fragile in a way that is not obvious from its signature. Each has a comment at the site explaining why — read it before changing the code, and keep it accurate after.

- `ServiceKeeper.StartAsync` — it stops whatever is running before it starts, so a stale process or a live port never stacks up into two services. The one service button is a toggle over it and `StopAsync`: `启动` while the port is free, `停止` while it is not. Restarting is stopping and starting, one click each. A start that succeeds also minimizes the launcher — what the user wants to see next is the app window, not the log — while a start that fails leaves the window up so the error stays in view, and a stop never minimizes, because that click is followed by another.
- `ServiceKeeper.StopAsync` — a killed process is not a released port; stop is only done once the port comes back.
- `ServiceKeeper.KillProcessTree` — the service is a `cmd → pwsh → node → node` chain, so a kill must reach the whole tree.
- `AppWindowCloser` — a web app window is identified by its own AUMID, because Edge co-locates several of them inside one process.

## Conventions

- Comments and XML docs are Chinese, matching the rest of the repo. XML docs carry the *why*; keep that split when you add any.
- `settings.json` describes one service and is copied next to the exe. Hosting a second service means a second install directory, not a second entry here. The display name is not in it: it is the window `Title` in `MainWindow.xaml`.
- Await with `ConfigureAwait(false)` outside the UI thread; `MainWindow` is the only place that resumes on the dispatcher.
- Commit messages are Conventional Commits — an English subject (`docs:`, `feat(ui):`, `refactor(config):`, `style(ui):`, `chore:`) — followed by a body of prose saying why the change is wanted, not what it does.

## The window

Both buttons — the service button (`启动`/`停止`) and the update button — sit on one row inside a `WrapPanel`, which is a fallback rather than the layout strategy: the window is pinned to 640x480 DIP (`Width`/`MinWidth` and `Height`/`MinHeight` in `MainWindow.xaml`). The row is far narrower than 640, so wrapping never happens; the 480 is what the log panel gets below the row. The width is a literal because the window used to be measured from the row, and once the row shrank to two buttons that measurement came out too narrow to read the log against. Both buttons are a fixed 80x40 DIP (`Width`/`Height`; no `MinWidth`, no `Padding`), a size the user chose by eye from screenshots — the literal is the decision, not a value to re-derive.

The log panel is the bare read-only `LogBox`: no container, no header, its own TextBox border being the edge of the panel, because an outer `GroupBox` only wrapped another ring of space around it. Its font is hardcoded `NSimSun` at `FontSize="12"` with `TextOptions.TextFormattingMode="Display"` rather than taken from the system theme: WPF draws outlines only and so ignores SimSun's embedded small-size bitmaps that GDI consumes, the user judged everything below 12 DIP blurry for this mixed Chinese and monospace text, and `FontSize` is DIP, so a smaller number does not help at high display scaling either.

`WindowStartupLocation` is `CenterScreen`, which is correct here precisely because nothing resizes the window after it is created. Measured on this machine: centered on the work area to within 0 DIP.

The maximize box is removed by clearing `WS_MAXIMIZEBOX` in `SourceInitialized`, then `SetWindowPos(..., SWP_FRAMECHANGED)` so the title bar is repainted. WPF has no "disable maximize but keep resizing" setting — the three `ResizeMode` values keep maximize or drop the resize border too — so this goes to the window style directly. `WS_THICKFRAME` and `WS_MINIMIZEBOX` stay.

## The update button

The button right of the service button is not part of the service contract and is not configurable — its commands are hardcoded in `MainWindow.RunUpdateAsync`. It updates Pi itself, not the service `settings.json` describes.
- One button runs both packages' own updaters, `pi update --all` then `pi-web update`, stopping at the first non-zero exit: what the user wants is Pi up to date, not a choice between two packages.
- An update never touches the service. It does not stop it to free the files npm is about to replace, and it does not restart anything afterwards.
- The update button is disabled while the port is listening. The running service is exactly what holds those files (npm reports `EBUSY rename` against them), so an update is only offered when the launcher does not have the service up. The tooltip states the reason. That is one layer, not the only one: `pi-web update` refuses on its own when it finds a running pi-web server in its run records. The two layers disagree about the evidence — here it is the port, there it is a record file — which is the point: each catches what the other misses.
- An update doubles as the version check: each updater reports for itself that it was already current.
- Close handles the service it owns and nothing else: no waiting for, and no cancelling of, a start or an update still in flight. The user is expected to let those finish, so neither path needs machinery at close time — the update is not cancellable at all, since a half-replaced npm package is worse than a slow update.

## Verifying a change

There is no test project, and there cannot usefully be one for the process, port, and window behaviour — a real machine is the oracle. For pure visual changes, build (`dotnet build PiWebKeeper.slnx`) and republish, and the user runs `publish/win-x64/PiWebKeeper.exe` himself; do not build a UI-automation harness to check cosmetics. For anything touching the lifecycle contract or the update buttons, you must drive the real app and watch its log: service starts, port becomes ready, window opens, and closing the launcher leaves no listener on the port.

Two techniques that work here:

- Drive the real window with UI Automation: find the window by PID, `InvokePattern.Invoke()` a button by name, then read the log panel back through `ValuePattern`. This catches wiring bugs that reading the code does not.
- Point `settings.json` at a throwaway command (`node` listening on a spare port) and copy the published exe somewhere disposable. That exercises start, readiness, and stop without touching the user's real service on port 30141.

Set `"AppUserModelId": ""` in a test config to skip opening the PWA window.

The user's live install is `~/Tools/Pi Web Keeper`, outside the repo. Replacing the exe there needs the launcher closed — a running exe is locked — and its `settings.json` is the user's own and must be left alone.
