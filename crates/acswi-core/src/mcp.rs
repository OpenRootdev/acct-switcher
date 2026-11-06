//! Sign-ins to remote MCP servers in Claude Code and Codex CLI. Each client keeps one sign-in
//! per server, apart from its own account. `acswi mcp add` writes a definition for one
//! server's sign-in, so it gets accounts of its own and switches on its own.
//!
//! Only server names, addresses and key names are read here, never tokens.

use std::path::PathBuf;

use serde::Serialize;
use serde_json::Value;

use crate::def::{AppDef, Defs, StateItem};
use crate::error::{Code, Error, Result};
use crate::paths::{expand, slug};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Client {
    Claude,
    Codex,
}

impl Client {
    pub const ALL: [Client; 2] = [Client::Claude, Client::Codex];

    pub fn id(self) -> &'static str {
        match self {
            Client::Claude => "claude",
            Client::Codex => "codex",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Client::Claude => "Claude Code",
            Client::Codex => "Codex CLI",
        }
    }

    pub fn parse(s: &str) -> Option<Client> {
        Client::ALL.into_iter().find(|c| c.id().eq_ignore_ascii_case(s) || c.name().eq_ignore_ascii_case(s))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct McpServer {
    pub client: Client,
    pub server: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The client has a sign-in stored for this server now.
    pub signed_in: bool,
    /// Where the client keeps the server's tokens; `None` if acswi can't switch them here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<StateItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    /// The acswi app id for this sign-in.
    pub app: String,
    /// That app is already set up (by `acswi mcp add` or by hand).
    pub defined: bool,
}

/// Remote MCP servers the clients know about or hold sign-ins for.
pub fn find(defs: &Defs) -> Vec<McpServer> {
    let mut out = claude();
    out.extend(codex());
    for s in &mut out {
        s.defined = defs.apps.contains_key(&s.app)
            || s.item.as_ref().is_some_and(|item| {
                let want = item.describe();
                defs.apps.values().any(|d| d.state.iter().any(|i| i.describe() == want))
            });
    }
    out.sort_by(|a, b| (a.client.id(), a.server.to_lowercase()).cmp(&(b.client.id(), b.server.to_lowercase())));
    out
}

fn app_id(client: Client, server: &str) -> String {
    slug(&format!("{}-mcp-{}", client.id(), server.to_lowercase()))
}

fn read_json(path: &std::path::Path) -> Option<Value> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(bytes.strip_prefix(b"\xef\xbb\xbf".as_slice()).unwrap_or(&bytes)).ok()
}

/// Escapes one JSON pointer part.
fn token(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}

/// Adds a server or fills in what is missing.
fn merge(list: &mut Vec<(String, Option<String>, Option<String>)>, name: &str, url: Option<String>, key: Option<String>) {
    match list.iter_mut().find(|(n, ..)| n == name) {
        Some((_, u, k)) => {
            if u.is_none() {
                *u = url;
            }
            if k.is_none() {
                *k = key;
            }
        }
        None => list.push((name.to_string(), url, key)),
    }
}

// ------------------------------------------------------------------ Claude Code

const CLAUDE_CREDENTIALS: &str = "~/.claude/.credentials.json";

/// Claude Code keeps MCP sign-ins under `mcpOAuth` in .credentials.json, keyed
/// `<server>|<hash of the server's settings>`, and the servers in ~/.claude.json.
fn claude() -> Vec<McpServer> {
    let mut found: Vec<(String, Option<String>, Option<String>)> = Vec::new();
    if let Some(Value::Object(tokens)) = read_json(&expand(CLAUDE_CREDENTIALS)).and_then(|d| d.get("mcpOAuth").cloned()) {
        for (key, v) in tokens {
            let name = v.get("serverName").and_then(Value::as_str).unwrap_or_else(|| key.split('|').next().unwrap_or(&key));
            let url = v.get("serverUrl").and_then(Value::as_str).map(str::to_string);
            merge(&mut found, &name.to_string(), url, Some(key.clone()));
        }
    }
    if let Some(config) = read_json(&expand("~/.claude.json")) {
        let mut tables: Vec<&Value> = config.get("mcpServers").into_iter().collect();
        if let Some(Value::Object(projects)) = config.get("projects") {
            tables.extend(projects.values().filter_map(|p| p.get("mcpServers")));
        }
        for table in tables.into_iter().filter_map(Value::as_object) {
            for (name, cfg) in table {
                let kind = cfg.get("type").and_then(Value::as_str).unwrap_or("");
                let url = cfg.get("url").and_then(Value::as_str);
                if url.is_some() || matches!(kind, "http" | "sse") {
                    merge(&mut found, name, url.map(str::to_string), None);
                }
            }
        }
    }
    found
        .into_iter()
        .map(|(server, url, key)| {
            // Match every settings hash, so editing the server's settings keeps working.
            let pattern = match &key {
                Some(k) if !k.contains('|') => token(k),
                _ => format!("{}|*", token(&server)),
            };
            McpServer {
                client: Client::Claude,
                app: app_id(Client::Claude, &server),
                signed_in: key.is_some(),
                item: Some(StateItem::JsonKey { path: CLAUDE_CREDENTIALS.into(), keys: vec![format!("/mcpOAuth/{pattern}")] }),
                problem: None,
                defined: false,
                server,
                url,
            }
        })
        .collect()
}

// ------------------------------------------------------------------ Codex CLI

const CODEX_KEYRING: &str = "Codex MCP Credentials";

/// Codex's folder as written in a definition: `~/.codex` unless CODEX_HOME moves it.
fn codex_home() -> String {
    std::env::var("CODEX_HOME").ok().filter(|h| !h.is_empty()).unwrap_or_else(|| "~/.codex".into())
}

/// Codex keeps MCP sign-ins in the system keyring (Credential Manager on Windows), service
/// "Codex MCP Credentials", account `<server>|<hash>`; or with `mcp_oauth_credentials_store =
/// "file"` (or no keyring) in .credentials.json in its folder, keyed the same way.
fn codex() -> Vec<McpServer> {
    let home = codex_home();
    let file = format!("{home}/.credentials.json");
    let mut found: Vec<(String, Option<String>, Option<String>)> = Vec::new();
    let config: Option<toml::Table> =
        std::fs::read_to_string(expand(&format!("{home}/config.toml"))).ok().and_then(|t| t.parse().ok());
    let store = config
        .as_ref()
        .and_then(|c| c.get("mcp_oauth_credentials_store"))
        .and_then(|v| v.as_str())
        .unwrap_or("auto")
        .to_string();
    if let Some(servers) = config.as_ref().and_then(|c| c.get("mcp_servers")).and_then(|v| v.as_table()) {
        for (name, cfg) in servers {
            if let Some(url) = cfg.get("url").and_then(|u| u.as_str()) {
                merge(&mut found, name, Some(url.to_string()), None);
            }
        }
    }
    let mut in_file: Vec<String> = Vec::new();
    if let Some(Value::Object(tokens)) = read_json(&expand(&file)) {
        for key in tokens.keys() {
            let name = key.split('|').next().unwrap_or(key).to_string();
            merge(&mut found, &name, None, Some(key.clone()));
            in_file.push(name);
        }
    }
    let suffix = format!(".{CODEX_KEYRING}");
    let mut in_keyring: Vec<String> = Vec::new();
    if cfg!(windows) {
        // Credential Manager filters only take a `*` at the end, so look through all names.
        for c in crate::credman::list_info(None).unwrap_or_default() {
            let Some(account) = c.target.strip_suffix(&suffix) else { continue };
            let name = account.split('|').next().unwrap_or(account).to_string();
            merge(&mut found, &name, None, Some(account.to_string()));
            in_keyring.push(name);
        }
    }
    found
        .into_iter()
        .map(|(server, url, key)| {
            let use_file = in_file.contains(&server) || (store == "file" && !in_keyring.contains(&server));
            let (item, problem) = if use_file {
                (Some(StateItem::JsonKey { path: file.clone(), keys: vec![format!("/{}|*", token(&server))] }), None)
            } else if cfg!(windows) {
                (Some(StateItem::Credential { pattern: format!("{server}|*{suffix}") }), None)
            } else {
                (
                    None,
                    Some(format!(
                        "Codex keeps this sign-in in the system keyring, which acswi can only switch on Windows. \
                         Set mcp_oauth_credentials_store = \"file\" in {home}/config.toml and sign in again to use acswi with it."
                    )),
                )
            };
            McpServer {
                client: Client::Codex,
                app: app_id(Client::Codex, &server),
                signed_in: key.is_some(),
                item,
                problem,
                defined: false,
                server,
                url,
            }
        })
        .collect()
}

// ------------------------------------------------------------------ definitions

fn quote(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

fn list(items: &[String]) -> String {
    format!("[{}]", items.iter().map(|s| quote(s)).collect::<Vec<_>>().join(", "))
}

/// The definition text for one server's sign-in. `client` is the client app's own
/// definition, for its process names and how to find it.
pub fn definition(s: &McpServer, client: &AppDef) -> Result<String> {
    let Some(item) = &s.item else {
        return Err(Error::new(Code::Unsupported, s.problem.clone().unwrap_or_else(|| "acswi can't switch this sign-in".into())));
    };
    let name = format!("{} MCP ({})", s.server, s.client.name());
    let mut t = String::new();
    t.push_str(&format!(
        "# Sign-in to the remote MCP server {:?} in {}{}.\n# Switched apart from the {} account itself. Made by `acswi mcp add {} {}`.\n",
        s.server,
        s.client.name(),
        s.url.as_ref().map(|u| format!(" ({u})")).unwrap_or_default(),
        s.client.name(),
        s.client.id(),
        s.server
    ));
    t.push_str(&format!("id = {}\nname = {}\nstrategy = \"swap\"\n", quote(&s.app), quote(&name)));
    t.push_str(&format!("platforms = [{}]\n", quote(std::env::consts::OS)));
    t.push_str(&format!("detect = {}\n", list(&client.detect)));
    t.push_str(&format!("processes = {}\n", list(&client.processes)));
    t.push_str(&format!("login = {}\n", list(&[s.client.id().into(), "mcp".into(), "login".into(), s.server.clone()])));
    t.push_str(&format!(
        "notes = {}\n",
        quote(&format!("Close {} sessions before switching: they keep the server's sign-in in memory.", s.client.name()))
    ));
    t.push_str("\n[[state]]\n");
    match item {
        StateItem::JsonKey { path, keys } => {
            t.push_str(&format!("kind = \"json_key\"\npath = {}\nkeys = {}\n", quote(path), list(keys)));
        }
        StateItem::Credential { pattern } => {
            t.push_str(&format!("kind = \"credential\"\npattern = {}\n", quote(pattern)));
        }
        other => return Err(Error::new(Code::Unsupported, format!("unexpected item {}", other.describe()))),
    }
    // Check it loads before anyone relies on it.
    AppDef::parse(&t, None)?;
    Ok(t)
}

/// Writes the definition for `server` of `client` into the user's apps folder.
pub fn add(defs: &Defs, user_defs: &std::path::Path, client: &str, server: &str) -> Result<(McpServer, PathBuf)> {
    let c = Client::parse(client).ok_or_else(|| {
        Error::new(Code::UnknownApp, format!("{client:?} is not an MCP client acswi knows. Use claude or codex."))
    })?;
    let all = find(defs);
    let names: Vec<&str> = all.iter().filter(|s| s.client == c).map(|s| s.server.as_str()).collect();
    let s = all
        .iter()
        .find(|s| s.client == c && s.server.eq_ignore_ascii_case(server))
        .cloned()
        .ok_or_else(|| {
            Error::new(
                Code::UnknownProfile,
                format!(
                    "{} has no remote MCP server called {server:?}. Known: {}",
                    c.name(),
                    if names.is_empty() { "none".into() } else { names.join(", ") }
                ),
            )
        })?;
    if s.defined {
        return Err(Error::new(Code::ProfileExists, format!("{} is already set up as the app {}", s.server, s.app)));
    }
    let client_def = defs.get(c.id())?;
    let text = definition(&s, client_def)?;
    std::fs::create_dir_all(user_defs).map_err(|e| Error::io(user_defs.display(), e))?;
    let path = user_defs.join(format!("{}.toml", s.app));
    std::fs::write(&path, text).map_err(|e| Error::io(path.display(), e))?;
    Ok((s, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_loadable_definitions() {
        let defs = Defs::load(std::path::Path::new("Z:\\does-not-exist"));
        let claude = defs.get("claude").unwrap();
        let s = McpServer {
            client: Client::Claude,
            server: "notion".into(),
            url: Some("https://mcp.notion.com/mcp".into()),
            signed_in: true,
            item: Some(StateItem::JsonKey { path: CLAUDE_CREDENTIALS.into(), keys: vec!["/mcpOAuth/notion|*".into()] }),
            problem: None,
            app: app_id(Client::Claude, "notion"),
            defined: false,
        };
        let def = AppDef::parse(&definition(&s, claude).unwrap(), None).unwrap();
        assert_eq!(def.id, "claude-mcp-notion");
        assert_eq!(def.login, ["claude", "mcp", "login", "notion"]);
        assert_eq!(def.processes, claude.processes);
        let s = McpServer { item: Some(StateItem::Credential { pattern: "figma|*.Codex MCP Credentials".into() }), ..s };
        assert!(definition(&s, claude).is_ok());
        assert_eq!(app_id(Client::Codex, "My Server/1"), "codex-mcp-my-server-1");
    }
}
