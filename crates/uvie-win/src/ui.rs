//! Bridge to the WinUI 3 settings window (`uvie-winui` crate).
//! The XAML app must live on its own dedicated thread — see uvie-winui docs.

use std::sync::OnceLock;

static UI: OnceLock<()> = OnceLock::new();

/// Open (or focus) the settings window. Called from the tray menu.
pub fn open_settings() {
    let _ = UI.get_or_init(|| {
        let paths = crate::app::Paths::appdata();
        uvie_winui::spawn_settings_thread(paths.settings, paths.macros);
    });
}
