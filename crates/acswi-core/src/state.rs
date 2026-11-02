//! `state.json`: profiles and the active profile per app. Never holds secrets.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default)]
    pub apps: BTreeMap<String, AppState>,
}

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
    /// A sign-in started with `acswi add` that has not been finished or cancelled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<PendingAdd>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    /// Who is signed in (e-mail, login), when the app can tell us.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
    /// Native strategy: the app's own account label this profile points at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    pub created: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used: Option<DateTime<Utc>>,
}

impl Profile {
    pub fn new() -> Self {
        Profile { identity: None, account: None, created: Utc::now(), saved: None, used: None }
    }
}

impl Default for Profile {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingAdd {
    pub profile: String,
    /// Profile that was active before, restored if the sign-in is cancelled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
    pub started: DateTime<Utc>,
}

impl State {
    pub fn load(path: &Path) -> Result<State> {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| Error::new(crate::Code::Io, format!("{} is damaged: {e}", path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(State { version: 1, ..Default::default() }),
            Err(e) => Err(Error::io(path.display(), e)),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = serde_json::to_vec_pretty(self).expect("state serializes");
        write_atomic(path, &text)
    }

    pub fn app(&self, id: &str) -> Option<&AppState> {
        self.apps.get(id)
    }

    pub fn app_mut(&mut self, id: &str) -> &mut AppState {
        self.apps.entry(id.to_string()).or_default()
    }
}

/// Writes via a temp file and rename so readers never see half a file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| Error::io(dir.display(), e))?;
    }
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    {
        let mut f = fs::File::create(&tmp).map_err(|e| Error::io(tmp.display(), e))?;
        f.write_all(bytes).map_err(|e| Error::io(tmp.display(), e))?;
        f.sync_all().map_err(|e| Error::io(tmp.display(), e))?;
    }
    fs::rename(&tmp, path).map_err(|e| Error::io(path.display(), e))
}
