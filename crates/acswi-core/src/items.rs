//! Reading, clearing and restoring the live sign-in state described by a `StateItem`.

use std::fs;
use std::path::{Path, PathBuf};

use globset::{Glob, GlobBuilder, GlobSet, GlobSetBuilder};
use serde_json::Value;

use crate::credman::{self, Credential};
use crate::registry;
use crate::def::{DirMode, StateItem};
use crate::error::{Code, Error, Result};
use crate::paths::expand;
use crate::state::write_atomic;
use crate::vault::{PackedFile, Saved};

/// Where `mode = "move"` folders of one profile are kept.
pub struct MoveStore<'a> {
    pub profile_dir: &'a Path,
}

impl MoveStore<'_> {
    pub fn slot(&self, item: &StateItem) -> PathBuf {
        self.profile_dir.join(slot_name(item))
    }
    pub fn incoming(&self, item: &StateItem) -> PathBuf {
        self.profile_dir.join(format!("{}.incoming", slot_name(item)))
    }
}

/// Stable folder name for a state item: readable tail plus a short hash of the full path.
fn slot_name(item: &StateItem) -> String {
    let key = item.describe();
    let mut h: u32 = 0x811c9dc5;
    for b in key.to_ascii_lowercase().bytes() {
        h = (h ^ b as u32).wrapping_mul(0x01000193);
    }
    let tail: String = match item {
        StateItem::Dir { path, .. } => expand(path)
            .file_name()
            .map(|n| n.to_string_lossy().chars().filter(|c| c.is_ascii_alphanumeric()).collect())
            .unwrap_or_default(),
        _ => String::new(),
    };
    format!("{tail}-{h:08x}")
}

// ---------- presence ----------

/// True if the app currently has state at this location.
pub fn present(item: &StateItem) -> Result<bool> {
    Ok(match item {
        StateItem::File { path } => expand(path).is_file(),
        StateItem::Dir { path, mode: DirMode::Move, .. } => expand(path).exists(),
        StateItem::Dir { path, exclude, .. } => !walk(&expand(path), &excludes(exclude)?)?.is_empty(),
        StateItem::Credential { pattern } => !matching_credentials(pattern)?.is_empty(),
        StateItem::Registry { key } => registry::present(key)?,
        StateItem::JsonKey { path, keys } => match read_json(&expand(path))? {
            Some(doc) => keys.iter().any(|k| doc.pointer(k).is_some()),
            None => false,
        },
    })
}

// ---------- capture ----------

/// Reads the live state. For moved folders this moves the folder into `store.incoming`.
pub fn capture(item: &StateItem, store: &MoveStore) -> Result<Saved> {
    Ok(match item {
        StateItem::File { path } => {
            let p = expand(path);
            match fs::read(&p) {
                Ok(b) => Saved::File(Some(b)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Saved::File(None),
                Err(e) => return Err(Error::io(p.display(), e)),
            }
        }
        StateItem::Dir { path, mode: DirMode::Move, .. } => {
            let live = expand(path);
            if !live.exists() {
                return Ok(Saved::Moved(false));
            }
            let dest = store.incoming(item);
            remove_any(&dest)?;
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(|e| Error::io(parent.display(), e))?;
            }
            move_dir(&live, &dest)?;
            Saved::Moved(true)
        }
        StateItem::Dir { path, exclude, .. } => {
            let root = expand(path);
            let mut files = Vec::new();
            for rel in walk(&root, &excludes(exclude)?)? {
                let full = root.join(native(&rel));
                let data = fs::read(&full).map_err(|e| Error::io(full.display(), e))?;
                files.push(PackedFile { path: rel, data });
            }
            Saved::Dir(files)
        }
        StateItem::Credential { pattern } => Saved::Credentials(matching_credentials(pattern)?),
        StateItem::Registry { key } => Saved::Registry(registry::capture(key)?),
        StateItem::JsonKey { path, keys } => {
            let doc = read_json(&expand(path))?;
            Saved::JsonKey(
                keys.iter()
                    .map(|k| doc.as_ref().and_then(|d| d.pointer(k)).map(|v| v.to_string()))
                    .collect(),
            )
        }
    })
}

// ---------- clear ----------

/// Removes the live state so the next profile starts clean. Moved folders were already
/// moved away by `capture`; if one is still there it is left alone (never deleted).
pub fn clear(item: &StateItem) -> Result<()> {
    match item {
        StateItem::File { path } => remove_any(&expand(path)),
        StateItem::Dir { mode: DirMode::Move, .. } => Ok(()),
        StateItem::Dir { path, exclude, .. } => {
            let root = expand(path);
            for rel in walk(&root, &excludes(exclude)?)? {
                let full = root.join(native(&rel));
                fs::remove_file(&full).map_err(|e| Error::io(full.display(), e))?;
            }
            Ok(())
        }
        StateItem::Credential { pattern } => {
            for c in matching_credentials(pattern)? {
                credman::delete(&c.target)?;
            }
            Ok(())
        }
        StateItem::Registry { key } => registry::clear(key),
        StateItem::JsonKey { path, keys } => {
            let p = expand(path);
            let Some(mut doc) = read_json(&p)? else { return Ok(()) };
            let mut changed = false;
            for k in keys {
                changed |= remove_pointer(&mut doc, k);
            }
            if changed {
                write_json(&p, &doc)?;
            }
            Ok(())
        }
    }
}

// ---------- restore ----------

/// Writes a saved state back to the live location. `saved == None` means the profile has
/// nothing stored for this item (e.g. the definition gained an item later).
pub fn restore(item: &StateItem, saved: Option<&Saved>, store: &MoveStore) -> Result<()> {
    match (item, saved) {
        (StateItem::Dir { path, mode: DirMode::Move, .. }, _) => {
            let slot = store.slot(item);
            let live = expand(path);
            if slot.exists() {
                if live.exists() {
                    return Err(Error::new(
                        Code::Recovery,
                        format!("{} already exists; refusing to overwrite it", live.display()),
                    ));
                }
                if let Some(parent) = live.parent() {
                    fs::create_dir_all(parent).map_err(|e| Error::io(parent.display(), e))?;
                }
                move_dir(&slot, &live)?;
            }
            Ok(())
        }
        (_, None) => Ok(()),
        (StateItem::File { path }, Some(Saved::File(data))) => match data {
            Some(bytes) => write_atomic(&expand(path), bytes),
            None => Ok(()),
        },
        (StateItem::Dir { path, .. }, Some(Saved::Dir(files))) => {
            let root = expand(path);
            for f in files {
                write_atomic(&root.join(native(&f.path)), &f.data)?;
            }
            Ok(())
        }
        (StateItem::Credential { .. }, Some(Saved::Credentials(creds))) => {
            creds.iter().try_for_each(credman::write)
        }
        (StateItem::Registry { key }, Some(Saved::Registry(node))) => match node {
            Some(n) => registry::restore(key, n),
            None => Ok(()),
        },
        (StateItem::JsonKey { path, keys }, Some(Saved::JsonKey(values))) => {
            if values.iter().all(Option::is_none) {
                return Ok(());
            }
            let p = expand(path);
            let mut doc = read_json(&p)?.unwrap_or_else(|| Value::Object(Default::default()));
            for (k, v) in keys.iter().zip(values) {
                if let Some(text) = v {
                    let value: Value = serde_json::from_str(text)
                        .map_err(|e| Error::new(Code::Io, format!("bad saved value for {k}: {e}")))?;
                    set_pointer(&mut doc, k, value);
                }
            }
            write_json(&p, &doc)
        }
        (item, Some(_)) => Err(Error::new(
            Code::Recovery,
            format!("saved data does not match {}; the definition changed in an incompatible way", item.describe()),
        )),
    }
}

/// After a successful save, moves `incoming` folders into their final slot.
pub fn commit_moved(item: &StateItem, store: &MoveStore) -> Result<()> {
    if let StateItem::Dir { mode: DirMode::Move, .. } = item {
        let incoming = store.incoming(item);
        let slot = store.slot(item);
        remove_any(&slot)?;
        if incoming.exists() {
            move_dir(&incoming, &slot)?;
        }
    }
    Ok(())
}

/// Undo of `capture` for moved folders: puts `incoming` back to the live location.
pub fn rollback_moved(item: &StateItem, store: &MoveStore) -> Result<()> {
    if let StateItem::Dir { path, mode: DirMode::Move, .. } = item {
        let incoming = store.incoming(item);
        let live = expand(path);
        if incoming.exists() && !live.exists() {
            move_dir(&incoming, &live)?;
        }
    }
    Ok(())
}

// ---------- helpers ----------

pub fn matching_credentials(pattern: &str) -> Result<Vec<Credential>> {
    let simple_prefix = pattern.ends_with('*') && !pattern[..pattern.len() - 1].contains(['*', '?']);
    if simple_prefix || !pattern.contains(['*', '?']) {
        return credman::read(Some(pattern));
    }
    let glob = GlobBuilder::new(pattern)
        .case_insensitive(true)
        .literal_separator(false)
        .build()
        .map_err(|e| Error::new(Code::InvalidDefinition, format!("credential pattern {pattern:?}: {e}")))?
        .compile_matcher();
    Ok(credman::read(None)?.into_iter().filter(|c| glob.is_match(&c.target)).collect())
}

fn excludes(patterns: &[String]) -> Result<GlobSet> {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        let g = Glob::new(&p.replace('\\', "/"))
            .map_err(|e| Error::new(Code::InvalidDefinition, format!("exclude {p:?}: {e}")))?;
        b.add(g);
    }
    b.build().map_err(|e| Error::new(Code::InvalidDefinition, e.to_string()))
}

/// `a/b.txt` as a path for this OS.
fn native(rel: &str) -> PathBuf {
    rel.split('/').collect()
}

/// Relative paths (with `/`) of files under `root`, minus excluded ones.
fn walk(root: &Path, exclude: &GlobSet) -> Result<Vec<String>> {
    let mut out = Vec::new();
    if !root.is_dir() {
        return Ok(out);
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).map_err(|e| Error::io(dir.display(), e))? {
            let entry = entry.map_err(|e| Error::io(dir.display(), e))?;
            let path = entry.path();
            let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
            if exclude.is_match(&rel) {
                continue;
            }
            let ft = entry.file_type().map_err(|e| Error::io(path.display(), e))?;
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() {
                out.push(rel);
            }
        }
    }
    out.sort();
    Ok(out)
}

fn remove_any(p: &Path) -> Result<()> {
    let r = if p.is_dir() { fs::remove_dir_all(p) } else { fs::remove_file(p) };
    match r {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::io(p.display(), e)),
    }
}

/// Renames a folder; retries briefly because Windows may hold a handle for a moment after
/// an app exits.
fn move_dir(from: &Path, to: &Path) -> Result<()> {
    let mut last = None;
    for attempt in 0..20 {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(100 + attempt * 50));
            }
        }
    }
    Err(Error::io(
        format!("could not move {} to {} (is the app still running?)", from.display(), to.display()),
        last.unwrap(),
    ))
}

fn read_json(p: &Path) -> Result<Option<Value>> {
    match fs::read(p) {
        Ok(b) => serde_json::from_slice(&b)
            .map(Some)
            .map_err(|e| Error::new(Code::Io, format!("{} is not valid JSON: {e}", p.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(p.display(), e)),
    }
}

fn write_json(p: &Path, doc: &Value) -> Result<()> {
    let text = serde_json::to_vec_pretty(doc).expect("json serializes");
    write_atomic(p, &text)
}

fn tokens(pointer: &str) -> Vec<String> {
    pointer.split('/').skip(1).map(|t| t.replace("~1", "/").replace("~0", "~")).collect()
}

fn remove_pointer(doc: &mut Value, pointer: &str) -> bool {
    let toks = tokens(pointer);
    let Some((last, parents)) = toks.split_last() else { return false };
    let mut cur = doc;
    for t in parents {
        match cur.get_mut(t.as_str()) {
            Some(next) => cur = next,
            None => return false,
        }
    }
    cur.as_object_mut().is_some_and(|o| o.shift_remove(last.as_str()).is_some())
}

fn set_pointer(doc: &mut Value, pointer: &str, value: Value) {
    let toks = tokens(pointer);
    let Some((last, parents)) = toks.split_last() else {
        *doc = value;
        return;
    };
    let mut cur = doc;
    for t in parents {
        if !cur.is_object() {
            *cur = Value::Object(Default::default());
        }
        cur = cur.as_object_mut().unwrap().entry(t.clone()).or_insert_with(|| Value::Object(Default::default()));
    }
    if !cur.is_object() {
        *cur = Value::Object(Default::default());
    }
    cur.as_object_mut().unwrap().insert(last.clone(), value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_pointer_edit_keeps_other_keys() {
        let mut doc: Value = serde_json::json!({"a": 1, "acswi": {"email": "x"}, "z": 2});
        assert!(remove_pointer(&mut doc, "/acswi"));
        assert_eq!(doc, serde_json::json!({"a": 1, "z": 2}));
        set_pointer(&mut doc, "/acswi/email", Value::String("y".into()));
        assert_eq!(doc["acswi"]["email"], "y");
        let keys: Vec<_> = doc.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["a", "z", "acswi"]);
    }

    #[test]
    fn file_and_dir_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("cfg");
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("auth.json"), b"A").unwrap();
        fs::write(root.join("sub").join("t.bin"), b"B").unwrap();
        fs::write(root.join("history.log"), b"keep").unwrap();
        let item = StateItem::Dir {
            path: root.to_string_lossy().into(),
            mode: DirMode::Encrypt,
            exclude: vec!["*.log".into()],
        };
        let store = MoveStore { profile_dir: tmp.path() };
        let saved = capture(&item, &store).unwrap();
        clear(&item).unwrap();
        assert!(!root.join("auth.json").exists());
        assert!(root.join("history.log").exists(), "excluded files are shared, not cleared");
        restore(&item, Some(&saved), &store).unwrap();
        assert_eq!(fs::read(root.join("sub").join("t.bin")).unwrap(), b"B");
    }

    #[test]
    fn moved_dir_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let live = tmp.path().join("Local Storage");
        fs::create_dir_all(&live).unwrap();
        fs::write(live.join("db"), b"one").unwrap();
        let item = StateItem::Dir { path: live.to_string_lossy().into(), mode: DirMode::Move, exclude: vec![] };
        let pdir = tmp.path().join("p1");
        let store = MoveStore { profile_dir: &pdir };
        capture(&item, &store).unwrap();
        assert!(!live.exists());
        commit_moved(&item, &store).unwrap();
        restore(&item, None, &store).unwrap();
        assert_eq!(fs::read(live.join("db")).unwrap(), b"one");
    }
}
