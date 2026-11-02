//! Native strategy (the app keeps several accounts itself) and identity lookups.

use serde::Serialize;
use serde_json::Value;

use crate::def::{IdentitySpec, NativeSpec};
use crate::error::{Code, Error, Result};
use crate::paths::expand;
use crate::process;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NativeAccount {
    pub account: String,
    /// Passed to the switch command.
    #[serde(skip)]
    pub value: String,
    pub active: bool,
}

fn text_at(v: &Value, pointer: &str) -> Option<String> {
    let found = if pointer.is_empty() { Some(v) } else { v.pointer(pointer) };
    match found? {
        Value::String(s) => Some(s.clone()),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

/// Accounts the app knows about, one entry per account label.
pub fn list(spec: &NativeSpec) -> Result<Vec<NativeAccount>> {
    if let Some(file) = &spec.list_file {
        let path = expand(file);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(Error::io(path.display(), e)),
        };
        let doc: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|e| Error::new(Code::CommandFailed, format!("{} is not valid JSON: {e}", path.display())))?;
        return parse_list(spec, &doc);
    }
    let out = process::run(&spec.list)?;
    // Some tools exit non-zero when one account has a problem but still print the list.
    let doc: Value = serde_json::from_str(out.stdout.trim()).map_err(|_| {
        Error::new(
            Code::CommandFailed,
            format!("`{}` did not print JSON: {}", spec.list.join(" "), out.stderr.trim()),
        )
    })?;
    parse_list(spec, &doc)
}

pub fn parse_list(spec: &NativeSpec, doc: &Value) -> Result<Vec<NativeAccount>> {
    let items = if spec.items.is_empty() { Some(doc) } else { doc.pointer(&spec.items) };
    let Some(items) = items else { return Ok(Vec::new()) };
    let Some(items) = items.as_array() else {
        return Err(Error::new(Code::InvalidDefinition, format!("{} is not a list", spec.items)));
    };
    let mut out: Vec<NativeAccount> = Vec::new();
    for item in items {
        let Some(account) = text_at(item, &spec.account) else { continue };
        let value = spec.value.as_deref().and_then(|p| text_at(item, p)).unwrap_or_else(|| account.clone());
        let active = spec.active.as_deref().and_then(|p| item.pointer(p)).and_then(Value::as_bool).unwrap_or(false);
        match out.iter_mut().find(|a| a.account == account) {
            Some(existing) => {
                // Several entries per account (e.g. Azure subscriptions): prefer the active one.
                if active && !existing.active {
                    existing.value = value;
                    existing.active = true;
                }
            }
            None => out.push(NativeAccount { account, value, active }),
        }
    }
    Ok(out)
}

pub fn switch(spec: &NativeSpec, value: &str) -> Result<()> {
    let cmd: Vec<String> = spec.switch.iter().map(|a| a.replace("{value}", value)).collect();
    let out = process::run(&cmd)?;
    if !out.ok {
        return Err(Error::new(Code::CommandFailed, format!("`{}` failed: {}", cmd.join(" "), out.stderr.trim())));
    }
    Ok(())
}

/// Who is signed in right now, if the definition says how to tell. `Ok(None)` = signed out
/// or unknown.
pub fn identity(spec: &IdentitySpec) -> Result<Option<String>> {
    let text = if !spec.command.is_empty() {
        let out = process::run(&spec.command)?;
        out.stdout
    } else if let Some(file) = &spec.file {
        match std::fs::read_to_string(expand(file)) {
            Ok(t) => t,
            Err(_) => return Ok(None),
        }
    } else {
        return Ok(None);
    };
    let found = match &spec.pointer {
        Some(p) => serde_json::from_str::<Value>(text.trim()).ok().and_then(|v| text_at(&v, p)),
        None => text.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_string),
    };
    Ok(found.filter(|s| !s.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn az() -> NativeSpec {
        NativeSpec {
            list: vec![],
            list_file: None,
            items: String::new(),
            account: "/user/name".into(),
            active: Some("/isDefault".into()),
            value: Some("/id".into()),
            switch: vec![],
        }
    }

    #[test]
    fn groups_subscriptions_by_user() {
        let doc = serde_json::json!([
            {"id": "s1", "isDefault": false, "user": {"name": "a@x"}},
            {"id": "s2", "isDefault": true, "user": {"name": "a@x"}},
            {"id": "s3", "isDefault": false, "user": {"name": "b@y"}}
        ]);
        let got = parse_list(&az(), &doc).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].value.as_str(), got[0].active), ("s2", true));
        assert_eq!((got[1].value.as_str(), got[1].active), ("s3", false));
    }
}
