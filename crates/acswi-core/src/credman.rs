//! Windows Credential Manager (generic credentials) read / write / delete.

use std::ptr;

use serde::{Deserialize, Serialize};
use windows_sys::Win32::Foundation::{GetLastError, ERROR_NOT_FOUND, FILETIME};
use windows_sys::Win32::Security::Credentials::{
    CredDeleteW, CredEnumerateW, CredFree, CredWriteW, CREDENTIALW, CREDENTIAL_ATTRIBUTEW,
    CRED_TYPE_GENERIC,
};
use zeroize::Zeroize;

use crate::error::{Code, Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attribute {
    pub keyword: String,
    pub flags: u32,
    #[serde(with = "serde_bytes")]
    pub value: Vec<u8>,
}

/// One generic credential, including its secret blob.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credential {
    pub target: String,
    pub user: String,
    pub comment: String,
    pub alias: String,
    pub persist: u32,
    pub flags: u32,
    #[serde(with = "serde_bytes")]
    pub blob: Vec<u8>,
    pub attributes: Vec<Attribute>,
    /// FILETIME as u64; not restored, used to spot changes.
    #[serde(default)]
    pub last_written: u64,
}

impl Drop for Credential {
    fn drop(&mut self) {
        self.blob.zeroize();
    }
}

/// Target name and last write time only — never the secret.
#[derive(Debug, Clone, Serialize)]
pub struct CredentialInfo {
    pub target: String,
    pub user: String,
    pub last_written: u64,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn from_wide(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0;
    while *p.add(len) != 0 {
        len += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
}

fn filetime(ft: FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
}

/// Lists generic credentials whose target matches `filter` (`*` allowed at the end, as
/// Credential Manager supports). `None` lists everything.
pub fn read(filter: Option<&str>) -> Result<Vec<Credential>> {
    let filter_w = filter.map(wide);
    let mut count = 0u32;
    let mut list: *mut *mut CREDENTIALW = ptr::null_mut();
    let ok = unsafe {
        CredEnumerateW(
            filter_w.as_ref().map_or(ptr::null(), |w| w.as_ptr()),
            0,
            &mut count,
            &mut list,
        )
    };
    if ok == 0 {
        let err = unsafe { GetLastError() };
        if err == ERROR_NOT_FOUND {
            return Ok(Vec::new());
        }
        return Err(Error::new(Code::Io, format!("Credential Manager read failed (error {err})")));
    }
    let mut out = Vec::with_capacity(count as usize);
    unsafe {
        for i in 0..count as usize {
            let c = &**list.add(i);
            if c.Type != CRED_TYPE_GENERIC {
                continue;
            }
            let mut blob = Vec::new();
            if !c.CredentialBlob.is_null() && c.CredentialBlobSize > 0 {
                blob = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec();
            }
            let mut attributes = Vec::new();
            for a in 0..c.AttributeCount as usize {
                let attr = &*c.Attributes.add(a);
                let value = if attr.Value.is_null() {
                    Vec::new()
                } else {
                    std::slice::from_raw_parts(attr.Value, attr.ValueSize as usize).to_vec()
                };
                attributes.push(Attribute { keyword: from_wide(attr.Keyword), flags: attr.Flags, value });
            }
            out.push(Credential {
                target: from_wide(c.TargetName),
                user: from_wide(c.UserName),
                comment: from_wide(c.Comment),
                alias: from_wide(c.TargetAlias),
                persist: c.Persist,
                flags: c.Flags,
                blob,
                attributes,
                last_written: filetime(c.LastWritten),
            });
        }
        CredFree(list as *const _);
    }
    Ok(out)
}

/// Names and timestamps only, for change detection (`acswi learn`).
pub fn list_info(filter: Option<&str>) -> Result<Vec<CredentialInfo>> {
    Ok(read(filter)?
        .iter()
        .map(|c| CredentialInfo { target: c.target.clone(), user: c.user.clone(), last_written: c.last_written })
        .collect())
}

pub fn write(cred: &Credential) -> Result<()> {
    let mut target = wide(&cred.target);
    let mut user = wide(&cred.user);
    let mut comment = wide(&cred.comment);
    let mut alias = wide(&cred.alias);
    let mut blob = cred.blob.clone();
    let mut keywords: Vec<Vec<u16>> = cred.attributes.iter().map(|a| wide(&a.keyword)).collect();
    let mut values: Vec<Vec<u8>> = cred.attributes.iter().map(|a| a.value.clone()).collect();
    let mut attrs: Vec<CREDENTIAL_ATTRIBUTEW> = cred
        .attributes
        .iter()
        .enumerate()
        .map(|(i, a)| CREDENTIAL_ATTRIBUTEW {
            Keyword: keywords[i].as_mut_ptr(),
            Flags: a.flags,
            ValueSize: values[i].len() as u32,
            Value: values[i].as_mut_ptr(),
        })
        .collect();
    let c = CREDENTIALW {
        Flags: 0,
        Type: CRED_TYPE_GENERIC,
        TargetName: target.as_mut_ptr(),
        Comment: if cred.comment.is_empty() { ptr::null_mut() } else { comment.as_mut_ptr() },
        LastWritten: FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 },
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: if blob.is_empty() { ptr::null_mut() } else { blob.as_mut_ptr() },
        Persist: cred.persist,
        AttributeCount: attrs.len() as u32,
        Attributes: if attrs.is_empty() { ptr::null_mut() } else { attrs.as_mut_ptr() },
        TargetAlias: if cred.alias.is_empty() { ptr::null_mut() } else { alias.as_mut_ptr() },
        UserName: if cred.user.is_empty() { ptr::null_mut() } else { user.as_mut_ptr() },
    };
    let ok = unsafe { CredWriteW(&c, 0) };
    blob.zeroize();
    if ok == 0 {
        return Err(Error::new(
            Code::Io,
            format!("Credential Manager write failed for {}: {}", cred.target, std::io::Error::last_os_error()),
        ));
    }
    Ok(())
}

pub fn delete(target: &str) -> Result<()> {
    let t = wide(target);
    let ok = unsafe { CredDeleteW(t.as_ptr(), CRED_TYPE_GENERIC, 0) };
    if ok == 0 {
        let err = unsafe { GetLastError() };
        if err != ERROR_NOT_FOUND {
            return Err(Error::new(Code::Io, format!("Credential Manager delete failed for {target} (error {err})")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_read_delete() {
        let target = "acswi-selftest:credman:1";
        let cred = Credential {
            target: target.into(),
            user: "tester".into(),
            comment: String::new(),
            alias: String::new(),
            persist: 2,
            flags: 0,
            blob: b"not-a-real-secret".to_vec(),
            attributes: vec![],
            last_written: 0,
        };
        write(&cred).unwrap();
        let got = read(Some("acswi-selftest:credman:*")).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].blob, b"not-a-real-secret");
        assert_eq!(got[0].user, "tester");
        delete(target).unwrap();
        assert!(read(Some(target)).unwrap().is_empty());
    }
}
