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
use std::sync::{Arc, Mutex};

use windows_core::{Interface, HSTRING};

use crate::bindings::Microsoft::UI::Xaml::Controls::{
    Border, Button, ColumnDefinition, ComboBox, ComboBoxItem, FontIcon, Grid, HyperlinkButton,
    NavigationView, NavigationViewItem, NavigationViewPaneDisplayMode,
    NavigationViewSelectionChangedEventArgs, Orientation, ScrollBarVisibility, ScrollViewer,
    StackPanel, TextBlock, TextBox, ToggleSwitch,
};
use crate::bindings::Microsoft::UI::Xaml::Media::Brush;
use crate::bindings::Microsoft::UI::Xaml::{
    Application, ApplicationInitializationCallback, CornerRadius, FrameworkElement, GridLength,
    GridUnitType, HorizontalAlignment, RoutedEventHandler, Thickness, VerticalAlignment,
    Visibility, Window,
};
use crate::bindings::Windows::Foundation::{PropertyValue, TypedEventHandler, Uri};

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

fn card() -> R<Border> {
    let b = Border::new()?;
    b.SetCornerRadius(CornerRadius {
        TopLeft: 8.0,
        TopRight: 8.0,
        BottomRight: 8.0,
        BottomLeft: 8.0,
    })?;
    if let Some(brush) = theme_brush("CardBackgroundFillColorDefaultBrush") {
        b.SetBackground(&brush)?;
    }
    b.SetBorderThickness(Thickness {
        Left: 1.0,
        Top: 1.0,
        Right: 1.0,
        Bottom: 1.0,
    })?;
    if let Some(brush) = theme_brush("CardStrokeColorDefaultBrush") {
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
    if let Some(brush) = theme_brush("TextFillColorSecondaryBrush") {
        t.SetForeground(&brush)?;
    }
    Ok(t)
}

fn divider() -> R<Border> {
    let b = Border::new()?;
    b.SetHeight(1.0)?;
    b.SetMargin(Thickness {
        Left: 50.0,
        Top: 0.0,
        Right: 0.0,
        Bottom: 0.0,
    })?;
    if let Some(brush) = theme_brush("DividerStrokeColorDefaultBrush") {
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
    if let Some(brush) = theme_brush("TextFillColorSecondaryBrush") {
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

/// icon + (title, description) + ToggleSwitch row — uvie-mac `SToggleRow`.
fn toggle_row<F>(card: &StackPanel, glyph: &str, title: &str, desc: &str, on: bool, f: F) -> R<()>
where
    F: FnMut(bool) + Send + 'static,
{
    let row = grid(&[px(30.0), STAR, AUTO])?; // icon, text, toggle
    row.SetPadding(Thickness {
        Left: 14.0,
        Top: 11.0,
        Right: 14.0,
        Bottom: 11.0,
    })?;

    let icon = font_icon(glyph, 16.0)?;
    icon.SetVerticalAlignment(VerticalAlignment::Center)?;
    if let Some(brush) = theme_brush("TextFillColorSecondaryBrush") {
        icon.SetForeground(&brush)?;
    }
    put(&row, 0, &icon)?;

    let texts = StackPanel::new()?;
    texts.SetOrientation(Orientation::Vertical)?;
    texts.SetSpacing(2.0)?;
    let t = text(title, 13.0)?;
    texts.Children()?.Append(&t)?;
    let d = text(desc, 11.0)?;
    secondary(&d)?;
    d.SetTextWrapping(crate::bindings::Microsoft::UI::Xaml::TextWrapping::Wrap)?;
    texts.Children()?.Append(&d)?;
    texts.SetVerticalAlignment(VerticalAlignment::Center)?;
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

/// icon + (title, description) row with no control — informational.
fn info_row(card: &StackPanel, glyph: &str, title: &str, desc: &str) -> R<()> {
    let row = grid(&[px(30.0), STAR])?;
    row.SetPadding(Thickness {
        Left: 14.0,
        Top: 11.0,
        Right: 14.0,
        Bottom: 11.0,
    })?;
    let icon = font_icon(glyph, 16.0)?;
    icon.SetVerticalAlignment(VerticalAlignment::Center)?;
    if let Some(brush) = theme_brush("TextFillColorSecondaryBrush") {
        icon.SetForeground(&brush)?;
    }
    put(&row, 0, &icon)?;
    let texts = StackPanel::new()?;
    texts.SetOrientation(Orientation::Vertical)?;
    texts.SetSpacing(2.0)?;
    texts.Children()?.Append(&text(title, 13.0)?)?;
    let d = text(desc, 11.0)?;
    secondary(&d)?;
    d.SetTextWrapping(crate::bindings::Microsoft::UI::Xaml::TextWrapping::Wrap)?;
    texts.Children()?.Append(&d)?;
    texts.SetVerticalAlignment(VerticalAlignment::Center)?;
    put(&row, 1, &texts)?;
    card.Children()?.Append(&row)?;
    Ok(())
}

/// A pane: ScrollViewer > StackPanel of (caption + card) sections.
fn pane() -> R<(ScrollViewer, StackPanel)> {
    let scroll = ScrollViewer::new()?;
    scroll.SetVerticalScrollBarVisibility(ScrollBarVisibility::Auto)?;
    let stack = StackPanel::new()?;
    stack.SetOrientation(Orientation::Vertical)?;
    stack.SetSpacing(22.0)?;
    stack.SetMargin(thickness(24.0))?;
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
        let hwnd: HWND = FindWindowW(None, w!("UVie for Windows")).unwrap_or_default();
        if hwnd.0.is_null() {
            return;
        }
        let _ = SetWindowPos(hwnd, None, 0, 0, 780, 600, SWP_NOMOVE | SWP_NOZORDER);
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

fn show_window(state: Shared) -> R<()> {
    let window = Window::new()?;
    window.SetTitle(&HSTRING::from("UVie for Windows"))?;
    dress_window();

    // -- sidebar ------------------------------------------------------------
    let nav = NavigationView::new()?;
    nav.SetPaneDisplayMode(NavigationViewPaneDisplayMode::Left)?;
    nav.SetIsPaneToggleButtonVisible(false)?;
    nav.SetIsSettingsVisible(false)?;
    nav.SetAlwaysShowHeader(false)?;
    nav.SetOpenPaneLength(200.0)?;

    let tabs: &[(&str, &str, &str)] = &[
        ("general", "Tổng quan", glyph::SETTINGS),
        ("keyboard", "Bàn phím", glyph::KEYBOARD),
        ("macro", "Macro", glyph::FONT),
        ("apps", "Ứng dụng", glyph::APPS),
        ("advanced", "Nâng cao", glyph::REPAIR),
        ("about", "Giới thiệu", glyph::INFO),
    ];

    let mut items = Vec::new();
    for (tag, label, g) in tabs {
        let item = NavigationViewItem::new()?;
        item.SetContent(&PropertyValue::CreateString(&HSTRING::from(*label))?)?;
        item.SetTag(&PropertyValue::CreateString(&HSTRING::from(*tag))?)?;
        let icon = font_icon(g, 15.0)?;
        item.SetIcon(&icon)?;
        item.SetSelectsOnInvoked(true)?;
        nav.MenuItems()?.Append(&item)?;
        items.push(item);
    }

    // -- panes ----------------------------------------------------------------
    let content = Grid::new()?;
    let panes: Vec<ScrollViewer> = vec![
        general_pane(&state)?,
        keyboard_pane(&state)?,
        macro_pane(&state)?,
        apps_pane(&state)?,
        advanced_pane(&state)?,
        about_pane()?,
    ];
    for (i, p) in panes.iter().enumerate() {
        p.SetVisibility(if i == 0 {
            Visibility::Visible
        } else {
            Visibility::Collapsed
        })?;
        content.Children()?.Append(p)?;
    }
    nav.SetContent(&content)?;

    // Selection → pane visibility.
    {
        let panes = panes.clone();
        let items = items.clone();
        nav.SelectionChanged(&TypedEventHandler::<
            NavigationView,
            NavigationViewSelectionChangedEventArgs,
        >::new(move |_nav, args| {
            let Some(args) = args.as_ref() else {
                return Ok(());
            };
            let selected = args.SelectedItem()?;
            // Compare canonical IUnknown identity — COM pointer equality.
            let selected_unk = selected.cast::<windows_core::IUnknown>().ok();
            let idx = items
                .iter()
                .position(|item| {
                    let item_unk = item.cast::<windows_core::IUnknown>().ok();
                    selected_unk.is_some()
                        && item_unk.is_some()
                        && Interface::as_raw(selected_unk.as_ref().unwrap())
                            == Interface::as_raw(item_unk.as_ref().unwrap())
                })
                .unwrap_or(0);
            for (i, p) in panes.iter().enumerate() {
                p.SetVisibility(if i == idx {
                    Visibility::Visible
                } else {
                    Visibility::Collapsed
                })?;
            }
            Ok(())
        }))?;
    }

    if let Some(first) = items.first() {
        nav.SetSelectedItem(first)?;
    }

    window.SetContent(&nav)?;
    window.Activate()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Panes
// ---------------------------------------------------------------------------

fn general_pane(state: &Shared) -> R<ScrollViewer> {
    let (scroll, stack) = pane()?;

    // Engine master toggle — the big "Tiếng Việt / English" card.
    {
        let c = card()?;
        let inner = card_stack(&c)?;
        let row = grid(&[px(44.0), STAR, AUTO])?;
        row.SetPadding(Thickness {
            Left: 16.0,
            Top: 14.0,
            Right: 16.0,
            Bottom: 14.0,
        })?;
        let icon = font_icon(glyph::KEYBOARD, 22.0)?;
        icon.SetVerticalAlignment(VerticalAlignment::Center)?;
        put(&row, 0, &icon)?;
        let texts = StackPanel::new()?;
        texts.SetOrientation(Orientation::Vertical)?;
        texts.SetSpacing(3.0)?;
        texts
            .Children()?
            .Append(&text("Tiếng Việt (UVie)", 14.0)?)?;
        let d = text("Bộ gõ Tiếng Việt cho Windows", 11.0)?;
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

    // Input method — segmented-style radio group.
    {
        let inner = section(&stack, "Bảng mã gõ")?;
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
        combo.SetHorizontalAlignment(HorizontalAlignment::Left)?;
        combo.SetMinWidth(220.0)?;
        combo.SetMargin(thickness(14.0))?;
        inner.Children()?.Append(&combo)?;
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
    let (scroll, stack) = pane()?;

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
    let (scroll, stack) = pane()?;

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

    // Add form: two inputs + add button.
    {
        let row = grid(&[px(140.0), STAR, AUTO])?;
        row.SetPadding(Thickness {
            Left: 0.0,
            Top: 10.0,
            Right: 0.0,
            Bottom: 0.0,
        })?;

        let abbr = TextBox::new()?;
        abbr.SetPlaceholderText(&HSTRING::from("Viết tắt (vd: sg)"))?;
        put(&row, 0, &abbr)?;

        let expansion = TextBox::new()?;
        expansion.SetPlaceholderText(&HSTRING::from("Văn bản thay thế (vd: Sài Gòn)"))?;
        expansion.SetMargin(Thickness {
            Left: 8.0,
            Top: 0.0,
            Right: 8.0,
            Bottom: 0.0,
        })?;
        put(&row, 1, &expansion)?;

        let add = Button::new()?;
        add.SetContent(&PropertyValue::CreateString(&HSTRING::from("Thêm"))?)?;
        let state_c = state.clone();
        let abbr_c = abbr.clone();
        let expansion_c = expansion.clone();
        let list_c = list.clone();
        add.Click(&RoutedEventHandler::new(move |_, _| {
            let trigger = abbr_c.Text()?.to_string_lossy().trim().to_string();
            let repl = expansion_c.Text()?.to_string_lossy().trim().to_string();
            if !trigger.is_empty() && !repl.is_empty() {
                state_c.macros.lock().unwrap().add(&trigger, &repl);
                state_c.save_macros();
                abbr_c.SetText(&HSTRING::new())?;
                expansion_c.SetText(&HSTRING::new())?;
                rebuild_macro_list(&list_c, &state_c)?;
            }
            Ok(())
        }))?;
        put(&row, 2, &add)?;

        stack.Children()?.Append(&row)?;
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

    // Footer: TextBox + Add + Reset.
    let footer = grid(&[STAR, AUTO, AUTO])?;
    footer.SetPadding(Thickness {
        Left: 14.0,
        Top: 0.0,
        Right: 14.0,
        Bottom: 12.0,
    })?;
    let input = TextBox::new()?;
    input.SetPlaceholderText(&HSTRING::from("vd: chrome.exe"))?;
    put(&footer, 0, &input)?;

    let add = Button::new()?;
    add.SetContent(&PropertyValue::CreateString(&HSTRING::from("Thêm"))?)?;
    add.SetMargin(Thickness {
        Left: 8.0,
        Top: 0.0,
        Right: 0.0,
        Bottom: 0.0,
    })?;
    {
        let state_c = state.clone();
        let input_c = input.clone();
        let list_c = list.clone();
        add.Click(&RoutedEventHandler::new(move |_, _| {
            let exe = input_c.Text()?.to_string_lossy().trim().to_lowercase();
            if !exe.is_empty() {
                let mut s = state_c.settings.lock().unwrap();
                let mut apps = get(&s);
                if !apps.iter().any(|a| a == &exe) {
                    apps.push(exe);
                    set(&mut s, apps);
                    drop(s);
                    state_c.save_settings();
                    input_c.SetText(&HSTRING::new())?;
                    rebuild_app_list(&list_c, &state_c, get, set)?;
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
    let (scroll, stack) = pane()?;

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
    let (scroll, stack) = pane()?;

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

fn about_pane() -> R<ScrollViewer> {
    let (scroll, stack) = pane()?;

    let center = StackPanel::new()?;
    center.SetOrientation(Orientation::Vertical)?;
    center.SetSpacing(16.0)?;
    center.SetHorizontalAlignment(HorizontalAlignment::Center)?;
    center.SetMargin(Thickness {
        Left: 0.0,
        Top: 48.0,
        Right: 0.0,
        Bottom: 0.0,
    })?;

    let title = text("UVie for Windows", 26.0)?;
    title.SetHorizontalAlignment(HorizontalAlignment::Center)?;
    center.Children()?.Append(&title)?;

    let version = text("Phiên bản 0.1.0", 13.0)?;
    secondary(&version)?;
    version.SetHorizontalAlignment(HorizontalAlignment::Center)?;
    center.Children()?.Append(&version)?;

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
    links.SetSpacing(24.0)?;
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
