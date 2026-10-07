//! WinUI 3 settings UI for UVie.
//!
//! Bindings for `Microsoft.UI.Xaml`/`Microsoft.Windows.*` are generated at
//! build time by `windows-bindgen` from the Windows App SDK .winmd files
//! (see `build.rs` and `scripts/fetch-windowsappsdk.ps1`).
//!
//! A WinUI 3 `Application` owns a dispatcher and pumps its own message loop,
//! so the settings window runs on a dedicated thread spawned by
//! [`spawn_settings_thread`]. Unpackaged processes must bootstrap the
//! Windows App Runtime (`MddBootstrapInitialize2`) before touching XAML.

#[allow(
    clippy::all,
    non_snake_case,
    non_camel_case_types,
    non_upper_case_globals,
    dead_code
)]
#[rustfmt::skip]
mod bindings {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

mod bootstrap;
mod settings_window;

use std::path::PathBuf;

/// Spawn the dedicated UI thread that hosts the WinUI 3 settings window.
/// Safe to call once; subsequent calls are no-ops (XAML `Application::Start`
/// may only run once per process).
pub fn spawn_settings_thread(settings_path: PathBuf, macros_path: PathBuf) {
    std::thread::spawn(move || {
        if let Err(e) = settings_window::run(settings_path, macros_path) {
            eprintln!("uvie-winui: settings window failed: {e}");
        }
    });
}
