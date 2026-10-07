//! WH_KEYBOARD_LL low-level keyboard hook — the Windows counterpart of
//! uvie-mac's CGEventTap.
//!
//! The hook proc runs on the message-loop thread and must stay fast; it
//! translates raw VK codes into `KeyEvent`s (via `ToUnicodeEx` for the active
//! layout) and hands them to the dispatcher. Returning `LRESULT(1)` consumes
//! the key — exactly what the dispatcher's `Consume` decision asks for.
//!
//! Known limitation vs uvie-mac's event tap: keys delivered to an *elevated*
//! foreground process are invisible to a non-elevated hook. uvie-mac works
//! everywhere because event taps live in the window server; on Windows the
//! fix is running uvie-win elevated (manifest `requireAdministrator`) or the
//! UIAccess + signed-binary route — tracked in the roadmap.

use std::cell::RefCell;

use uvie_core::keys::{KeyEvent, KeyKind};
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyboardLayout, GetKeyboardState, ToUnicodeEx, VIRTUAL_KEY, VK_BACK,
    VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_MENU, VK_NEXT,
    VK_PACKET, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetForegroundWindow, GetWindowThreadProcessId, SetWindowsHookExW,
    UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, LLKHF_ALTDOWN, LLKHF_EXTENDED, LLKHF_INJECTED,
    WH_KEYBOARD_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
};

type KeyHandler = RefCell<Option<Box<dyn FnMut(KeyEvent) -> bool>>>;

thread_local! {
    /// The per-thread key handler installed by `App`. The LL hook proc runs
    /// on the same thread that pumps messages, so a thread_local is sound.
    static HANDLER: KeyHandler = const { RefCell::new(None) };
}

pub struct KeyboardHook(HHOOK);

impl KeyboardHook {
    /// `handler` returns true to consume the key.
    pub fn install(handler: impl FnMut(KeyEvent) -> bool + 'static) -> windows::core::Result<Self> {
        HANDLER.with(|h| {
            *h.borrow_mut() = Some(Box::new(handler));
        });
        let hhook = unsafe {
            // A null module handle is valid for LL hooks whose proc lives in
            // this exe (the hook proc is invoked on our own message loop).
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0)?
        };
        Ok(Self(hhook))
    }
}

impl Drop for KeyboardHook {
    fn drop(&mut self) {
        unsafe {
            let _ = UnhookWindowsHookEx(self.0);
        }
        HANDLER.with(|h| *h.borrow_mut() = None);
    }
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let msg = wparam.0 as u32;
        if msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN {
            let k = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            let event = translate(k);
            let consume = HANDLER
                .try_with(|h| match h.try_borrow_mut() {
                    Ok(mut g) => g.as_mut().map(|f| f(event)).unwrap_or(false),
                    Err(_) => false,
                })
                .unwrap_or(false);
            if consume {
                return LRESULT(1);
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

/// Modifier-key VK codes we classify but never feed.
fn is_modifier_vk(vk: u16) -> bool {
    matches!(vk, 0x10..=0x12 | 0x5B | 0x5C | 0xA0..=0xA5)
}

fn key_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(vk.0 as i32) & 0x8000u16 as i16 != 0 }
}

fn translate(k: &KBDLLHOOKSTRUCT) -> KeyEvent {
    let vk = VIRTUAL_KEY(k.vkCode as u16);
    let ctrl = key_down(VK_CONTROL);
    let alt = k.flags.contains(LLKHF_ALTDOWN) || key_down(VK_MENU);

    let kind = match vk {
        VK_BACK => KeyKind::Backspace,
        VK_RETURN | VK_TAB | VK_ESCAPE => KeyKind::Break,
        VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN | VK_HOME | VK_END | VK_PRIOR | VK_NEXT
        | VK_DELETE => KeyKind::Navigation,
        _ if is_modifier_vk(vk.0) => KeyKind::Other,
        // Unicode-packet input (KEYEVENTF_UNICODE: automation, remote
        // desktops, password-manager auto-type) carries the UTF-16 code
        // unit in scanCode; vkCode is always VK_PACKET.
        VK_PACKET => match char::from_u32(k.scanCode) {
            Some(c) => KeyKind::Char(c),
            None => KeyKind::Other,
        },
        _ => match vk_to_char(vk, k.scanCode, k.flags.contains(LLKHF_EXTENDED)) {
            Some(c) => KeyKind::Char(c),
            None => KeyKind::Other,
        },
    };

    KeyEvent {
        kind,
        ctrl,
        alt,
        // Only treat *our own* SendInput echoes as injected — injected
        // keystrokes from VNC/RDP/automation must still compose.
        injected: k.flags.contains(LLKHF_INJECTED)
            && k.dwExtraInfo == crate::inject::UVIE_EXTRA_INFO,
    }
}

/// Resolve a VK to the character it produces under the foreground window's
/// keyboard layout (input layouts are per-focus).
fn vk_to_char(vk: VIRTUAL_KEY, scan: u32, extended: bool) -> Option<char> {
    unsafe {
        let mut state = [0u8; 256];
        if GetKeyboardState(&mut state).is_err() {
            return None;
        }
        let fg = GetForegroundWindow();
        let tid = GetWindowThreadProcessId(fg, None);
        let hkl = GetKeyboardLayout(tid);
        let mut buf = [0u16; 8];
        let scan = if extended { scan | 0xE000 } else { scan };
        let n = ToUnicodeEx(vk.0 as u32, scan, &state, &mut buf, 0, Some(hkl));
        match n.cmp(&0) {
            std::cmp::Ordering::Greater => {
                String::from_utf16(&buf[..n as usize]).ok()?.chars().next()
            }
            // Dead key (n < 0): the layout buffers a diacritic — treat as no
            // char; the composed result arrives with the following key.
            _ => None,
        }
    }
}
