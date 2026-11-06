//! Expiry dates recorded per profile, MCP sign-ins switched apart from the app's own account
//! in the same file, and profiles saved while a definition kept the whole file.

use std::fs;
use std::path::Path;

use acswi_core::engine::{Acswi, Actor, SwitchOptions};
use acswi_core::paths::Paths;
use acswi_core::Level;
use serde_json::{json, Value};

const LOGIN: &str = if cfg!(windows) { "['cmd', '/c', 'exit', '0']" } else { "['true']" };
const DAY_MS: i64 = 86_400_000;

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn read(p: &Path) -> Value {
    serde_json::from_slice(&fs::read(p).unwrap()).unwrap()
}

/// Edits one key of the shared credentials file, like the app would.
fn put(p: &Path, pointer: &str, v: Value) {
    let mut doc = fs::read(p).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| json!({}));
    let (parent, last) = pointer.rsplit_once('/').unwrap();
    let mut cur = &mut doc;
    for t in parent.split('/').skip(1) {
        cur = cur.as_object_mut().unwrap().entry(t).or_insert_with(|| json!({}));
    }
    cur[last] = v;
    fs::write(p, serde_json::to_vec(&doc).unwrap()).unwrap();
}

struct Setup {
    _tmp: tempfile::TempDir,
    home: std::path::PathBuf,
    creds: std::path::PathBuf,
}

fn setup(defs: &[(&str, String)]) -> Setup {
    std::env::set_var("ACSWI_NO_KEYRING", "1");
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("acswi");
    let creds = tmp.path().join("creds.json");
    fs::create_dir_all(home.join("apps")).unwrap();
    let c = creds.to_string_lossy().replace('\\', "/");
    for (id, body) in defs {
        let text = format!("id = \"{id}\"\nname = \"{id}\"\nlogin = {LOGIN}\n{}", body.replace("{creds}", &c));
        fs::write(home.join("apps").join(format!("{id}.toml")), text).unwrap();
    }
    Setup { _tmp: tmp, home, creds }
}

fn open(s: &Setup) -> Acswi {
    let a = Acswi::with_paths(Paths { root: s.home.clone(), user_defs: s.home.join("apps") }).unwrap();
    assert!(a.defs.errors.is_empty(), "{:?}", a.defs.errors);
    a
}

const MAIN: &str = r#"
[identity]
file = "{creds}"
pointer = "/oauth/who"
[expiry]
file = "{creds}"
pointer = "/oauth/refreshExpiresAt"
[[state]]
kind = "json_key"
path = "{creds}"
keys = ["/oauth"]
"#;

const MCP: &str = r#"
[[state]]
kind = "json_key"
path = "{creds}"
keys = ["/mcpOAuth/notion|*"]
"#;

fn expiry_of(a: &Acswi, app: &str, profile: &str) -> acswi_core::Expiry {
    let v = a.overview(Some(app)).unwrap().remove(0);
    v.profiles.into_iter().find(|p| p.name == profile).unwrap().expiry.expect("expiry known")
}

#[test]
fn expiry_follows_each_account() {
    let s = setup(&[("main", MAIN.into())]);
    let a = open(&s);
    let opts = SwitchOptions { relaunch: false, ..Default::default() };
    let alice_exp = now_ms() + 30 * DAY_MS;
    let bob_exp = now_ms() + 3 * DAY_MS;

    put(&s.creds, "/oauth", json!({ "who": "alice", "refreshExpiresAt": alice_exp }));
    a.save("main", "alice", Actor::Cli).unwrap();
    a.add_begin("main", "bob", &opts).unwrap();
    put(&s.creds, "/oauth", json!({ "who": "bob", "refreshExpiresAt": bob_exp }));
    a.add_finish("main", &opts).unwrap();

    let alice = expiry_of(&a, "main", "alice");
    assert_eq!(alice.at.timestamp_millis() / 1000, alice_exp / 1000);
    assert_eq!((alice.level, alice.estimated), (Level::Ok, false));
    assert_eq!(expiry_of(&a, "main", "bob").level, Level::Soon);

    // The active account's date is read live, so a token refresh shows at once.
    put(&s.creds, "/oauth/refreshExpiresAt", json!(now_ms() - DAY_MS));
    assert_eq!(expiry_of(&a, "main", "bob").level, Level::Expired);
    let st = a.status("main").unwrap();
    assert_eq!(st.expiring.len(), 1);
    assert_eq!(st.expiring[0].profile, "bob");

    // Switching away records the date the account had when it was put away.
    a.switch("main", "alice", &opts).unwrap();
    assert_eq!(expiry_of(&a, "main", "bob").level, Level::Expired);
    assert_eq!(expiry_of(&a, "main", "alice").level, Level::Ok);
}

#[test]
fn mcp_sign_in_switches_apart_from_the_account() {
    let s = setup(&[("main", MAIN.into()), ("main-mcp-notion", MCP.into())]);
    let a = open(&s);
    let opts = SwitchOptions { relaunch: false, ..Default::default() };
    put(&s.creds, "/oauth", json!({ "who": "alice", "refreshExpiresAt": now_ms() + DAY_MS * 60 }));
    put(&s.creds, "/mcpOAuth/notion|h1", json!({ "token": "notion-personal" }));
    put(&s.creds, "/mcpOAuth/figma|h2", json!({ "token": "figma" }));

    a.save("main-mcp-notion", "personal", Actor::Cli).unwrap();
    a.add_begin("main-mcp-notion", "work", &opts).unwrap();
    let doc = read(&s.creds);
    assert!(doc.pointer("/mcpOAuth/notion|h1").is_none(), "notion sign-in put away");
    assert_eq!(doc.pointer("/mcpOAuth/figma|h2/token"), Some(&json!("figma")), "other servers untouched");
    assert_eq!(doc.pointer("/oauth/who"), Some(&json!("alice")), "the account itself untouched");

    // Signing in again may land under another settings hash; the wildcard still covers it.
    put(&s.creds, "/mcpOAuth/notion|h9", json!({ "token": "notion-work" }));
    a.add_finish("main-mcp-notion", &opts).unwrap();
    a.switch("main-mcp-notion", "personal", &opts).unwrap();
    let doc = read(&s.creds);
    assert_eq!(doc.pointer("/mcpOAuth/notion|h1/token"), Some(&json!("notion-personal")));
    assert!(doc.pointer("/mcpOAuth/notion|h9").is_none());
    assert_eq!(doc.pointer("/oauth/who"), Some(&json!("alice")));

    // And switching the account leaves the MCP sign-in alone.
    a.save("main", "alice", Actor::Cli).unwrap();
    a.add_begin("main", "bob", &opts).unwrap();
    assert_eq!(read(&s.creds).pointer("/mcpOAuth/notion|h1/token"), Some(&json!("notion-personal")));
}

#[test]
fn old_whole_file_profiles_still_restore() {
    let whole = r#"
[[state]]
kind = "file"
path = "{creds}"
"#;
    let s = setup(&[("main", whole.into())]);
    let opts = SwitchOptions { relaunch: false, ..Default::default() };
    {
        let a = open(&s);
        put(&s.creds, "/oauth", json!({ "who": "old" }));
        a.save("main", "old", Actor::Cli).unwrap();
        a.add_begin("main", "new", &opts).unwrap();
        put(&s.creds, "/oauth", json!({ "who": "new" }));
        a.add_finish("main", &opts).unwrap();
    }
    // The definition now keeps only the account key (as Claude Code's does since 0.3).
    let c = s.creds.to_string_lossy().replace('\\', "/");
    fs::write(
        s.home.join("apps").join("main.toml"),
        format!("id = \"main\"\nname = \"main\"\nlogin = {LOGIN}\n{}", MAIN.replace("{creds}", &c)),
    )
    .unwrap();
    let a = open(&s);
    put(&s.creds, "/mcpOAuth/notion|h1", json!({ "token": "kept" }));
    a.switch("main", "old", &opts).unwrap();
    let doc = read(&s.creds);
    assert_eq!(doc.pointer("/oauth/who"), Some(&json!("old")));
    assert_eq!(doc.pointer("/mcpOAuth/notion|h1/token"), Some(&json!("kept")));
    a.switch("main", "new", &opts).unwrap();
    assert_eq!(read(&s.creds).pointer("/oauth/who"), Some(&json!("new")));
}
