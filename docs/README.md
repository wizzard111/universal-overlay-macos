# Universal Overlay for macOS

A lightweight overlay on top of games (full-screen ones included): FPS, frame
time with a graph, 1% / 0.1% lows, graphs of any sensor, flexible styling and
size, and automatic updates from GitHub. Apple Silicon and Intel, macOS 13+.

**[Download the latest version](https://github.com/wizzard111/universal-overlay-macos/releases/latest)** —
`universal-overlay-macos.dmg`: open it and run Install.

Other systems: [Windows](https://github.com/wizzard111/universal-overlay-windows) · [Linux](https://github.com/wizzard111/universal-overlay-linux).

License: [MIT](../LICENSE). What's new: [CHANGELOG](CHANGELOG.md). Built together with Claude.

## Install, update, uninstall

Only three files are at the top; everything else is in folders:

| File | What it does |
|---|---|
| `install.command` | Installs. The Terminal window is the setup log. From the DMG a window opens where you drag the app onto "Applications" (or press Enter and it is copied for you). In a clone of the repository it builds the program first (needs [Rust](https://rustup.rs) and the Xcode Command Line Tools) |
| `update.command` | In a git clone: pulls the changes, rebuilds and updates. Otherwise downloads the latest GitHub release, verifies its SHA-256 and replaces the app. Your settings are kept |
| `uninstall.command` | Removes the app, autostart and the Screen Recording permission; asks about your settings |

On the DMG they are called Install and Uninstall. The administrator password is
asked only when /Applications is not writable without it. For FPS macOS asks for
the Screen Recording permission. You can also update from the app: General →
Updates.

> Without an Apple Developer ID certificate Gatekeeper stops the first launch
> once: System Settings → Privacy & Security → Open Anyway. Updates through the
> built-in updater do not trigger it again.

## Where FPS comes from

macOS does not tell other apps when an app presents a frame, but the window
server knows when a window's content changes. The overlay captures the active
app's window through ScreenCaptureKit at 16×16 pixels (the GPU scales it down;
the pixels are never read) and counts frames with the `Complete` status;
unchanged frames arrive as `Idle`. CPU cost is a fraction of a percent; FPS is
capped by the display refresh rate. If the game window appears later or is
recreated (resolution or full-screen switch), capture restarts by itself.
Nothing is injected into the game.

## Features

* **Frame time and FPS**, a graph that highlights bad frames, 1% / 0.1% lows.
* **Graphs of any sensor** — as many as you like, each with its own color,
  height, history window and scale.
* **Sensors:** CPU and GPU load, GPU memory in use, RAM.
* **Styling:** layout, font, sizes, colors, background, corner radius,
  **overlay size** (25–400%), **opacity**, screen corner and monitor.
* **Settings app** — dark, light or system theme, 8 accent colors,
  animations; changes apply to the overlay instantly.

The icon lives in the menu bar; the default hotkey is **Ctrl+Option+O**.
Opening the app again from Finder or Launchpad shows the settings.

## Resource use

The hidden overlay sleeps with no timers and captures nothing. The overlay
window moves only when its position changes, sensor graphs are redrawn once per
tick, rendering is software-only.

## Building

```
./install.command                 build and install
sh source/build/build.sh          build a release only → source/target/dist (.tar.gz and .dmg)
```

With a stable certificate (the `MACOS_SIGN_IDENTITY` secret, even a
self-signed one) macOS remembers the Screen Recording permission across
updates. With a Developer ID and the `APPLE_*` secrets the build is also
notarized. Pushing a `v*` tag runs `.github/workflows/release.yml`.

## Layout

```
install.command  update.command  uninstall.command   install, update, uninstall
docs/                                                documentation
source/
  crates/common     config, hotkeys, fonts, IPC
  crates/overlay    overlay: core, renderer, frames (ScreenCaptureKit), sensors, menu bar
  crates/settings   settings app
  crates/updater    updater (the system's curl and tar, SHA-256 check)
  resources/        Info.plist
  build/            release build (.app, .tar.gz, .dmg)
```
