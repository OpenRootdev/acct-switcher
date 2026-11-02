//! End-to-end swap cycle on a fake app that uses every kind of state item.

use std::fs;
use std::path::Path;

use acct_core::credman::{self, Credential};
use acct_core::engine::{Acct, Actor, AddStep, SwitchOptions};
use acct_core::paths::Paths;
use acct_core::Code;

const CRED: &str = "acct-selftest-swap:token";

fn sign_in(live: &Path, who: &str, token: &str) {
    fs::create_dir_all(live.join("Session")).unwrap();
    fs::write(live.join("auth.json"), token).unwrap();
    fs::write(live.join("Session").join("db"), who).unwrap();
    let mut cfg: serde_json::Value = fs::read(live.join("cfg.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| serde_json::json!({ "theme": "dark" }));
    cfg["account"] = serde_json::Value::String(who.into());
    fs::write(live.join("cfg.json"), serde_json::to_vec(&cfg).unwrap()).unwrap();
    credman::write(&Credential {
        target: CRED.into(),
        user: who.into(),
        comment: String::new(),
        alias: String::new(),
        persist: 2,
        flags: 0,
        blob: token.as_bytes().to_vec(),
        attributes: vec![],
        last_written: 0,
    })
    .unwrap();
}

fn live_token(live: &Path) -> Option<String> {
    fs::read_to_string(live.join("auth.json")).ok()
}

fn cred_user() -> Option<String> {
    credman::read(Some(CRED)).unwrap().first().map(|c| c.user.clone())
}

#[test]
fn full_cycle() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("acct");
    let live = tmp.path().join("app");
    fs::create_dir_all(home.join("apps")).unwrap();
    let l = live.to_string_lossy().replace('\\', "/");
    fs::write(
        home.join("apps").join("selftest.toml"),
        format!(
            r#"
id = "selftest"
name = "Self Test"
login = ["cmd", "/c", "exit", "0"]
[identity]
file = "{l}/cfg.json"
pointer = "/account"
[[state]]
kind = "file"
path = "{l}/auth.json"
[[state]]
kind = "credential"
pattern = "acct-selftest-swap:*"
[[state]]
kind = "json_key"
path = "{l}/cfg.json"
keys = ["/account"]
[[state]]
kind = "dir"
path = "{l}/Session"
mode = "move"
"#
        ),
    )
    .unwrap();
    let acct = Acct::with_paths(Paths { root: home.clone(), user_defs: home.join("apps") }).unwrap();
    assert!(acct.defs.errors.is_empty(), "{:?}", acct.defs.errors);
    let opts = SwitchOptions { relaunch: false, ..Default::default() };

    // An account that is signed in but not saved is never thrown away.
    sign_in(&live, "alice", "tokA1");
    let err = acct.add_begin("selftest", "bob", &opts).unwrap_err();
    assert_eq!(err.code, Code::UnsavedSession);

    acct.save("selftest", "alice", Actor::Cli).unwrap();
    assert_eq!(acct.overview(Some("selftest")).unwrap()[0].active.as_deref(), Some("alice"));

    // Add bob: alice is saved, live state cleared (shared keys kept), then bob signs in.
    match acct.add_begin("selftest", "bob", &opts).unwrap() {
        AddStep::RunLogin { .. } => {}
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(live_token(&live), None);
    assert_eq!(cred_user(), None);
    assert!(!live.join("Session").exists());
    let cfg: serde_json::Value = serde_json::from_slice(&fs::read(live.join("cfg.json")).unwrap()).unwrap();
    assert_eq!(cfg, serde_json::json!({ "theme": "dark" }), "only the account key is removed");
    sign_in(&live, "bob", "tokB1");
    assert_eq!(acct.add_finish("selftest", &opts).unwrap(), "bob");

    // Bob's refresh token rotates while he is active.
    fs::write(live.join("auth.json"), "tokB2").unwrap();

    let r = acct.switch("selftest", "alice", &opts).unwrap();
    assert!(r.changed);
    assert_eq!(r.identity.as_deref(), Some("alice"));
    assert_eq!(live_token(&live).as_deref(), Some("tokA1"));
    assert_eq!(cred_user().as_deref(), Some("alice"));
    assert_eq!(fs::read_to_string(live.join("Session").join("db")).unwrap(), "alice");

    // Back to bob: the rotated token was kept.
    acct.switch("selftest", "bob", &opts).unwrap();
    assert_eq!(live_token(&live).as_deref(), Some("tokB2"));
    assert_eq!(fs::read_to_string(live.join("Session").join("db")).unwrap(), "bob");
    let cfg: serde_json::Value = serde_json::from_slice(&fs::read(live.join("cfg.json")).unwrap()).unwrap();
    assert_eq!(cfg["theme"], "dark");
    assert_eq!(cfg["account"], "bob");

    // If someone signs in to a different account behind acct's back, acct refuses to
    // file those tokens under bob and changes nothing.
    sign_in(&live, "mallory", "tokM");
    let err = acct.switch("selftest", "alice", &opts).unwrap_err();
    assert_eq!(err.code, Code::IdentityMismatch);
    assert_eq!(live_token(&live).as_deref(), Some("tokM"));
    sign_in(&live, "bob", "tokB3");

    // The vault holds no plaintext.
    let blob = fs::read(home.join("vault").join("selftest").join("alice.bin")).unwrap();
    assert!(!blob.windows(5).any(|w| w == b"tokA1"));

    // Rename, then remove (the active profile is protected).
    acct.rename("selftest", "alice", "work").unwrap();
    assert_eq!(acct.remove("selftest", "bob", Actor::Cli).unwrap_err().code, Code::Unsupported);
    acct.switch("selftest", "work", &opts).unwrap();
    assert_eq!(live_token(&live).as_deref(), Some("tokA1"));
    acct.remove("selftest", "bob", Actor::Cli).unwrap();
    assert!(!home.join("vault").join("selftest").join("bob.bin").exists());

    let audit = fs::read_to_string(home.join("audit.log")).unwrap();
    assert!(audit.lines().count() >= 4);
    assert!(!audit.contains("tok"), "audit log must not contain secrets");

    credman::delete(CRED).unwrap();
}

#[test]
fn cancel_add_restores_previous() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("acct");
    let live = tmp.path().join("app");
    fs::create_dir_all(home.join("apps")).unwrap();
    fs::create_dir_all(&live).unwrap();
    let l = live.to_string_lossy().replace('\\', "/");
    fs::write(
        home.join("apps").join("t2.toml"),
        format!("id='t2'\nname='T2'\nlogin=['cmd','/c','exit','0']\n[[state]]\nkind='file'\npath='{l}/auth.json'\n"),
    )
    .unwrap();
    let acct = Acct::with_paths(Paths { root: home.clone(), user_defs: home.join("apps") }).unwrap();
    let opts = SwitchOptions::default();
    fs::write(live.join("auth.json"), "first").unwrap();
    acct.save("t2", "one", Actor::Cli).unwrap();
    acct.add_begin("t2", "two", &opts).unwrap();
    assert!(!live.join("auth.json").exists());
    fs::write(live.join("auth.json"), "half-done").unwrap();
    assert_eq!(acct.add_cancel("t2", &opts).unwrap().as_deref(), Some("one"));
    assert_eq!(fs::read_to_string(live.join("auth.json")).unwrap(), "first");
    let v = &acct.overview(Some("t2")).unwrap()[0];
    assert_eq!(v.active.as_deref(), Some("one"));
    assert!(v.pending_add.is_none());
    assert_eq!(v.profiles.len(), 1);
}
