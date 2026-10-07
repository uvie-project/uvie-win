//! Text macros: user-defined trigger → expansion pairs, applied when a
//! word boundary commits a matching trigger word (mirrors uvie-mac's
//! MacroManager).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacroEntry {
    /// The literal keystroke sequence that triggers expansion (e.g. `sg`).
    pub trigger: String,
    /// The replacement text (e.g. `Sài Gòn`).
    pub expansion: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MacroTable {
    pub entries: Vec<MacroEntry>,
}

impl MacroTable {
    pub fn new(entries: Vec<MacroEntry>) -> Self {
        Self { entries }
    }

    pub fn add(&mut self, trigger: impl Into<String>, expansion: impl Into<String>) {
        let trigger = trigger.into();
        self.entries.retain(|e| e.trigger != trigger);
        self.entries.push(MacroEntry {
            trigger,
            expansion: expansion.into(),
        });
    }

    pub fn remove(&mut self, trigger: &str) {
        self.entries.retain(|e| e.trigger != trigger);
    }

    /// Look up the expansion for a just-committed word.
    pub fn lookup(&self, word: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|e| e.trigger == word)
            .map(|e| e.expansion.as_str())
    }
}
