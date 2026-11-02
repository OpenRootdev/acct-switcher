//! App definitions: where an app keeps its sign-in state and how to switch it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Code, Error, Result};
use crate::paths::{expand, valid_name, which};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    /// Save live state into the vault and restore another profile's state.
    #[default]
    Swap,
    /// The app already keeps several accounts; acct calls its own switch command.
    Native,
    /// One config folder per profile, selected by a user environment variable.
    Env,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DirMode {
    /// Pack the folder's files into the encrypted vault (small config folders).
    #[default]
    Encrypt,
    /// Move the whole folder aside (large app profiles). Not re-encrypted.
    Move,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StateItem {
    File {
        path: String,
    },
    Dir {
        path: String,
        #[serde(default)]
        mode: DirMode,
        /// Globs (relative to the folder) left out of the profile and shared by all profiles.
        #[serde(default)]
        exclude: Vec<String>,
    },
    Credential {
        /// Credential Manager target name; `*` and `?` wildcards allowed.
        pattern: String,
    },
    Registry {
        /// Key under HKEY_CURRENT_USER, e.g. `Software\Vendor\App\Auth`.
        key: String,
    },
    JsonKey {
        path: String,
        /// JSON pointers of the keys that belong to the account, e.g. `/oauthAccount`.
        keys: Vec<String>,
    },
}

impl StateItem {
    pub fn describe(&self) -> String {
        match self {
            StateItem::File { path } => format!("file {path}"),
            StateItem::Dir { path, mode, .. } => format!("folder {path} ({})", if *mode == DirMode::Move { "moved" } else { "encrypted" }),
            StateItem::Credential { pattern } => format!("credentials {pattern}"),
            StateItem::Registry { key } => format!("registry HKCU\\{key}"),
            StateItem::JsonKey { path, keys } => format!("{} in {path}", keys.join(", ")),
        }
    }
}

/// How to find out who is signed in. The result must be an account label, never a secret.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentitySpec {
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default)]
    pub file: Option<String>,
    /// JSON pointer into the command output or file; without it the first output line is used.
    #[serde(default)]
    pub pointer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSpec {
    /// Command printing the known accounts as JSON.
    #[serde(default)]
    pub list: Vec<String>,
    /// Or: a JSON file holding them (faster than running the tool; must not hold secrets).
    #[serde(default)]
    pub list_file: Option<String>,
    /// Pointer to the array of accounts in that output ("" = the root).
    #[serde(default)]
    pub items: String,
    /// Pointer (inside one item) to the account label.
    pub account: String,
    /// Pointer to a boolean that is true for the active account.
    #[serde(default)]
    pub active: Option<String>,
    /// Pointer to the value passed to `switch` as `{value}`; defaults to the account label.
    #[serde(default)]
    pub value: Option<String>,
    /// Command that makes `{value}` active.
    pub switch: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvSpec {
    /// User environment variable that points the app at a config folder.
    pub var: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppDef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub strategy: Strategy,
    /// Executables or paths that show the app is installed (any match).
    #[serde(default)]
    pub detect: Vec<String>,
    /// Processes that must be closed before swapping: an image name (`Discord.exe`)
    /// or a full-path pattern with `*` (`*\.local\bin\claude.exe`).
    #[serde(default)]
    pub processes: Vec<String>,
    /// Program (and arguments) to start again after switching, if the app was running.
    #[serde(default)]
    pub launch: Vec<String>,
    /// Command that signs in a new account. Without it, acct opens the app and waits.
    #[serde(default)]
    pub login: Vec<String>,
    #[serde(default)]
    pub identity: Option<IdentitySpec>,
    #[serde(default)]
    pub state: Vec<StateItem>,
    #[serde(default)]
    pub native: Option<NativeSpec>,
    #[serde(default)]
    pub env: Option<EnvSpec>,
    /// Shown to the user: limits, tips.
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(skip)]
    pub source: Option<PathBuf>,
}

impl AppDef {
    pub fn parse(text: &str, source: Option<&Path>) -> Result<AppDef> {
        let mut def: AppDef = toml::from_str(text).map_err(|e| {
            let at = source.map(|p| format!("{}: ", p.display())).unwrap_or_default();
            Error::new(Code::InvalidDefinition, format!("{at}{e}"))
        })?;
        def.source = source.map(Path::to_path_buf);
        def.validate()?;
        Ok(def)
    }

    fn validate(&self) -> Result<()> {
        let fail = |msg: String| Err(Error::new(Code::InvalidDefinition, format!("{}: {msg}", self.id)));
        if !valid_name(&self.id) {
            return fail("id may only use letters, digits, - _ . @".into());
        }
        match self.strategy {
            Strategy::Swap if self.state.is_empty() => fail("swap strategy needs at least one [[state]] item".into()),
            Strategy::Native if self.native.is_none() => fail("native strategy needs a [native] table".into()),
            Strategy::Native if self.native.as_ref().is_some_and(|n| n.list.is_empty() && n.list_file.is_none()) => {
                fail("[native] needs `list` or `list_file`".into())
            }
            Strategy::Env if self.env.is_none() => fail("env strategy needs an [env] table".into()),
            _ => Ok(()),
        }?;
        for item in &self.state {
            if let StateItem::Credential { pattern } = item {
                if pattern.trim_matches(['*', '?']).is_empty() {
                    return fail(format!("credential pattern {pattern:?} would match every credential"));
                }
            }
        }
        Ok(())
    }

    pub fn installed(&self) -> bool {
        self.detect.is_empty() || self.detect.iter().any(|d| which(d).is_some() || expand(d).exists())
    }
}

/// Built-in definitions, embedded at compile time.
const BUILTIN: &[(&str, &str)] = &[
    ("gh", include_str!("../../../defs/gh.toml")),
    ("az", include_str!("../../../defs/az.toml")),
    ("claude", include_str!("../../../defs/claude.toml")),
    ("codex", include_str!("../../../defs/codex.toml")),
    ("github-desktop", include_str!("../../../defs/github-desktop.toml")),
    ("discord", include_str!("../../../defs/discord.toml")),
];

#[derive(Debug, Clone, Default)]
pub struct Defs {
    pub apps: BTreeMap<String, AppDef>,
    /// Definitions that failed to load; reported by `acct doctor`.
    pub errors: Vec<Error>,
}

impl Defs {
    pub fn load(user_dir: &Path) -> Defs {
        let mut defs = Defs::default();
        for (name, text) in BUILTIN {
            match AppDef::parse(text, None) {
                Ok(d) => {
                    defs.apps.insert(d.id.clone(), d);
                }
                Err(e) => defs.errors.push(Error::new(e.code, format!("built-in {name}: {}", e.message))),
            }
        }
        if let Ok(entries) = std::fs::read_dir(user_dir) {
            let mut files: Vec<PathBuf> = entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("toml")))
                .collect();
            files.sort();
            for path in files {
                match std::fs::read_to_string(&path)
                    .map_err(|e| Error::io(path.display(), e))
                    .and_then(|t| AppDef::parse(&t, Some(&path)))
                {
                    Ok(d) => {
                        defs.apps.insert(d.id.clone(), d);
                    }
                    Err(e) => defs.errors.push(e),
                }
            }
        }
        defs
    }

    pub fn get(&self, id: &str) -> Result<&AppDef> {
        if let Some(d) = self.apps.get(id) {
            return Ok(d);
        }
        let lower = id.to_ascii_lowercase();
        self.apps
            .values()
            .find(|d| d.id.eq_ignore_ascii_case(&lower) || d.name.eq_ignore_ascii_case(id))
            .ok_or_else(|| {
                Error::new(Code::UnknownApp, format!("No app called {id:?}. Run `acct apps` to see the list."))
                    .with(serde_json::json!({ "known": self.apps.keys().collect::<Vec<_>>() }))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_parse() {
        let defs = Defs::load(Path::new("Z:\\does-not-exist"));
        assert!(defs.errors.is_empty(), "{:?}", defs.errors);
        assert!(defs.apps.contains_key("gh"));
        assert_eq!(defs.get("GH").unwrap().id, "gh");
    }

    #[test]
    fn rejects_bad_pattern() {
        let t = "id='x'\nname='X'\n[[state]]\nkind='credential'\npattern='**'\n";
        assert_eq!(AppDef::parse(t, None).unwrap_err().code, Code::InvalidDefinition);
    }
}
