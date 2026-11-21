//! Slint settings UI for UVie.
//!
//! Six panes and the persistence contract (this window is the sole writer
//! of `settings.json` / `macros.json`). UI sources live in `ui/`:
//! `widgets.slint` (shared controls), `pages/*.slint` (one per pane),
//! `settings.slint` (window root), all fluent-styled.
//!
//! The Slint event loop runs on a dedicated thread spawned by
//! [`spawn_settings_thread`]. `open` requests are delivered over a channel so
//! the tray can re-show the window after the user closes it.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender};

use uvie_core::macros::MacroTable;
use uvie_core::settings::{Hotkey, Settings, UiMode, DEFAULT_CHROMIUM_APPS};
use uvie_core::InputMethod;

slint::include_modules!();

struct State {
    settings_path: PathBuf,
    macros_path: PathBuf,
    settings: RefCell<Settings>,
    macros: RefCell<MacroTable>,
}

impl State {
    fn save_settings(&self) {
        let _ = self.settings.borrow().save(&self.settings_path);
    }
    fn save_macros(&self) {
        let _ = self.macros.borrow().save(&self.macros_path);
    }
}

/// Which settings list an `AppListSection` edits.
#[derive(Clone, Copy)]
enum AppList {
    Excluded,
    Chromium,
}

/// Spawn the dedicated UI thread that hosts the Slint settings window.
/// Returns a channel — send `()` to show/focus the window (e.g. from the
/// tray "Settings…" item). If the thread ever dies, send fails — callers
/// should respawn.
pub fn spawn_settings_thread(settings_path: PathBuf, macros_path: PathBuf) -> Sender<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        if let Err(e) = run(rx, settings_path, macros_path) {
            eprintln!("uvie-slintui: settings window failed: {e}");
        }
    });
    tx
}

fn run(
    rx: Receiver<()>,
    settings_path: PathBuf,
    macros_path: PathBuf,
) -> Result<(), slint::PlatformError> {
    // Per-monitor v2 DPI awareness so text renders at native resolution.
    // The exe's manifest already declares it, but binaries without one
    // (e.g. the preview example) would be bitmap-scaled → aliased text.
    unsafe {
        use windows::Win32::UI::HiDpi::{
            SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        };
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    let win = SettingsWindow::new()?;
    let state = Rc::new(State {
        settings: RefCell::new(Settings::load(&settings_path)),
        macros: RefCell::new(MacroTable::load(&macros_path)),
        settings_path,
        macros_path,
    });

    // -- populate initial values ------------------------------------------
    win.set_version(env!("CARGO_PKG_VERSION").into());
    {
        let s = state.settings.borrow();
        win.set_enabled(s.enabled);
        win.set_input_method_index(match s.input_method {
            InputMethod::Telex => 0,
            InputMethod::Vni => 1,
            InputMethod::SimpleTelex => 2,
        });
        win.set_scheme_index(match s.mode {
            UiMode::System => 0,
            UiMode::Light => 1,
            UiMode::Dark => 2,
        });
        win.set_per_app_language(s.per_app_language);
        win.set_english_override(s.english_override);
        win.set_launch_at_login(s.launch_at_login);
        win.set_relaxed_coda(s.relaxed_coda);
        win.set_auto_capitalize(s.auto_capitalize);
        win.set_macro_enabled(s.macro_enabled);
        win.set_modern_orthography(s.modern_orthography);
        win.set_quick_telex(s.quick_telex);
        win.set_quick_start(s.quick_start);
        win.set_hotkey_label(hotkey_label(&s.hotkey).into());
    }
    refresh_models(&win, &state);

    // -- callbacks ---------------------------------------------------------
    // One `save-settings` callback reads every in-out property back into
    // `Settings` and persists — controls two-way-bind their field directly.
    {
        let win = win.as_weak();
        let state = state.clone();
        win.unwrap().on_save_settings(move || {
            let win = win.unwrap();
            {
                let mut s = state.settings.borrow_mut();
                s.enabled = win.get_enabled();
                s.input_method = match win.get_input_method_index() {
                    1 => InputMethod::Vni,
                    2 => InputMethod::SimpleTelex,
                    _ => InputMethod::Telex,
                };
                s.mode = match win.get_scheme_index() {
                    1 => UiMode::Light,
                    2 => UiMode::Dark,
                    _ => UiMode::System,
                };
                s.per_app_language = win.get_per_app_language();
                s.english_override = win.get_english_override();
                s.launch_at_login = win.get_launch_at_login();
                s.relaxed_coda = win.get_relaxed_coda();
                s.auto_capitalize = win.get_auto_capitalize();
                s.macro_enabled = win.get_macro_enabled();
                s.modern_orthography = win.get_modern_orthography();
                s.quick_telex = win.get_quick_telex();
                s.quick_start = win.get_quick_start();
                win.set_hotkey_label(hotkey_label(&s.hotkey).into());
            }
            state.save_settings();
        });
    }
    {
        let win = win.as_weak();
        let state = state.clone();
        win.unwrap().on_add_macro(move |trigger, expansion| {
            let trigger = trigger.trim().to_string();
            let expansion = expansion.trim().to_string();
            if !trigger.is_empty() && !expansion.is_empty() {
                state.macros.borrow_mut().add(trigger, expansion);
                state.save_macros();
                refresh_models(&win.unwrap(), &state);
            }
        });
    }
    {
        let win = win.as_weak();
        let state = state.clone();
        win.unwrap().on_remove_macro(move |trigger| {
            state.macros.borrow_mut().remove(&trigger);
            state.save_macros();
            refresh_models(&win.unwrap(), &state);
        });
    }

    wire_app_list(
        &win,
        &state,
        AppList::Excluded,
        |s| s.excluded_apps.clone(),
        |s, v| s.excluded_apps = v,
        Vec::new(),
    );
    wire_app_list(
        &win,
        &state,
        AppList::Chromium,
        |s| s.chromium_apps.clone(),
        |s, v| s.chromium_apps = v,
        DEFAULT_CHROMIUM_APPS
            .iter()
            .map(|x| x.to_string())
            .collect(),
    );

    win.on_open_url(|url| open_url(&url));

    // Running-process picker (Apps pane): re-enumerate on each expand so
    // the list reflects what is actually running right now.
    win.set_running_apps(strings_model(&running_processes()));
    {
        let win = win.as_weak();
        win.unwrap().on_refresh_running_apps(move || {
            win.unwrap()
                .set_running_apps(strings_model(&running_processes()));
        });
    }

    // Close → hide, keep the event loop + component alive so the tray can
    // re-show the window later.
    win.window()
        .on_close_requested(|| slint::CloseRequestResponse::HideWindow);

    // Watcher: `open_settings()` sends `()` → re-show the window on the
    // Slint thread.
    {
        let weak = win.as_weak();
        std::thread::spawn(move || {
            while rx.recv().is_ok() {
                // upgrade_in_event_loop fails once the Slint loop is
                // gone — drop `rx` then so `open_settings` sees a dead
                // channel and respawns the UI thread instead of sending
                // into the void.
                if weak
                    .upgrade_in_event_loop(|win| {
                        let _ = win.show();
                        win.window().request_redraw();
                    })
                    .is_err()
                {
                    return;
                }
            }
        });
    }

    win.show()?;
    // Keep running while the window is hidden — the tray re-shows it.
    slint::run_event_loop_until_quit()
}

/// Register add/remove/reset handlers for one app list against `Settings`
/// and refresh the models afterwards.
fn wire_app_list(
    win: &SettingsWindow,
    state: &Rc<State>,
    list: AppList,
    get: fn(&Settings) -> Vec<String>,
    set: fn(&mut Settings, Vec<String>),
    defaults: Vec<String>,
) {
    let add = {
        let win = win.as_weak();
        let state = state.clone();
        move |exe: slint::SharedString| {
            let exe = exe.trim().to_lowercase();
            if !exe.is_empty() {
                {
                    let mut s = state.settings.borrow_mut();
                    let mut apps = get(&s);
                    if !apps.iter().any(|a| a == &exe) {
                        apps.push(exe);
                        set(&mut s, apps);
                    }
                }
                state.save_settings();
                refresh_models(&win.unwrap(), &state);
            }
        }
    };
    let remove = {
        let win = win.as_weak();
        let state = state.clone();
        move |exe: slint::SharedString| {
            {
                let mut s = state.settings.borrow_mut();
                let mut apps = get(&s);
                apps.retain(|a| a != exe.as_str());
                set(&mut s, apps);
            }
            state.save_settings();
            refresh_models(&win.unwrap(), &state);
        }
    };
    let reset = {
        let win = win.as_weak();
        let state = state.clone();
        move || {
            {
                let mut s = state.settings.borrow_mut();
                set(&mut s, defaults.clone());
            }
            state.save_settings();
            refresh_models(&win.unwrap(), &state);
        }
    };

    match list {
        AppList::Excluded => {
            win.on_add_excluded(add);
            win.on_remove_excluded(remove);
            win.on_reset_excluded(reset);
        }
        AppList::Chromium => {
            win.on_add_chromium(add);
            win.on_remove_chromium(remove);
            win.on_reset_chromium(reset);
        }
    }
}

/// Push `state` (macros + app lists) into the UI's model properties.
fn refresh_models(win: &SettingsWindow, state: &Rc<State>) {
    let macros: Vec<MacroItem> = state
        .macros
        .borrow()
        .entries
        .iter()
        .map(|e| MacroItem {
            trigger: e.trigger.as_str().into(),
            expansion: e.expansion.as_str().into(),
        })
        .collect();
    win.set_macros(std::rc::Rc::new(slint::VecModel::from(macros)).into());

    let s = state.settings.borrow();
    win.set_excluded_apps(strings_model(&s.excluded_apps));
    win.set_chromium_apps(strings_model(&s.chromium_apps));
}

fn strings_model(items: &[String]) -> slint::ModelRc<slint::SharedString> {
    let v: Vec<slint::SharedString> = items.iter().map(|s| s.as_str().into()).collect();
    std::rc::Rc::new(slint::VecModel::from(v)).into()
}

/// Human-readable hotkey label for the General pane ("Ctrl + Shift + Z").
fn hotkey_label(h: &Hotkey) -> String {
    let mut s = String::new();
    if h.ctrl {
        s += "Ctrl + ";
    }
    if h.alt {
        s += "Alt + ";
    }
    if h.shift {
        s += "Shift + ";
    }
    s += &vk_name(h.vk);
    s
}

/// Common Win32 VK codes → display names for `hotkey_label`.
fn vk_name(vk: u32) -> String {
    match vk {
        0x41..=0x5A => char::from_u32(vk).unwrap().to_string(), // VK_A..VK_Z
        0x30..=0x39 => char::from_u32(vk).unwrap().to_string(), // VK_0..VK_9
        0x70..=0x87 => format!("F{}", vk - 0x6F),               // VK_F1..VK_F24
        0x20 => "Space".into(),
        0x09 => "Tab".into(),
        0x0D => "Enter".into(),
        0x1B => "Esc".into(),
        0x08 => "Backspace".into(),
        0x2E => "Delete".into(),
        _ => format!("0x{vk:02X}"),
    }
}

/// Snapshot running processes via ToolHelp32 → unique lowercase exe names,
/// sorted. Powers the Apps pane's "pick a running app" flow so users don't
/// have to type `chrome.exe` by hand. Our own exe is omitted — excluding
/// UVie itself is meaningless (the hook already skips our windows).
fn running_processes() -> Vec<String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    let mut names = Vec::new();
    unsafe {
        let snap = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            Ok(h) => h,
            Err(_) => return names,
        };
        let mut pe = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snap, &mut pe).is_ok() {
            loop {
                let len = pe
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(pe.szExeFile.len());
                let name = String::from_utf16_lossy(&pe.szExeFile[..len]).to_lowercase();
                if !name.is_empty() && name != "uvie-win.exe" {
                    names.push(name);
                }
                if Process32NextW(snap, &mut pe).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    names.sort_unstable();
    names.dedup();
    names
}

/// Open a URL in the system browser (About-pane links).
fn open_url(url: &str) {
    use windows::core::{w, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let url_w: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let _ = ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(url_w.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uvie_core::settings::Hotkey;

    #[test]
    fn hotkey_label_lists_modifiers_in_ctrl_alt_shift_order() {
        let h = Hotkey {
            ctrl: true,
            alt: false,
            shift: true,
            vk: 0x5A,
        };
        assert_eq!(hotkey_label(&h), "Ctrl + Shift + Z");
        let h = Hotkey {
            ctrl: true,
            alt: true,
            shift: false,
            vk: 0x30,
        };
        assert_eq!(hotkey_label(&h), "Ctrl + Alt + 0");
        let h = Hotkey {
            ctrl: false,
            alt: false,
            shift: false,
            vk: 0x70,
        };
        assert_eq!(hotkey_label(&h), "F1");
    }

    #[test]
    fn vk_name_covers_named_and_numeric_ranges() {
        assert_eq!(vk_name(0x41), "A");
        assert_eq!(vk_name(0x5A), "Z");
        assert_eq!(vk_name(0x30), "0");
        assert_eq!(vk_name(0x39), "9");
        assert_eq!(vk_name(0x70), "F1");
        assert_eq!(vk_name(0x87), "F24");
        for (vk, want) in [
            (0x20, "Space"),
            (0x09, "Tab"),
            (0x0D, "Enter"),
            (0x1B, "Esc"),
            (0x08, "Backspace"),
            (0x2E, "Delete"),
        ] {
            assert_eq!(vk_name(vk), want, "vk {vk:#04X}");
        }
        // Unmapped codes fall back to hex.
        assert_eq!(vk_name(0x10), "0x10");
        assert_eq!(vk_name(0xFF), "0xFF");
    }
}
