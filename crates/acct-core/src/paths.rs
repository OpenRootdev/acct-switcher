use std::env;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Where acct keeps its own files.
#[derive(Debug, Clone)]
pub struct Paths {
    /// `%LOCALAPPDATA%\acct` (or `ACCT_HOME`): state, vault, moved folders.
    pub root: PathBuf,
    /// `%APPDATA%\acct\apps` (or `ACCT_HOME\apps`): user app definitions.
    pub user_defs: PathBuf,
}

impl Paths {
    pub fn from_env() -> Result<Self> {
        if let Some(home) = env::var_os("ACCT_HOME") {
            let root = PathBuf::from(home);
            return Ok(Paths { user_defs: root.join("apps"), root });
        }
        let local = env::var_os("LOCALAPPDATA").map(PathBuf::from);
        let roaming = env::var_os("APPDATA").map(PathBuf::from);
        match (local, roaming) {
            (Some(local), Some(roaming)) => Ok(Paths {
                root: local.join("acct"),
                user_defs: roaming.join("acct").join("apps"),
            }),
            _ => Err(Error::new(crate::Code::Io, "LOCALAPPDATA/APPDATA are not set")),
        }
    }

    pub fn state_file(&self) -> PathBuf {
        self.root.join("state.json")
    }
    pub fn journal_file(&self) -> PathBuf {
        self.root.join("journal.json")
    }
    pub fn audit_file(&self) -> PathBuf {
        self.root.join("audit.log")
    }
    pub fn lock_file(&self) -> PathBuf {
        self.root.join("lock")
    }
    pub fn entropy_file(&self) -> PathBuf {
        self.root.join("entropy")
    }
    pub fn vault_blob(&self, app: &str, profile: &str) -> PathBuf {
        self.root.join("vault").join(app).join(format!("{profile}.bin"))
    }
    pub fn profile_dir(&self, app: &str, profile: &str) -> PathBuf {
        self.root.join("profiles").join(app).join(profile)
    }
    pub fn app_profiles_dir(&self, app: &str) -> PathBuf {
        self.root.join("profiles").join(app)
    }
    pub fn app_vault_dir(&self, app: &str) -> PathBuf {
        self.root.join("vault").join(app)
    }
}

/// Expands `~`, `%VAR%` and `$VAR`-free paths used in app definitions.
pub fn expand(raw: &str) -> PathBuf {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    if let Some(stripped) = rest.strip_prefix('~') {
        if stripped.is_empty() || stripped.starts_with('/') || stripped.starts_with('\\') {
            out.push_str(&home().to_string_lossy());
            rest = stripped;
        }
    }
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                match env::var(name) {
                    Ok(value) => out.push_str(&value),
                    Err(_) => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    PathBuf::from(out.replace('/', "\\"))
}

pub fn home() -> PathBuf {
    env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\"))
}

/// Finds an executable on PATH (honouring PATHEXT), or checks an expanded path.
pub fn which(program: &str) -> Option<PathBuf> {
    let candidate = expand(program);
    if candidate.is_absolute() || program.contains(['\\', '/']) {
        return candidate.exists().then_some(candidate);
    }
    let exts: Vec<String> = env::var("PATHEXT")
        .unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".into())
        .split(';')
        .filter(|e| !e.is_empty())
        .map(|e| e.to_ascii_lowercase())
        .collect();
    let has_ext = Path::new(program).extension().is_some();
    for dir in env::split_paths(&env::var_os("PATH")?) {
        if has_ext {
            let p = dir.join(program);
            if p.is_file() {
                return Some(p);
            }
        }
        for ext in &exts {
            let p = dir.join(format!("{program}{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Profile and app names: short, safe as file names, easy to type.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@'))
        && !name.starts_with('.')
        && !name.ends_with('.')
}

/// Turns an arbitrary account label (e-mail, login) into a valid profile name.
pub fn slug(raw: &str) -> String {
    let s: String = raw
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@') { c } else { '-' })
        .collect();
    let s = s.trim_matches(|c| c == '.' || c == '-').to_string();
    let s: String = s.chars().take(64).collect();
    if s.is_empty() { "account".into() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_env_and_home() {
        env::set_var("ACCT_TEST_VAR", "C:\\x");
        assert_eq!(expand("%ACCT_TEST_VAR%/y"), PathBuf::from("C:\\x\\y"));
        assert_eq!(expand("%NOPE_NOT_SET%"), PathBuf::from("%NOPE_NOT_SET%"));
        assert!(expand("~/.codex").to_string_lossy().ends_with("\\.codex"));
    }

    #[test]
    fn names() {
        assert!(valid_name("work"));
        assert!(valid_name("me@example.com"));
        assert!(!valid_name("../x"));
        assert!(!valid_name(""));
        assert_eq!(slug("Kristofer J@corp.onmicrosoft.com"), "Kristofer-J@corp.onmicrosoft.com");
    }
}
