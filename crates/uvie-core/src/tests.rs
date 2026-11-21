//! Windows settings-schema tests plus an engine-pin smoke suite that drives
//! the shared `uvie-host` dispatcher the same way the app does
//! (`Shared::apply_engine_options` in crates/uvie-win/src/app.rs over the
//! default `Settings`), replaying real Telex sequences validated against
//! uvie-rs's own test vectors at the pinned tag.

use crate::settings::{Hotkey, Settings};
use crate::{
    Context, Dispatch, Dispatcher, EngineOptions, EngineSession, InputAction, InputLanguage,
    KeyEvent, MacroTable,
};
use uvie_host::InputMethod;

fn temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("uvie-core-test-{}-{name}", std::process::id()))
}

#[test]
fn settings_json_roundtrip() {
    let s = Settings {
        input_method: InputMethod::Vni,
        quick_telex: true,
        excluded_apps: vec!["notepad.exe".into()],
        macro_enabled: true,
        chromium_apps: vec!["mybrowser.exe".into()],
        ..Default::default()
    };
    let text = serde_json::to_string(&s).unwrap();
    let back: Settings = serde_json::from_str(&text).unwrap();
    assert_eq!(back, s);
    assert!(back.is_excluded("NOTEPAD.EXE"));
}

#[test]
fn settings_defaults_ship_chromium_list() {
    // Mirrors uvie-mac's `defaultChromiumBrowsers`: a fresh profile knows
    // the mainstream Chromium browsers and gets the overwrite workaround.
    let s = Settings::default();
    for exe in ["chrome.exe", "msedge.exe", "brave.exe", "arc.exe"] {
        assert!(s.is_chromium(exe), "{exe} should default to chromium list");
    }
    assert!(!s.is_chromium("notepad.exe"));
    assert!(!s.is_chromium("code.exe"));
    // Case-insensitive, like is_excluded.
    let s = Settings {
        chromium_apps: vec!["MyBrowser.EXE".into()],
        ..Default::default()
    };
    assert!(s.is_chromium("mybrowser.exe"));
}

#[test]
fn settings_load_missing_file_returns_defaults() {
    let s = Settings::load(&temp_path("does-not-exist.json"));
    assert_eq!(s, Settings::default());
}

#[test]
fn settings_load_corrupt_json_returns_defaults() {
    let path = temp_path("corrupt.json");
    std::fs::write(&path, "{ not valid json !!!").unwrap();
    let s = Settings::load(&path);
    std::fs::remove_file(&path).ok();
    assert_eq!(s, Settings::default());
}

#[test]
fn settings_save_load_roundtrip_through_file() {
    let path = temp_path("roundtrip.json");
    let s = Settings {
        input_method: InputMethod::SimpleTelex,
        auto_capitalize: true,
        excluded_apps: vec!["a.exe".into(), "B.exe".into()],
        hotkey: Hotkey {
            ctrl: false,
            alt: true,
            shift: true,
            vk: 0x31,
        },
        ..Default::default()
    };
    s.save(&path).unwrap();
    let back = Settings::load(&path);
    std::fs::remove_file(&path).ok();
    assert_eq!(back, s);
}

#[test]
fn settings_partial_json_fills_defaults() {
    // `#[serde(default)]`: a file with a single field keeps every other
    // setting at its default.
    let back: Settings = serde_json::from_str(r#"{"auto_capitalize": true}"#).unwrap();
    assert!(back.auto_capitalize);
    let d = Settings::default();
    assert_eq!(back.enabled, d.enabled);
    assert!(back.input_method == d.input_method);
    assert_eq!(back.excluded_apps, d.excluded_apps);
    assert_eq!(back.chromium_apps, d.chromium_apps);
}

#[test]
fn settings_unknown_fields_are_tolerated() {
    // No `deny_unknown_fields`: forward-compatible with settings written by
    // a newer build.
    let back: Settings =
        serde_json::from_str(r#"{"future_option": 42, "enabled": false}"#).unwrap();
    assert!(!back.enabled);
}

#[test]
fn settings_input_method_serializes_by_display_name() {
    for (method, name) in [
        (InputMethod::Telex, "telex"),
        (InputMethod::Vni, "vni"),
        (InputMethod::SimpleTelex, "simple-telex"),
    ] {
        let s = Settings {
            input_method: method,
            ..Default::default()
        };
        let text = serde_json::to_string(&s).unwrap();
        assert!(
            text.contains(&format!("\"input_method\":\"{name}\"")),
            "{text}"
        );
        let back: Settings = serde_json::from_str(&text).unwrap();
        assert!(back.input_method == method);
    }
}

#[test]
fn settings_input_method_unknown_name_falls_back_to_telex() {
    // `input_method_serde` maps anything unrecognized to Telex rather than
    // failing the whole parse.
    let back: Settings = serde_json::from_str(r#"{"input_method":"qwerty"}"#).unwrap();
    assert!(back.input_method == InputMethod::Telex);
    // And the accepted SimpleTelex aliases.
    for alias in ["simpletelex", "simple"] {
        let back: Settings =
            serde_json::from_str(&format!(r#"{{"input_method":"{alias}"}}"#)).unwrap();
        assert!(back.input_method == InputMethod::SimpleTelex, "{alias}");
    }
}

#[test]
fn settings_is_excluded_is_case_insensitive() {
    let s = Settings {
        excluded_apps: vec!["Notepad.EXE".into()],
        ..Default::default()
    };
    assert!(s.is_excluded("notepad.exe"));
    assert!(s.is_excluded("NOTEPAD.EXE"));
    assert!(!s.is_excluded("notepad2.exe"));
}

#[test]
fn hotkey_default_is_ctrl_shift_z() {
    let h = Hotkey::default();
    assert!(h.ctrl);
    assert!(h.shift);
    assert!(!h.alt);
    assert_eq!(h.vk, 0x5A); // VK_Z
}

#[test]
fn hotkey_partial_json_fills_defaults() {
    // `#[serde(default)]` on Hotkey: a hand-edited object missing a field
    // must not reset the whole Settings parse.
    let back: Settings = serde_json::from_str(r#"{"hotkey": {"vk": 65}}"#).unwrap();
    assert_eq!(back.hotkey.vk, 65);
    assert!(back.hotkey.ctrl);
    assert!(back.hotkey.shift);
    assert!(!back.hotkey.alt);
}

// MARK: - Engine-pin smoke through the crate re-exports
//
// The app builds its dispatcher via Shared::apply_engine_options over the
// current Settings; mirror it with the *default* Settings (english_override
// on, every other engine flag off).

fn dispatcher() -> Dispatcher {
    let s = Settings::default();
    Dispatcher::new(EngineSession::with_options(
        s.input_method,
        EngineOptions {
            quick_start: s.quick_start,
            quick_telex: s.quick_telex,
            modern_orthography: s.modern_orthography,
            relaxed_coda: s.relaxed_coda,
            english_override: s.english_override,
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

/// Replay `text` like a real event source: consumed plans mutate the
/// screen, passed keys append themselves.
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

fn type_text(d: &mut Dispatcher, text: &str) {
    for ch in text.chars() {
        d.handle(KeyEvent::char(ch), &Context::default());
    }
}

fn expect_consume(d: Dispatch) -> Vec<InputAction> {
    match d {
        Dispatch::Consume(plan) => plan,
        other => panic!("expected Consume, got {other:?}"),
    }
}

fn expect_pass(d: Dispatch) -> Vec<InputAction> {
    match d {
        Dispatch::Pass(plan) => plan,
        other => panic!("expected Pass, got {other:?}"),
    }
}

fn text(s: &str) -> InputAction {
    InputAction::Text(s.to_string())
}

#[test]
fn engine_telex_vectors() {
    let mut d = dispatcher();
    assert_eq!(replay(&mut d, "chaof "), "chào ");
    assert_eq!(replay(&mut d, "vieetj "), "việt ");
    assert_eq!(replay(&mut d, "dduwowcj "), "được ");
    assert_eq!(replay(&mut d, "tos "), "tó ");
}

#[test]
fn engine_telex_multi_word_sentence() {
    let mut d = dispatcher();
    assert_eq!(replay(&mut d, "xin chaof vieetj "), "xin chào việt ");
}

#[test]
fn engine_vni_vector() {
    let mut d = dispatcher();
    d.session.set_input_method(InputMethod::Vni);
    // VNI: 6 = circumflex, 5 = nặng (per uvie-rs tests: a6→â, a5→ạ).
    assert_eq!(replay(&mut d, "vie65t "), "việt ");
}

#[test]
fn engine_backspace_while_composing_renders() {
    let mut d = dispatcher();
    let mut screen = String::new();
    for ch in "vie".chars() {
        match d.handle(KeyEvent::char(ch), &Context::default()) {
            Dispatch::Consume(plan) | Dispatch::Pass(plan) => apply(&mut screen, &plan),
        }
    }
    let plan = expect_consume(d.handle(KeyEvent::backspace(), &Context::default()));
    apply(&mut screen, &plan);
    assert_eq!(screen, "vi");
}

#[test]
fn engine_english_mode_passes_keys_untouched() {
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
fn engine_excluded_app_passes_keys_untouched() {
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
fn engine_ctrl_chord_passes_and_resets() {
    let mut d = dispatcher();
    type_text(&mut d, "vie");
    let mut chord = KeyEvent::char('c');
    chord.ctrl = true;
    assert!(matches!(
        d.handle(chord, &Context::default()),
        Dispatch::Pass(_)
    ));
    assert!(!d.session.is_composing());
}

#[test]
fn engine_injected_keys_pass_through() {
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
fn engine_escape_restores_raw_while_composing() {
    // "vieet" renders "viêt"; Escape erases the render and posts the raw
    // keystrokes back, consuming the key.
    let mut d = dispatcher();
    type_text(&mut d, "vieet");
    let plan = expect_consume(d.handle(KeyEvent::escape(), &Context::default()));
    assert_eq!(plan, vec![InputAction::Backspace(4), text("vieet")]);
    assert!(!d.session.is_composing());
}

#[test]
fn engine_macro_expands_on_space() {
    let mut d = dispatcher();
    d.macro_enabled = true;
    d.macros.add("sg", "Sài Gòn");
    // The space itself is consumed - the expansion replaces
    // abbreviation+terminator.
    assert_eq!(replay(&mut d, "sg "), "Sài Gòn");
}

#[test]
fn engine_macro_disabled_leaves_abbreviation_alone() {
    let mut d = dispatcher();
    d.macros.add("sg", "Sài Gòn");
    assert_eq!(replay(&mut d, "sg "), "sg ");
}

#[test]
fn engine_auto_capitalize_after_sentence_end() {
    let mut d = dispatcher();
    d.auto_capitalize = true;
    type_text(&mut d, "hello");
    expect_pass(d.handle(KeyEvent::char('.'), &Context::default()));
    expect_pass(d.handle(KeyEvent::char(' '), &Context::default()));
    let plan = expect_consume(d.handle(KeyEvent::char('a'), &Context::default()));
    assert_eq!(plan, vec![text("A")]);
}

#[test]
fn engine_edit_committed_word_tone_key_replaces_in_place() {
    // Verbatim post-commit-edit case from uvie-host tests (v3.1.1):
    // "don" + space commits; arrow-left onto the word end; 's' re-renders
    // it as "dón".
    let mut d = dispatcher();
    type_text(&mut d, "don");
    expect_pass(d.handle(KeyEvent::char(' '), &Context::default()));
    expect_pass(d.handle(KeyEvent::left(), &Context::default()));
    let plan = expect_consume(d.handle(KeyEvent::char('s'), &Context::default()));
    assert_eq!(plan, vec![InputAction::Backspace(2), text("ón")]);
}

#[test]
fn engine_macro_table_load_missing_file_is_empty() {
    let t = MacroTable::load(&temp_path("no-macros.json"));
    assert!(t.lookup("sg").is_none());
}

// MARK: - v3.1.3 cross-word-rewrite pin regressions

/// Feed one key through the dispatcher, apply its plan to `screen` and push
/// the passed-through char for Pass dispatches. Returns the plan.
fn feed_key(
    d: &mut Dispatcher,
    screen: &mut String,
    key: KeyEvent,
    pass: Option<char>,
) -> Vec<InputAction> {
    match d.handle(key, &Context::default()) {
        Dispatch::Consume(plan) => {
            apply(screen, &plan);
            plan
        }
        Dispatch::Pass(plan) => {
            apply(screen, &plan);
            if let Some(c) = pass {
                screen.push(c);
            }
            plan
        }
    }
}

fn feed_str(d: &mut Dispatcher, screen: &mut String, text: &str) {
    for ch in text.chars() {
        feed_key(d, screen, KeyEvent::char(ch), Some(ch));
    }
}

/// A passed Backspace deletes one char natively on the app side.
fn feed_backspace(d: &mut Dispatcher, screen: &mut String) {
    match d.handle(KeyEvent::backspace(), &Context::default()) {
        Dispatch::Consume(plan) => apply(screen, &plan),
        Dispatch::Pass(plan) => {
            apply(screen, &plan);
            screen.pop();
        }
    }
}

#[test]
fn pin_regression_apostrophe_deleted_keeps_vietnamese() {
    // "vieetj '" then Backspace drops the apostrophe: the surviving
    // separator is unknowable, so 's' must not read as a contraction.
    let mut d = dispatcher();
    let mut screen = String::new();
    feed_str(&mut d, &mut screen, "vieetj '");
    feed_backspace(&mut d, &mut screen);
    feed_str(&mut d, &mut screen, "s ");
    assert_eq!(screen, "việt s ");
}

#[test]
fn pin_regression_overflowed_word_contraction() {
    // A word longer than the 24-key raw buffer only knows its tail; the
    // contraction rewrite must not rebuild it.
    let mut d = dispatcher();
    assert_eq!(
        replay(&mut d, "getCurrentApplicationSettings's "),
        "getCurrentApplicationSettings's "
    );
}

#[test]
fn pin_regression_comma_survives_separator_delete() {
    // "this, " losing the space leaves "this,": the comma is a mixed-gap
    // leftover, so no rewrite may replace it. "thí" stays Vietnamese
    // because its separator is unknowable (typing the comma directly knows
    // it and restores "this").
    let mut d = dispatcher();
    let mut screen = String::new();
    feed_str(&mut d, &mut screen, "this, ");
    feed_backspace(&mut d, &mut screen);
    feed_str(&mut d, &mut screen, "is a test ");
    assert_eq!(screen, "thí,is a test ");
}

#[test]
fn pin_regression_enter_never_rewrites_across_line() {
    let mut d = dispatcher();
    let mut screen = String::new();
    feed_str(&mut d, &mut screen, "this");
    feed_key(&mut d, &mut screen, KeyEvent::enter(), Some('\n'));
    feed_str(&mut d, &mut screen, "test");
    let plan = feed_key(&mut d, &mut screen, KeyEvent::char(' '), Some(' '));
    for action in &plan {
        match action {
            // "test" renders at most 4 screen chars; a larger erase reaches
            // past the newline into the previous line.
            InputAction::Backspace(n) => assert!(*n <= 4, "plan {plan:?}"),
            InputAction::Text(t) => assert!(!t.contains('\n'), "plan {plan:?}"),
            _ => {}
        }
    }
    assert_eq!(screen.lines().next_back().unwrap(), "test ");
}

#[test]
fn pin_regression_edit_committed_off_bounds_backspaces() {
    let mut d = dispatcher();
    // ibus/fcitx-style host: committed text cannot be rewritten.
    d.edit_committed = false;
    let mut screen = String::new();
    let mut word_len = 0usize;
    for ch in "this is a test ".chars() {
        let plan = feed_key(&mut d, &mut screen, KeyEvent::char(ch), Some(ch));
        for action in &plan {
            if let InputAction::Backspace(n) = action {
                // No plan may erase past the word typed so far.
                assert!(*n <= word_len, "plan {plan:?} at {ch:?}");
            }
        }
        if ch == ' ' {
            word_len = 0;
        } else {
            word_len += 1;
        }
    }
    assert_eq!(screen, "thí í a test ");
}

#[test]
fn pin_regression_edit_committed_on_restores_sentence() {
    let mut d = dispatcher();
    assert_eq!(replay(&mut d, "this is a test "), "this is a test ");
}
