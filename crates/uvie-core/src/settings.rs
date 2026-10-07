//! Persistent user settings — the Windows port of uvie-mac's AppDefaults.
//! Stored as JSON; the host app resolves the file location
//! (`%APPDATA%\UVie\settings.json` on Windows).

use serde::{Deserialize, Serialize};
use std::path::Path;
use uvie::InputMethod;

/// Vi/En preference remembered per app when `per_app_language` is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LanguagePref {
    Vietnamese,
    English,
}

/// Global keyboard shortcut binding (e.g. the Vi/En toggle).
/// Serialized as e.g. `"Ctrl+Shift+V"`; parsing lives in the host crate
/// where VK codes are available.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Virtual-key code (Win32 VK_*).
    pub vk: u32,
}

impl Default for Hotkey {
    fn default() -> Self {
        // Ctrl+Shift+Z — same default spirit as uvie-mac's toggle chord.
        Self {
            ctrl: true,
            alt: false,
            shift: true,
            vk: 0x5A, // VK_Z
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Master on/off switch for the IME.
    pub enabled: bool,
    /// Telex or VNI.
    #[serde(with = "input_method_serde")]
    pub input_method: InputMethod,
    /// `j`→`gi`, `f`→`ph`, `w`→`qu` on word start.
    pub quick_start: bool,
    /// `cc`→`ch` style shorthand.
    pub quick_telex: bool,
    /// Modern orthography tone placement.
    pub modern_orthography: bool,
    /// Relaxed coda rules.
    pub relaxed_coda: bool,
    /// English dictionary override (real English words pass through).
    pub english_override: bool,
    /// Remember Vi/En choice per application.
    pub per_app_language: bool,
    /// Capitalize first letter of sentences.
    pub auto_capitalize: bool,
    /// Launch with Windows (HKCU Run key).
    pub launch_at_login: bool,
    /// Executables the IME ignores entirely (lowercase exe names).
    pub excluded_apps: Vec<String>,
    /// Vi/En toggle hotkey.
    pub hotkey: Hotkey,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            input_method: InputMethod::Telex,
            quick_start: false,
            quick_telex: false,
            modern_orthography: false,
            relaxed_coda: false,
            english_override: true,
            per_app_language: true,
            auto_capitalize: false,
            launch_at_login: false,
            excluded_apps: Vec::new(),
            hotkey: Hotkey::default(),
        }
    }
}

impl Settings {
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

    pub fn is_excluded(&self, exe_name: &str) -> bool {
        let exe = exe_name.to_ascii_lowercase();
        self.excluded_apps.iter().any(|e| e == &exe)
    }
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("enabled", &self.enabled)
            .field("quick_start", &self.quick_start)
            .field("quick_telex", &self.quick_telex)
            .field("modern_orthography", &self.modern_orthography)
            .field("relaxed_coda", &self.relaxed_coda)
            .field("english_override", &self.english_override)
            .field("per_app_language", &self.per_app_language)
            .field("auto_capitalize", &self.auto_capitalize)
            .field("launch_at_login", &self.launch_at_login)
            .field("excluded_apps", &self.excluded_apps)
            .field("hotkey", &self.hotkey)
            .finish_non_exhaustive()
    }
}

// InputMethod isn't serde-aware in uvie-rs; serialize as its display name.
mod input_method_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use uvie::InputMethod;

    pub fn serialize<S: Serializer>(m: &InputMethod, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(match m {
            InputMethod::Telex => "telex",
            InputMethod::Vni => "vni",
            InputMethod::SimpleTelex => "simple-telex",
        })
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<InputMethod, D::Error> {
        match <&str>::deserialize(d)?.to_ascii_lowercase().as_str() {
            "vni" => Ok(InputMethod::Vni),
            "simple-telex" | "simpletelex" | "simple" => Ok(InputMethod::SimpleTelex),
            _ => Ok(InputMethod::Telex),
        }
    }
}
