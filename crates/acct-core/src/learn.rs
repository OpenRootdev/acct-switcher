//! `acct learn`: find where an unknown app keeps its sign-in by comparing the disk and
//! Credential Manager before and after you sign in.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Serialize;

use crate::credman;
use crate::error::Result;
use crate::paths::home;
use crate::process;

const MAX_FILES_PER_ROOT: usize = 60_000;

/// Folders Chromium/Electron apps keep sessions in (cookies, local storage).
const ELECTRON_SESSION: &[&str] = &["Local Storage", "Network", "Cookies", "IndexedDB", "Session Storage"];

/// Changes in these are noise, not sign-in state.
const NOISE: &[&str] = &[
    "cache", "code cache", "gpucache", "dawncache", "dawngraphitecache", "dawnwebgpucache", "grshadercache",
    "shadercache", "crashpad", "logs", "log", "blob_storage", "service worker", "temp", "tmp", "dips",
    "shared dictionary", "sentry", "update",
];

pub struct Fingerprint {
    roots: Vec<PathBuf>,
    files: HashMap<PathBuf, (u64, u64)>,
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
    pub changed_files: usize,
    pub findings: Vec<Finding>,
    pub processes: Vec<String>,
    pub launch: Option<String>,
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase()
}

/// Folders whose name looks like the app (`hint`), in the usual places.
pub fn candidate_roots(hint: &str) -> Vec<PathBuf> {
    let h = norm(hint);
    let mut bases: Vec<PathBuf> = ["APPDATA", "LOCALAPPDATA"]
        .iter()
        .filter_map(|v| std::env::var_os(v).map(PathBuf::from))
        .collect();
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

fn mtime(meta: &fs::Metadata) -> u64 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as u64)
}

pub fn take(roots: &[PathBuf]) -> Result<Fingerprint> {
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
    Ok(Fingerprint { roots: roots.to_vec(), files, creds })
}

fn is_noise(rel: &Path) -> bool {
    rel.components().any(|c| NOISE.contains(&c.as_os_str().to_string_lossy().to_ascii_lowercase().as_str()))
        || rel.extension().is_some_and(|e| e.eq_ignore_ascii_case("log") || e.eq_ignore_ascii_case("tmp"))
}

pub fn compare(before: &Fingerprint, after: &Fingerprint) -> Report {
    let mut findings = Vec::new();
    let mut changed_files = 0;
    // root -> first-level entry -> changed files
    let mut by_root: BTreeMap<PathBuf, BTreeMap<String, Vec<PathBuf>>> = BTreeMap::new();
    for (path, stamp) in &after.files {
        if before.files.get(path) == Some(stamp) {
            continue;
        }
        let Some(root) = after.roots.iter().find(|r| path.starts_with(r)) else { continue };
        let rel = path.strip_prefix(root).unwrap();
        if is_noise(rel) {
            continue;
        }
        changed_files += 1;
        let first = rel.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned()).unwrap_or_default();
        by_root.entry(root.clone()).or_default().entry(first).or_default().push(path.clone());
    }
    for (root, groups) in &by_root {
        let electron = root.join("Local State").exists() && root.join("Local Storage").exists();
        for (first, paths) in groups {
            let target = root.join(first);
            if electron && ELECTRON_SESSION.iter().any(|s| s.eq_ignore_ascii_case(first)) {
                let kind = if target.is_dir() { "dir" } else { "file" };
                findings.push(Finding {
                    kind: kind.into(),
                    target: target.to_string_lossy().into(),
                    reason: format!("Electron session data ({} changed files)", paths.len()),
                });
            } else if !electron {
                for p in paths {
                    findings.push(Finding {
                        kind: "file".into(),
                        target: p.to_string_lossy().into(),
                        reason: "changed during sign-in".into(),
                    });
                }
            }
        }
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

    Report {
        roots: before.roots.iter().map(|r| r.to_string_lossy().into_owned()).collect(),
        changed_files,
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
    let mut t = format!("# Draft made by `acct learn`. Review it, then save it to %APPDATA%\\acct\\apps\\{id}.toml\n");
    t += &format!("id = {}\nname = {}\nstrategy = \"swap\"\n", toml_str(id), toml_str(name));
    if !report.processes.is_empty() {
        let list: Vec<String> = report.processes.iter().map(|p| toml_str(p)).collect();
        t += &format!("processes = [{}]\n", list.join(", "));
    }
    if let Some(l) = &report.launch {
        t += &format!("launch = [{}]\ndetect = [{}]\n", toml_str(&portable(l)), toml_str(&portable(l)));
    }
    for f in &report.findings {
        t += &format!("\n# {}\n[[state]]\n", f.reason);
        match f.kind.as_str() {
            "credential" => t += &format!("kind = \"credential\"\npattern = {}\n", toml_str(&f.target)),
            "dir" => t += &format!("kind = \"dir\"\npath = {}\nmode = \"move\"\n", toml_str(&portable(&f.target))),
            _ => t += &format!("kind = \"file\"\npath = {}\n", toml_str(&portable(&f.target))),
        }
    }
    t
}
