//! The WinUI 3 settings window — the Windows counterpart of uvie-mac's
//! SwiftUI `SettingsView`/`Preferences` window.
//!
//! Runs on a dedicated thread (see `lib.rs`): bootstraps the Windows App
//! Runtime, starts `Microsoft.UI.Xaml.Application`, and builds the window
//! programmatically (no XAML markup — keeps the crate free of .xaml resource
//! packaging for now).
//!
//! Changes save immediately to `settings.json`, matching uvie-mac's
//! apply-on-change behaviour.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use windows_core::{Interface, HSTRING};

use crate::bindings::Microsoft::UI::Xaml::Controls::SelectionChangedEventHandler;
use crate::bindings::Microsoft::UI::Xaml::Controls::{
    ComboBox, ComboBoxItem, Orientation, ScrollViewer, StackPanel, TextBlock, ToggleSwitch,
};
use crate::bindings::Microsoft::UI::Xaml::{
    Application, ApplicationInitializationCallback, HorizontalAlignment, RoutedEventHandler,
    TextWrapping, Thickness, Window,
};
use crate::bindings::Windows::Foundation::PropertyValue;

use uvie_core::settings::Settings;
use uvie_core::InputMethod;

use crate::bootstrap::Bootstrap;

/// Entry point called on the dedicated UI thread by `spawn_settings_thread`.
pub fn run(settings_path: PathBuf) -> windows_core::Result<()> {
    // Keep the bootstrap handle alive until Application::Start returns.
    let _bootstrap = Bootstrap::init()?;

    let settings = Arc::new(Mutex::new(Settings::load(&settings_path)));

    Application::Start(&ApplicationInitializationCallback::new(move |_| {
        if let Err(e) = show_window(settings_path.clone(), settings.clone()) {
            eprintln!("uvie-winui: settings window failed to open: {e}");
        }
        Ok(())
    }))
}

fn show_window(settings_path: PathBuf, settings: Arc<Mutex<Settings>>) -> windows_core::Result<()> {
    let window = Window::new()?;
    window.SetTitle(&HSTRING::from("UVie Settings"))?;

    let root = StackPanel::new()?;
    root.SetOrientation(Orientation::Vertical)?;
    root.SetSpacing(12.0)?;
    root.SetMargin(Thickness {
        Left: 24.0,
        Top: 24.0,
        Right: 24.0,
        Bottom: 24.0,
    })?;

    let title = TextBlock::new()?;
    title.SetText(&HSTRING::from("UVie — Vietnamese Input Method"))?;
    title.SetFontSize(20.0)?;
    root.Children()?.Append(&title)?;

    // Enable / disable master switch.
    {
        let t = ToggleSwitch::new()?;
        t.SetHeader(&PropertyValue::CreateString(&HSTRING::from("Enable UVie"))?)?;
        t.SetIsOn(settings.lock().unwrap().enabled)?;
        let settings = settings.clone();
        let path = settings_path.clone();
        t.Toggled(&RoutedEventHandler::new(move |sender, _| {
            let t: ToggleSwitch = sender.unwrap().cast()?;
            settings.lock().unwrap().enabled = t.IsOn()?;
            let _ = settings.lock().unwrap().save(&path);
            Ok(())
        }))?;
        root.Children()?.Append(&t)?;
    }

    // Input method picker.
    {
        let label = TextBlock::new()?;
        label.SetText(&HSTRING::from("Input method"))?;
        root.Children()?.Append(&label)?;

        let combo = ComboBox::new()?;
        for name in ["Telex", "VNI", "SimpleTelex"] {
            let item = ComboBoxItem::new()?;
            item.SetContent(&PropertyValue::CreateString(&HSTRING::from(name))?)?;
            combo.Items()?.Append(&item)?;
        }
        combo.SetSelectedIndex(match settings.lock().unwrap().input_method {
            InputMethod::Telex => 0,
            InputMethod::Vni => 1,
            InputMethod::SimpleTelex => 2,
        })?;
        let settings = settings.clone();
        let path = settings_path.clone();
        combo.SelectionChanged(&SelectionChangedEventHandler::new(move |sender, _| {
            let combo: ComboBox = sender.unwrap().cast()?;
            let method = match combo.SelectedIndex()? {
                1 => InputMethod::Vni,
                2 => InputMethod::SimpleTelex,
                _ => InputMethod::Telex,
            };
            settings.lock().unwrap().input_method = method;
            let _ = settings.lock().unwrap().save(&path);
            Ok(())
        }))?;
        combo.SetHorizontalAlignment(HorizontalAlignment::Left)?;
        combo.SetMinWidth(200.0)?;
        root.Children()?.Append(&combo)?;
    }

    // Engine option toggles — same names/semantics as uvie-mac's preferences.
    type Getter = fn(&Settings) -> bool;
    type Setter = fn(&mut Settings, bool);
    let toggles: &[(&str, Getter, Setter)] = &[
        (
            "Quick Telex (cc, gg for đ/g...)",
            |s| s.quick_telex,
            |s, v| s.quick_telex = v,
        ),
        (
            "Quick start (type marks anywhere)",
            |s| s.quick_start,
            |s, v| s.quick_start = v,
        ),
        (
            "Modern orthography (òa, úy placement)",
            |s| s.modern_orthography,
            |s, v| s.modern_orthography = v,
        ),
        (
            "Relaxed coda rules",
            |s| s.relaxed_coda,
            |s, v| s.relaxed_coda = v,
        ),
        (
            "Skip Vietnamese in English words",
            |s| s.english_override,
            |s, v| s.english_override = v,
        ),
        (
            "Per-app language memory",
            |s| s.per_app_language,
            |s, v| s.per_app_language = v,
        ),
        (
            "Auto-capitalize",
            |s| s.auto_capitalize,
            |s, v| s.auto_capitalize = v,
        ),
        (
            "Launch at login",
            |s| s.launch_at_login,
            |s, v| s.launch_at_login = v,
        ),
    ];

    for (label, get, set) in toggles {
        let t = ToggleSwitch::new()?;
        t.SetHeader(&PropertyValue::CreateString(&HSTRING::from(*label))?)?;
        t.SetIsOn(get(&settings.lock().unwrap()))?;
        let settings = settings.clone();
        let path = settings_path.clone();
        t.Toggled(&RoutedEventHandler::new(move |sender, _| {
            let t: ToggleSwitch = sender.unwrap().cast()?;
            set(&mut settings.lock().unwrap(), t.IsOn()?);
            let _ = settings.lock().unwrap().save(&path);
            Ok(())
        }))?;
        root.Children()?.Append(&t)?;
    }

    let hint = TextBlock::new()?;
    hint.SetText(&HSTRING::from(
        "Toggle Vietnamese/English from the tray icon or the hotkey (default Ctrl+Shift+Z).",
    ))?;
    hint.SetTextWrapping(TextWrapping::Wrap)?;
    root.Children()?.Append(&hint)?;

    let scroll = ScrollViewer::new()?;
    scroll.SetContent(&root)?;
    window.SetContent(&scroll)?;
    window.Activate()?;
    Ok(())
}
