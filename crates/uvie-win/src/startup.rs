//! Launch-at-login via the HKCU Run key — the Windows counterpart of
//! uvie-mac's `LaunchAtLoginManager` (SMAppService).

use windows::core::{w, PCWSTR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_SET_VALUE, REG_SZ,
};

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE_NAME: PCWSTR = w!("UVie");

fn exe_path() -> Option<Vec<u16>> {
    let mut buf = vec![0u16; 512];
    let n = unsafe { windows::Win32::System::LibraryLoader::GetModuleFileNameW(None, &mut buf) };
    if n == 0 {
        return None;
    }
    buf.truncate(n as usize);
    buf.push(0);
    Some(buf)
}

/// Add or remove the HKCU Run entry for this exe.
pub fn set_launch_at_login(enable: bool) -> windows::core::Result<()> {
    unsafe {
        let mut key = HKEY::default();
        RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, Some(0), KEY_SET_VALUE, &mut key).ok()?;
        let result = if enable {
            let path = exe_path().ok_or_else(windows::core::Error::from_thread)?;
            let bytes = std::slice::from_raw_parts(path.as_ptr() as *const u8, (path.len()) * 2);
            RegSetValueExW(key, VALUE_NAME, Some(0), REG_SZ, Some(bytes))
        } else {
            RegDeleteValueW(key, VALUE_NAME)
        };
        let _ = RegCloseKey(key);
        result.ok()
    }
}
