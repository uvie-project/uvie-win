//! Notification-area (system tray) icon + context menu — the Windows
//! counterpart of uvie-mac's `MenuBarController`.
//!
//! A hidden message-only window owns the icon and receives the menu clicks;
//! actions are forwarded to the app via a callback.

use std::cell::RefCell;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    GetCursorPos, LoadIconW, PostQuitMessage, RegisterClassExW, SetForegroundWindow,
    TrackPopupMenu, CW_USEDEFAULT, IDI_APPLICATION, MF_SEPARATOR, MF_STRING, TPM_BOTTOMALIGN,
    TPM_LEFTALIGN, WM_COMMAND, WM_DESTROY, WM_RBUTTONUP, WM_USER, WNDCLASSEXW, WS_EX_NOACTIVATE,
};

/// Menu item ids (WM_COMMAND wParam).
pub const CMD_TOGGLE_LANGUAGE: usize = 1;
pub const CMD_OPEN_SETTINGS: usize = 2;
pub const CMD_QUIT: usize = 3;

/// Tray callback message id (NOTIFYICONDATAW::uCallbackMessage).
const WM_TRAYICON: u32 = WM_USER + 0x55;

const WINDOW_CLASS: PCWSTR = w!("UVieWinTrayWindow");

type MenuHandler = RefCell<Option<Box<dyn FnMut(usize)>>>;

thread_local! {
    static MENU_HANDLER: MenuHandler = const { RefCell::new(None) };
}

pub struct TrayIcon {
    hwnd: HWND,
}

impl TrayIcon {
    /// Creates the hidden message window + adds the tray icon.
    /// `on_menu` receives one of the `CMD_*` constants.
    pub fn create(on_menu: impl FnMut(usize) + 'static) -> windows::core::Result<Self> {
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
                w!("UVie"),
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
        let text = "UVie — Vietnamese input (Telex)";
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
        data.hIcon = unsafe { LoadIconW(None, IDI_APPLICATION)? };
        unsafe {
            Shell_NotifyIconW(NIM_ADD, &data).ok()?;
        }
        Ok(Self { hwnd })
    }

    /// Window handle behind the tray icon (e.g. for SetForegroundWindow).
    #[allow(dead_code)]
    pub fn hwnd(&self) -> HWND {
        self.hwnd
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
            if lparam.0 as u32 == WM_RBUTTONUP {
                show_menu(hwnd);
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
    let _ = DestroyMenu(menu);
}
