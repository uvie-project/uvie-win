//! A tiny Win32 modal input dialog.
//!
//! `TextBox` (and anything hosting the XAML text-input stack, like
//! `NavigationView`'s pane search box) fast-fails on Windows Server 2022 with
//! WASDK 1.6 — the control template spins up the text-services path, which is
//! unavailable in this environment and aborts the process instead of throwing.
//! Pure user32 EDIT controls don't touch XAML, so "add macro / add app" flows
//! collect text through this dialog instead.

use std::ffi::c_void;

use windows::core::w;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetStockObject, DEFAULT_GUI_FONT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, IsDialogMessageW,
    PostQuitMessage, RegisterClassExW, SendMessageW, SetForegroundWindow, SetWindowLongPtrW,
    SetWindowPos, TranslateMessage, BS_DEFPUSHBUTTON, CREATESTRUCTW, CW_USEDEFAULT, GWLP_USERDATA,
    HMENU, MSG, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE,
    WM_COMMAND, WM_CREATE, WM_DESTROY, WM_SETFONT, WNDCLASSEXW, WS_BORDER, WS_CAPTION, WS_CHILD,
    WS_CLIPCHILDREN, WS_EX_CONTROLPARENT, WS_EX_DLGMODALFRAME, WS_POPUP, WS_SYSMENU, WS_TABSTOP,
    WS_VISIBLE,
};

const ID_OK: u16 = 1;
const ID_CANCEL: u16 = 2;
const CLASS: windows::core::PCWSTR = w!("UVieWinEntryDialog");

/// Per-window state: the EDIT HWNDs and where to store the result.
struct Ctx {
    fields: Vec<FieldSpec>,
    edits: Vec<HWND>,
    result: *mut Option<Vec<String>>,
}

struct FieldSpec {
    label: Vec<u16>,
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_string(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        let got = GetWindowTextW(hwnd, &mut buf);
        String::from_utf16_lossy(&buf[..got as usize])
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            let cs = lparam.0 as *const CREATESTRUCTW;
            let ctx = (*cs).lpCreateParams as *mut Ctx;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, ctx as isize);

            let hinst = HINSTANCE(GetModuleHandleW(None).unwrap_or_default().0);
            let font = GetStockObject(DEFAULT_GUI_FONT);

            let mut y = 14i32;
            for spec in &(*ctx).fields {
                let label = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("STATIC"),
                    windows::core::PCWSTR(spec.label.as_ptr()),
                    WS_CHILD | WS_VISIBLE,
                    16,
                    y,
                    340,
                    16,
                    Some(hwnd),
                    None,
                    Some(hinst),
                    None,
                )
                .unwrap_or_default();
                SendMessageW(label, WM_SETFONT, Some(WPARAM(font.0 as usize)), None);
                y += 20;
                let edit = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("EDIT"),
                    windows::core::PCWSTR::null(),
                    WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP,
                    16,
                    y,
                    340,
                    22,
                    Some(hwnd),
                    None,
                    Some(hinst),
                    None,
                )
                .unwrap_or_default();
                SendMessageW(edit, WM_SETFONT, Some(WPARAM(font.0 as usize)), None);
                (*ctx).edits.push(edit);
                y += 32;
            }

            for (i, (text, id)) in [("OK", ID_OK), ("Hủy", ID_CANCEL)].iter().enumerate() {
                let style = if *id == ID_OK {
                    WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_DEFPUSHBUTTON as u32)
                } else {
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP
                };
                let wide = to_wide(text);
                let btn = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("BUTTON"),
                    windows::core::PCWSTR(wide.as_ptr()),
                    style,
                    196 + 92 * i as i32,
                    y,
                    76,
                    24,
                    Some(hwnd),
                    Some(HMENU(*id as usize as *mut c_void)),
                    Some(hinst),
                    None,
                )
                .unwrap_or_default();
                SendMessageW(btn, WM_SETFONT, Some(WPARAM(font.0 as usize)), None);
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let ctx = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ctx;
            let id = (wparam.0 & 0xFFFF) as u16;
            if id == ID_OK {
                if !ctx.is_null() {
                    let out: Vec<String> = (*ctx)
                        .edits
                        .iter()
                        .map(|e| wide_string(*e).trim().to_string())
                        .collect();
                    *(*ctx).result = Some(out);
                }
                let _ = DestroyWindow(hwnd);
            } else if id == ID_CANCEL {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn register_class() -> windows::core::Result<()> {
    unsafe {
        let hinst = HINSTANCE(GetModuleHandleW(None)?.0);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            hInstance: hinst,
            lpszClassName: CLASS,
            lpfnWndProc: Some(wnd_proc),
            // COLOR_3DFACE + 1 — the standard dialog face brush.
            hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH(16isize as *mut c_void),
            ..Default::default()
        };
        RegisterClassExW(&wc);
        Ok(())
    }
}

/// Show a modal input dialog over `owner`. Returns the field values
/// (one trimmed string per label) or `None` on cancel.
/// Must be called on the thread that owns `owner` — it runs a nested
/// message loop, which also keeps the XAML dispatcher pumping.
pub fn prompt(owner: HWND, title: &str, labels: &[&str]) -> Option<Vec<String>> {
    register_class().ok()?;

    let fields: Vec<FieldSpec> = labels
        .iter()
        .map(|l| FieldSpec { label: to_wide(l) })
        .collect();
    let height = 14 + fields.len() as i32 * 52 + 44;
    let mut result: Option<Vec<String>> = None;
    let mut ctx = Box::new(Ctx {
        fields,
        edits: Vec::new(),
        result: &mut result,
    });

    unsafe {
        let hinst = HINSTANCE(GetModuleHandleW(None).ok()?.0);
        let title_w = to_wide(title);
        let hwnd = CreateWindowExW(
            WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
            CLASS,
            windows::core::PCWSTR(title_w.as_ptr()),
            WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_VISIBLE | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            372,
            height + 40, // caption + borders
            Some(owner),
            None,
            Some(hinst),
            Some(&mut *ctx as *mut Ctx as *const c_void),
        )
        .ok()?;

        // Center over the owner window.
        let mut rc = std::mem::zeroed();
        let mut orc = std::mem::zeroed();
        let _ = GetWindowRect(hwnd, &mut rc);
        if GetWindowRect(owner, &mut orc).is_ok() {
            let w = rc.right - rc.left;
            let h = rc.bottom - rc.top;
            let x = orc.left + ((orc.right - orc.left) - w) / 2;
            let y = orc.top + ((orc.bottom - orc.top) - h) / 2;
            let _ = SetWindowPos(
                hwnd,
                None,
                x.max(0),
                y.max(0),
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }

        let _ = EnableWindow(owner, false);
        let _ = SetForegroundWindow(hwnd);

        // Nested modal loop: pumps messages for this thread — including XAML
        // dispatcher work — until the dialog window is destroyed (OK/Cancel/
        // close each post WM_QUIT via PostQuitMessage in WM_DESTROY).
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if !IsDialogMessageW(hwnd, &msg).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        let _ = EnableWindow(owner, true);
        let _ = SetForegroundWindow(owner);
        drop(ctx);
    }
    result
}
