//! Per-app Vi/En memory — the Windows port of uvie-mac's MemoryManager.
//! Remembers which input language the user last used in each app and
//! restores it on focus change.

use crate::settings::LanguagePref;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct LanguageMemory {
    /// exe name (lowercase, e.g. `code.exe`) → last-used language.
    #[serde(flatten)]
    apps: HashMap<String, LanguagePref>,
}

impl LanguageMemory {
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, text)
    }

    pub fn remember(&mut self, exe_name: &str, lang: LanguagePref) {
        self.apps.insert(exe_name.to_ascii_lowercase(), lang);
    }

    pub fn recall(&self, exe_name: &str) -> Option<LanguagePref> {
        self.apps.get(&exe_name.to_ascii_lowercase()).copied()
    }
}
