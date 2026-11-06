//! App definitions: where an app keeps its sign-in state and how to switch it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Code, Error, Result};
use crate::expiry::ExpirySpec;
use crate::paths::{expand, valid_name, which};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    /// Save live state into the vault and restore another profile's state.
    #[default]
    Swap,
    /// The app already keeps several accounts; acswi calls its own switch command.
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
    /// Command that signs in a new account. Without it, acswi opens the app and waits.
    #[serde(default)]
    pub login: Vec<String>,
    /// Extra environment variables for the sign-in command only.
    #[serde(default)]
    pub login_env: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub identity: Option<IdentitySpec>,
    #[serde(default)]
    pub state: Vec<StateItem>,
    #[serde(default)]
    pub native: Option<NativeSpec>,
    #[serde(default)]
    pub env: Option<EnvSpec>,
    /// When a saved sign-in stops working, so acswi can warn before it does.
    #[serde(default)]
    pub expiry: Option<ExpirySpec>,
    /// Shown to the user: limits, tips.
    #[serde(default)]
    pub notes: Option<String>,
    /// Systems the definition works on (`windows`, `linux`, `macos`); empty means all.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub platforms: Vec<String>,
    /// Replacements for one system, e.g. `[os.linux]` with Linux paths and process names.
    /// Applied when the definition is loaded, so the rest of acswi only sees the result.
    #[serde(default, skip_serializing)]
    pub os: BTreeMap<String, OsOverride>,
    #[serde(skip)]
    pub source: Option<PathBuf>,
}

/// Fields an `[os.<name>]` table may replace. Unset fields keep the main value.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OsOverride {
    pub detect: Option<Vec<String>>,
    pub processes: Option<Vec<String>>,
    pub launch: Option<Vec<String>>,
    pub login: Option<Vec<String>>,
    pub login_env: Option<BTreeMap<String, String>>,
    pub identity: Option<IdentitySpec>,
    pub state: Option<Vec<StateItem>>,
    pub native: Option<NativeSpec>,
    pub env: Option<EnvSpec>,
    pub expiry: Option<ExpirySpec>,
    pub notes: Option<String>,
}

const SYSTEMS: &[&str] = &["windows", "linux", "macos"];

impl AppDef {
    pub fn parse(text: &str, source: Option<&Path>) -> Result<AppDef> {
        let mut def: AppDef = toml::from_str(text).map_err(|e| {
            let at = source.map(|p| format!("{}: ", p.display())).unwrap_or_default();
            Error::new(Code::InvalidDefinition, format!("{at}{e}"))
        })?;
        def.source = source.map(Path::to_path_buf);
        def.apply_os(std::env::consts::OS)?;
        def.validate()?;
        Ok(def)
    }

    /// Whether the definition is meant for this system.
    pub fn supported(&self) -> bool {
        self.platforms.is_empty() || self.platforms.iter().any(|p| p == std::env::consts::OS)
    }

    fn apply_os(&mut self, system: &str) -> Result<()> {
        if let Some(bad) = self.platforms.iter().chain(self.os.keys()).find(|p| !SYSTEMS.contains(&p.as_str())) {
            return Err(Error::new(
                Code::InvalidDefinition,
                format!("{}: unknown system {bad:?}; use {}", self.id, SYSTEMS.join(", ")),
            ));
        }
        let Some(o) = self.os.remove(system) else { return Ok(()) };
        self.os.clear();
        macro_rules! take {
            ($($field:ident),*) => { $(if let Some(v) = o.$field { self.$field = v; })* };
        }
        take!(detect, processes, launch, login, login_env, state);
        macro_rules! take_opt {
            ($($field:ident),*) => { $(if o.$field.is_some() { self.$field = o.$field; })* };
        }
        take_opt!(identity, native, env, expiry, notes);
        Ok(())
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
        if let Some(e) = &self.expiry {
            e.validate().or_else(fail)?;
        }
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
    ("cursor", include_str!("../../../defs/cursor.toml")),
];

#[derive(Debug, Clone, Default)]
pub struct Defs {
    pub apps: BTreeMap<String, AppDef>,
    /// Definitions that failed to load; reported by `acswi doctor`.
    pub errors: Vec<Error>,
}

impl Defs {
    pub fn load(user_dir: &Path) -> Defs {
        let mut defs = Defs::default();
        for (name, text) in BUILTIN {
            match AppDef::parse(text, None) {
                Ok(d) if !d.supported() => {}
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
                    Ok(d) if !d.supported() => {}
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
                Error::new(Code::UnknownApp, format!("No app called {id:?}. Run `acswi apps` to see the list."))
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
    fn os_table_replaces_fields() {
        let t = format!(
            "id='x'\nname='X'\nprocesses=['x.exe']\nnotes='main'\n[[state]]\nkind='file'\npath='a'\n\
             [os.{os}]\nprocesses=['x']\n[[os.{os}.state]]\nkind='file'\npath='b'\n",
            os = std::env::consts::OS
        );
        let d = AppDef::parse(&t, None).unwrap();
        assert_eq!(d.processes, ["x"]);
        assert!(matches!(&d.state[..], [StateItem::File { path }] if path == "b"));
        assert_eq!(d.notes.as_deref(), Some("main"));
        assert!(AppDef::parse("id='x'\nname='X'\nplatforms=['dos']\n[[state]]\nkind='file'\npath='a'\n", None).is_err());
        let other = SYSTEMS.iter().find(|s| **s != std::env::consts::OS).unwrap();
        let t = format!("id='x'\nname='X'\nplatforms=['{other}']\n[[state]]\nkind='file'\npath='a'\n");
        assert!(!AppDef::parse(&t, None).unwrap().supported());
    }

    #[test]
    fn rejects_bad_pattern() {
        let t = "id='x'\nname='X'\n[[state]]\nkind='credential'\npattern='**'\n";
        assert_eq!(AppDef::parse(t, None).unwrap_err().code, Code::InvalidDefinition);
    }
}
