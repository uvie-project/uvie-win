//! Application wiring — hook → dispatcher → injector, plus tray menu,
//! focus tracking, settings persistence. Mirrors `UVieMacApp` +
//! `EventTap` orchestration in uvie-mac.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use uvie_core::dispatcher::{Context, Dispatch, Dispatcher, InputLanguage};
use uvie_core::engine_session::{EngineOptions, EngineSession};
use uvie_core::keys::KeyEvent;
use uvie_core::macros::MacroTable;
use uvie_core::memory::LanguageMemory;
use uvie_core::settings::{LanguagePref, Settings};

use crate::foreground::FocusWatcher;
use crate::inject;
use crate::keyboard::KeyboardHook;
use crate::startup;
use crate::tray::{self, TrayIcon};

/// Settings/memory file locations (`%APPDATA%\UVie\`).
pub struct Paths {
    pub settings: PathBuf,
    pub memory: PathBuf,
    /// Macro-definition file (`macros.json`): written by the settings
    /// window, loaded once here at startup.
    pub macros: PathBuf,
}

impl Paths {
    pub fn appdata() -> Self {
        let dir = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("UVie");
        Self {
            settings: dir.join("settings.json"),
            memory: dir.join("memory.json"),
            macros: dir.join("macros.json"),
        }
    }
}

struct Shared {
    dispatcher: Dispatcher,
    settings: Settings,
    memory: LanguageMemory,
    foreground_exe: String,
    paths: Paths,
}

impl Shared {
    fn apply_engine_options(&mut self) {
        let s = &self.settings;
        self.dispatcher.session.apply_options(EngineOptions {
            quick_start: s.quick_start,
            quick_telex: s.quick_telex,
            modern_orthography: s.modern_orthography,
            relaxed_coda: s.relaxed_coda,
            english_override: s.english_override,
        });
        self.dispatcher.session.set_input_method(s.input_method);
    }

    /// Persist app-owned state. The app only *reads* `settings.json` — it is
    /// written exclusively by the settings window — so saving it here would
    /// clobber UI edits made after startup.
    fn save(&self) {
        let _ = self.memory.save(&self.paths.memory);
    }
}

/// Owns the OS plumbing. Lives on the message-loop thread.
pub struct App {
    shared: Rc<RefCell<Shared>>,
    _hook: KeyboardHook,
    _tray: TrayIcon,
    _focus: FocusWatcher,
}

impl App {
    pub fn new() -> windows::core::Result<Self> {
        let paths = Paths::appdata();
        let settings = Settings::load(&paths.settings);
        let memory = LanguageMemory::load(&paths.memory);

        let mut dispatcher = Dispatcher::new(EngineSession::new());
        dispatcher.macros = MacroTable::load(&paths.macros);
        dispatcher.macro_enabled = settings.macro_enabled;
        let shared = Rc::new(RefCell::new(Shared {
            dispatcher,
            settings,
            memory,
            foreground_exe: String::new(),
            paths,
        }));
        shared.borrow_mut().apply_engine_options();

        // -- keyboard hook -------------------------------------------------
        let shared_key = Rc::clone(&shared);
        let hook = KeyboardHook::install(move |key: KeyEvent| {
            let mut s = shared_key.borrow_mut();
            // Never process keys destined for our own windows (settings,
            // dialogs, tray menus) — otherwise Telex marks would rewrite
            // what the user types into UVie's own UI.
            let self_focused = foreground_belongs_to_self();
            let ctx = Context {
                language: None,
                app_excluded: self_focused
                    || s.settings.is_excluded(&s.foreground_exe)
                    || !s.settings.enabled,
                non_latin_layout: false, // TODO: GetKeyboardLayout language id check
            };
            let chromium = s.settings.is_chromium(&s.foreground_exe);
            match s.dispatcher.handle(key, &ctx) {
                Dispatch::Pass(plan) => {
                    inject::inject(&plan, chromium);
                    false
                }
                Dispatch::Consume(plan) => {
                    inject::inject(&plan, chromium);
                    true
                }
            }
        })?;

        // -- tray ----------------------------------------------------------
        let shared_menu = Rc::clone(&shared);
        let tray = TrayIcon::create(move |cmd| {
            let mut s = shared_menu.borrow_mut();
            match cmd {
                tray::CMD_TOGGLE_LANGUAGE => {
                    let lang = match s.dispatcher.language {
                        InputLanguage::Vietnamese => InputLanguage::English,
                        InputLanguage::English => InputLanguage::Vietnamese,
                    };
                    s.dispatcher.set_language(lang);
                    if s.settings.per_app_language && !s.foreground_exe.is_empty() {
                        let pref = match lang {
                            InputLanguage::Vietnamese => LanguagePref::Vietnamese,
                            InputLanguage::English => LanguagePref::English,
                        };
                        let exe = s.foreground_exe.clone();
                        s.memory.remember(&exe, pref);
                    }
                    s.save();
                }
                tray::CMD_OPEN_SETTINGS => {
                    crate::ui::open_settings();
                }
                tray::CMD_QUIT => unsafe {
                    windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(0);
                },
                _ => {}
            }
        })?;

        // -- foreground-app watcher -----------------------------------------
        // Our own exe name, so the focus watcher can ignore the tray
        // window stealing foreground (it would otherwise attribute
        // language toggles to uvie-win.exe).
        let own_exe = std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "uvie-win.exe".to_string());

        let shared_focus = Rc::clone(&shared);
        let focus = FocusWatcher::install(move |exe| {
            if exe.eq_ignore_ascii_case(&own_exe) {
                return;
            }
            let mut s = shared_focus.borrow_mut();
            s.foreground_exe = exe;
            if s.settings.per_app_language {
                let exe = s.foreground_exe.clone();
                if let Some(pref) = s.memory.recall(&exe) {
                    let lang = match pref {
                        LanguagePref::Vietnamese => InputLanguage::Vietnamese,
                        LanguagePref::English => InputLanguage::English,
                    };
                    s.dispatcher.set_language(lang);
                }
            }
            // Focus change always drops in-flight composition, like uvie-mac.
            s.dispatcher.session.reset();
        })?;

        Ok(Self {
            shared,
            _hook: hook,
            _tray: tray,
            _focus: focus,
        })
    }

    /// Persist the launch-at-login preference into the Run key.
    pub fn sync_launch_at_login(&self) {
        let s = self.shared.borrow();
        let _ = startup::set_launch_at_login(s.settings.launch_at_login);
    }
}

/// True when the foreground window belongs to this process — used to keep
/// the engine from rewriting keys typed into our own UI.
fn foreground_belongs_to_self() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    unsafe {
        let mut pid = 0u32;
        let _ = GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid));
        pid != 0 && pid == std::process::id()
    }
}
