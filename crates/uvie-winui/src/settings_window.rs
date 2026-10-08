//! The WinUI 3 settings window — the Windows counterpart of uvie-mac's
//! SwiftUI `SettingsView`.
//!
//! Mirrors the mac layout: a left sidebar (NavigationView) with six panes —
//! Tổng quan / Bàn phím / Macro / Ứng dụng / Nâng cao / Giới thiệu — each a
//! scrollable column of section captions + rounded cards with icon/title/
//! description rows. Built programmatically (no XAML markup) on a dedicated
//! UI thread.
//!
//! This window is the sole writer of `settings.json` and `macros.json`;
//! the running app only reads them (see `Shared::save` in uvie-win).

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use windows_core::{Interface, HSTRING};

use crate::bindings::Microsoft::UI::Text::FontWeights;
use crate::bindings::Microsoft::UI::Xaml::Controls::{
    Border, Button, ColumnDefinition, ComboBox, ComboBoxItem, FontIcon, Grid, HyperlinkButton,
    Image, Orientation, RowDefinition, ScrollBarVisibility, ScrollViewer, StackPanel, TextBlock,
    ToggleSwitch,
};
use crate::bindings::Microsoft::UI::Xaml::Input::PointerEventHandler;
use crate::bindings::Microsoft::UI::Xaml::Markup::XamlReader;
use crate::bindings::Microsoft::UI::Xaml::Media::Imaging::BitmapImage;
use crate::bindings::Microsoft::UI::Xaml::Media::{
    Brush, ImageSource, MicaBackdrop, Stretch, SystemBackdrop,
};
use crate::bindings::Microsoft::UI::Xaml::{
    Application, ApplicationInitializationCallback, CornerRadius, FrameworkElement, GridLength,
    GridUnitType, HorizontalAlignment, RoutedEventHandler, Style, Thickness, UIElement,
    VerticalAlignment, Visibility, Window,
};
use crate::bindings::Windows::Foundation::{PropertyValue, Uri};

use uvie_core::settings::{Settings, DEFAULT_CHROMIUM_APPS};
use uvie_core::InputMethod;
use uvie_core::MacroTable;

use crate::bootstrap::Bootstrap;

// Segoe Fluent Icons glyphs used by sidebar items and row icons.
mod glyph {
    pub const SETTINGS: &str = "\u{E713}"; // gear
    pub const KEYBOARD: &str = "\u{E765}";
    pub const FONT: &str = "\u{E8D2}"; // text / macro
    pub const APPS: &str = "\u{E71D}"; // tiles
    pub const REPAIR: &str = "\u{E90F}"; // wrench
    pub const INFO: &str = "\u{E946}"; // info circle
    pub const DELETE: &str = "\u{E74D}";
    pub const SWITCH: &str = "\u{E8AB}"; // sync-ish
    pub const SEARCH: &str = "\u{E721}";
    pub const POWER: &str = "\u{E7E8}";
    pub const BOOK: &str = "\u{E736}";
    pub const BOLT: &str = "\u{E945}";
}

/// Shared mutable state for every pane.
struct State {
    settings_path: PathBuf,
    macros_path: PathBuf,
    settings: Mutex<Settings>,
    macros: Mutex<MacroTable>,
}

impl State {
    fn save_settings(&self) {
        let _ = self.settings.lock().unwrap().save(&self.settings_path);
    }
    fn save_macros(&self) {
        let _ = self.macros.lock().unwrap().save(&self.macros_path);
    }
}

type Shared = Arc<State>;
type R<T> = windows_core::Result<T>;

/// Entry point called on the dedicated UI thread by `spawn_settings_thread`.
pub fn run(settings_path: PathBuf, macros_path: PathBuf) -> R<()> {
    // Keep the bootstrap handle alive until Application::Start returns.
    let _bootstrap = Bootstrap::init()?;

    let state = Arc::new(State {
        settings: Mutex::new(Settings::load(&settings_path)),
        macros: Mutex::new(MacroTable::load(&macros_path)),
        settings_path,
        macros_path,
    });

    Application::Start(&ApplicationInitializationCallback::new(move |_| {
        if let Err(e) = show_window(state.clone()) {
            eprintln!("uvie-winui: settings window failed to open: {e}");
        }
        Ok(())
    }))
}

// ---------------------------------------------------------------------------
// Theme helpers
// ---------------------------------------------------------------------------

/// Look up a WinUI theme resource brush (e.g. "TextFillColorSecondaryBrush").
/// Falls back to `None` — callers leave the default brush in that case.
fn theme_brush(key: &str) -> Option<Brush> {
    let res = Application::Current().ok()?.Resources().ok()?;
    let boxed = PropertyValue::CreateString(&HSTRING::from(key)).ok()?;
    res.Lookup(&boxed).ok()?.cast().ok()
}

/// First available theme brush from a preference list.
fn theme_brush_any(keys: &[&str]) -> Option<Brush> {
    keys.iter().find_map(|k| theme_brush(k))
}

/// Parse a color string ("Transparent", "#AARRGGBB", named colors) into a
/// Brush via the real XAML parser — `Windows.UI.Color`/`SolidColorBrush.SetColor`
/// aren't in our generated bindings, so this is how colored brushes get built.
fn brush_from_str(s: &str) -> Option<Brush> {
    let xaml = format!(
        "<SolidColorBrush xmlns='http://schemas.microsoft.com/winfx/2006/xaml/presentation' Color='{s}'/>"
    );
    XamlReader::Load(&HSTRING::from(xaml)).ok()?.cast().ok()
}

/// Theme brush if any key resolves, else a parsed string fallback.
fn fill_or(keys: &[&str], fallback: &str) -> Option<Brush> {
    theme_brush_any(keys).or_else(|| brush_from_str(fallback))
}

/// System accent blue (Win11 default) for the nav pill / accent chips.
fn accent_brush() -> Option<Brush> {
    fill_or(
        &["AccentFillColorDefaultBrush", "SystemAccentColor"],
        "#FF0067C0",
    )
}

/// Look up a theme `Style` resource (e.g. "AccentButtonStyle").
fn theme_style(key: &str) -> Option<Style> {
    let res = Application::Current().ok()?.Resources().ok()?;
    let boxed = PropertyValue::CreateString(&HSTRING::from(key)).ok()?;
    res.Lookup(&boxed).ok()?.cast().ok()
}

/// Try to apply a named style to a control; no-op if absent.
fn apply_style(control: &impl Interface, key: &str) {
    if let Ok(c) = control.cast::<crate::bindings::Microsoft::UI::Xaml::Controls::Control>() {
        if let Some(s) = theme_style(key) {
            let _ = c.SetStyle(&s);
        }
    }
}

/// The org avatar PNG shared with `uvie-win`'s .ico resource — embedded so the
/// settings window can show the app logo without shipping a loose asset.
const ICON_PNG: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../uvie-win/assets/uvie-icon.png"
));

/// App logo as a XAML Image. The PNG is extracted once into the config dir
/// (next to settings.json) so `file:///` UriSource can load it —
/// `InMemoryRandomAccessStream` isn't in our generated bindings.
fn icon_image(state: &Shared, size: f64) -> R<Image> {
    let img = Image::new()?;
    img.SetWidth(size)?;
    img.SetHeight(size)?;
    img.SetStretch(Stretch::Uniform)?;
    if let Some(dir) = state.settings_path.parent() {
        let path = dir.join("uvie-icon.png");
        if !path.exists() {
            let _ = std::fs::write(&path, ICON_PNG);
        }
        let uri = format!("file:///{}", path.to_string_lossy().replace('\\', "/"));
        if let Ok(u) = Uri::CreateUri(&HSTRING::from(uri)) {
            let bmp = BitmapImage::new()?;
            let _ = bmp.SetUriSource(&u);
            let _ = img.SetSource(&bmp.cast::<ImageSource>()?);
        }
    }
    Ok(img)
}

fn uniform_radius(v: f64) -> CornerRadius {
    CornerRadius {
        TopLeft: v,
        TopRight: v,
        BottomRight: v,
        BottomLeft: v,
    }
}

/// Rounded-square chip behind a row icon (Win11 Settings look).
fn icon_chip(glyph: &str, chip_bg: &str, icon_fg: Option<&str>) -> R<Border> {
    let chip = Border::new()?;
    chip.SetWidth(32.0)?;
    chip.SetHeight(32.0)?;
    chip.SetCornerRadius(uniform_radius(8.0))?;
    if let Some(b) = fill_or(&[chip_bg], "#0F000000") {
        chip.SetBackground(&b)?;
    }
    let icon = font_icon(glyph, 15.0)?;
    icon.SetHorizontalAlignment(HorizontalAlignment::Center)?;
    icon.SetVerticalAlignment(VerticalAlignment::Center)?;
    if let Some(fg) = icon_fg {
        if let Some(b) = fill_or(&[fg], "#FFFFFFFF") {
            icon.SetForeground(&b)?;
        }
    }
    chip.SetChild(&icon)?;
    Ok(chip)
}

fn card() -> R<Border> {
    let b = Border::new()?;
    b.SetCornerRadius(CornerRadius {
        TopLeft: 8.0,
        TopRight: 8.0,
        BottomRight: 8.0,
        BottomLeft: 8.0,
    })?;
    if let Some(brush) = fill_or(&["CardBackgroundFillColorDefaultBrush"], "#B3FFFFFF") {
        b.SetBackground(&brush)?;
    }
    b.SetBorderThickness(Thickness {
        Left: 1.0,
        Top: 1.0,
        Right: 1.0,
        Bottom: 1.0,
    })?;
    if let Some(brush) = fill_or(&["CardStrokeColorDefaultBrush"], "#0F000000") {
        b.SetBorderBrush(&brush)?;
    }
    let inner = StackPanel::new()?;
    inner.SetOrientation(Orientation::Vertical)?;
    b.SetChild(&inner)?;
    Ok(b)
}

fn card_stack(card: &Border) -> R<StackPanel> {
    card.Child()?.cast()
}

/// Uppercase small caption above a card — `PaneSection` in uvie-mac.
fn section_header(title: &str) -> R<TextBlock> {
    let t = TextBlock::new()?;
    t.SetText(&HSTRING::from(title.to_uppercase()))?;
    t.SetFontSize(11.0)?;
    t.SetFontWeight(FontWeights::SemiBold()?)?;
    t.SetMargin(Thickness {
        Left: 2.0,
        Top: 0.0,
        Right: 0.0,
        Bottom: 0.0,
    })?;
    if let Some(brush) = fill_or(&["TextFillColorSecondaryBrush"], "#8A000000") {
        t.SetForeground(&brush)?;
    }
    Ok(t)
}

fn divider() -> R<Border> {
    let b = Border::new()?;
    b.SetHeight(1.0)?;
    b.SetMargin(Thickness {
        Left: 60.0,
        Top: 0.0,
        Right: 0.0,
        Bottom: 0.0,
    })?;
    if let Some(brush) = fill_or(&["DividerStrokeColorDefaultBrush"], "#0F000000") {
        b.SetBackground(&brush)?;
    }
    Ok(b)
}

fn text(s: &str, size: f64) -> R<TextBlock> {
    let t = TextBlock::new()?;
    t.SetText(&HSTRING::from(s))?;
    t.SetFontSize(size)?;
    Ok(t)
}

fn secondary(t: &TextBlock) -> R<()> {
    if let Some(brush) = fill_or(&["TextFillColorSecondaryBrush"], "#8A000000") {
        t.SetForeground(&brush)?;
    }
    Ok(())
}

fn font_icon(glyph: &str, size: f64) -> R<FontIcon> {
    let i = FontIcon::new()?;
    i.SetGlyph(&HSTRING::from(glyph))?;
    i.SetFontSize(size)?;
    Ok(i)
}

const STAR: GridLength = GridLength {
    Value: 1.0,
    GridUnitType: GridUnitType::Star,
};
const AUTO: GridLength = GridLength {
    Value: 1.0,
    GridUnitType: GridUnitType::Auto,
};

fn px(w: f64) -> GridLength {
    GridLength {
        Value: w,
        GridUnitType: GridUnitType::Pixel,
    }
}

fn grid(col_defs: &[GridLength]) -> R<Grid> {
    let g = Grid::new()?;
    for w in col_defs {
        let c = ColumnDefinition::new()?;
        c.SetWidth(*w)?;
        g.ColumnDefinitions()?.Append(&c)?;
    }
    Ok(g)
}

fn put(grid: &Grid, col: i32, child: &impl Interface) -> R<()> {
    let child = child.cast::<FrameworkElement>()?;
    Grid::SetColumn(&child, col)?;
    grid.Children()?
        .Append(&child.cast::<crate::bindings::Microsoft::UI::Xaml::UIElement>()?)?;
    Ok(())
}

fn thickness(v: f64) -> Thickness {
    Thickness {
        Left: v,
        Top: v,
        Right: v,
        Bottom: v,
    }
}

// ---------------------------------------------------------------------------
// Row builders
// ---------------------------------------------------------------------------

/// icon-chip + (title, description) + ToggleSwitch row — uvie-mac `SToggleRow`.
fn toggle_row<F>(card: &StackPanel, glyph: &str, title: &str, desc: &str, on: bool, f: F) -> R<()>
where
    F: FnMut(bool) + Send + 'static,
{
    let row = grid(&[px(46.0), STAR, AUTO])?; // chip, text, toggle
    row.SetPadding(Thickness {
        Left: 14.0,
        Top: 12.0,
        Right: 14.0,
        Bottom: 12.0,
    })?;

    let chip = icon_chip(glyph, "SubtleFillColorSecondaryBrush", None)?;
    chip.SetVerticalAlignment(VerticalAlignment::Center)?;
    put(&row, 0, &chip)?;

    let texts = StackPanel::new()?;
    texts.SetOrientation(Orientation::Vertical)?;
    texts.SetSpacing(2.0)?;
    let t = text(title, 13.0)?;
    t.SetFontWeight(FontWeights::Medium()?)?;
    texts.Children()?.Append(&t)?;
    let d = text(desc, 12.0)?;
    secondary(&d)?;
    d.SetTextWrapping(crate::bindings::Microsoft::UI::Xaml::TextWrapping::Wrap)?;
    texts.Children()?.Append(&d)?;
    texts.SetVerticalAlignment(VerticalAlignment::Center)?;
    texts.SetPadding(Thickness {
        Left: 0.0,
        Top: 0.0,
        Right: 12.0,
        Bottom: 0.0,
    })?;
    put(&row, 1, &texts)?;

    let toggle = ToggleSwitch::new()?;
    toggle.SetIsOn(on)?;
    toggle.SetVerticalAlignment(VerticalAlignment::Center)?;
    let mut f = f;
    toggle.Toggled(&RoutedEventHandler::new(move |sender, _| {
        let t: ToggleSwitch = sender.unwrap().cast()?;
        f(t.IsOn()?);
        Ok(())
    }))?;
    put(&row, 2, &toggle)?;

    card.Children()?.Append(&row)?;
    Ok(())
}

/// icon-chip + (title, description) row with no control — informational.
fn info_row(card: &StackPanel, glyph: &str, title: &str, desc: &str) -> R<()> {
    let row = grid(&[px(46.0), STAR])?;
    row.SetPadding(Thickness {
        Left: 14.0,
        Top: 12.0,
        Right: 14.0,
        Bottom: 12.0,
    })?;
    let chip = icon_chip(glyph, "SubtleFillColorSecondaryBrush", None)?;
    chip.SetVerticalAlignment(VerticalAlignment::Center)?;
    put(&row, 0, &chip)?;
    let texts = StackPanel::new()?;
    texts.SetOrientation(Orientation::Vertical)?;
    texts.SetSpacing(2.0)?;
    let t = text(title, 13.0)?;
    t.SetFontWeight(FontWeights::Medium()?)?;
    texts.Children()?.Append(&t)?;
    let d = text(desc, 12.0)?;
    secondary(&d)?;
    d.SetTextWrapping(crate::bindings::Microsoft::UI::Xaml::TextWrapping::Wrap)?;
    texts.Children()?.Append(&d)?;
    texts.SetVerticalAlignment(VerticalAlignment::Center)?;
    put(&row, 1, &texts)?;
    card.Children()?.Append(&row)?;
    Ok(())
}

/// A pane: ScrollViewer > page header + StackPanel of (caption + card) sections.
fn pane(title: &str, subtitle: &str) -> R<(ScrollViewer, StackPanel)> {
    let scroll = ScrollViewer::new()?;
    scroll.SetVerticalScrollBarVisibility(ScrollBarVisibility::Auto)?;
    let stack = StackPanel::new()?;
    stack.SetOrientation(Orientation::Vertical)?;
    stack.SetSpacing(20.0)?;
    stack.SetMaxWidth(880.0)?;
    stack.SetMargin(Thickness {
        Left: 36.0,
        Top: 24.0,
        Right: 36.0,
        Bottom: 32.0,
    })?;

    let header = StackPanel::new()?;
    header.SetOrientation(Orientation::Vertical)?;
    header.SetSpacing(4.0)?;
    header.SetMargin(Thickness {
        Left: 2.0,
        Top: 0.0,
        Right: 0.0,
        Bottom: 8.0,
    })?;
    let t = text(title, 26.0)?;
    t.SetFontWeight(FontWeights::SemiBold()?)?;
    header.Children()?.Append(&t)?;
    if !subtitle.is_empty() {
        let s = text(subtitle, 13.0)?;
        secondary(&s)?;
        header.Children()?.Append(&s)?;
    }
    stack.Children()?.Append(&header)?;

    scroll.SetContent(&stack)?;
    Ok((scroll, stack))
}

/// Append "section caption + card" to a pane; returns the card's inner stack.
fn section(stack: &StackPanel, title: &str) -> R<StackPanel> {
    let group = StackPanel::new()?;
    group.SetOrientation(Orientation::Vertical)?;
    group.SetSpacing(8.0)?;
    group.Children()?.Append(&section_header(title)?)?;
    let c = card()?;
    group.Children()?.Append(&c)?;
    let inner = card_stack(&c)?;
    stack.Children()?.Append(&group)?;
    Ok(inner)
}

// ---------------------------------------------------------------------------
// Window
// ---------------------------------------------------------------------------

/// Icon resource id shared with `app.rc` in uvie-win (org avatar).
const IDI_APP_ICON: usize = 101;

/// HWND of the XAML settings window (for parenting Win32 dialogs).
fn xaml_hwnd() -> windows::Win32::Foundation::HWND {
    use windows::core::w;
    use windows::Win32::UI::WindowsAndMessaging::FindWindowW;
    unsafe {
        FindWindowW(w!("WinUIDesktopWin32WindowClass"), w!("UVie for Windows")).unwrap_or_default()
    }
}

/// Size + title-bar icon for the XAML window, via its HWND.
/// (`AppWindow.Resize` needs `Windows::Graphics::SizeInt32`, which isn't in
/// the WASDK winmds we bind — the Win32 path needs no extra metadata.)
fn dress_window() {
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, LoadIconW, SendMessageW, SetWindowPos, SWP_NOMOVE, SWP_NOZORDER, WM_SETICON,
    };

    unsafe {
        // The tray host also creates a hidden window titled "UVie for Windows"
        // — match the XAML window by its class name so we resize the right one.
        let hwnd: HWND = FindWindowW(w!("WinUIDesktopWin32WindowClass"), w!("UVie for Windows"))
            .unwrap_or_default();
        if hwnd.0.is_null() {
            return;
        }
        let _ = SetWindowPos(hwnd, None, 0, 0, 880, 640, SWP_NOMOVE | SWP_NOZORDER);
        if let Ok(hinst) = GetModuleHandleW(None) {
            if let Ok(icon) = LoadIconW(
                Some(hinst.into()),
                PCWSTR::from_raw(IDI_APP_ICON as *const u16),
            ) {
                // ICON_SMALL = 0 (wParam); also set ICON_BIG for the title bar.
                let _ = SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(0)),
                    Some(LPARAM(icon.0 as isize)),
                );
                let _ = SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(1)),
                    Some(LPARAM(icon.0 as isize)),
                );
            }
        }
    }
}

/// Root grid with a custom titlebar row + content row underneath.
/// With `ExtendsContentIntoTitleBar` the strip behaves like Win11 Settings:
/// icon + app name at the left, system caption buttons overlaid at the right.
fn titlebar(state: &Shared) -> R<Grid> {
    let bar = Grid::new()?;
    bar.SetHeight(40.0)?;
    let inner = StackPanel::new()?;
    inner.SetOrientation(Orientation::Horizontal)?;
    inner.SetSpacing(10.0)?;
    inner.SetVerticalAlignment(VerticalAlignment::Center)?;
    inner.SetMargin(Thickness {
        Left: 14.0,
        Top: 0.0,
        Right: 148.0, // keep clear of the overlay caption buttons
        Bottom: 0.0,
    })?;
    inner.Children()?.Append(&icon_image(state, 16.0)?)?;
    let name = text("UVie for Windows", 12.0)?;
    name.SetFontWeight(FontWeights::SemiBold()?)?;
    name.SetVerticalAlignment(VerticalAlignment::Center)?;
    inner.Children()?.Append(&name)?;
    bar.Children()?.Append(&inner)?;
    Ok(bar)
}

fn show_window(state: Shared) -> R<()> {
    let window = Window::new()?;
    window.SetTitle(&HSTRING::from("UVie for Windows"))?;

    // Mica + content under the caption area — both degrade gracefully.
    let _ = window.SetExtendsContentIntoTitleBar(true);
    if let Ok(mica) = MicaBackdrop::new() {
        if let Ok(backdrop) = mica.cast::<SystemBackdrop>() {
            let _ = window.SetSystemBackdrop(&backdrop);
        }
    }

    let tabs: &[(&str, &str, &str)] = &[
        ("general", "Tổng quan", glyph::SETTINGS),
        ("keyboard", "Bàn phím", glyph::KEYBOARD),
        ("macro", "Macro", glyph::FONT),
        ("apps", "Ứng dụng", glyph::APPS),
        ("advanced", "Nâng cao", glyph::REPAIR),
        ("about", "Giới thiệu", glyph::INFO),
    ];

    // -- panes ---------------------------------------------------------------
    let panes: Vec<ScrollViewer> = vec![
        general_pane(&state)?,
        keyboard_pane(&state)?,
        macro_pane(&state)?,
        apps_pane(&state)?,
        advanced_pane(&state)?,
        about_pane(&state)?,
    ];
    let content = Grid::new()?;
    for (i, p) in panes.iter().enumerate() {
        p.SetVisibility(if i == 0 {
            Visibility::Visible
        } else {
            Visibility::Collapsed
        })?;
        content.Children()?.Append(p)?;
    }

    // -- sidebar (hand-rolled: NavigationView's template fast-fails on
    //    WASDK 1.6 / Server 2022; a Border + Buttons gives the same look) ----
    let side = Border::new()?;
    side.SetWidth(224.0)?;
    if let Some(b) = theme_brush_any(&[
        "LayerFillColorDefaultBrush",
        "CardBackgroundFillColorSecondaryBrush",
    ]) {
        side.SetBackground(&b)?;
    }
    side.SetBorderThickness(Thickness {
        Left: 0.0,
        Top: 0.0,
        Right: 1.0,
        Bottom: 0.0,
    })?;
    if let Some(b) = fill_or(&["DividerStrokeColorDefaultBrush"], "#0F000000") {
        side.SetBorderBrush(&b)?;
    }

    // Sidebar grid: [AUTO logo header][STAR nav items][AUTO version footer].
    let side_grid = Grid::new()?;
    {
        let r0 = RowDefinition::new()?;
        r0.SetHeight(AUTO)?;
        let r1 = RowDefinition::new()?;
        r1.SetHeight(STAR)?;
        let r2 = RowDefinition::new()?;
        r2.SetHeight(AUTO)?;
        side_grid.RowDefinitions()?.Append(&r0)?;
        side_grid.RowDefinitions()?.Append(&r1)?;
        side_grid.RowDefinitions()?.Append(&r2)?;
    }

    // Logo header.
    let brand = StackPanel::new()?;
    brand.SetOrientation(Orientation::Horizontal)?;
    brand.SetSpacing(10.0)?;
    brand.SetMargin(Thickness {
        Left: 14.0,
        Top: 12.0,
        Right: 0.0,
        Bottom: 14.0,
    })?;
    brand.Children()?.Append(&icon_image(&state, 26.0)?)?;
    let brand_text = StackPanel::new()?;
    brand_text.SetOrientation(Orientation::Vertical)?;
    brand_text.SetSpacing(1.0)?;
    brand_text.SetVerticalAlignment(VerticalAlignment::Center)?;
    let bt = text("UVie", 15.0)?;
    bt.SetFontWeight(FontWeights::SemiBold()?)?;
    brand_text.Children()?.Append(&bt)?;
    let bs = text("Tiếng Việt", 11.0)?;
    secondary(&bs)?;
    brand_text.Children()?.Append(&bs)?;
    brand.Children()?.Append(&brand_text)?;
    Grid::SetRow(&brand, 0)?;
    side_grid.Children()?.Append(&brand)?;

    // Nav items.
    let side_stack = StackPanel::new()?;
    side_stack.SetSpacing(2.0)?;
    side_stack.SetMargin(Thickness {
        Left: 10.0,
        Top: 0.0,
        Right: 10.0,
        Bottom: 8.0,
    })?;

    let selected = Arc::new(AtomicUsize::new(0));
    let mut buttons: Vec<Button> = Vec::new();
    let mut pills: Vec<Border> = Vec::new();
    for (i, (_, label, g)) in tabs.iter().enumerate() {
        // Cell = nav button with an accent pill overlaid at its left edge.
        let cell = Grid::new()?;
        let btn = Button::new()?;
        let row = grid(&[px(32.0), STAR])?;
        let ic = font_icon(g, 15.0)?;
        ic.SetVerticalAlignment(VerticalAlignment::Center)?;
        ic.SetHorizontalAlignment(HorizontalAlignment::Center)?;
        put(&row, 0, &ic)?;
        let tb = text(label, 13.0)?;
        tb.SetVerticalAlignment(VerticalAlignment::Center)?;
        if i == 0 {
            tb.SetFontWeight(FontWeights::SemiBold()?)?;
        }
        put(&row, 1, &tb)?;
        btn.SetContent(&row)?;
        btn.SetHorizontalAlignment(HorizontalAlignment::Stretch)?;
        btn.SetHorizontalContentAlignment(HorizontalAlignment::Left)?;
        if let Some(b) = fill_or(&["ControlFillColorTransparentBrush"], "Transparent") {
            btn.SetBackground(&b)?;
        }
        cell.Children()?.Append(&btn.cast::<UIElement>()?)?;

        let pill = Border::new()?;
        pill.SetWidth(3.0)?;
        pill.SetHeight(18.0)?;
        pill.SetCornerRadius(uniform_radius(1.5))?;
        pill.SetHorizontalAlignment(HorizontalAlignment::Left)?;
        pill.SetVerticalAlignment(VerticalAlignment::Center)?;
        pill.SetMargin(Thickness {
            Left: 0.0,
            Top: 0.0,
            Right: 0.0,
            Bottom: 0.0,
        })?;
        pill.SetOpacity(if i == 0 { 1.0 } else { 0.0 })?;
        pill.SetIsHitTestVisible(false)?;
        if let Some(b) = accent_brush() {
            pill.SetBackground(&b)?;
        }
        cell.Children()?.Append(&pill.cast::<UIElement>()?)?;

        side_stack.Children()?.Append(&cell)?;
        buttons.push(btn);
        pills.push(pill);
    }
    Grid::SetRow(&side_stack, 1)?;
    side_grid.Children()?.Append(&side_stack)?;

    // Version footer.
    let footer = StackPanel::new()?;
    footer.SetOrientation(Orientation::Vertical)?;
    footer.SetSpacing(1.0)?;
    footer.SetMargin(Thickness {
        Left: 16.0,
        Top: 0.0,
        Right: 0.0,
        Bottom: 12.0,
    })?;
    let fv = text(&format!("Phiên bản {}", env!("CARGO_PKG_VERSION")), 11.0)?;
    secondary(&fv)?;
    footer.Children()?.Append(&fv)?;
    let fe = text("Powered by uvie-rs", 11.0)?;
    secondary(&fe)?;
    footer.Children()?.Append(&fe)?;
    Grid::SetRow(&footer, 2)?;
    side_grid.Children()?.Append(&footer)?;

    side.SetChild(&side_grid)?;

    // Nav interactions: click selects (pane swap + pill + fill), hover tints.
    let nav_fill = |i: usize, buttons: &[Button], pills: &[Border]| -> R<()> {
        for (j, b) in buttons.iter().enumerate() {
            let brush = if j == i {
                fill_or(&["SubtleFillColorSecondaryBrush"], "#0F000000")
            } else {
                brush_from_str("Transparent")
            };
            if let Some(brush) = brush {
                b.SetBackground(&brush)?;
            }
            pills[j].SetOpacity(if j == i { 1.0 } else { 0.0 })?;
        }
        Ok(())
    };
    for (i, btn) in buttons.iter().enumerate() {
        let panes_c = panes.clone();
        let buttons_c = buttons.clone();
        let pills_c = pills.clone();
        let selected_c = selected.clone();
        btn.Click(&RoutedEventHandler::new(move |_, _| {
            selected_c.store(i, Ordering::Relaxed);
            for (j, p) in panes_c.iter().enumerate() {
                p.SetVisibility(if j == i {
                    Visibility::Visible
                } else {
                    Visibility::Collapsed
                })?;
            }
            nav_fill(i, &buttons_c, &pills_c)
        }))?;

        let buttons_e = buttons.clone();
        let selected_e = selected.clone();
        btn.PointerEntered(&PointerEventHandler::new(move |_, _| {
            if selected_e.load(Ordering::Relaxed) != i {
                if let Some(brush) = fill_or(&["SubtleFillColorTertiaryBrush"], "#08000000") {
                    buttons_e[i].SetBackground(&brush)?;
                }
            }
            Ok(())
        }))?;
        let buttons_x = buttons.clone();
        let pills_x = pills.clone();
        let selected_x = selected.clone();
        btn.PointerExited(&PointerEventHandler::new(move |_, _| {
            nav_fill(selected_x.load(Ordering::Relaxed), &buttons_x, &pills_x)
        }))?;
    }
    nav_fill(0, &buttons, &pills)?;

    // -- root: [AUTO titlebar][STAR (sidebar | content)] ----------------------
    let root = Grid::new()?;
    {
        let r0 = RowDefinition::new()?;
        r0.SetHeight(AUTO)?;
        let r1 = RowDefinition::new()?;
        r1.SetHeight(STAR)?;
        root.RowDefinitions()?.Append(&r0)?;
        root.RowDefinitions()?.Append(&r1)?;
    }
    let bar = titlebar(&state)?;
    Grid::SetRow(&bar, 0)?;
    root.Children()?.Append(&bar)?;

    let main = grid(&[px(224.0), STAR])?;
    put(&main, 0, &side)?;
    put(&main, 1, &content)?;
    Grid::SetRow(&main, 1)?;
    root.Children()?.Append(&main)?;

    window.SetContent(&root)?;
    // Mark the strip as the drag region (after content is set, per docs).
    if let Ok(uie) = bar.cast::<UIElement>() {
        let _ = window.SetTitleBar(&uie);
    }
    window.Activate()?;
    dress_window();
    Ok(())
}

// ---------------------------------------------------------------------------
// Panes
// ---------------------------------------------------------------------------

fn general_pane(state: &Shared) -> R<ScrollViewer> {
    let (scroll, stack) = pane(
        "Tổng quan",
        "Kiểm soát cách UVie gõ Tiếng Việt trong mọi ứng dụng",
    )?;

    // Engine master toggle — the big "Tiếng Việt / English" card.
    {
        let c = card()?;
        let inner = card_stack(&c)?;
        let row = grid(&[px(50.0), STAR, AUTO])?;
        row.SetPadding(Thickness {
            Left: 16.0,
            Top: 16.0,
            Right: 16.0,
            Bottom: 16.0,
        })?;
        let chip = Border::new()?;
        chip.SetWidth(36.0)?;
        chip.SetHeight(36.0)?;
        chip.SetCornerRadius(uniform_radius(8.0))?;
        if let Some(b) = accent_brush() {
            chip.SetBackground(&b)?;
        }
        let chip_icon = font_icon(glyph::KEYBOARD, 17.0)?;
        chip_icon.SetHorizontalAlignment(HorizontalAlignment::Center)?;
        chip_icon.SetVerticalAlignment(VerticalAlignment::Center)?;
        if let Some(b) = fill_or(&["TextOnAccentFillColorPrimaryBrush"], "#FFFFFFFF") {
            chip_icon.SetForeground(&b)?;
        }
        chip.SetChild(&chip_icon)?;
        chip.SetVerticalAlignment(VerticalAlignment::Center)?;
        put(&row, 0, &chip)?;
        let texts = StackPanel::new()?;
        texts.SetOrientation(Orientation::Vertical)?;
        texts.SetSpacing(3.0)?;
        let title = text("Tiếng Việt (UVie)", 15.0)?;
        title.SetFontWeight(FontWeights::SemiBold()?)?;
        texts.Children()?.Append(&title)?;
        let d = text("Bộ gõ Tiếng Việt cho Windows", 12.0)?;
        secondary(&d)?;
        texts.Children()?.Append(&d)?;
        texts.SetVerticalAlignment(VerticalAlignment::Center)?;
        put(&row, 1, &texts)?;
        let toggle = ToggleSwitch::new()?;
        toggle.SetIsOn(state.settings.lock().unwrap().enabled)?;
        toggle.SetVerticalAlignment(VerticalAlignment::Center)?;
        {
            let state = state.clone();
            toggle.Toggled(&RoutedEventHandler::new(move |sender, _| {
                let t: ToggleSwitch = sender.unwrap().cast()?;
                state.settings.lock().unwrap().enabled = t.IsOn()?;
                state.save_settings();
                Ok(())
            }))?;
        }
        put(&row, 2, &toggle)?;
        inner.Children()?.Append(&row)?;
        stack.Children()?.Append(&c)?;
    }

    // Input method — labeled row with the picker on the right.
    {
        let inner = section(&stack, "Bảng mã gõ")?;
        let row = grid(&[px(46.0), STAR, AUTO])?;
        row.SetPadding(Thickness {
            Left: 14.0,
            Top: 12.0,
            Right: 14.0,
            Bottom: 12.0,
        })?;
        let chip = icon_chip(glyph::KEYBOARD, "SubtleFillColorSecondaryBrush", None)?;
        chip.SetVerticalAlignment(VerticalAlignment::Center)?;
        put(&row, 0, &chip)?;
        let texts = StackPanel::new()?;
        texts.SetOrientation(Orientation::Vertical)?;
        texts.SetSpacing(2.0)?;
        let t = text("Kiểu gõ", 13.0)?;
        t.SetFontWeight(FontWeights::Medium()?)?;
        texts.Children()?.Append(&t)?;
        let d = text("Chọn Telex, VNI hoặc Simple Telex", 12.0)?;
        secondary(&d)?;
        texts.Children()?.Append(&d)?;
        texts.SetVerticalAlignment(VerticalAlignment::Center)?;
        put(&row, 1, &texts)?;

        let combo = ComboBox::new()?;
        for name in ["Telex", "VNI", "Simple Telex"] {
            let item = ComboBoxItem::new()?;
            item.SetContent(&PropertyValue::CreateString(&HSTRING::from(name))?)?;
            combo.Items()?.Append(&item)?;
        }
        combo.SetSelectedIndex(match state.settings.lock().unwrap().input_method {
            InputMethod::Telex => 0,
            InputMethod::Vni => 1,
            InputMethod::SimpleTelex => 2,
        })?;
        {
            let state = state.clone();
            combo.SelectionChanged(
                &crate::bindings::Microsoft::UI::Xaml::Controls::SelectionChangedEventHandler::new(
                    move |sender, _| {
                        let combo: ComboBox = sender.unwrap().cast()?;
                        let method = match combo.SelectedIndex()? {
                            1 => InputMethod::Vni,
                            2 => InputMethod::SimpleTelex,
                            _ => InputMethod::Telex,
                        };
                        state.settings.lock().unwrap().input_method = method;
                        state.save_settings();
                        Ok(())
                    },
                ),
            )?;
        }
        combo.SetHorizontalAlignment(HorizontalAlignment::Right)?;
        combo.SetVerticalAlignment(VerticalAlignment::Center)?;
        combo.SetMinWidth(180.0)?;
        put(&row, 2, &combo)?;
        inner.Children()?.Append(&row)?;
    }

    // Smart switching.
    {
        let inner = section(&stack, "Thông minh")?;
        let state_c = state.clone();
        toggle_row(
            &inner,
            glyph::SWITCH,
            "Nhớ ngôn ngữ từng ứng dụng",
            "Tự động Tiếng Việt / English khi chuyển app",
            state.settings.lock().unwrap().per_app_language,
            move |v| {
                state_c.settings.lock().unwrap().per_app_language = v;
                state_c.save_settings();
            },
        )?;
    }

    // Language detection.
    {
        let inner = section(&stack, "Phát hiện ngôn ngữ")?;
        let state_c = state.clone();
        toggle_row(
            &inner,
            glyph::SEARCH,
            "Bỏ qua khi gõ tiếng Anh",
            "Từ tiếng Anh thật sẽ không bị sửa thành tiếng Việt",
            state.settings.lock().unwrap().english_override,
            move |v| {
                state_c.settings.lock().unwrap().english_override = v;
                state_c.save_settings();
            },
        )?;
    }

    // System.
    {
        let inner = section(&stack, "Hệ thống")?;
        let state_c = state.clone();
        toggle_row(
            &inner,
            glyph::POWER,
            "Khởi động cùng Windows",
            "Tự động chạy khi đăng nhập",
            state.settings.lock().unwrap().launch_at_login,
            move |v| {
                state_c.settings.lock().unwrap().launch_at_login = v;
                state_c.save_settings();
            },
        )?;
    }

    // Hotkey (fixed for now — recorder is a mac-only extra).
    {
        let inner = section(&stack, "Phím tắt chuyển ngôn ngữ")?;
        info_row(
            &inner,
            glyph::KEYBOARD,
            "Ctrl + Shift + Z",
            "Bật / tắt Tiếng Việt từ bất kỳ ứng dụng nào",
        )?;
    }

    Ok(scroll)
}

fn keyboard_pane(state: &Shared) -> R<ScrollViewer> {
    let (scroll, stack) = pane("Bàn phím", "Tuỳ chọn kiểu gõ và hành vi bàn phím")?;

    let inner = section(&stack, "Vần cuối")?;
    {
        let state_c = state.clone();
        toggle_row(
            &inner,
            glyph::FONT,
            "Viết tắt vần cuối (g→ng, h→nh)",
            "Bật để gõ đặg, nhàh thay vì đặng, nhành. Tiện khi gõ nhanh.",
            state.settings.lock().unwrap().relaxed_coda,
            move |v| {
                state_c.settings.lock().unwrap().relaxed_coda = v;
                state_c.save_settings();
            },
        )?;
    }

    let inner = section(&stack, "Tự động hóa")?;
    {
        let state_c = state.clone();
        toggle_row(
            &inner,
            glyph::FONT,
            "Viết hoa chữ cái đầu câu",
            "Tự động viết hoa sau dấu chấm hoặc xuống dòng mới",
            state.settings.lock().unwrap().auto_capitalize,
            move |v| {
                state_c.settings.lock().unwrap().auto_capitalize = v;
                state_c.save_settings();
            },
        )?;
    }

    Ok(scroll)
}

fn macro_pane(state: &Shared) -> R<ScrollViewer> {
    let (scroll, stack) = pane("Macro", "Gõ viết tắt, mở rộng thành văn bản đầy đủ")?;

    {
        let inner = section(&stack, "Macro văn bản")?;
        let state_c = state.clone();
        toggle_row(
            &inner,
            glyph::BOLT,
            "Bật Macro văn bản",
            "Gõ viết tắt, nhấn Space / Enter để mở rộng thành văn bản đầy đủ",
            state.settings.lock().unwrap().macro_enabled,
            move |v| {
                state_c.settings.lock().unwrap().macro_enabled = v;
                state_c.save_settings();
            },
        )?;
    }

    // Macro list.
    let list: StackPanel;
    {
        let inner = section(&stack, "Danh sách Macro")?;

        // Header row.
        let header = grid(&[px(140.0), STAR, AUTO])?;
        header.SetPadding(Thickness {
            Left: 14.0,
            Top: 8.0,
            Right: 14.0,
            Bottom: 8.0,
        })?;
        let h1 = text("Viết tắt", 11.0)?;
        secondary(&h1)?;
        put(&header, 0, &h1)?;
        let h2 = text("Văn bản thay thế", 11.0)?;
        secondary(&h2)?;
        put(&header, 1, &h2)?;
        inner.Children()?.Append(&header)?;

        list = StackPanel::new()?;
        list.SetOrientation(Orientation::Vertical)?;
        inner.Children()?.Append(&list)?;
        rebuild_macro_list(&list, state)?;
    }

    // Add button — opens a Win32 dialog (XAML TextBox fast-fails on
    // Server 2022 + WASDK 1.6, see `entry_dialog`).
    {
        let add = Button::new()?;
        add.SetContent(&PropertyValue::CreateString(&HSTRING::from(
            "＋ Thêm macro",
        ))?)?;
        apply_style(&add, "AccentButtonStyle");
        add.SetHorizontalAlignment(HorizontalAlignment::Left)?;
        add.SetMargin(Thickness {
            Left: 0.0,
            Top: 10.0,
            Right: 0.0,
            Bottom: 0.0,
        })?;
        let state_c = state.clone();
        let list_c = list.clone();
        add.Click(&RoutedEventHandler::new(move |_, _| {
            if let Some(values) = crate::entry_dialog::prompt(
                xaml_hwnd(),
                "Thêm macro",
                &["Viết tắt (vd: sg)", "Văn bản thay thế (vd: Sài Gòn)"],
            ) {
                let trigger = values[0].trim();
                let repl = values.get(1).map(|s| s.trim()).unwrap_or("");
                if !trigger.is_empty() && !repl.is_empty() {
                    state_c.macros.lock().unwrap().add(trigger, repl);
                    state_c.save_macros();
                    rebuild_macro_list(&list_c, &state_c)?;
                }
            }
            Ok(())
        }))?;
        stack.Children()?.Append(&add)?;
    }

    Ok(scroll)
}

/// Rebuild the macro rows inside `list` (called after every add/remove).
fn rebuild_macro_list(list: &StackPanel, state: &Shared) -> R<()> {
    let children = list.Children()?;
    // Remove all rows (children.Clear isn't generated; walk backwards).
    let count = children.Size()?;
    for _ in 0..count {
        children.RemoveAtEnd()?;
    }

    let entries = state.macros.lock().unwrap().entries.clone();
    if entries.is_empty() {
        let t = text("Chưa có macro nào", 12.0)?;
        secondary(&t)?;
        t.SetHorizontalAlignment(HorizontalAlignment::Center)?;
        t.SetMargin(thickness(12.0))?;
        children.Append(&t)?;
        return Ok(());
    }

    for (i, e) in entries.iter().enumerate() {
        if i > 0 {
            list.Children()?.Append(&divider()?)?;
        }
        let row = grid(&[px(140.0), STAR, AUTO])?;
        row.SetPadding(Thickness {
            Left: 14.0,
            Top: 9.0,
            Right: 14.0,
            Bottom: 9.0,
        })?;
        let t1 = text(&e.trigger, 12.0)?;
        put(&row, 0, &t1)?;
        let t2 = text(&e.expansion, 12.0)?;
        put(&row, 1, &t2)?;

        let del = Button::new()?;
        let icon = font_icon(glyph::DELETE, 12.0)?;
        del.SetContent(&icon)?;
        let trigger = e.trigger.clone();
        let state_c = state.clone();
        let list_c = list.clone();
        del.Click(&RoutedEventHandler::new(move |_, _| {
            state_c.macros.lock().unwrap().remove(&trigger);
            state_c.save_macros();
            let _ = rebuild_macro_list(&list_c, &state_c);
            Ok(())
        }))?;
        put(&row, 2, &del)?;
        list.Children()?.Append(&row)?;
    }
    Ok(())
}

/// An app-list card: description + rows (exe name + delete) + add box.
fn app_list_card(
    stack: &StackPanel,
    section_title: &str,
    desc: &str,
    defaults: &'static [&'static str],
    get: fn(&Settings) -> Vec<String>,
    set: fn(&mut Settings, Vec<String>),
    state: &Shared,
) -> R<()> {
    let inner = section(stack, section_title)?;

    let d = text(desc, 11.0)?;
    secondary(&d)?;
    d.SetTextWrapping(crate::bindings::Microsoft::UI::Xaml::TextWrapping::Wrap)?;
    d.SetMargin(Thickness {
        Left: 14.0,
        Top: 12.0,
        Right: 14.0,
        Bottom: 4.0,
    })?;
    inner.Children()?.Append(&d)?;

    let list = StackPanel::new()?;
    list.SetOrientation(Orientation::Vertical)?;
    inner.Children()?.Append(&list)?;

    rebuild_app_list(&list, state, get, set)?;

    // Footer: Add (Win32 dialog — XAML TextBox fast-fails) + Reset.
    let footer = grid(&[STAR, AUTO, AUTO])?;
    footer.SetPadding(Thickness {
        Left: 14.0,
        Top: 0.0,
        Right: 14.0,
        Bottom: 12.0,
    })?;

    let add = Button::new()?;
    add.SetContent(&PropertyValue::CreateString(&HSTRING::from("＋ Thêm…"))?)?;
    apply_style(&add, "AccentButtonStyle");
    {
        let state_c = state.clone();
        let list_c = list.clone();
        add.Click(&RoutedEventHandler::new(move |_, _| {
            if let Some(values) = crate::entry_dialog::prompt(
                xaml_hwnd(),
                "Thêm ứng dụng",
                &["Tên tiến trình (vd: chrome.exe)"],
            ) {
                let exe = values[0].trim().to_lowercase();
                if !exe.is_empty() {
                    let mut s = state_c.settings.lock().unwrap();
                    let mut apps = get(&s);
                    if !apps.iter().any(|a| a == &exe) {
                        apps.push(exe);
                        set(&mut s, apps);
                        drop(s);
                        state_c.save_settings();
                        rebuild_app_list(&list_c, &state_c, get, set)?;
                    }
                }
            }
            Ok(())
        }))?;
    }
    put(&footer, 1, &add)?;

    let reset = Button::new()?;
    reset.SetContent(&PropertyValue::CreateString(&HSTRING::from(
        "Reset mặc định",
    ))?)?;
    reset.SetMargin(Thickness {
        Left: 8.0,
        Top: 0.0,
        Right: 0.0,
        Bottom: 0.0,
    })?;
    {
        let state_c = state.clone();
        let list_c = list.clone();
        reset.Click(&RoutedEventHandler::new(move |_, _| {
            {
                let mut s = state_c.settings.lock().unwrap();
                set(&mut s, defaults.iter().map(|x| x.to_string()).collect());
            }
            state_c.save_settings();
            rebuild_app_list(&list_c, &state_c, get, set)
        }))?;
    }
    put(&footer, 2, &reset)?;

    inner.Children()?.Append(&footer)?;
    Ok(())
}

fn rebuild_app_list(
    list: &StackPanel,
    state: &Shared,
    get: fn(&Settings) -> Vec<String>,
    set: fn(&mut Settings, Vec<String>),
) -> R<()> {
    let children = list.Children()?;
    let count = children.Size()?;
    for _ in 0..count {
        children.RemoveAtEnd()?;
    }

    let apps = get(&state.settings.lock().unwrap());
    if apps.is_empty() {
        let t = text("Chưa có ứng dụng nào", 12.0)?;
        secondary(&t)?;
        t.SetHorizontalAlignment(HorizontalAlignment::Center)?;
        t.SetMargin(thickness(12.0))?;
        children.Append(&t)?;
        return Ok(());
    }

    for (i, exe) in apps.iter().enumerate() {
        if i > 0 {
            children.Append(&divider()?)?;
        }
        let row = grid(&[STAR, AUTO])?;
        row.SetPadding(Thickness {
            Left: 14.0,
            Top: 9.0,
            Right: 14.0,
            Bottom: 9.0,
        })?;
        let t = text(exe, 12.0)?;
        t.SetVerticalAlignment(VerticalAlignment::Center)?;
        put(&row, 0, &t)?;

        let del = Button::new()?;
        let icon = font_icon(glyph::DELETE, 12.0)?;
        del.SetContent(&icon)?;
        let exe_c = exe.clone();
        let state_c = state.clone();
        let list_c = list.clone();
        del.Click(&RoutedEventHandler::new(move |_, _| {
            {
                let mut s = state_c.settings.lock().unwrap();
                let mut apps = get(&s);
                apps.retain(|a| a != &exe_c);
                set(&mut s, apps);
            }
            state_c.save_settings();
            rebuild_app_list(&list_c, &state_c, get, set)
        }))?;
        put(&row, 1, &del)?;
        children.Append(&row)?;
    }
    Ok(())
}

fn apps_pane(state: &Shared) -> R<ScrollViewer> {
    let (scroll, stack) = pane("Ứng dụng", "Quản lý app loại trừ và tương thích gõ")?;

    app_list_card(
        &stack,
        "Ứng dụng loại trừ",
        "Tắt UVie for Windows cho các ứng dụng trong danh sách",
        &[],
        |s| s.excluded_apps.clone(),
        |s, v| s.excluded_apps = v,
        state,
    )?;

    app_list_card(
        &stack,
        "Chromium / trình duyệt",
        "Nếu gõ trong trình duyệt Chromium bị lỗi (mất ký tự, nhân đôi), thêm app vào danh sách này — UVie sẽ chọn-và-gõ-đè thay vì xoá lùi",
        DEFAULT_CHROMIUM_APPS,
        |s| s.chromium_apps.clone(),
        |s, v| s.chromium_apps = v,
        state,
    )?;

    Ok(scroll)
}

fn advanced_pane(state: &Shared) -> R<ScrollViewer> {
    let (scroll, stack) = pane("Nâng cao", "Tuỳ chọn nâng cao và lưu ý khi sử dụng")?;

    let inner = section(&stack, "Ngôn ngữ")?;
    {
        let state_c = state.clone();
        toggle_row(
            &inner,
            glyph::BOOK,
            "Chính tả hiện đại",
            "Bật: hoas → hoá (quy tắc mới). Tắt: hoas → hoà (chính tả truyền thống).",
            state.settings.lock().unwrap().modern_orthography,
            move |v| {
                state_c.settings.lock().unwrap().modern_orthography = v;
                state_c.save_settings();
            },
        )?;
    }

    let inner = section(&stack, "Gõ tắt Telex")?;
    {
        let state_c = state.clone();
        toggle_row(
            &inner,
            glyph::BOLT,
            "Quick Telex (gõ kép)",
            "Gõ đôi phụ âm: cc→ch, gg→gi, hh→nh, kk→kh, nn→ng, qq→qu, pp→ph, tt→th.",
            state.settings.lock().unwrap().quick_telex,
            move |v| {
                state_c.settings.lock().unwrap().quick_telex = v;
                state_c.save_settings();
            },
        )?;
    }
    {
        let state_c = state.clone();
        toggle_row(
            &inner,
            glyph::BOLT,
            "Quick Start (gõ nhanh)",
            "j→gi, f→ph, w→qu ở đầu từ. Tiện khi gõ nhanh.",
            state.settings.lock().unwrap().quick_start,
            move |v| {
                state_c.settings.lock().unwrap().quick_start = v;
                state_c.save_settings();
            },
        )?;
    }

    let inner = section(&stack, "Ghi chú")?;
    info_row(
        &inner,
        glyph::INFO,
        "Cài đặt áp dụng khi khởi động lại",
        "Ứng dụng đọc cài đặt lúc khởi động — thay đổi có hiệu lực sau khi mở lại UVie",
    )?;
    info_row(
        &inner,
        glyph::REPAIR,
        "Ứng dụng chạy quyền Admin",
        "UVie không thể gõ vào các app chạy quyền Administrator",
    )?;

    Ok(scroll)
}

fn about_pane(state: &Shared) -> R<ScrollViewer> {
    let (scroll, stack) = pane("Giới thiệu", "Về UVie for Windows")?;

    let center = StackPanel::new()?;
    center.SetOrientation(Orientation::Vertical)?;
    center.SetSpacing(14.0)?;
    center.SetHorizontalAlignment(HorizontalAlignment::Center)?;
    center.SetMargin(Thickness {
        Left: 0.0,
        Top: 40.0,
        Right: 0.0,
        Bottom: 0.0,
    })?;

    center.Children()?.Append(&icon_image(state, 72.0)?)?;

    let title = text("UVie for Windows", 24.0)?;
    title.SetFontWeight(FontWeights::SemiBold()?)?;
    title.SetHorizontalAlignment(HorizontalAlignment::Center)?;
    center.Children()?.Append(&title)?;

    // Version badge — rounded accent-tinted pill.
    let badge = Border::new()?;
    badge.SetCornerRadius(uniform_radius(12.0))?;
    badge.SetPadding(Thickness {
        Left: 12.0,
        Top: 4.0,
        Right: 12.0,
        Bottom: 4.0,
    })?;
    if let Some(b) = fill_or(&["AccentFillColorSecondaryBrush"], "#1A0067C0") {
        badge.SetBackground(&b)?;
    }
    let badge_text = text(&format!("v{}", env!("CARGO_PKG_VERSION")), 12.0)?;
    badge.SetChild(&badge_text)?;
    center.Children()?.Append(&badge)?;

    let desc = text(
        "Bộ gõ Tiếng Việt nhanh, nhẹ và chính xác cho Windows.\nPowered by uvie-rs — zero-cost Rust engine.",
        13.0,
    )?;
    secondary(&desc)?;
    desc.SetHorizontalAlignment(HorizontalAlignment::Center)?;
    desc.SetTextAlignment(crate::bindings::Microsoft::UI::Xaml::TextAlignment::Center)?;
    center.Children()?.Append(&desc)?;

    // Footer links.
    let links = StackPanel::new()?;
    links.SetOrientation(Orientation::Horizontal)?;
    links.SetSpacing(10.0)?;
    links.SetMargin(Thickness {
        Left: 0.0,
        Top: 8.0,
        Right: 0.0,
        Bottom: 0.0,
    })?;
    links.SetHorizontalAlignment(HorizontalAlignment::Center)?;
    for (label, url) in [
        ("GitHub", "https://github.com/uvie-project/uvie-win"),
        (
            "Cập nhật",
            "https://github.com/uvie-project/uvie-win/releases",
        ),
        ("Báo lỗi", "https://github.com/uvie-project/uvie-win/issues"),
    ] {
        let link = HyperlinkButton::new()?;
        link.SetContent(&PropertyValue::CreateString(&HSTRING::from(label))?)?;
        if let Ok(uri) = Uri::CreateUri(&HSTRING::from(url)) {
            let _ = link.SetNavigateUri(&uri);
        }
        links.Children()?.Append(&link)?;
    }
    center.Children()?.Append(&links)?;

    stack.Children()?.Append(&center)?;
    Ok(scroll)
}
