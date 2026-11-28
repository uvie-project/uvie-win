//! Notification-area (system tray) icon + context menu — the Windows
//! counterpart of uvie-mac's `MenuBarController`.
//!
//! A hidden message-only window owns the icon and receives the menu clicks;
//! actions are forwarded to the app via a callback.

use std::cell::{Cell, RefCell};

use uvie_core::settings::Hotkey;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetDoubleClickTime, RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL,
    MOD_NOREPEAT, MOD_SHIFT,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    GetCursorPos, KillTimer, LoadIconW, PostMessageW, PostQuitMessage, RegisterClassExW,
    SetForegroundWindow, SetTimer, TrackPopupMenu, CW_USEDEFAULT, HICON, IDI_APPLICATION,
    MF_SEPARATOR, MF_STRING, TPM_BOTTOMALIGN, TPM_LEFTALIGN, WM_COMMAND, WM_DESTROY, WM_HOTKEY,
    WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WM_TIMER, WM_USER, WNDCLASSEXW,
    WS_EX_NOACTIVATE,
};

/// App icon resource id (app.rc: `IDI_APP_ICON ICON "assets/uvie.ico"`).
const IDI_APP_ICON: u16 = 101;

/// Loads the embedded org-avatar icon; falls back to the stock app icon.
fn app_icon(hinst: HINSTANCE) -> HICON {
    unsafe {
        LoadIconW(
            Some(hinst),
            PCWSTR::from_raw(IDI_APP_ICON as usize as *const u16),
        )
        .or_else(|_| LoadIconW(None, IDI_APPLICATION))
        .unwrap_or_default()
    }
}

/// Menu item ids (WM_COMMAND wParam).
pub const CMD_TOGGLE_LANGUAGE: usize = 1;
pub const CMD_OPEN_SETTINGS: usize = 2;
pub const CMD_QUIT: usize = 3;
/// Synthetic command driven by `TIMER_SETTINGS`: re-read settings.json /
/// macros.json when their mtimes change (the settings window runs in this
/// process but owns its own copy of the files).
pub const CMD_SYNC_SETTINGS: usize = 4;

/// Timer id for the settings/macro file poll.
const TIMER_SETTINGS: usize = 1;
/// One-shot timer used to tell a single tray click apart from the first
/// half of a double-click.
const TIMER_CLICK: usize = 2;

/// Tray callback message id (NOTIFYICONDATAW::uCallbackMessage).
const WM_TRAYICON: u32 = WM_USER + 0x55;

const WINDOW_CLASS: PCWSTR = w!("UVieWinTrayWindow");

/// RegisterHotKey id for the global Vi/En toggle.
const HOTKEY_TOGGLE: i32 = 1;

/// `Settings.hotkey` (Ctrl/Alt/Shift + VK) → RegisterHotKey modifier flags.
/// MOD_NOREPEAT keeps a held chord from toggling on every key repeat.
fn hotkey_modifiers(h: &Hotkey) -> HOT_KEY_MODIFIERS {
    let mut m = MOD_NOREPEAT;
    if h.ctrl {
        m |= MOD_CONTROL;
    }
    if h.alt {
        m |= MOD_ALT;
    }
    if h.shift {
        m |= MOD_SHIFT;
    }
    m
}

type MenuHandler = RefCell<Option<Box<dyn FnMut(usize)>>>;

thread_local! {
    static MENU_HANDLER: MenuHandler = const { RefCell::new(None) };
    /// The up event immediately following WM_LBUTTONDBLCLK is the second
    /// half of that double-click, not a new single click.
    static SUPPRESS_TRAY_CLICK_UP: Cell<bool> = const { Cell::new(false) };
}

pub struct TrayIcon {
    hwnd: HWND,
}

impl TrayIcon {
    /// Creates the hidden message window + adds the tray icon.
    /// `hotkey` is the Vi/En toggle binding from `Settings`; `on_menu`
    /// receives one of the `CMD_*` constants.
    pub fn create(
        hotkey: &Hotkey,
        on_menu: impl FnMut(usize) + 'static,
    ) -> windows::core::Result<Self> {
        MENU_HANDLER.with(|h| *h.borrow_mut() = Some(Box::new(on_menu)));

        let hinst = unsafe { GetModuleHandleW(None)? };
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinst.into(),
            lpszClassName: WINDOW_CLASS,
            ..Default::default()
        };
        unsafe {
            RegisterClassExW(&wc);
        }
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE,
                WINDOW_CLASS,
                w!("UVie for Windows"),
                Default::default(),
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                None,
                None,
                Some(HINSTANCE(hinst.0)),
                None,
            )?
        };

        let mut tip = [0u16; 128];
        let text = "UVie for Windows — Vietnamese input (Telex)";
        for (i, c) in text.encode_utf16().enumerate() {
            tip[i] = c;
        }

        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: WM_TRAYICON,
            szTip: tip,
            ..Default::default()
        };
        data.hIcon = app_icon(hinst.into());
        unsafe {
            Shell_NotifyIconW(NIM_ADD, &data).ok()?;
            // Global Vi/En toggle — the binding from Settings.hotkey.
            // WM_HOTKEY is delivered to this window's proc.
            let _ = RegisterHotKey(
                Some(hwnd),
                HOTKEY_TOGGLE,
                hotkey_modifiers(hotkey),
                hotkey.vk,
            );
            // Poll the settings/macro files once a second: the settings
            // window writes them in-process, and this is how the engine
            // side learns about changes (cheaper than directory watching).
            let _ = SetTimer(Some(hwnd), TIMER_SETTINGS, 1000, None);
        }
        Ok(Self { hwnd })
    }

    /// Window handle behind the tray icon — needed to re-register the
    /// global hotkey when Settings.hotkey changes.
    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }
}

/// Re-register the Vi/En toggle hotkey after the binding changed.
pub fn update_hotkey(hwnd: HWND, hotkey: &Hotkey) {
    unsafe {
        let _ = UnregisterHotKey(Some(hwnd), HOTKEY_TOGGLE);
        let _ = RegisterHotKey(
            Some(hwnd),
            HOTKEY_TOGGLE,
            hotkey_modifiers(hotkey),
            hotkey.vk,
        );
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        let data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            ..Default::default()
        };
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_TOGGLE);
            let _ = KillTimer(Some(self.hwnd), TIMER_SETTINGS);
            let _ = KillTimer(Some(self.hwnd), TIMER_CLICK);
            let _ = DestroyWindow(self.hwnd);
        }
        MENU_HANDLER.with(|h| *h.borrow_mut() = None);
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TRAYICON => {
            match lparam.0 as u32 {
                WM_RBUTTONUP => show_menu(hwnd),
                // Single click toggles Vi/En like EVKey. Wait one
                // double-click interval so double-click can keep its
                // existing Settings action without toggling twice first.
                WM_LBUTTONUP => {
                    let skip = SUPPRESS_TRAY_CLICK_UP.with(|s| s.replace(false));
                    if !skip {
                        unsafe {
                            let _ = SetTimer(Some(hwnd), TIMER_CLICK, GetDoubleClickTime(), None);
                        }
                    }
                }
                // Double-click opens the settings window.
                WM_LBUTTONDBLCLK => {
                    SUPPRESS_TRAY_CLICK_UP.with(|s| s.set(true));
                    unsafe {
                        let _ = KillTimer(Some(hwnd), TIMER_CLICK);
                    }
                    MENU_HANDLER.with(|h| {
                        if let Ok(mut g) = h.try_borrow_mut() {
                            if let Some(f) = g.as_mut() {
                                f(CMD_OPEN_SETTINGS);
                            }
                        }
                    })
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_HOTKEY => {
            MENU_HANDLER.with(|h| {
                if let Ok(mut g) = h.try_borrow_mut() {
                    if let Some(f) = g.as_mut() {
                        f(CMD_TOGGLE_LANGUAGE);
                    }
                }
            });
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == TIMER_SETTINGS {
                MENU_HANDLER.with(|h| {
                    if let Ok(mut g) = h.try_borrow_mut() {
                        if let Some(f) = g.as_mut() {
                            f(CMD_SYNC_SETTINGS);
                        }
                    }
                });
            } else if wparam.0 == TIMER_CLICK {
                unsafe {
                    let _ = KillTimer(Some(hwnd), TIMER_CLICK);
                }
                MENU_HANDLER.with(|h| {
                    if let Ok(mut g) = h.try_borrow_mut() {
                        if let Some(f) = g.as_mut() {
                            f(CMD_TOGGLE_LANGUAGE);
                        }
                    }
                });
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = wparam.0 & 0xFFFF;
            MENU_HANDLER.with(|h| {
                if let Ok(mut g) = h.try_borrow_mut() {
                    if let Some(f) = g.as_mut() {
                        f(id);
                    }
                }
            });
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn show_menu(hwnd: HWND) {
    let menu = match CreatePopupMenu() {
        Ok(m) => m,
        Err(_) => return,
    };
    let _ = AppendMenuW(
        menu,
        MF_STRING,
        CMD_TOGGLE_LANGUAGE,
        w!("Vietnamese / English"),
    );
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(menu, MF_STRING, CMD_OPEN_SETTINGS, w!("Settings…"));
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(menu, MF_STRING, CMD_QUIT, w!("Quit"));

    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    // Required so the menu dismisses when clicking elsewhere.
    let _ = SetForegroundWindow(hwnd);
    let _ = TrackPopupMenu(
        menu,
        TPM_LEFTALIGN | TPM_BOTTOMALIGN,
        pt.x,
        pt.y,
        Some(0),
        hwnd,
        None,
    );
    // Documented companion to the SetForegroundWindow above: nudges input
    // ownership back after the modal menu loop ends.
    let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
    let _ = DestroyMenu(menu);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkey_modifiers_always_sets_norepeat() {
        let h = Hotkey {
            ctrl: false,
            alt: false,
            shift: false,
            vk: 0x5A,
        };
        assert_eq!(hotkey_modifiers(&h), MOD_NOREPEAT);
    }

    #[test]
    fn hotkey_modifiers_maps_each_flag() {
        for (ctrl, alt, shift, want) in [
            (true, false, false, MOD_NOREPEAT | MOD_CONTROL),
            (false, true, false, MOD_NOREPEAT | MOD_ALT),
            (false, false, true, MOD_NOREPEAT | MOD_SHIFT),
            (
                true,
                true,
                true,
                MOD_NOREPEAT | MOD_CONTROL | MOD_ALT | MOD_SHIFT,
            ),
        ] {
            let h = Hotkey {
                ctrl,
                alt,
                shift,
                vk: 0x5A,
            };
            assert_eq!(hotkey_modifiers(&h), want, "{ctrl} {alt} {shift}");
        }
    }

    #[test]
    fn hotkey_modifiers_default_chord_is_ctrl_shift() {
        assert_eq!(
            hotkey_modifiers(&Hotkey::default()),
            MOD_NOREPEAT | MOD_CONTROL | MOD_SHIFT
        );
    }
}
