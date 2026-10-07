//! Text injection via `SendInput` — the Windows counterpart of uvie-mac's
//! `SyntheticOutput`/CGEvent posting.
//!
//! Every injected event is marked with `UVIE_EXTRA_INFO` in `dwExtraInfo`
//! *and* arrives back through our hook with `LLKHF_INJECTED` set, so echoes
//! are reliably identified by the dispatcher.

use uvie_core::dispatcher::InputAction;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_BACK, VK_DELETE, VK_LEFT, VK_SHIFT,
};

/// Magic `dwExtraInfo` tag for our own synthesized keystrokes. The
/// keyboard hook checks this so that only *our* echoes are filtered —
/// injected input from VNC/RDP/automation still composes normally.
pub(crate) const UVIE_EXTRA_INFO: usize = 0x5556_4945; // "UVIE"

/// Execute a dispatch plan.
///
/// `chromium_mode` enables the select-and-overwrite workaround uvie-mac
/// applies to Chromium browsers: instead of plain Backspaces (which some
/// Chromium text fields — e.g. the omnibox — swallow or reorder), each
/// `Backspace(n)` becomes `Shift+Left × n` to select the text, then the
/// following `Text` action overwrites the selection. When no text follows,
/// a single `Delete` removes the selection.
pub fn inject(plan: &[InputAction], chromium_mode: bool) {
    let inputs = build_inputs(plan, chromium_mode);
    if inputs.is_empty() {
        return;
    }
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

/// Translate a dispatch plan into raw `INPUT` events (testable — the
/// SendInput call is the only un-testable shell around it).
fn build_inputs(plan: &[InputAction], chromium_mode: bool) -> Vec<INPUT> {
    let mut inputs: Vec<INPUT> = Vec::new();
    for (i, action) in plan.iter().enumerate() {
        match action {
            InputAction::Backspace(n) => {
                if chromium_mode {
                    let overwrites = plan[i + 1..]
                        .iter()
                        .any(|a| matches!(a, InputAction::Text(s) if !s.is_empty()));
                    push_select_back(inputs.as_mut(), *n);
                    if !overwrites {
                        push_key(inputs.as_mut(), VK_DELETE);
                    }
                } else {
                    for _ in 0..*n {
                        push_key(inputs.as_mut(), VK_BACK);
                    }
                }
            }
            InputAction::ForwardDelete(n) => {
                for _ in 0..*n {
                    push_key(inputs.as_mut(), VK_DELETE);
                }
            }
            InputAction::Text(s) => push_text(inputs.as_mut(), s),
        }
    }
    inputs
}

/// Select `n` characters to the left of the caret (Shift held, Left × n).
/// A subsequent text input replaces the selection — Chromium workaround.
fn push_select_back(inputs: &mut Vec<INPUT>, n: usize) {
    inputs.push(key_input(VK_SHIFT, 0, KEYBD_EVENT_FLAGS(0)));
    for _ in 0..n {
        push_key(inputs, VK_LEFT);
    }
    inputs.push(key_input(VK_SHIFT, 0, KEYEVENTF_KEYUP));
}

fn key_input(vk: VIRTUAL_KEY, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: UVIE_EXTRA_INFO,
            },
        },
    }
}

fn push_key(inputs: &mut Vec<INPUT>, vk: VIRTUAL_KEY) {
    inputs.push(key_input(vk, 0, KEYBD_EVENT_FLAGS(0)));
    inputs.push(key_input(vk, 0, KEYEVENTF_KEYUP));
}

/// Unicode text via `KEYEVENTF_UNICODE` surrogate pairs — the reliable
/// SendInput path for arbitrary UTF-16 (including precomposed Vietnamese).
fn push_text(inputs: &mut Vec<INPUT>, text: &str) {
    for unit in text.encode_utf16() {
        inputs.push(key_input(VIRTUAL_KEY(0), unit, KEYEVENTF_UNICODE));
        inputs.push(key_input(
            VIRTUAL_KEY(0),
            unit,
            KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Flatten a built INPUT sequence to (vk, scan, flags) triples for
    /// assertions. VK 0 entries carry UTF-16 code units in `scan`.
    fn k(inputs: &[INPUT]) -> Vec<(u16, u16, u32)> {
        inputs
            .iter()
            .map(|i| unsafe {
                let ki = i.Anonymous.ki;
                (ki.wVk.0, ki.wScan, ki.dwFlags.0)
            })
            .collect()
    }

    const UP: u32 = KEYEVENTF_KEYUP.0;
    const UNI: u32 = KEYEVENTF_UNICODE.0;

    #[test]
    fn normal_mode_uses_plain_backspaces() {
        let inputs = build_inputs(&[InputAction::Backspace(2)], false);
        assert_eq!(
            k(&inputs),
            vec![
                (VK_BACK.0, 0, 0),
                (VK_BACK.0, 0, UP),
                (VK_BACK.0, 0, 0),
                (VK_BACK.0, 0, UP),
            ]
        );
    }

    #[test]
    fn chromium_selects_then_deletes_when_nothing_follows() {
        let inputs = build_inputs(&[InputAction::Backspace(2)], true);
        assert_eq!(
            k(&inputs),
            vec![
                (VK_SHIFT.0, 0, 0), // Shift down
                (VK_LEFT.0, 0, 0),
                (VK_LEFT.0, 0, UP),
                (VK_LEFT.0, 0, 0),
                (VK_LEFT.0, 0, UP),
                (VK_SHIFT.0, 0, UP), // Shift up
                (VK_DELETE.0, 0, 0), // delete the selection
                (VK_DELETE.0, 0, UP),
            ]
        );
    }

    #[test]
    fn chromium_selects_then_overwrites_when_text_follows() {
        // uvie-mac's omnibox workaround: Shift+Left selects, the typed text
        // replaces the selection — no Delete key needed.
        let inputs = build_inputs(
            &[InputAction::Backspace(2), InputAction::Text("v".into())],
            true,
        );
        assert_eq!(
            k(&inputs),
            vec![
                (VK_SHIFT.0, 0, 0),
                (VK_LEFT.0, 0, 0),
                (VK_LEFT.0, 0, UP),
                (VK_LEFT.0, 0, 0),
                (VK_LEFT.0, 0, UP),
                (VK_SHIFT.0, 0, UP),
                (0, 'v' as u16, UNI),
                (0, 'v' as u16, UNI | UP),
            ]
        );
    }

    #[test]
    fn forward_delete_and_unicode_text() {
        let inputs = build_inputs(
            &[InputAction::ForwardDelete(1), InputAction::Text("ệ".into())],
            false,
        );
        let ks = k(&inputs);
        assert_eq!(&ks[..2], &[(VK_DELETE.0, 0, 0), (VK_DELETE.0, 0, UP)]);
        // "ệ" is a single UTF-16 code unit (U+1EC7).
        assert_eq!(&ks[2..], &[(0, 0x1EC7, UNI), (0, 0x1EC7, UNI | UP)]);
    }
}
