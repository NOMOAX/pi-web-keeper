# Pi Web Keeper

A Rust launcher for Windows — a GUI-subsystem exe (`windows_subsystem = "windows"`, so it owns no console) — that manages exactly one local WebUI service: it starts the service, waits for its port, opens the installed PWA in its own window, and sits in the notification area while it runs. A second button updates Pi itself.

`cargo build --release` produces `target/release/PiWebKeeper.exe`, one self-contained file: `.cargo/config.toml` links the CRT statically and `[profile.release]` uses `opt-level = "z"`, `lto`, `panic = "abort"` and `strip`. Measured 11.6 MB, of which 8.1 MB is the embedded font.

Users get the exe from the GitHub release for that version — a zip holding `PiWebKeeper.exe`, a sample `settings.json`, `THIRD-PARTY-NOTICES.md` and `assets/fonts/LICENSE.txt`. The last two are not documentation: the font is SIL OFL 1.1, which obliges whoever redistributes it to ship the licence with it, and embedding it in the exe is redistribution. A release is tagged `v`-prefixed, made by the release rather than by hand (`gh release create v0.1.0 <zip> --title v0.1.0 --notes-file <notes>`).

## The lifecycle contract

The launcher's lifetime is the service's lifetime, and the kernel enforces it: the service tree lives in a Job Object created with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` (`src/process_ext.rs`), so the service dies with the launcher — a launcher crash included. The only two things that end this on purpose are the leave actions: `退出` stops the service and then exits, `重启程序` hands the job to a successor and exits (see [Restarting](#restarting)). The window is not the launcher — minimizing it and closing it both only put it in the tray. Starting the exe means starting the service (see [Launching](#launching)). Every change must hold this true.

A second launcher in the same install directory is refused: `single_instance` holds a mutex named from the install directory's SHA-256 (first 8 bytes, uppercase hex), so a second launch signals the existing window and exits without starting anything. The key is per-directory on purpose — two install directories are two services and must coexist. The name format (`Local\PiWebKeeper.Instance.<key>` and `…Activate.<key>`) and the exact hash are fixed: change either and two builds of the launcher in one directory take two different locks, which is two launchers and two services.

Four mechanisms carry the contract, and each is fragile in a way its signature does not show. Each has a comment at the site saying why — read it before changing the code, and keep it accurate after.

- `ServiceKeeper::start` — it stops whatever is running first, so a stale process or a live port never stacks up into two services. The suspension is part of that: the child is created `CREATE_SUSPENDED`, assigned to the job, and only then resumed, because `cmd.exe → pwsh → node` can reach its grandchildren faster than we can call `AssignProcessToJobObject`.
- `ServiceKeeper::stop` — a killed process is not a released port; stop is only done once the port comes back, and falls back to `taskkill /T /F` on whatever still owns it.
- `ServiceKeeper::wait_until_ready` — it ends when the port opens *or* when the service process exits, whichever comes first. A process that is already gone will never open that port, and making the user watch the whole `WaitSeconds` before the log admits it died is the worse answer.
- `app_window::close_windows` — a web app window is identified by its own AUMID, because Edge co-locates several of them inside one process.

## Threads

Three threads, and the split is forced by egui rather than chosen: **eframe calls `App::update` only while the window is visible** (measured — a heartbeat froze the moment the window hid and resumed after a Win32 `ShowWindow`). Anything driven by the UI thread would stop watching the service the moment the launcher went to the tray, which is the state it spends most of its life in.

- The **controller thread** (`controller.rs`) owns `ServiceKeeper` and performs every blocking call: start, stop, update, restart, exit. It is synchronous on purpose; there is no async runtime.
- The **tray thread** (`tray.rs`) pumps its own Win32 messages, turns menu clicks into `Command` values, and shows/hides the window itself.
- The **UI thread** (`ui.rs`) only reads `Shared` and sends `Command`. It never blocks.

Show and hide go through `window.rs` (raw `ShowWindow`, with the `HWND` and `egui::Context` stashed at creation) rather than through an egui viewport command, because a hidden window never runs the frame that would have applied one. Quitting ends with `std::process::exit` after asking the tray thread to remove its icon, for the same reason: a hidden window never turns `WM_CLOSE` into anything.

Neither the tray click nor the second-instance wake-up is routed through the controller, and that is deliberate: the controller is *inside* a start or an update for tens of seconds at a time, blocked on a child process, and it does not read its command channel until that finishes. A `ToggleWindow` queued there is "clicking the tray icon does nothing" for exactly as long as the user most wants to look at the log. So both callers invoke `window::show`/`window::toggle` on their own threads — Win32 handles the cross-thread call, and the UI thread picks the frame up from there.

The refresh rates are what keeps the window and the menu in agreement without any event plumbing: the controller ticks every 300 ms and re-reads `listening`, the UI repaints every 100 ms, the tray syncs its menu every 20 ms.

## Conventions

- Comments and doc comments are Chinese, matching the rest of the repo. `///` carries the *why* for a type, a member or a function; `//` is for what happens inside a body.
- `settings.json` describes one service and sits next to the exe. Hosting a second service means a second install directory, not a second entry here. The display name is not in it: it is the window title in `ui.rs`, which the tray tooltip repeats.
- Config keys are PascalCase (`#[serde(rename_all = "PascalCase")]`), lowercase and camelCase accepted as aliases: the `settings.json` people already have is written that way and has to keep working unchanged. The file is read from the exe's directory only, with no cwd fallback — a dev run needs a copy beside `target/release/`, and without one it shows `配置加载失败` and manages nothing.
- External commands — the service, the updaters, `taskkill` — go through `cmd.exe` with `raw_arg("/c …")`. `Command::arg` applies its own per-argument escaping and the line reaches `cmd` mangled: the child exits 0 with no output and the port never opens. `CommandExt::creation_flags` is the other trap: it assigns the whole flag word rather than OR-ing into it, so a second call silently drops the first — `spawn_suspended` therefore sets `CREATE_SUSPENDED | CREATE_NO_WINDOW` in one call, and a `CREATE_NO_WINDOW` that gets overwritten shows up as a Windows Terminal tab.
- Commit messages are Conventional Commits — an English subject (`docs:`, `feat(ui):`, `refactor(config):`, `style(ui):`, `chore:`) — followed by prose saying why the change is wanted, not what it does.

## Launching

Starting the exe is starting the service, and the window is the progress display while that happens.

- `--no-autostart` is the one exception: window, no service. It is also what the jump list's `任务 → 以普通方式启动` runs.
- The jump list (`jump_list.rs`) is registered against the AppUserModelID the shell *derives from the exe path*. There is deliberately no `SetCurrentProcessExplicitAppUserModelID`: an explicit one does not match what a pinned Start shortcut resolves to (that shortcut carries no `System.AppUserModel.ID`), and the `任务` section then simply never appears. The trade is that the jump list follows the exe path, so moving the install directory means re-pinning the tile. Registration is best effort on every launch; failing it writes one log line and nothing else.
- The window is visible from the first frame until the start succeeds, then the controller hides it — that moment reads as "the app is about to open". A start that fails leaves the window up so the error stays in view, and `--no-autostart` never reaches the hide at all.
- That is the design rather than a leak: eframe shows the window after the first paint regardless of `visible(false)`, so the only thing `with_visible(false)` buys is the absence of a white rectangle before the first frame.
- `hide_after_it_appears` waits for the window to have appeared before hiding it (10 ms poll, 1.5 s bound). Hiding a window eframe has not shown yet is a no-op, and eframe then shows it and leaves it there — which really happened with a service that was ready in ~200 ms.
- Geometry: outer 640x480, which is `inner_size` 624x441 because the non-client frame measures 16x39 here. `centered: true`; the resize border and the minimize box stay, and only maximize is gone.
- The maximize box is cleared with `WS_MAXIMIZEBOX` plus `SetWindowPos(SWP_FRAMECHANGED)` in `App::new`, before the first paint: eframe's `with_maximize_button(false)` reaches winit but the frame gets drawn anyway, and clearing the bit afterwards only leaves it greyed. Windows 11 still paints the glyph greyed out rather than removing it; that is expected, not a bug to chase. Double-clicking the title bar cannot maximize either, since it goes through the same bit.

## The window

Four buttons — the service button (`启动`/`停止`), the update button, `重启程序`, `退出` — on one row, then the message box filling the rest. All four are a fixed 80x40 (`min_size`), a size the user chose by eye from screenshots: the literal is the decision, not a value to re-derive. `重启程序` fits that width because it is four characters; the originally asked-for `重新启动此程序` did not.

To the right of them, separated by an allocated spacer, is one 44x28 theme button that cycles 系统 → 深色 → 浅色. Its label *is* the current mode, so one button says both "this is now" and "click for the next"; three buttons with a selected highlight were tried first and said less. Smaller than the four, because it is a setting rather than an action.

Theme is two independent things, and the split is the point (see [The theme](#the-theme)).

The message box gets the leftover height, and its row count is computed from `available_height` measured *outside* the `ScrollArea` — inside, it reports the content height and the box collapses.

The message box is an egui `TextEdit` used read-only. `interactive(false)` is not the way: it also kills selection, and this box is meant to be selectable. Instead the visible string is a shadow copy rebuilt from the shared log whenever `log_revision` changes, so anything typed is overwritten on the next frame. The blinking caret is suppressed by drawing it zero-width (`text_cursor.stroke = Stroke::NONE`) with blinking off inside a scope around the widget: a caret would promise an editing that never happens, and a blinking one requests a repaint every half second for nothing. Its right-click menu holds one item, `清空`, padded 14x7 because a menu is a mouse target; there is no copy, cut or paste, and the padding is set inside that closure only, so the hand-sized window buttons cannot move with it. `清空` writes `----- Message cleared. -----` into the log itself rather than into the view, so the placeholder survives the next rebuild and an empty box cannot be confused with a box that was never written to.

The log is capped at 2000 lines. The cap keeps the per-frame layout cost constant, not the memory: the whole box is laid out every frame.

The font comes from `src/font.rs` and `assets/fonts/NotoSansSC-Regular.otf`: one embedded face, installed at the front of both the `Proportional` and `Monospace` families, with egui's own fonts behind it as a fallback for what Source Han Sans lacks. There is no runtime search and no candidate list — the font is part of the executable, so the same exe looks the same on every machine, and swapping it is a rebuild. `FontData::index` would pick a face inside a collection if one were ever embedded instead.

## The theme

Two independent ideas share the word "theme", and keeping them apart is the whole design.

- **Window content** follows the user's choice, cycled by the button right of `退出`: 系统, 深色, 浅色. Both palettes are egui's own (`Visuals::dark()`/`light()`) — there is no palette of this program's, which is what makes switching cheap and keeps the widgets looking like the framework's intent.
- **The title bar** — caption, minimize, maximize, close — follows the *Windows* mode and ignores that choice entirely. It is drawn by DWM, so `window::set_title_bar_dark` sets `DWMWA_USE_IMMERSIVE_DARK_MODE` (both attribute 20 and 19, since the id changed in 20H1 and querying the build number is not worth it). Without it Windows 11 defaults the title bar to the *app* mode, which would tie it to the wrong one of the two.

The registry carries both, side by side under `HKCU\...\Themes\Personalize`: `AppsUseLightTheme` is the app mode, `SystemUsesLightTheme` is the Windows mode, each 0 = dark. They are separate keys because Windows lets a user pick them independently, and this window is one of the places where that shows.

Neither is pushed at us: Windows announces the change to nobody, so the UI thread re-reads both once a second (`SYSTEM_THEME_POLL`) while it is running. A reading failure is treated as dark in both cases — the program has always looked that way, and the usual cause is a registry layout we do not recognise, where guessing either way is a guess.

The choice lives in memory only; every launch starts at 系统. A launcher this small has no preference worth a file, and starting from the system setting means the window is never a surprise on first run.

## The tray icon

The tray lives on its own thread (`tray.rs`) and pumps its own messages with `PeekMessageW` rather than blocking on `GetMessage`, because that thread also polls the command channels. `tray-icon`'s `TrayIcon` is kept alive to the end of the loop: its `Drop` is what removes the icon.

- The icon is the executable's own Win32 icon resource — id 1, where `build.rs` puts `assets/PiWebKeeper.ico` via `winresource`. There is no second copy of the `.ico` inside the binary. The artwork is the official Pi mark (`https://pi.dev/logo-auto.svg`) cropped to the mark's own 4x4 grid and drawn on it from 16 to 256 px, so every cell edge lands on a whole pixel and the small sizes stay crisp; the source SVG's surrounding whitespace is not part of the icon. If that resource cannot be loaded the icon falls back to a generated 32×32 square and the log says so: a plain icon beats a program nobody can get back.
- Only button-*up* messages are accepted. The shell sends down and up, and taking both turns one click into two, which for a toggle reads as "nothing happened".
- `.with_menu_on_left_click(false)` is load-bearing: the crate's default is true, so the attached menu pops on left click as well, and the symptom is exactly "left click opens the context menu, the window only comes out after clicking elsewhere".
- Left click toggles the window between screen and tray. The second-instance wake-up only *shows*: double-clicking the exe means "show me the window", not "close it", so the two paths are separate commands.
- Explorer restarting is routine and takes every tray icon with it; `tray-icon` re-adds ours on the `TaskbarCreated` broadcast itself, so nothing here has to handle it.
- The menu is a native Win32 popup (window class `#32768`), light-themed under a dark window even though the launcher is dark: system-drawn, not something to restyle. Items are `启动`/`停止`, `更新`, a separator, then `重启程序` and `退出`; the separator divides the two actions that belong to the service from the two that leave the program. Item *text* changes only with the service's own state — `启动`/`停止` and `正在启动` — never with the update, which only greys things out: an update runs for minutes, and a label that says `正在更新` for that long says nothing about what the service is doing while it looks like it is flickering.
- Text and enablement come from `sync_menu`, which reads the same `Shared` the window buttons read, so the menu and the buttons cannot disagree. Nothing keeps the popup open after a click — a native menu closes itself.
- A tray registration that fails is not fatal: the window stays usable and the log says so.

## Restarting

`重启程序` hands the launcher's job to a successor instead of ending it. It and `退出` go through one `leave`, and the two differ in exactly one thing: whether a successor is arranged first.

- The handover cannot happen inside this process. A new instance refuses to run while an old one holds the single-instance mutex — it signals the existing window and exits — so a successor started before this process exits is simply swallowed, and both end up gone.
- So this process starts a hidden copy of itself, `PiWebKeeper.exe --take-over <pid>` (`CREATE_NO_WINDOW`), and exits. That copy waits for the pid (50 ms poll, 30 s bound, and a pid that is already gone counts as waited), then starts a plain `PiWebKeeper.exe`.
- The helper is deliberately **outside** the service's Job Object. It is our child, but it must outlive us, and `KILL_ON_JOB_CLOSE` would take it with us.
- It is started before the service is stopped, so a helper that cannot start cancels the restart with nothing stopped: `重启失败：…` in the log, window up.
- Both leave actions stop the service before exiting. The successor's start flow would stop it anyway, but leaving it running would open a window with the service up and no launcher, and the contract says one launcher, not two. Cost: the downtime is paid once, on the old instance's side — the successor finds a free port.
- The successor is an ordinary instance in every respect: it takes the instance mutex, adds its own tray icon, starts the service, and hides its window when that succeeds.
- A helper that fails is silent by construction: it lives outside our log, in a process we are no longer around to write from. That was accepted when the handover was chosen — the worst case is double-clicking the exe again.

## The update button

The button right of the service button is not part of the service contract and is not configurable — its commands are hardcoded in `controller::run_update`. It updates Pi itself, not the service `settings.json` describes.

- One button runs both packages' own updaters, `pi update --all` then `pi-web update`, stopping at the first non-zero exit: what the user wants is Pi up to date, not a choice between two packages. `更新完成。` is appended only when both succeed — an abort already says what happened.
- An update never touches the service. It does not stop it to free the files npm is about to replace, and it does not restart anything afterwards.
- The update button is disabled while the port is listening, and `run_update` refuses again if it is reached anyway. The running service is exactly what holds those files (npm reports `EBUSY rename` against them), so an update is only offered when the launcher does not have the service up; the tooltip states the reason. That is one layer, not the only one: `pi-web update` refuses on its own when it finds a running pi-web server in its run records. The two layers disagree about the evidence — here it is the port, there it is a record file — which is the point: each catches what the other misses.
- An update doubles as the version check: each updater reports for itself that it was already current.
- A running update also greys out `重启程序` and `退出` — the two ways out of the program — on the window and in the tray menu alike: the updater cannot be cancelled, so the alternative would be quitting on top of a half-replaced npm package. Both take that one decision from the same shared flags, and both are greyed again while a leave is under way, so a second click cannot start a second handover.
- Neither leave action waits for, nor cancels, a start still in flight — the user is expected to let that finish, so the path needs no machinery at exit time.

## Verifying a change

There is no test project, and there cannot usefully be one for the process, port, and window behaviour — a real machine is the oracle. For pure visual changes, `cargo build --release` and let the user run `target/release/PiWebKeeper.exe` himself; do not build a UI-automation harness to check cosmetics. For anything touching the lifecycle contract or the update buttons, drive the real app and watch the log: service starts, port becomes ready, window opens, `退出` in the tray menu leaves no listener on the port, and `重启程序` leaves exactly one launcher with the port listening again.

What works here, and what does not:

- **UI Automation cannot read this window.** egui draws every pixel; there are no automation elements and no `ValuePattern` behind the message box. A screenshot of the window rect is the only way to see what it says. UIA is still how the *tray icon* is found — a button named `Pi Web Keeper`, inside the `TopLevelWindowForOverflowXamlIsland` flyout once the `显示隐藏的图标` chevron is open, or in the tray directly if the user dragged it there.
- Find the window with `EnumWindows` by PID **and** title `Pi Web Keeper`: the process owns hidden helper windows too. `IsWindowVisible` is the only way to tell "hidden" from "not created yet".
- Synthetic input needs care on this machine. `SetForegroundWindow` is refused under the foreground lock, so send `VK_MENU` down/up first, then `SetForegroundWindow` and `BringWindowToTop`. `mouse_event` uses `0x0002`/`0x0004` for left down/up and `0x0008`/`0x0010` for right; mixing them up looks exactly like a UI bug that is not there.
- The tray menu is a native popup that ignores synthetic clicks but honours the keyboard: right-click the icon, then `VK_DOWN` N times and `VK_RETURN`, arrows skipping disabled items and separators. An egui popup (the `清空` menu) is the opposite — drawn inside the window, so a window screenshot shows it and a click at its centre works.
- A hidden window does not repaint, so wake it before reading anything: a second instance does that, and `--no-autostart` gives a window that stays up with the service untouched.
- Point `settings.json` at a throwaway command (`node` listening on a spare port) and copy the release exe somewhere disposable under `%TEMP%`. That exercises start, readiness, and stop without touching the user's real service on port 30141. The update commands are hardcoded, so that path is real either way.
- Set `"AppUserModelId": ""` in a test config to skip opening the PWA window.
- Never kill `node.exe` broadly: the user runs the pi TUI as one, and killing it ends their session. Kill test pids by id.

The user's live install is `~/Tools/Pi Web Keeper`, outside the repo. Replacing the exe there needs the launcher closed — a running exe is locked — and its `settings.json` is the user's own and must be left alone.
