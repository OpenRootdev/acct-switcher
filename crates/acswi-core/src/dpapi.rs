//! DPAPI (CurrentUser) encryption: only this Windows user on this PC can decrypt.

use std::ptr;

use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};

use crate::error::{Code, Error, Result};

fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 }
}

/// Copies a DPAPI-allocated output blob into a Vec and frees it.
unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
    // Wipe DPAPI's copy before freeing: it may be plaintext.
    ptr::write_bytes(out.pbData, 0, out.cbData as usize);
    LocalFree(out.pbData as _);
    bytes
}

pub fn protect(plain: &[u8], entropy: &[u8]) -> Result<Vec<u8>> {
    let input = blob(plain);
    let extra = blob(entropy);
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: ptr::null_mut() };
    let ok = unsafe {
        CryptProtectData(
            &input,
            ptr::null(),
            &extra,
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
    };
    if ok == 0 {
        return Err(Error::new(Code::Crypto, format!("DPAPI encrypt failed: {}", std::io::Error::last_os_error())));
    }
    Ok(unsafe { take(out) })
}

pub fn unprotect(cipher: &[u8], entropy: &[u8]) -> Result<Vec<u8>> {
    let input = blob(cipher);
    let extra = blob(entropy);
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: ptr::null_mut() };
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            ptr::null_mut(),
            &extra,
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
    };
    if ok == 0 {
        return Err(Error::new(
            Code::Crypto,
            format!(
                "DPAPI decrypt failed ({}). The vault can only be opened by the Windows user and PC that created it.",
                std::io::Error::last_os_error()
            ),
        ));
    }
    Ok(unsafe { take(out) })
}

#[cfg(test)]
mod tests {
    #[test]
    fn round_trip() {
        let c = super::protect(b"secret", b"salt").unwrap();
        assert_ne!(c, b"secret");
        assert_eq!(super::unprotect(&c, b"salt").unwrap(), b"secret");
        assert!(super::unprotect(&c, b"other").is_err());
    }
}
