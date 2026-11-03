use std::env;
#[cfg(windows)]
use std::path::Path;
use std::path::PathBuf;

#[cfg(windows)]
use crate::error::Error;
use crate::error::Result;

/// Where acswi keeps its own files.
#[derive(Debug, Clone)]
pub struct Paths {
    /// `%LOCALAPPDATA%\acswi`, `~/.local/share/acswi` on Linux, or `ACSWI_HOME`: state,
    /// vault, moved folders.
    pub root: PathBuf,
    /// `%APPDATA%\acswi\apps`, `~/.config/acswi/apps` on Linux, or `ACSWI_HOME/apps`: user
    /// app definitions.
    pub user_defs: PathBuf,
}

impl Paths {
    pub fn from_env() -> Result<Self> {
        if let Some(home) = env::var_os("ACSWI_HOME") {
            let root = PathBuf::from(home);
            return Ok(Paths { user_defs: root.join("apps"), root });
        }
        Self::system()
    }

    #[cfg(windows)]
    fn system() -> Result<Self> {
        let local = env::var_os("LOCALAPPDATA").map(PathBuf::from);
        let roaming = env::var_os("APPDATA").map(PathBuf::from);
        match (local, roaming) {
            (Some(local), Some(roaming)) => Ok(Paths {
                root: renamed_from_acct(&local),
                user_defs: renamed_from_acct(&roaming).join("apps"),
            }),
            _ => Err(Error::new(crate::Code::Io, "LOCALAPPDATA/APPDATA are not set")),
        }
    }

    #[cfg(target_os = "macos")]
    fn system() -> Result<Self> {
        let base = home().join("Library").join("Application Support").join("acswi");
        Ok(Paths { user_defs: base.join("apps"), root: base })
    }

    /// XDG base directories: data in `~/.local/share`, definitions in `~/.config`.
    #[cfg(all(unix, not(target_os = "macos")))]
    fn system() -> Result<Self> {
        let xdg = |var: &str, fallback: &str| {
            env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home().join(fallback))
        };
        Ok(Paths {
            root: xdg("XDG_DATA_HOME", ".local/share").join("acswi"),
            user_defs: xdg("XDG_CONFIG_HOME", ".config").join("acswi").join("apps"),
        })
    }
}

/// `<base>\acswi`. The tool used to be called acct: an `<base>\acct` folder from then is
/// renamed to `acswi` once. If it can't be renamed yet (an old acct is still running), it is
/// used where it is, and renamed on a later run.
#[cfg(windows)]
fn renamed_from_acct(base: &Path) -> PathBuf {
    let new = base.join("acswi");
    let old = base.join("acct");
    if new.exists() || !old.is_dir() {
        return new;
    }
    match std::fs::rename(&old, &new) {
        Ok(()) => new,
        Err(_) => old,
    }
}

impl Paths {
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
    /// Linux and macOS without a keyring: the vault key itself.
    pub fn key_file(&self) -> PathBuf {
        self.root.join("key")
    }
    /// Linux and macOS: present when the vault key is in the desktop keyring.
    pub fn keyring_marker(&self) -> PathBuf {
        self.root.join("key-in-keyring")
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
    if cfg!(windows) {
        out = out.replace('/', "\\");
    }
    PathBuf::from(out)
}

#[cfg(windows)]
pub fn home() -> PathBuf {
    env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\"))
}

#[cfg(not(windows))]
pub fn home() -> PathBuf {
    env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// Finds an executable on PATH (honouring PATHEXT on Windows), or checks an expanded path.
pub fn which(program: &str) -> Option<PathBuf> {
    let candidate = expand(program);
    if candidate.is_absolute() || program.contains(['\\', '/']) {
        return candidate.exists().then_some(candidate);
    }
    search_path(program)
}

#[cfg(not(windows))]
fn search_path(program: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    env::split_paths(&env::var_os("PATH")?)
        .map(|dir| dir.join(program))
        .find(|p| p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
}

#[cfg(windows)]
fn search_path(program: &str) -> Option<PathBuf> {
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

    #[cfg(not(windows))]
    #[test]
    fn expands_env_and_home() {
        env::set_var("ACSWI_TEST_VAR", "/x");
        assert_eq!(expand("%ACSWI_TEST_VAR%/y"), PathBuf::from("/x/y"));
        assert_eq!(expand("~/.codex"), home().join(".codex"));
        assert!(which("sh").is_some());
        assert!(which("acswi-not-a-program").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn expands_env_and_home() {
        env::set_var("ACSWI_TEST_VAR", "C:\\x");
        assert_eq!(expand("%ACSWI_TEST_VAR%/y"), PathBuf::from("C:\\x\\y"));
        assert_eq!(expand("%NOPE_NOT_SET%"), PathBuf::from("%NOPE_NOT_SET%"));
        assert!(expand("~/.codex").to_string_lossy().ends_with("\\.codex"));
    }

    #[cfg(windows)]
    #[test]
    fn old_acct_folder_is_renamed_once() {
        let base = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(base.path().join("acct").join("vault")).unwrap();
        assert_eq!(renamed_from_acct(base.path()), base.path().join("acswi"));
        assert!(base.path().join("acswi").join("vault").is_dir());
        assert!(!base.path().join("acct").exists());
        // A later acct folder doesn't replace data that is already in acswi.
        std::fs::create_dir(base.path().join("acct")).unwrap();
        assert_eq!(renamed_from_acct(base.path()), base.path().join("acswi"));
        assert!(base.path().join("acct").is_dir());
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
