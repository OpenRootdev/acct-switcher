//! Persistent user environment variables (HKCU\Environment). Windows only: elsewhere there
//! is no single place that every new program reads them from.

use crate::error::Result;

#[cfg(windows)]
pub fn set_user(name: &str, value: &str) -> Result<()> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
    use winreg::RegKey;

    let env = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags("Environment", KEY_SET_VALUE)
        .map_err(|e| crate::Error::io("HKCU\\Environment", e))?;
    env.set_value(name, &value).map_err(|e| crate::Error::io(name, e))?;
    std::env::set_var(name, value);
    broadcast();
    Ok(())
}

#[cfg(not(windows))]
pub fn set_user(_name: &str, _value: &str) -> Result<()> {
    Err(crate::Error::new(crate::Code::Unsupported, "the env strategy only works on Windows"))
}

/// Tells Explorer and other apps that the environment changed, so new programs see it.
#[cfg(windows)]
fn broadcast() {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };

    let param: Vec<u16> = "Environment\0".encode_utf16().collect();
    let mut result = 0usize;
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST as HWND,
            WM_SETTINGCHANGE,
            0,
            param.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            2000,
            &mut result,
        );
    }
}
