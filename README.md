# UVie for Windows

Vietnamese input method for Windows, powered by the
[uvie-rs](https://github.com/uvie-project/uvie-rs) engine — the same core as
[UVieMac](https://github.com/uvie-project/uvie-mac).

Telex, VNI and SimpleTelex input at the OS level: a low-level keyboard hook
feeds keys to the engine and re-injects the composed UTF-16 text, so UVie
works in every application.

## Features

Same feature set as uvie-mac:

- Telex / VNI / SimpleTelex input methods (uvie-rs `UltraFastViEngine`)
- Per-application Vi/En language memory
- Text macros: abbreviation → full text on Space/Enter (editable in
  Settings → Macro, stored in `%APPDATA%\UVie\macros.json`)
- App exclusion list (e.g. terminals, password fields) plus a Chromium-app
  compatibility list — browsers get a select-and-overwrite injection mode
  instead of synthetic backspaces (omnibox-safe, like uvie-mac)
- System-tray icon with Vi/En toggle, settings and quit
- Launch at login (per-user registry key)
- English-override mode: Ctrl+Shift+Z toggles Vi/En globally (tray menu too)
- Engine options: quick Telex, quick-start consonants, modern orthography,
  relaxed coda, auto-capitalize
- WinUI 3 settings window (unpackaged app, bootstrapped at runtime) —
  sidebar + panes mirroring uvie-mac's preferences: Tổng quan, Bàn phím,
  Macro, Ứng dụng, Nâng cao, Giới thiệu (Vietnamese UI)

## Architecture

Cargo workspace with three crates:

| Crate | Role |
|---|---|
| `uvie-core` | Platform-neutral engine plumbing: `Dispatcher` (key → `Action` state machine), `EngineSession` (wraps `UltraFastViEngine`), settings/macros/language-memory persistence, `InputMethod`. Fully unit-tested — no Win32 imports. |
| `uvie-win` | The IME binary. Win32 integration via `windows-rs`: LL keyboard hook, SendInput injection (marked `LLKHF_INJECTED` + magic `dwExtraInfo` to detect echoes), foreground-app watcher (WinEvent hook → exe name), tray icon (hidden window + `Shell_NotifyIcon`), registry launch-at-login, message loop. |
| `uvie-winui` | WinUI 3 settings window. Generates its own `windows-rs` bindings at build time with `windows-bindgen` from the Windows App SDK winmds in `Frameworks/WindowsAppSDK` (fetched at dev time, never committed — same convention as uvie-mac's `Frameworks/`). Spawns XAML on its own thread so it can live in the same process as the hook. |

The engine dependency is a git pin:

```toml
uvie = { git = "https://github.com/uvie-project/uvie-rs", tag = "v2.8.1" }
```

The pinned version is also recorded in `uvie-rs-version`.

## Prerequisites

- Windows 10 1809+ (x86_64)
- [Rust](https://rustup.rs) stable (MSVC toolchain) with `rustfmt` + `clippy`
- **Windows App Runtime 1.6+** — required only for the WinUI 3 settings
  window; the IME itself is pure Win32. Install it once via
  [`WindowsAppRuntimeInstall-x64.exe`](https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/downloads)
  (the exe then self-bootstraps via `MddBootstrapInitialize2`; the
  `Microsoft.WindowsAppRuntime.Bootstrap.dll` stub is copied next to
  `uvie-win.exe` by the build script).

## Build

```powershell
# 1) Fetch Windows App SDK winmds + bootstrap DLL into Frameworks/
pwsh scripts/fetch-windowsappsdk.ps1
#    (or: bash scripts/fetch-windowsappsdk.sh)

# 2) Build
cargo build --release
```

The binary is `target/release/uvie-win.exe`
(`Microsoft.WindowsAppRuntime.Bootstrap.dll` is copied next to it).

## Run

```powershell
./target/release/uvie-win.exe
```

A tray icon appears. Type Telex (e.g. `vieetj` → `việt`) anywhere.
Right-click the tray icon to toggle Vi/En, open settings, or quit — or press
`Ctrl+Shift+Z` anywhere to toggle Vi/En.

## Known limitations

- Keystrokes for **elevated** apps are invisible to the low-level hook and
  pass through uncomposed (same gap as uvie-mac's secure-input mode).
- Changes made in the Settings window apply on the next launch (no live
  reload yet).

## Development

```powershell
cargo test --workspace          # uvie-core unit tests (headless)
cargo clippy --all-targets      # must be warning-free (CI enforces -D warnings)
cargo fmt --all --check         # CI enforces rustfmt
```

## License

MIT OR Apache-2.0, matching the rest of the uvie-project family.
