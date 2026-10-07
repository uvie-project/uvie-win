//! Keystroke dispatcher — the platform-independent half of the keyboard
//! pipeline, mirroring `EventTap.handle()` in uvie-mac.
//!
//! For every key press the dispatcher decides whether to let it through to
//! the foreground app (`Pass`) or to consume it and synthesize output
//! (`Consume`), and produces the `InjectionPlan` the platform injector
//! (SendInput) executes.

use crate::engine_session::EngineSession;
use crate::keys::{is_feedable_ascii, KeyEvent, KeyKind};
use crate::macros::MacroTable;

/// A single action for the host injector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputAction {
    /// Send N Backspace keystrokes.
    Backspace(usize),
    /// Send N forward-Delete keystrokes (mid-word post-commit edits).
    ForwardDelete(usize),
    /// Type UTF-16 text via unicode key events.
    Text(String),
}

/// What the host should do with a key press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dispatch {
    /// Let the original key reach the application; optionally emit text
    /// alongside (commit flush). `bool` = whether plan is non-empty.
    Pass(Vec<InputAction>),
    /// Swallow the original key and emit this replacement output instead.
    Consume(Vec<InputAction>),
}

impl Dispatch {
    pub fn pass() -> Self {
        Dispatch::Pass(Vec::new())
    }
}

/// Which input language the user currently wants, mirroring uvie-mac's
/// Vi/En toggle (per-app remembered).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputLanguage {
    Vietnamese,
    English,
}

/// Per-key decision context supplied by the host.
#[derive(Debug, Clone, Copy, Default)]
pub struct Context {
    /// Global Vi/En state for the foreground app.
    pub language: Option<InputLanguage>,
    /// The foreground app is on the user exclusion list.
    pub app_excluded: bool,
    /// The active keyboard layout is non-Latin (IME passes everything).
    pub non_latin_layout: bool,
}

/// The dispatcher owns the engine session and applies user settings.
pub struct Dispatcher {
    pub session: EngineSession,
    pub language: InputLanguage,
    pub macros: MacroTable,
    /// When true, auto-capitalize the first letter of a sentence (uvie-mac
    /// "sentence start" behavior — placeholder flag; implementation TODO).
    pub auto_capitalize: bool,
    /// Caret distance back to the end of the newest committed word, used to
    /// arm LabanKey-style post-commit editing (`edit_at`). Stays 0 until the
    /// host implements caret tracking (UI Automation / TSF on Windows).
    caret_back: usize,
}

impl Dispatcher {
    pub fn new(session: EngineSession) -> Self {
        Self {
            session,
            language: InputLanguage::Vietnamese,
            macros: MacroTable::default(),
            auto_capitalize: false,
            caret_back: 0,
        }
    }

    /// Toggle Vi/En. Switching clears engine state — mirrors uvie-mac.
    pub fn set_language(&mut self, lang: InputLanguage) {
        if self.language != lang {
            self.language = lang;
            self.session.reset();
            self.caret_back = 0;
        }
    }

    /// Handle one key press. `ctx` carries host state (excluded app, layout).
    pub fn handle(&mut self, key: KeyEvent, ctx: &Context) -> Dispatch {
        // Our own synthesized output echoing back through the hook.
        if key.injected {
            return Dispatch::pass();
        }

        // Shortcut chords (Ctrl+C, Alt+Tab, ...) never reach the engine and
        // cancel any composing word, like uvie-mac does.
        if key.ctrl || key.alt {
            self.session.reset();
            self.caret_back = 0;
            return Dispatch::pass();
        }

        let lang = ctx.language.unwrap_or(self.language);
        if ctx.app_excluded || ctx.non_latin_layout || lang == InputLanguage::English {
            // Disabled paths still keep the engine clean so returning to Vi
            // mode doesn't resurrect stale state.
            self.session.reset();
            self.caret_back = 0;
            return Dispatch::pass();
        }

        match key.kind {
            KeyKind::Char(c) => self.handle_char(c),
            KeyKind::Backspace => self.handle_backspace(),
            KeyKind::Break => self.handle_break(),
            KeyKind::Navigation => {
                // Caret moved: commit the in-flight word (standard IME
                // behavior) and pass the key through. Committed history is
                // preserved so `edit_at` stays usable once caret tracking
                // (UIA/TSF) lands — see AGENTS.md roadmap.
                let (bs, suffix) = self.session.commit();
                let mut plan = Vec::new();
                if bs > 0 {
                    plan.push(InputAction::Backspace(bs));
                }
                if !suffix.is_empty() {
                    plan.push(InputAction::Text(suffix));
                }
                Dispatch::Pass(plan)
            }
            KeyKind::Other => Dispatch::pass(),
        }
    }

    fn handle_char(&mut self, c: char) -> Dispatch {
        if !is_feedable_ascii(c) {
            self.session.reset();
            return Dispatch::pass();
        }

        // Word-terminating printable key (space/punctuation): commit first.
        if crate::keys::is_word_break_char(c) {
            let (bs, suffix) = self.session.commit();
            let mut plan = Vec::new();
            if bs > 0 {
                plan.push(InputAction::Backspace(bs));
            }
            if !suffix.is_empty() {
                plan.push(InputAction::Text(suffix));
            }
            // TODO(macros): check the just-committed word against
            // `self.macros` and rewrite it when a trigger matches.
            self.caret_back = 0;
            return Dispatch::Pass(plan);
        }

        // If the caret had walked back onto a committed word, this char may be
        // a post-commit tone/edit key (LabanKey-style).
        if self.caret_back > 0 {
            if let Some((bs_plus_one, fwd, suffix)) = self.session.edit_at(self.caret_back, c) {
                let mut plan = Vec::new();
                if fwd > 0 {
                    plan.push(InputAction::ForwardDelete(fwd));
                }
                plan.push(InputAction::Backspace(bs_plus_one));
                plan.push(InputAction::Text(suffix));
                self.caret_back = 0;
                return Dispatch::Consume(plan);
            }
            self.caret_back = 0;
        }

        let (bs, suffix) = self.session.feed(c);
        let mut plan = Vec::new();
        if bs > 0 {
            plan.push(InputAction::Backspace(bs));
        }
        plan.push(InputAction::Text(suffix));
        Dispatch::Consume(plan)
    }

    fn handle_backspace(&mut self) -> Dispatch {
        let (bs, suffix) = self.session.backspace();
        if bs == 0 && suffix.is_empty() && !self.session.is_composing() {
            // Nothing buffered — a real Backspace should reach the app.
            return Dispatch::pass();
        }
        let mut plan = Vec::new();
        if bs > 0 {
            plan.push(InputAction::Backspace(bs));
        }
        if !suffix.is_empty() {
            plan.push(InputAction::Text(suffix));
        }
        Dispatch::Consume(plan)
    }

    fn handle_break(&mut self) -> Dispatch {
        let (bs, suffix) = self.session.commit();
        self.caret_back = 0;
        let mut plan = Vec::new();
        if bs > 0 {
            plan.push(InputAction::Backspace(bs));
        }
        if !suffix.is_empty() {
            plan.push(InputAction::Text(suffix));
        }
        Dispatch::Pass(plan)
    }
}
