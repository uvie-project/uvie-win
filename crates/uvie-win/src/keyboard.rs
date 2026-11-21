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
use windows::core::PCWSTR;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::Globalization::{GetLocaleInfoEx, LCIDToLocaleName, LOCALE_SSCRIPTS};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyboardLayout, GetKeyboardState, ToUnicodeEx, VIRTUAL_KEY, VK_APPS,
    VK_BACK, VK_CAPITAL, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_INSERT,
    VK_LCONTROL, VK_LEFT, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_NEXT, VK_PACKET, VK_PRIOR,
    VK_RCONTROL, VK_RETURN, VK_RIGHT, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT, VK_TAB, VK_UP,
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
    // Only treat *our own* SendInput echoes as injected — injected
    // keystrokes from VNC/RDP/automation must still compose. Echoes skip
    // translation entirely: the dispatcher drops them anyway, and unicode
    // echoes carry garbage scan codes (UTF-16 units, vkCode 0).
    if k.flags.contains(LLKHF_INJECTED) && k.dwExtraInfo == crate::inject::UVIE_EXTRA_INFO {
        return KeyEvent {
            kind: KeyKind::Other,
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
            injected: true,
        };
    }

    let vk = VIRTUAL_KEY(k.vkCode as u16);
    let ctrl = key_down(VK_CONTROL);
    let alt = k.flags.contains(LLKHF_ALTDOWN) || key_down(VK_MENU);
    let win = key_down(VK_LWIN) || key_down(VK_RWIN);
    // The LL hook runs before the async key state is updated for the key that
    // triggered this callback (LowLevelKeyboardProc docs), so this reads the
    // modifier state as of the *previous* event. That is exactly what the
    // dispatcher needs: shift/ctrl/alt on a letter or arrow were pressed
    // before the event being translated.
    let shift = key_down(VK_SHIFT);

    let kind = if ctrl || alt || win {
        // Modifier chords (Ctrl+C, Alt+Tab, Win+R, Win+arrows, ...) are never
        // engine input — the dispatcher resets and passes on the flags alone.
        // Skipping ToUnicodeEx here also avoids needlessly disturbing the
        // kernel dead-key state on every shortcut.
        KeyKind::Other
    } else {
        match vk {
            VK_BACK => KeyKind::Backspace,
            VK_RETURN => KeyKind::Enter,
            VK_TAB => KeyKind::Tab,
            VK_ESCAPE => KeyKind::Escape,
            VK_LEFT => KeyKind::Left,
            VK_RIGHT => KeyKind::Right,
            VK_UP | VK_DOWN | VK_HOME | VK_END | VK_PRIOR | VK_NEXT | VK_INSERT | VK_APPS => {
                KeyKind::Navigation
            }
            VK_DELETE => KeyKind::ForwardDelete,
            _ if is_modifier_vk(vk.0) => KeyKind::Other,
            // Unicode-packet input (KEYEVENTF_UNICODE: automation, remote
            // desktops, password-manager auto-type) carries the UTF-16 code
            // unit in scanCode; vkCode is always VK_PACKET.
            VK_PACKET => match char::from_u32(k.scanCode) {
                Some(c) => KeyKind::Char(c),
                None => KeyKind::Other,
            },
            _ => match vk_to_char(vk, k.scanCode, k.flags.contains(LLKHF_EXTENDED)) {
                Some(VkTranslation::Char(c)) => KeyKind::Char(c),
                Some(VkTranslation::Sequence) => KeyKind::Break,
                None => KeyKind::Other,
            },
        }
    };

    KeyEvent {
        kind,
        ctrl,
        alt,
        shift,
        win,
        injected: false,
    }
}

thread_local! {
    /// Per-HKL script cache — the locale-name + scripts lookup would
    /// otherwise run on every keystroke inside the LL hook.
    static LAYOUT_NON_LATIN: RefCell<Option<(usize, bool)>> = const { RefCell::new(None) };
}

/// True when the foreground thread's input language uses a non-Latin
/// script (Cyrillic, Arabic, CJK IMEs, ...) — the dispatcher then passes
/// every key through untouched (`Context::non_latin_layout`).
///
/// Resolution: the low word of the foreground thread's `HKL` is a LANGID,
/// mapped to a locale name whose `LOCALE_SSCRIPTS` lists its scripts
/// (e.g. "Latn;", "Cyrl;Latn;"). A layout whose script list lacks "Latn"
/// is non-Latin; ambiguous mixed-script languages keep "Latn" and stay
/// enabled. Every failure defaults to Latin: erring toward processing is
/// better than silently disabling the IME on exotic-but-Latin layouts.
/// Note this keys off the input *language* — a Latin layout registered
/// under a non-Latin LANGID can misclassify, but the common direction
/// (CJK/Cyrillic input) is handled.
pub fn active_layout_is_non_latin() -> bool {
    unsafe {
        let fg = GetForegroundWindow();
        let tid = GetWindowThreadProcessId(fg, None);
        let hkl = GetKeyboardLayout(tid);
        if let Some(hit) = LAYOUT_NON_LATIN.with(|c| match *c.borrow() {
            Some((h, non_latin)) if h == hkl.0 as usize => Some(non_latin),
            _ => None,
        }) {
            return hit;
        }
        let non_latin = langid_is_non_latin((hkl.0 as usize & 0xFFFF) as u32);
        LAYOUT_NON_LATIN.with(|c| *c.borrow_mut() = Some((hkl.0 as usize, non_latin)));
        non_latin
    }
}

fn langid_is_non_latin(langid: u32) -> bool {
    unsafe {
        let mut name = [0u16; 85]; // LOCALE_NAME_MAX_LENGTH
                                   // MAKELCID(langid, SORT_DEFAULT) == langid (sort 0).
        if LCIDToLocaleName(langid, Some(&mut name), 0) == 0 {
            return false;
        }
        let mut scripts = [0u16; 160];
        let n = GetLocaleInfoEx(PCWSTR(name.as_ptr()), LOCALE_SSCRIPTS, Some(&mut scripts));
        if n <= 0 || n as usize > scripts.len() {
            return false;
        }
        let len = scripts[..n as usize]
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(n as usize);
        !String::from_utf16_lossy(&scripts[..len])
            .split(';')
            .any(|s| s == "Latn")
    }
}

/// `ToUnicodeEx` wFlags bit 2: do not change the kernel-mode keyboard
/// buffer state (Windows 10 1607+). Without it, every translation we run in
/// the hook mutates the shared dead-key/ligature buffer — e.g. translating a
/// user's `^` dead key on US-International consumes it, and the app renders
/// a plain `e` instead of `ê`.
const TU_DONT_CHANGE_STATE: u32 = 4;

/// What a VK produces under the foreground layout.
enum VkTranslation {
    /// Exactly one character — feedable to the engine.
    Char(char),
    /// Multi-unit output (ligature or combining sequence): passed through
    /// natively, but it still terminates the composing word.
    Sequence,
}

/// Resolve a VK to the character it produces under the foreground window's
/// keyboard layout (input layouts are per-focus).
fn vk_to_char(vk: VIRTUAL_KEY, scan: u32, extended: bool) -> Option<VkTranslation> {
    unsafe {
        let mut state = [0u8; 256];
        if GetKeyboardState(&mut state).is_err() {
            return None;
        }
        // GetKeyboardState reflects this thread's own message-queue state,
        // which can lag the real modifiers by an event (keystrokes queue
        // on the foreground thread, not ours). Overlay the live modifier
        // bits so Shift/CapsLock-dependent translations can't mis-map.
        sync_modifiers(&mut state);
        let fg = GetForegroundWindow();
        let tid = GetWindowThreadProcessId(fg, None);
        let hkl = GetKeyboardLayout(tid);
        let mut buf = [0u16; 8];
        // wScanCode uses the lParam flag layout: bit 8 (KF_EXTENDED)
        // marks extended keys, bit 15 (KF_UP) a release. `0xE000` is the
        // raw scancode-prefix encoding from MAPVK_VSC_TO_VK_EX — passing
        // it here would read as key-up + repeat + alt and translate to 0.
        let scan = if extended { scan | 0x0100 } else { scan };
        let n = ToUnicodeEx(
            vk.0 as u32,
            scan,
            &state,
            &mut buf,
            TU_DONT_CHANGE_STATE,
            Some(hkl),
        );
        match n.cmp(&0) {
            std::cmp::Ordering::Greater => {
                let s = String::from_utf16(&buf[..n as usize]).ok()?;
                match s.chars().count() {
                    1 => Some(VkTranslation::Char(s.chars().next()?)),
                    // Feeding just the first unit of a ligature/combining
                    // sequence would desync engine and screen.
                    _ => Some(VkTranslation::Sequence),
                }
            }
            // Dead key (n < 0): the layout buffers a diacritic — treat as no
            // char; the composed result arrives with the following key.
            _ => None,
        }
    }
}

/// Force the modifier bits of a `GetKeyboardState` array to match the
/// real-time async state (down bit 0x80, toggle bit 0x01 for CapsLock).
fn sync_modifiers(state: &mut [u8; 256]) {
    for vk in [
        VK_SHIFT,
        VK_LSHIFT,
        VK_RSHIFT,
        VK_CONTROL,
        VK_LCONTROL,
        VK_RCONTROL,
        VK_MENU,
        VK_LMENU,
        VK_RMENU,
    ] {
        if key_down(vk) {
            state[vk.0 as usize] |= 0x80;
        } else {
            state[vk.0 as usize] &= !0x80;
        }
    }
    if unsafe { GetAsyncKeyState(VK_CAPITAL.0 as i32) } & 1 != 0 {
        state[VK_CAPITAL.0 as usize] |= 0x01;
    } else {
        state[VK_CAPITAL.0 as usize] &= !0x01;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_modifier_vk_covers_modifier_ranges_only() {
        // VK_SHIFT..VK_MENU (0x10-0x12)
        assert!(!is_modifier_vk(0x0F));
        for vk in 0x10..=0x12 {
            assert!(is_modifier_vk(vk), "vk {vk:#04X}");
        }
        assert!(!is_modifier_vk(0x13));
        // VK_LWIN / VK_RWIN
        assert!(is_modifier_vk(0x5B));
        assert!(is_modifier_vk(0x5C));
        assert!(!is_modifier_vk(0x5A)); // VK_Z is not a modifier
        assert!(!is_modifier_vk(0x5D));
        // VK_LSHIFT..VK_RMENU (0xA0-0xA5)
        assert!(!is_modifier_vk(0x9F));
        for vk in 0xA0..=0xA5 {
            assert!(is_modifier_vk(vk), "vk {vk:#04X}");
        }
        assert!(!is_modifier_vk(0xA6));
        // Ordinary keys are never modifiers.
        for vk in [0x41, 0x30, 0x20, 0x0D, 0x70] {
            assert!(!is_modifier_vk(vk), "vk {vk:#04X}");
        }
    }

    // These call real Win32 locale APIs; they only execute on Windows CI.
    #[test]
    fn langid_is_non_latin_known_langids() {
        // en-US reports "Latn;" -> Latin -> IME enabled.
        assert!(!langid_is_non_latin(0x0409));
        // ru-RU reports "Cyrl;" -> non-Latin -> pass-through.
        assert!(langid_is_non_latin(0x0419));
    }

    #[test]
    fn langid_is_non_latin_unknown_langid_defaults_to_latin() {
        // A bogus LANGID fails LCIDToLocaleName -> treated as Latin (the
        // documented failure direction).
        assert!(!langid_is_non_latin(0xFFFF));
    }
}
