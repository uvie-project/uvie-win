# UVie for Windows

Vietnamese input method for Windows, powered by the
[uvie-rs](https://github.com/uvie-project/uvie-rs) engine - the same core
as [UVieMac](https://github.com/uvie-project/uvie-mac), and the same
`uvie-host` dispatch layer as
[uvie-linux](https://github.com/uvie-project/uvie-linux).

Telex, VNI and SimpleTelex input at the OS level: a low-level keyboard
hook feeds keys to the engine and re-injects the composed UTF-16 text, so
UVie works in every application.

## Features

Same feature set as uvie-mac:

- Telex / VNI / SimpleTelex input methods (uvie-rs `UltraFastViEngine`)
- Per-application Vi/En language memory
- Caret-aware editing: caret movement (arrow keys, Home/End, ...) is
  tracked, so typing inside already-committed text edits in place and
  forward-delete works (LabanKey-style)
- Escape restores the raw keystrokes of the composing word (EVKey-style)
- Auto-capitalize keeps a per-app sentence state that swaps on app switch
- Ctrl/Alt/Win chords never reach the engine - Win+R, Ctrl+C and friends
  pass straight through to the shell/app
- Text macros: abbreviation → full text on Space/Enter (editable in
  Settings → Macro, stored in `%APPDATA%\UVie\macros.json`)
- App exclusion list (e.g. terminals, password fields) plus a Chromium-app
  compatibility list - browsers get a select-and-overwrite injection mode
  instead of synthetic backspaces (omnibox-safe, like uvie-mac). Both
  lists can pick from currently running processes, no need to type exe
  names.
- System-tray icon with Vi/En toggle, settings and quit
- Launch at login (per-user registry key)
- English-override mode: Ctrl+Shift+Z toggles Vi/En globally (tray menu
  too)
- Engine options: quick Telex, quick-start consonants, modern orthography,
  relaxed coda, auto-capitalize
- Slint settings window - sidebar + panes mirroring uvie-mac's
  preferences: Tổng quan, Bàn phím, Macro, Ứng dụng, Nâng cao, Giới thiệu
  (Vietnamese UI, light/dark)

## Architecture

Cargo workspace with three crates:

| Crate | Role |
|---|---|
| `uvie-core` | Thin facade over the shared `uvie-host` crate plus the Windows-specific settings schema (Win32 virtual-key hotkeys, chromium exe list, UI mode). Dispatch, engine sessions, macros and per-app memory live in `uvie-host`. |
| `uvie-win` | The IME binary. Win32 integration via `windows-rs`: LL keyboard hook, SendInput injection (marked `LLKHF_INJECTED` + magic `dwExtraInfo` to detect echoes), foreground-app watcher (WinEvent hook → exe name), tray icon (hidden window + `Shell_NotifyIcon`), registry launch-at-login, settings hot-reload timer, message loop. |
| `uvie-slintui` | Slint settings window (fluent style, light/dark). Runs on its own thread inside `uvie-win.exe`, shows/hides via a channel from the tray, and is the sole writer of `settings.json` / `macros.json` - the app side polls file mtimes once a second to pick up changes. |

The engine and the shared host layer are pinned together (same uvie-rs
tag):

```toml
uvie      = { git = "https://github.com/uvie-project/uvie-rs", tag = "v3.1.3" }
uvie-host = { git = "https://github.com/uvie-project/uvie-rs", tag = "v3.1.3" }
```

`uvie` is the Vietnamese engine; `uvie-host` is the platform-neutral
dispatch layer (dispatcher state machine, engine session, macros, per-app
memory) shared with uvie-linux. The pinned tag is also recorded in
`uvie-rs-version`.

## Prerequisites

- Windows 10 1809+ (x86_64)
- [Rust](https://rustup.rs) stable (MSVC toolchain) with `rustfmt` +
  `clippy`

## Build

```powershell
cargo build --release
```

The binary is `target/release/uvie-win.exe` - fully self-contained (the
settings UI is Slint, so no Windows App SDK runtime is needed).

### Installer

```powershell
iscc installer\uvie-win.iss
```

Produces `dist\uvie-win-<version>-x64-setup.exe` - a per-user installer
(no UAC) that bundles `uvie-win.exe`. Requires
[Inno Setup 6](https://jrsoftware.org/isinfo.php).

## Run

```powershell
./target/release/uvie-win.exe
```

A tray icon appears. Type Telex (e.g. `vieetj` → `việt`) anywhere.
Right-click the tray icon to toggle Vi/En, open settings, or quit - or
press `Ctrl+Shift+Z` anywhere to toggle Vi/En.

## Known limitations

- Keystrokes for **elevated** apps are invisible to the low-level hook and
  pass through uncomposed (same gap as uvie-mac's secure-input mode).
- Non-Latin *input languages* pass through untouched (the foreground
  layout's LANGID is checked against `LOCALE_SSCRIPTS`), but a Latin
  layout registered under a non-Latin language can still misclassify.

## Development

```powershell
cargo test                    # uvie-core unit tests (headless)
cargo clippy --all-targets    # must be warning-free (CI enforces -D warnings)
cargo fmt --all --check       # CI enforces rustfmt
cargo run -p uvie-slintui --example preview   # preview the settings window
```

## License

MIT OR Apache-2.0, matching the rest of the uvie-project family.
