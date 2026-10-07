//! Embeds the application manifest (DPI awareness, common controls v6,
//! Windows 10/11 compatibility) into the exe and copies the Windows App
//! Runtime bootstrap DLL next to the binary so the WinUI 3 settings window
//! can call `MddBootstrapInitialize2` in unpackaged mode.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-changed=app.manifest");
    embed_resource::compile("app.rc", embed_resource::NONE)
        .manifest_optional()
        .expect("failed to embed app.manifest");

    // <crate>/../../Frameworks/WindowsAppSDK — fetched by
    // scripts/fetch-windowsappsdk.{ps1,sh} (dev-time dependency, like
    // uvie-mac's Frameworks/ convention).
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let dll = manifest_dir
        .join("../../Frameworks/WindowsAppSDK/Microsoft.WindowsAppRuntime.Bootstrap.dll");
    println!("cargo:rerun-if-changed={}", dll.display());
    if dll.exists() {
        // OUT_DIR = target/<profile>/build/<pkg>/out → profile dir is ../../..
        let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
        let profile_dir = out_dir.join("../../..").canonicalize().unwrap();
        let dest = profile_dir.join("Microsoft.WindowsAppRuntime.Bootstrap.dll");
        std::fs::copy(&dll, &dest).expect("failed to copy bootstrap DLL");
    } else {
        println!(
            "cargo:warning=Microsoft.WindowsAppRuntime.Bootstrap.dll not found at {}; \
             the WinUI settings window will be unavailable until \
             scripts/fetch-windowsappsdk is run",
            dll.display()
        );
    }
}
