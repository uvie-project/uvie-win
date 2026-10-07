//! uvie-core — platform-independent core of UVie for Windows.
//!
//! Mirrors uvie-mac's `Core/` + `Features/` layers: it owns the `uvie`
//! engine session, decides how each keystroke is dispatched, and holds the
//! user settings / macros / per-app memory that the Win32 front end
//! (`uvie-win`) and the WinUI 3 settings window (`uvie-winui`) share.
//!
//! The crate is intentionally free of Windows API calls so all dispatch and
//! typing behavior is unit-testable on any platform — the same split that
//! lets uvie-mac run `DispatcherTests`/`EngineTypingTests` headlessly.

pub mod dispatcher;
pub mod engine_session;
pub mod keys;
pub mod macros;
pub mod memory;
pub mod settings;

pub use dispatcher::{Context, Dispatch, Dispatcher, InputAction, InputLanguage};
pub use engine_session::{EngineOptions, EngineSession};
pub use keys::{KeyEvent, KeyKind};
pub use macros::{MacroEntry, MacroTable};
pub use memory::LanguageMemory;
pub use settings::{Hotkey, LanguagePref, Settings};
pub use uvie::InputMethod;

#[cfg(test)]
mod tests;
