//! Persistent user environment variables (HKCU\Environment).

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
};
use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
use winreg::RegKey;

use crate::error::{Error, Result};

pub fn set_user(name: &str, value: &str) -> Result<()> {
    let env = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags("Environment", KEY_SET_VALUE)
        .map_err(|e| Error::io("HKCU\\Environment", e))?;
    env.set_value(name, &value).map_err(|e| Error::io(name, e))?;
    std::env::set_var(name, value);
    broadcast();
    Ok(())
}

/// Tells Explorer and other apps that the environment changed, so new programs see it.
fn broadcast() {
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
