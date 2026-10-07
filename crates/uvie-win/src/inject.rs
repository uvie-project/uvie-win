//! Text injection via `SendInput` — the Windows counterpart of uvie-mac's
//! `SyntheticOutput`/CGEvent posting.
//!
//! Every injected event is marked with `UVIE_EXTRA_INFO` in `dwExtraInfo`
//! *and* arrives back through our hook with `LLKHF_INJECTED` set, so echoes
//! are reliably identified by the dispatcher.

use uvie_core::dispatcher::InputAction;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_BACK, VK_DELETE,
};

/// Magic `dwExtraInfo` tag for our own synthesized keystrokes.
const UVIE_EXTRA_INFO: usize = 0x5556_4945; // "UVIE"

pub fn inject(plan: &[InputAction]) {
    let mut inputs: Vec<INPUT> = Vec::new();
    for action in plan {
        match action {
            InputAction::Backspace(n) => {
                for _ in 0..*n {
                    push_key(inputs.as_mut(), VK_BACK);
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
    if inputs.is_empty() {
        return;
    }
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
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
