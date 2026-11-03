//! Encryption of vault files for the current user.
//!
//! Windows: DPAPI, so only this Windows user on this PC can decrypt. Elsewhere: XChaCha20-
//! Poly1305 with a random key kept in the desktop keyring (Secret Service on Linux, the
//! Keychain on macOS), or in a file only this user can read when there is no keyring. That
//! matches how the apps themselves keep their tokens on those systems.

use crate::error::Result;
use crate::paths::Paths;

/// Opens the per-user key lazily: on Linux reading it may mean a keyring prompt, which
/// commands that never touch the vault (listing apps) should not trigger.
pub struct Sealer {
    #[cfg(not(windows))]
    key: std::sync::OnceLock<std::result::Result<zeroize::Zeroizing<[u8; 32]>, crate::Error>>,
    #[cfg(not(windows))]
    paths: Paths,
}

impl Sealer {
    pub fn new(paths: &Paths) -> Sealer {
        #[cfg(windows)]
        {
            let _ = paths;
            Sealer {}
        }
        #[cfg(not(windows))]
        Sealer { key: std::sync::OnceLock::new(), paths: paths.clone() }
    }

    /// Where the key is kept, for `acswi doctor`.
    pub fn describe(&self) -> String {
        #[cfg(windows)]
        return "Windows DPAPI".into();
        #[cfg(not(windows))]
        match unix::Store::current(&self.paths) {
            Some(unix::Store::File) => format!("key file {}", self.paths.key_file().display()),
            Some(unix::Store::Keyring) => "key in the desktop keyring".into(),
            None => "no key yet".into(),
        }
    }

    pub fn seal(&self, plain: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
        #[cfg(windows)]
        return crate::dpapi::protect(plain, aad);
        #[cfg(not(windows))]
        unix::seal(self.key()?, plain, aad)
    }

    pub fn open(&self, sealed: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
        #[cfg(windows)]
        return crate::dpapi::unprotect(sealed, aad);
        #[cfg(not(windows))]
        unix::open(self.key()?, sealed, aad)
    }

    #[cfg(not(windows))]
    fn key(&self) -> Result<&[u8; 32]> {
        match self.key.get_or_init(|| unix::load_or_create(&self.paths)) {
            Ok(k) => Ok(k),
            Err(e) => Err(e.clone()),
        }
    }
}

#[cfg(not(windows))]
mod unix {
    use std::fs;
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    use chacha20poly1305::aead::{Aead, KeyInit, Payload};
    use chacha20poly1305::{XChaCha20Poly1305, XNonce};
    use zeroize::Zeroizing;

    use crate::error::{Code, Error, Result};
    use crate::paths::Paths;

    const NONCE: usize = 24;
    const SERVICE: &str = "acswi";

    pub enum Store {
        File,
        Keyring,
    }

    impl Store {
        /// Which store holds the key, once one was made. The keyring marker matters: if the
        /// keyring can't be reached later, acswi must not quietly start over with a new key.
        pub fn current(paths: &Paths) -> Option<Store> {
            if paths.key_file().exists() {
                Some(Store::File)
            } else if paths.keyring_marker().exists() {
                Some(Store::Keyring)
            } else {
                None
            }
        }
    }

    fn crypto(msg: impl std::fmt::Display) -> Error {
        Error::new(Code::Crypto, msg.to_string())
    }

    pub fn seal(key: &[u8; 32], plain: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
        let mut nonce = [0u8; NONCE];
        getrandom::getrandom(&mut nonce).map_err(crypto)?;
        let cipher = XChaCha20Poly1305::new(key.into())
            .encrypt(XNonce::from_slice(&nonce), Payload { msg: plain, aad })
            .map_err(|_| crypto("encryption failed"))?;
        let mut out = nonce.to_vec();
        out.extend_from_slice(&cipher);
        Ok(out)
    }

    pub fn open(key: &[u8; 32], sealed: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
        if sealed.len() < NONCE {
            return Err(crypto("vault file is too short"));
        }
        let (nonce, body) = sealed.split_at(NONCE);
        XChaCha20Poly1305::new(key.into())
            .decrypt(XNonce::from_slice(nonce), Payload { msg: body, aad })
            .map_err(|_| crypto("could not decrypt: the vault key does not match this file"))
    }

    pub fn load_or_create(paths: &Paths) -> Result<Zeroizing<[u8; 32]>> {
        match Store::current(paths) {
            Some(Store::File) => read_file(paths),
            Some(Store::Keyring) => keyring::get(paths)?.ok_or_else(|| {
                crypto("the vault key is missing from your keyring, so saved accounts can't be opened")
            }),
            None => {
                let mut key = Zeroizing::new([0u8; 32]);
                getrandom::getrandom(key.as_mut()).map_err(crypto)?;
                if keyring::put(paths, &key) {
                    // Empty marker: tells later runs the key is in the keyring.
                    fs::write(paths.keyring_marker(), b"").map_err(|e| Error::io(paths.keyring_marker().display(), e))?;
                } else if !write_file(paths, &key)? {
                    // Another acswi (the CLI and the window at once) made the file first.
                    return read_file(paths);
                }
                Ok(key)
            }
        }
    }

    fn read_file(paths: &Paths) -> Result<Zeroizing<[u8; 32]>> {
        let file = paths.key_file();
        let bytes = Zeroizing::new(fs::read(&file).map_err(|e| Error::io(file.display(), e))?);
        let key: [u8; 32] = bytes.as_slice().try_into().map_err(|_| crypto(format!("{} is damaged", file.display())))?;
        Ok(Zeroizing::new(key))
    }

    /// Writes the key file complete or not at all; false if one already exists.
    fn write_file(paths: &Paths, key: &[u8; 32]) -> Result<bool> {
        let file = paths.key_file();
        let tmp = file.with_extension(format!("tmp-{}", std::process::id()));
        let write = || -> std::io::Result<()> {
            let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
            f.write_all(key)?;
            f.sync_all()
        };
        let linked = write().and_then(|_| fs::hard_link(&tmp, &file));
        let _ = fs::remove_file(&tmp);
        match linked {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
            Err(e) => Err(Error::io(file.display(), e)),
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn keyring_disabled() -> bool {
        std::env::var_os("ACSWI_NO_KEYRING").is_some()
    }

    /// Secret Service (GNOME Keyring, KWallet). A new key only goes in when the keyring is
    /// already unlocked, as it is after a desktop login, so acswi never causes a password
    /// prompt on its own; reading a key back may ask the user to unlock.
    #[cfg(target_os = "linux")]
    mod keyring {
        use std::collections::HashMap;

        use secret_service::blocking::SecretService;
        use secret_service::EncryptionType;

        use super::*;

        fn attributes(root: &str) -> HashMap<&str, &str> {
            // One key per data folder, so ACSWI_HOME test folders never share a key.
            HashMap::from([("application", SERVICE), ("data", root)])
        }

        fn unreachable(e: secret_service::Error) -> Error {
            crypto(format!("the keyring can't be reached ({e}). Unlock it and try again."))
        }

        pub fn get(paths: &Paths) -> Result<Option<Zeroizing<[u8; 32]>>> {
            let ss = SecretService::connect(EncryptionType::Dh).map_err(unreachable)?;
            let root = paths.root.to_string_lossy();
            let found = ss.search_items(attributes(&root)).map_err(unreachable)?;
            let Some(item) = found.unlocked.into_iter().chain(found.locked).next() else { return Ok(None) };
            item.ensure_unlocked().map_err(unreachable)?;
            let secret = Zeroizing::new(item.get_secret().map_err(unreachable)?);
            let key: [u8; 32] =
                secret.as_slice().try_into().map_err(|_| crypto("the vault key in your keyring is damaged"))?;
            Ok(Some(Zeroizing::new(key)))
        }

        /// Stores a new key; false if there is no usable keyring (then a key file is used).
        pub fn put(paths: &Paths, key: &[u8; 32]) -> bool {
            if keyring_disabled() {
                return false;
            }
            let Ok(ss) = SecretService::connect(EncryptionType::Dh) else { return false };
            let Ok(collection) = ss.get_default_collection() else { return false };
            if collection.is_locked().unwrap_or(true) {
                return false;
            }
            let root = paths.root.to_string_lossy();
            let stored = collection
                .create_item("acswi vault key", attributes(&root), key, true, "application/octet-stream")
                .is_ok();
            // Read it back: some keyrings accept a write and lose it.
            stored && matches!(get(paths), Ok(Some(k)) if *k == *key)
        }
    }

    #[cfg(target_os = "macos")]
    mod keyring {
        use super::*;

        fn entry(paths: &Paths) -> Option<::keyring::Entry> {
            // One key per data folder, so ACSWI_HOME test folders never share a key.
            ::keyring::Entry::new(SERVICE, &paths.root.to_string_lossy()).ok()
        }

        pub fn get(paths: &Paths) -> Result<Option<Zeroizing<[u8; 32]>>> {
            let entry = entry(paths).ok_or_else(|| crypto("the keyring can't be reached"))?;
            match entry.get_secret() {
                Ok(secret) => {
                    let secret = Zeroizing::new(secret);
                    let key: [u8; 32] = secret.as_slice().try_into().map_err(|_| crypto("the vault key in your keyring is damaged"))?;
                    Ok(Some(Zeroizing::new(key)))
                }
                Err(::keyring::Error::NoEntry) => Ok(None),
                Err(e) => Err(crypto(format!("the keyring can't be reached ({e}). Unlock it and try again."))),
            }
        }

        /// Stores a new key; false if there is no usable keyring (then a key file is used).
        pub fn put(paths: &Paths, key: &[u8; 32]) -> bool {
            if keyring_disabled() {
                return false;
            }
            let Some(entry) = entry(paths) else { return false };
            // Read it back: some keyrings accept a write and lose it.
            entry.set_secret(key).is_ok() && matches!(get(paths), Ok(Some(k)) if *k == *key)
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    mod keyring {
        use super::*;

        pub fn get(_: &Paths) -> Result<Option<Zeroizing<[u8; 32]>>> {
            Err(crypto("no keyring on this system"))
        }

        pub fn put(_: &Paths, _: &[u8; 32]) -> bool {
            false
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        #[test]
        fn key_file_round_trip() {
            std::env::set_var("ACSWI_NO_KEYRING", "1");
            let tmp = tempfile::tempdir().unwrap();
            let paths = Paths { root: tmp.path().to_path_buf(), user_defs: tmp.path().join("apps") };
            let key = load_or_create(&paths).unwrap();
            assert_eq!(fs::metadata(paths.key_file()).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(*load_or_create(&paths).unwrap(), *key, "the same key is read back");
            let sealed = seal(&key, b"secret", b"aad").unwrap();
            assert_eq!(open(&key, &sealed, b"aad").unwrap(), b"secret");
            assert!(open(&key, &sealed, b"other").is_err());
        }
    }
}
