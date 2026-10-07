//! Headless tests in the same spirit as uvie-mac's EngineTypingTests /
//! DispatcherTests / HelperTests: drive the real engine through the
//! dispatcher and assert the reconstructed on-screen text.

use crate::dispatcher::{Context, Dispatch, Dispatcher, InputAction, InputLanguage};
use crate::engine_session::{EngineOptions, EngineSession};
use crate::keys::KeyEvent;
use crate::memory::LanguageMemory;
use crate::settings::{LanguagePref, Settings};
use uvie::InputMethod;

fn dispatcher() -> Dispatcher {
    Dispatcher::new(EngineSession::with_options(
        InputMethod::Telex,
        EngineOptions {
            english_override: true,
            ..Default::default()
        },
    ))
}

fn apply(screen: &mut String, plan: &[InputAction]) {
    for action in plan {
        match action {
            InputAction::Backspace(n) => {
                for _ in 0..*n {
                    screen.pop();
                }
            }
            InputAction::ForwardDelete(_) => {}
            InputAction::Text(t) => screen.push_str(t),
        }
    }
}

/// Replay `text` like a real hook: consumed plans mutate the screen,
/// passed keys append themselves.
fn replay(d: &mut Dispatcher, text: &str) -> String {
    let mut screen = String::new();
    for ch in text.chars() {
        match d.handle(KeyEvent::char(ch), &Context::default()) {
            Dispatch::Pass(plan) => {
                apply(&mut screen, &plan);
                screen.push(ch);
            }
            Dispatch::Consume(plan) => apply(&mut screen, &plan),
        }
    }
    screen
}

#[test]
fn telex_tones_and_modifiers() {
    let mut d = dispatcher();
    // Classic Telex: 's'->sắc, 'f'->huyền, 'r'->hỏi, 'x'->ngã, 'j'->nặng,
    // double vowels for circumflex, 'w' for breve/horn.
    assert_eq!(replay(&mut d, "vieetj "), "việt ");
    assert_eq!(replay(&mut d, "xin chaof "), "xin chào ");
    assert_eq!(replay(&mut d, "dduowjc "), "được ");
    assert_eq!(replay(&mut d, "tieengs "), "tiếng ");
}

#[test]
fn backspace_walks_back_through_composition() {
    let mut d = dispatcher();
    let mut screen = String::new();
    for ch in "vie".chars() {
        match d.handle(KeyEvent::char(ch), &Context::default()) {
            Dispatch::Consume(plan) | Dispatch::Pass(plan) => apply(&mut screen, &plan),
        }
    }
    // Backspace removes the last raw key; engine re-renders the remainder.
    match d.handle(KeyEvent::backspace(), &Context::default()) {
        Dispatch::Consume(plan) => apply(&mut screen, &plan),
        Dispatch::Pass(plan) => {
            apply(&mut screen, &plan);
            screen.pop();
        }
    }
    assert_eq!(screen, "vi");
}

#[test]
fn excluded_app_passes_keys_untouched() {
    let mut d = dispatcher();
    let ctx = Context {
        app_excluded: true,
        ..Default::default()
    };
    for ch in "viets".chars() {
        assert!(matches!(
            d.handle(KeyEvent::char(ch), &ctx),
            Dispatch::Pass(_)
        ));
    }
}

#[test]
fn english_mode_passes_keys_untouched() {
    let mut d = dispatcher();
    d.set_language(InputLanguage::English);
    for ch in "viets".chars() {
        assert!(matches!(
            d.handle(KeyEvent::char(ch), &Context::default()),
            Dispatch::Pass(_)
        ));
    }
}

#[test]
fn ctrl_chord_passes_and_resets() {
    let mut d = dispatcher();
    let _ = replay(&mut d, "vie");
    let mut chord = KeyEvent::char('c');
    chord.ctrl = true;
    assert!(matches!(
        d.handle(chord, &Context::default()),
        Dispatch::Pass(_)
    ));
    assert!(!d.session.is_composing());
}

#[test]
fn injected_echoes_are_passed() {
    let mut d = dispatcher();
    let mut key = KeyEvent::char('a');
    key.injected = true;
    assert!(matches!(
        d.handle(key, &Context::default()),
        Dispatch::Pass(_)
    ));
    assert!(!d.session.is_composing());
}

#[test]
fn settings_json_roundtrip() {
    let s = Settings {
        input_method: InputMethod::Vni,
        quick_telex: true,
        excluded_apps: vec!["notepad.exe".into()],
        ..Default::default()
    };
    let text = serde_json::to_string(&s).unwrap();
    let back: Settings = serde_json::from_str(&text).unwrap();
    assert_eq!(back, s);
    assert!(back.is_excluded("NOTEPAD.EXE"));
}

#[test]
fn language_memory_roundtrip() {
    let mut mem = LanguageMemory::default();
    mem.remember("Code.EXE", LanguagePref::English);
    assert_eq!(mem.recall("code.exe"), Some(LanguagePref::English));
    assert_eq!(mem.recall("notepad.exe"), None);
}

#[test]
fn macro_lookup() {
    let mut t = crate::macros::MacroTable::default();
    t.add("sg", "Sài Gòn");
    t.add("sg", "Sài Gòn mới"); // replace, not duplicate
    assert_eq!(t.lookup("sg"), Some("Sài Gòn mới"));
    assert_eq!(t.entries.len(), 1);
    assert_eq!(t.lookup("hn"), None);
}
