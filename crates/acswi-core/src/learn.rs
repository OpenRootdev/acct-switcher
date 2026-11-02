//! `acswi learn`: find where an unknown app keeps its sign-in by comparing the disk, the
//! registry and Credential Manager before and after you sign in.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Serialize;
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
use winreg::RegKey;

use crate::credman;
use crate::error::Result;
use crate::paths::home;
use crate::process;

const MAX_FILES_PER_ROOT: usize = 60_000;
const MAX_KEYS_PER_ROOT: usize = 5_000;
/// Above this many changed files in one folder, the draft names the folder instead.
const MAX_FILES_LISTED: usize = 8;

/// Folders Chromium/Electron apps keep sessions in (cookies, local storage).
const ELECTRON_SESSION: &[&str] = &["Local Storage", "Network", "Cookies", "IndexedDB", "Session Storage"];

/// Changes in these are noise, not sign-in state.
const NOISE: &[&str] = &[
    "cache", "code cache", "gpucache", "dawncache", "dawngraphitecache", "dawnwebgpucache", "grshadercache",
    "shadercache", "crashpad", "logs", "log", "blob_storage", "service worker", "temp", "tmp", "dips",
    "shared dictionary", "sentry", "update", "tempstate", "inetcache", "inethistory",
    // Editors and Electron apps update these on every start; never sign-in state.
    "extensions", "cacheddata", "cachedextensionvsixs", "cachedprofilesdata", "snapshots", ".trash",
    "webstorage", "languagepacks.json",
];

pub struct Fingerprint {
    roots: Vec<PathBuf>,
    files: HashMap<PathBuf, (u64, u64)>,
    /// Registry key (under HKCU) -> hash of its values.
    keys: HashMap<String, u64>,
    creds: HashMap<String, u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    /// TOML snippet for one `[[state]]` item.
    pub kind: String,
    pub target: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub roots: Vec<String>,
    pub registry: Vec<String>,
    /// Package family name, when the app is a Microsoft Store (MSIX) app.
    pub package: Option<String>,
    pub changed_files: usize,
    /// Changed files that were ignored as caches, logs and the like.
    pub noise_files: usize,
    pub findings: Vec<Finding>,
    pub processes: Vec<String>,
    pub launch: Option<String>,
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase()
}

/// Folders whose name looks like the app (`hint`), in the usual places, including
/// Microsoft Store app folders (`%LOCALAPPDATA%\Packages\<family name>`).
pub fn candidate_roots(hint: &str) -> Vec<PathBuf> {
    let h = norm(hint);
    let mut bases: Vec<PathBuf> = ["APPDATA", "LOCALAPPDATA"]
        .iter()
        .filter_map(|v| std::env::var_os(v).map(PathBuf::from))
        .collect();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        bases.push(PathBuf::from(local).join("Packages"));
    }
    bases.push(home());
    bases.push(home().join(".config"));
    let mut out = Vec::new();
    for base in bases {
        let Ok(entries) = fs::read_dir(&base) else { continue };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if e.file_type().is_ok_and(|t| t.is_dir()) && !h.is_empty() && norm(&name).contains(&h) {
                out.push(e.path());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Registry keys named like the app: `HKCU\Software\<App>` and `HKCU\Software\<Vendor>\<App>`.
pub fn registry_roots(hint: &str) -> Vec<String> {
    let h = norm(hint);
    let mut out = Vec::new();
    let Ok(software) = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags("Software", KEY_READ) else {
        return out;
    };
    if h.is_empty() {
        return out;
    }
    for vendor in software.enum_keys().flatten() {
        if vendor.eq_ignore_ascii_case("Classes") || vendor.eq_ignore_ascii_case("Microsoft") {
            continue; // file associations and Windows itself: huge, and not an app's sign-in
        }
        if norm(&vendor).contains(&h) {
            out.push(format!("Software\\{vendor}"));
            continue;
        }
        let Ok(vk) = software.open_subkey_with_flags(&vendor, KEY_READ) else { continue };
        for app in vk.enum_keys().flatten() {
            if norm(&app).contains(&h) {
                out.push(format!("Software\\{vendor}\\{app}"));
            }
        }
    }
    out
}

/// Whether `root` is a Microsoft Store app folder; returns its package family name.
fn package_name(root: &Path) -> Option<String> {
    let parent = root.parent()?.file_name()?.to_string_lossy();
    parent.eq_ignore_ascii_case("Packages").then(|| root.file_name().unwrap().to_string_lossy().into_owned())
}

/// Hashes every key's values under each root, so a later snapshot shows which keys changed.
fn take_registry(roots: &[String]) -> HashMap<String, u64> {
    let mut keys = HashMap::new();
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    for root in roots {
        let mut stack = vec![root.clone()];
        let mut count = 0;
        while let Some(path) = stack.pop() {
            let Ok(key) = hkcu.open_subkey_with_flags(&path, KEY_READ) else { continue };
            let mut h = DefaultHasher::new();
            for (name, value) in key.enum_values().flatten() {
                name.hash(&mut h);
                format!("{:?}", value.vtype).hash(&mut h);
                value.bytes.hash(&mut h);
            }
            keys.insert(path.clone(), h.finish());
            stack.extend(key.enum_keys().flatten().map(|k| format!("{path}\\{k}")));
            count += 1;
            if count >= MAX_KEYS_PER_ROOT {
                break;
            }
        }
    }
    keys
}

/// Keeps only the outermost keys: saving a key saves everything under it.
fn outermost(mut keys: Vec<String>) -> Vec<String> {
    keys.sort_by_key(|k| k.to_ascii_lowercase());
    let mut out: Vec<String> = Vec::new();
    for k in keys {
        let inside = out.last().is_some_and(|p| k.to_ascii_lowercase().starts_with(&format!("{}\\", p.to_ascii_lowercase())));
        if !inside {
            out.push(k);
        }
    }
    out
}

fn mtime(meta: &fs::Metadata) -> u64 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as u64)
}

pub fn take(roots: &[PathBuf], registry: &[String]) -> Result<Fingerprint> {
    let mut files = HashMap::new();
    for root in roots {
        let mut count = 0;
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                let Ok(meta) = e.metadata() else { continue };
                if meta.is_dir() {
                    stack.push(e.path());
                } else {
                    files.insert(e.path(), (mtime(&meta), meta.len()));
                    count += 1;
                }
                if count >= MAX_FILES_PER_ROOT {
                    break;
                }
            }
        }
    }
    let creds = credman::list_info(None)?.into_iter().map(|c| (c.target, c.last_written)).collect();
    Ok(Fingerprint { roots: roots.to_vec(), files, keys: take_registry(registry), creds })
}

fn is_noise(rel: &Path) -> bool {
    rel.components().any(|c| NOISE.contains(&c.as_os_str().to_string_lossy().to_ascii_lowercase().as_str()))
        || rel.extension().is_some_and(|e| e.eq_ignore_ascii_case("log") || e.eq_ignore_ascii_case("tmp"))
}

pub fn compare(before: &Fingerprint, after: &Fingerprint) -> Report {
    let mut findings = Vec::new();
    let mut changed_files = 0;
    let mut noise_files = 0;
    // root -> first-level entry -> changed files
    let mut by_root: BTreeMap<PathBuf, BTreeMap<String, Vec<PathBuf>>> = BTreeMap::new();
    for (path, stamp) in &after.files {
        if before.files.get(path) == Some(stamp) {
            continue;
        }
        let Some(root) = after.roots.iter().find(|r| path.starts_with(r)) else { continue };
        let rel = path.strip_prefix(root).unwrap();
        if is_noise(rel) {
            noise_files += 1;
            continue;
        }
        changed_files += 1;
        let first = rel.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned()).unwrap_or_default();
        by_root.entry(root.clone()).or_default().entry(first).or_default().push(path.clone());
    }
    for (root, groups) in &by_root {
        let electron = root.join("Local State").exists() && root.join("Local Storage").exists();
        let package = package_name(root).is_some();
        for (first, paths) in groups {
            let target = root.join(first);
            if package && target.is_dir() {
                // Store apps keep their state in a few fixed folders (LocalState, Settings, RoamingState…).
                findings.push(Finding {
                    kind: "dir".into(),
                    target: target.to_string_lossy().into(),
                    reason: format!("Store app data ({} changed files)", paths.len()),
                });
            } else if electron && ELECTRON_SESSION.iter().any(|s| s.eq_ignore_ascii_case(first)) {
                let kind = if target.is_dir() { "dir" } else { "file" };
                findings.push(Finding {
                    kind: kind.into(),
                    target: target.to_string_lossy().into(),
                    reason: format!("Electron session data ({} changed files)", paths.len()),
                });
            } else if paths.len() > MAX_FILES_LISTED {
                // Many changes in one folder: name the folder, not every file
                // (e.g. VS Code-style `User\globalStorage\state.vscdb` plus its neighbours).
                let place = if electron { "outside the session folders" } else { "in this folder" };
                findings.push(Finding {
                    kind: "dir".into(),
                    target: target.to_string_lossy().into(),
                    reason: format!("{} files changed {place}; may include settings", paths.len()),
                });
            } else {
                let reason = if electron { "changed outside the session folders; may be settings" } else { "changed during sign-in" };
                for p in paths {
                    findings.push(Finding { kind: "file".into(), target: p.to_string_lossy().into(), reason: reason.into() });
                }
            }
        }
    }
    let changed_keys: Vec<String> = after
        .keys
        .iter()
        .filter(|(k, h)| before.keys.get(*k) != Some(*h))
        .map(|(k, _)| k.clone())
        .chain(before.keys.keys().filter(|k| !after.keys.contains_key(*k)).cloned())
        .collect();
    for key in outermost(changed_keys) {
        findings.push(Finding { kind: "registry".into(), target: key, reason: "registry key changed during sign-in".into() });
    }
    let mut new_creds: BTreeSet<String> = BTreeSet::new();
    for (target, written) in &after.creds {
        if before.creds.get(target) != Some(written) {
            new_creds.insert(target.clone());
        }
    }
    for target in new_creds {
        findings.push(Finding { kind: "credential".into(), target, reason: "Credential Manager entry added or changed".into() });
    }

    let mut registry: Vec<String> = before.keys.keys().filter(|k| k.matches('\\').count() <= 2).cloned().collect();
    registry.sort();
    Report {
        roots: before.roots.iter().map(|r| r.to_string_lossy().into_owned()).collect(),
        registry,
        package: before.roots.iter().find_map(|r| package_name(r)),
        changed_files,
        noise_files,
        findings,
        processes: Vec::new(),
        launch: None,
    }
}

/// Running processes that look like the app (inside its folders, or named like it), for
/// `processes` and `launch`. Call while the app is open.
pub fn app_processes(roots: &[PathBuf], hint: &str) -> (Vec<String>, Option<String>) {
    let h = norm(hint);
    let mut names = BTreeSet::new();
    let mut launch = None;
    for p in process::find(&["*".into()]) {
        let lower = p.path.to_ascii_lowercase();
        let in_root = !lower.is_empty() && roots.iter().any(|r| lower.starts_with(&r.to_string_lossy().to_ascii_lowercase()));
        if in_root || (!h.is_empty() && norm(&p.name).contains(&h)) {
            names.insert(p.name.clone());
            if launch.is_none() && !p.path.is_empty() {
                launch = Some(p.path.clone());
            }
        }
    }
    (names.into_iter().collect(), launch)
}

fn toml_str(s: &str) -> String {
    if s.contains('\'') { format!("{s:?}") } else { format!("'{s}'") }
}

/// A draft definition the user reviews before saving.
pub fn draft(id: &str, name: &str, report: &Report) -> String {
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let roaming = std::env::var("APPDATA").unwrap_or_default();
    let user = home().to_string_lossy().into_owned();
    let portable = |p: &str| -> String {
        for (prefix, var) in [(&roaming, "%APPDATA%"), (&local, "%LOCALAPPDATA%"), (&user, "~")] {
            if !prefix.is_empty() && p.to_ascii_lowercase().starts_with(&prefix.to_ascii_lowercase()) {
                return format!("{var}{}", &p[prefix.len()..]);
            }
        }
        p.to_string()
    };
    let mut t = format!("# Draft made by `acswi learn`. Review it, then save it to %APPDATA%\\acswi\\apps\\{id}.toml\n");
    t += &format!("id = {}\nname = {}\nstrategy = \"swap\"\n", toml_str(id), toml_str(name));
    if !report.processes.is_empty() {
        let list: Vec<String> = report.processes.iter().map(|p| toml_str(p)).collect();
        t += &format!("processes = [{}]\n", list.join(", "));
    }
    if let Some(pkg) = &report.package {
        // Store apps can't be started from their .exe; Windows starts them by app id.
        t += "# Store app: if it doesn't reopen, find its id with `Get-StartApps` in PowerShell.\n";
        t += &format!("launch = ['explorer.exe', 'shell:AppsFolder\\{pkg}!App']\n");
        t += &format!("detect = [{}]\n", toml_str(&format!("%LOCALAPPDATA%\\Packages\\{pkg}")));
    } else if let Some(l) = &report.launch {
        t += &format!("launch = [{}]\ndetect = [{}]\n", toml_str(&portable(l)), toml_str(&portable(l)));
    }
    for f in &report.findings {
        t += &format!("\n# {}\n[[state]]\n", f.reason);
        match f.kind.as_str() {
            "credential" => t += &format!("kind = \"credential\"\npattern = {}\n", toml_str(&f.target)),
            "registry" => t += &format!("kind = \"registry\"\nkey = {}\n", toml_str(&f.target)),
            "dir" => t += &format!("kind = \"dir\"\npath = {}\nmode = \"move\"\n", toml_str(&portable(&f.target))),
            _ => t += &format!("kind = \"file\"\npath = {}\n", toml_str(&portable(&f.target))),
        }
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_outermost_keys() {
        let keys = vec![
            r"Software\Vendor\App\Auth".to_string(),
            r"Software\Vendor\App".to_string(),
            r"Software\Vendor\Apple".to_string(),
            r"Software\Vendor\App\Auth\Tokens".to_string(),
        ];
        assert_eq!(outermost(keys), vec![r"Software\Vendor\App".to_string(), r"Software\Vendor\Apple".to_string()]);
    }

    #[test]
    fn store_app_folders() {
        let root = Path::new(r"C:\Users\a\AppData\Local\Packages\SpotifyAB.SpotifyMusic_zpdnekdrzrea0");
        assert_eq!(package_name(root).as_deref(), Some("SpotifyAB.SpotifyMusic_zpdnekdrzrea0"));
        assert_eq!(package_name(Path::new(r"C:\Users\a\AppData\Roaming\discord")), None);
    }

    #[test]
    fn registry_changes_are_found() {
        let root = r"Software\acswi-selftest-learn";
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let _ = hkcu.delete_subkey_all(root);
        let (app, _) = hkcu.create_subkey(root).unwrap();
        app.set_value("theme", &"dark").unwrap();
        let snapshot = || Fingerprint { roots: vec![], files: HashMap::new(), keys: take_registry(&[root.into()]), creds: HashMap::new() };
        let before = snapshot();
        let (auth, _) = app.create_subkey("Auth").unwrap();
        auth.set_value("token", &"x").unwrap();
        let after = snapshot();
        let _ = hkcu.delete_subkey_all(root);
        let report = compare(&before, &after);
        let keys: Vec<&str> = report.findings.iter().filter(|f| f.kind == "registry").map(|f| f.target.as_str()).collect();
        assert_eq!(keys, vec![r"Software\acswi-selftest-learn\Auth"]);
    }
}
