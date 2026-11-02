//! Encrypted per-profile snapshots: `vault\<app>\<profile>.bin`.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::credman::Credential;
use crate::dpapi;
use crate::error::{Code, Error, Result};
use crate::paths::Paths;
use crate::state::write_atomic;

// From when the tool was called acct; kept so vaults saved then still open.
const MAGIC: &[u8; 6] = b"ACCT1\n";

#[derive(Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub app: String,
    pub saved: DateTime<Utc>,
    /// Keyed by `StateItem::describe()`, so editing a definition does not misalign items.
    pub items: Vec<(String, Saved)>,
}

impl Snapshot {
    pub fn get(&self, key: &str) -> Option<&Saved> {
        self.items.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PackedFile {
    /// Relative path with `/` separators.
    pub path: String,
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
}

impl Drop for PackedFile {
    fn drop(&mut self) {
        self.data.zeroize();
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegValue {
    pub name: String,
    pub kind: u32,
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct RegNode {
    pub values: Vec<RegValue>,
    pub keys: Vec<(String, RegNode)>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Saved {
    /// `None`: the file did not exist.
    File(#[serde(with = "serde_bytes")] Option<Vec<u8>>),
    Dir(Vec<PackedFile>),
    /// The folder was moved into the profile store; `false` if there was none.
    Moved(bool),
    Credentials(Vec<Credential>),
    Registry(Option<RegNode>),
    /// JSON text per key (`None`: key absent).
    JsonKey(Vec<Option<String>>),
}

impl Drop for Saved {
    fn drop(&mut self) {
        match self {
            Saved::File(Some(d)) => d.zeroize(),
            Saved::JsonKey(v) => v.iter_mut().flatten().for_each(|s| s.zeroize()),
            Saved::Registry(Some(n)) => wipe(n),
            _ => {}
        }
    }
}

fn wipe(n: &mut RegNode) {
    n.values.iter_mut().for_each(|v| v.data.zeroize());
    n.keys.iter_mut().for_each(|(_, k)| wipe(k));
}

pub struct Vault {
    paths: Paths,
    entropy: Vec<u8>,
}

impl Vault {
    pub fn open(paths: &Paths) -> Result<Vault> {
        let file = paths.entropy_file();
        let entropy = match fs::read(&file) {
            Ok(b) if b.len() == 32 => b,
            Ok(_) => return Err(Error::new(Code::Crypto, format!("{} is damaged", file.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut b = vec![0u8; 32];
                getrandom::getrandom(&mut b).map_err(|e| Error::new(Code::Crypto, e.to_string()))?;
                write_atomic(&file, &b)?;
                b
            }
            Err(e) => return Err(Error::io(file.display(), e)),
        };
        Ok(Vault { paths: paths.clone(), entropy })
    }

    pub fn path(&self, app: &str, profile: &str) -> PathBuf {
        self.paths.vault_blob(app, profile)
    }

    pub fn exists(&self, app: &str, profile: &str) -> bool {
        self.path(app, profile).exists()
    }

    pub fn write(&self, path: &Path, snap: &Snapshot) -> Result<()> {
        let mut plain = rmp_serde::to_vec(snap).map_err(|e| Error::new(Code::Io, e.to_string()))?;
        let cipher = dpapi::protect(&plain, &self.entropy);
        plain.zeroize();
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&cipher?);
        write_atomic(path, &out)
    }

    pub fn read(&self, app: &str, profile: &str) -> Result<Option<Snapshot>> {
        self.read_path(&self.path(app, profile))
    }

    pub fn read_path(&self, path: &Path) -> Result<Option<Snapshot>> {
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(Error::io(path.display(), e)),
        };
        let body = bytes
            .strip_prefix(MAGIC.as_slice())
            .ok_or_else(|| Error::new(Code::Crypto, format!("{} is not an acswi vault file", path.display())))?;
        let mut plain = dpapi::unprotect(body, &self.entropy)?;
        let snap = rmp_serde::from_slice(&plain).map_err(|e| Error::new(Code::Crypto, format!("{}: {e}", path.display())));
        plain.zeroize();
        snap.map(Some)
    }
}
