//! Application wiring — hook → dispatcher → injector, plus tray menu,
//! focus tracking, settings persistence. Mirrors `UVieMacApp` +
//! `EventTap` orchestration in uvie-mac.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use uvie_core::dispatcher::{Context, Dispatch, Dispatcher, InputLanguage};
use uvie_core::engine_session::{EngineOptions, EngineSession};
use uvie_core::keys::KeyEvent;
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
    /// Macro-definition file; read once macro UI/editing lands.
    #[allow(dead_code)]
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

    fn save(&self) {
        let _ = self.settings.save(&self.paths.settings);
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

        let dispatcher = Dispatcher::new(EngineSession::new());
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
            let ctx = Context {
                language: None,
                app_excluded: s.settings.is_excluded(&s.foreground_exe) || !s.settings.enabled,
                non_latin_layout: false, // TODO: GetKeyboardLayout language id check
            };
            match s.dispatcher.handle(key, &ctx) {
                Dispatch::Pass(plan) => {
                    inject::inject(&plan);
                    false
                }
                Dispatch::Consume(plan) => {
                    inject::inject(&plan);
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
        let shared_focus = Rc::clone(&shared);
        let focus = FocusWatcher::install(move |exe| {
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
