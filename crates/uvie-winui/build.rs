//! Generates the `Microsoft.UI.Xaml` / `Microsoft.Windows.*` bindings from the
//! Windows App SDK metadata (.winmd) files.
//!
//! The SDK metadata is fetched by `scripts/fetch-windowsappsdk.ps1` (or .sh)
//! into `Frameworks/WindowsAppSDK/` at the repo root — the same fetch-at-dev-
//! time convention uvie-mac uses for `libuvie.a` and Sparkle.

use std::path::{Path, PathBuf};

/// winmds the XAML surface needs, relative to the SDK dir.
const REQUIRED_WINMDS: &[&str] = &[
    "Microsoft.UI.Xaml.winmd",
    "Microsoft.UI.Text.winmd",
    "Microsoft.UI.winmd",
    "Microsoft.Foundation.winmd",
    "Microsoft.Graphics.winmd",
    "Microsoft.Windows.AppLifecycle.winmd",
    "Microsoft.Windows.ApplicationModel.DynamicDependency.winmd",
    "Microsoft.Windows.ApplicationModel.Resources.winmd",
    // Xaml's WebView2 control references these (from the WebView2 NuGet).
    "Microsoft.Web.WebView2.Core.winmd",
];

fn sdk_dir(manifest_dir: &Path) -> PathBuf {
    // Repo layout: <root>/crates/uvie-winui → <root>/Frameworks/WindowsAppSDK.
    manifest_dir
        .join("../../Frameworks/WindowsAppSDK")
        .canonicalize()
        .unwrap_or_else(|_| manifest_dir.join("../../Frameworks/WindowsAppSDK"))
}

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let sdk = sdk_dir(&manifest_dir);

    let mut inputs: Vec<String> = vec!["default".to_owned()];
    let mut missing = Vec::new();
    for name in REQUIRED_WINMDS {
        let p = sdk.join(name);
        println!("cargo:rerun-if-changed={}", p.display());
        if p.exists() {
            inputs.push(p.to_string_lossy().into_owned());
        } else {
            missing.push(name);
        }
    }

    if !missing.is_empty() {
        panic!(
            "Windows App SDK metadata not found under {}: missing {missing:?}.\n\
             Run scripts/fetch-windowsappsdk.ps1 (Windows) or \
             scripts/fetch-windowsappsdk.sh to download it.",
            sdk.display()
        );
    }

    let out = out_dir.join("bindings.rs");
    let mut args: Vec<String> = vec!["--out".into(), out.to_string_lossy().into_owned()];
    for input in &inputs {
        args.push("--in".into());
        args.push(input.clone());
    }
    for f in [
        "Microsoft.UI.Xaml",
        "Microsoft.UI.Xaml.Controls",
        "Microsoft.UI.Xaml.Controls.Primitives",
        "Microsoft.UI.Xaml.Input",
        "Microsoft.UI.Xaml.Media",
        "Microsoft.UI.Xaml.Markup",
        "Microsoft.UI.Windowing",
        "Microsoft.UI.Dispatching",
        "Microsoft.UI.Text",
        "Microsoft.Windows.ApplicationModel.DynamicDependency",
        // PropertyValue::CreateString/… boxes primitives into IInspectable
        // (control Header/Content params take IInspectable).
        "Windows.Foundation.PropertyValue",
    ] {
        args.push("--filter".into());
        args.push(f.into());
    }
    // windows-bindgen 0.62: single flag generating `*_Impl` traits for the
    // filtered interfaces (XAML callbacks/handlers). `--no-allow` so the
    // output can be wrapped in a `mod` (inner `#![allow]` is illegal there).
    args.push("--implement".into());
    args.push("--no-allow".into());
    // Generate Windows.Foundation.{Collections,Numerics,Async*} locally
    // instead of referencing the windows-collections/-future/-numerics
    // crates — the published crates don't carry every type (e.g.
    // IObservableVector<T> still lives in the `windows` crate).
    args.push("--no-deps".into());

    eprintln!("bindgen args: {args:?}");
    let result = std::panic::catch_unwind(|| {
        let _warnings = windows_bindgen::bindgen(args);
    });
    if let Err(e) = result {
        let msg = e
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_else(|| "<unknown panic>".into());
        panic!("windows-bindgen panicked: {msg}");
    }

    if !out.exists() {
        panic!("windows-bindgen produced no output at {}", out.display());
    }

    // windows-core 0.62 exposes `windows_core::imp::array_proxy(data, len)`
    // returning an `ArrayProxy` (DerefMut to `Array`), while bindgen emits
    // the newer `windows_core::ArrayProxy::from_raw_parts(..).as_array()`.
    // Rewrite to `&mut windows_core::imp::array_proxy(..)` which coerces to
    // the `&mut Array<T>` the _Impl traits expect.
    let src = std::fs::read_to_string(&out).unwrap();
    let mut fixed = String::with_capacity(src.len());
    let mut rest = src.as_str();
    const NEEDLE: &str = "windows_core::ArrayProxy::from_raw_parts(";
    while let Some(i) = rest.find(NEEDLE) {
        fixed.push_str(&rest[..i]);
        let call = &rest[i + NEEDLE.len() - 1..]; // starts at '('
                                                  // Match the closing ')' by depth — the args contain nested parens.
        let mut depth = 0usize;
        let mut close = None;
        for (j, ch) in call.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(j);
                        break;
                    }
                }
                _ => {}
            }
        }
        let close = close.expect("unbalanced ArrayProxy call");
        let tail = &call[close + 1..];
        let trimmed = tail.trim_start();
        if let Some(after) = trimmed.strip_prefix(".as_array()") {
            fixed.push_str("&mut windows_core::imp::array_proxy");
            fixed.push_str(&call[..close + 1]);
            rest = after;
        } else {
            fixed.push_str(NEEDLE);
            rest = &rest[i + NEEDLE.len()..];
        }
    }
    fixed.push_str(rest);
    if fixed != src {
        std::fs::write(&out, fixed).unwrap();
    }

    println!("cargo:warning=generated {}", out.display());
}
