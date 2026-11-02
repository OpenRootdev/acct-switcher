# acswi — design

`acswi` keeps several signed-in accounts per app or CLI on one Windows PC and switches which
one the app uses, without signing in again. It is one account at a time per app, not
side-by-side sessions.

## Principles

1. **Generic first.** The engine knows nothing about specific apps. Each app is described by
   an *app definition* (TOML) that says where its sign-in state lives. Built-in definitions
   ship for common apps; anyone can add more without writing code.
2. **Native when it exists.** If an app already keeps several accounts (`gh auth switch`,
   `az account set`), a definition can use that instead of swapping files. Same CLI, same UI.
3. **Never lose a session.** Before switching away, the live state is saved back to the
   active profile, so rotated refresh tokens are kept. Swaps happen only while the app is
   closed, and each switch is journaled so an interrupted one can be finished or rolled back.
4. **As safe as the app itself, or safer.** Saved state is encrypted with DPAPI (bound to
   this Windows user on this PC). The CLI has no command that prints a secret.
5. **Simple to use.** `acswi gh work` switches. `acswi` alone shows everything.

## App definition

```toml
id       = "codex"
name     = "Codex CLI"
strategy = "swap"                      # swap | native | env
processes = ["codex.exe"]              # must be closed before swapping
launch    = ""                         # relaunch after switching (GUI apps)
login     = ["codex", "login"]         # how to sign in a new account

[identity]                             # optional: who is signed in (never a secret)
command = ["codex", "login", "status"]
# or: file = "~/.codex/auth.json", pointer = "/tokens/account_id"

[[state]]                              # where sign-in state lives
kind = "file"                          # file | dir | credential | registry | json_key
path = "~/.codex/auth.json"
```

State kinds:

| kind | live location | how it is stored per profile |
|---|---|---|
| `file` | one file | encrypted (DPAPI) in the vault |
| `dir` | a folder | `mode = "encrypt"`: packed + encrypted (small config dirs, with `exclude` globs); `mode = "move"`: the folder is moved aside unencrypted (large Electron profiles whose secrets are already DPAPI-wrapped by the app) |
| `credential` | Windows Credential Manager targets matching a pattern (`gh:github.com:*`) | encrypted in the vault |
| `registry` | an `HKCU` key, recursively | encrypted in the vault |
| `json_key` | one key inside a shared JSON file (e.g. `oauthAccount` in `~/.claude.json`); the rest of the file is left alone | encrypted in the vault |

Strategies:

- **swap**: close the app → save live state into the active profile → clear live state →
  restore the target profile → relaunch.
- **native**: list accounts with a command, switch with a command. Accounts the tool already
  knows show up as profiles automatically; `acswi rename` gives them friendly names.
- **env**: each profile gets its own config folder; switching sets a user environment variable
  (`CODEX_HOME`, `CLAUDE_CONFIG_DIR`, …). Nothing is copied, but settings and history are
  per-profile too, and only newly started programs see the change.

Definitions are loaded from the built-ins and from `%APPDATA%\acswi\apps\*.toml` (user
definitions win).

## Adding a new app without knowing where it stores things: `acswi learn`

`acswi learn <name>` records a fingerprint of likely locations (folders in `%APPDATA%`,
`%LOCALAPPDATA%`, dot-folders in the home folder, Credential Manager targets), asks you to
sign in to the app, then shows what changed and writes a draft definition for you to review.

## Storage — `%LOCALAPPDATA%\acswi\`

| path | contents |
|---|---|
| `state.json` | profiles, active profile per app, recorded identities, timestamps — no secrets |
| `vault\<app>\<profile>.bin` | DPAPI-encrypted state (with per-install entropy) |
| `profiles\<app>\<profile>\` | moved folders (`mode = "move"`) and env-strategy config folders |
| `journal.json` | in-progress switch, used to recover after a crash |
| `audit.log` | one line per switch: time, app, from, to, who (cli / ui / agent) |
| `lock` | one switch at a time across CLI, UI and agents |

## Switch, step by step (swap strategy)

1. Take the lock. Load the definition, find running processes.
2. If the app is running: the CLI stops with `app_running` unless `--close`; the UI asks.
   Closing sends a normal close first, then force-closes after a timeout.
3. Write the journal: `{app, from, to, phase}`.
4. **Save**: read every live state item; if an identity is defined, check it matches the
   active profile (otherwise stop with `identity_mismatch` rather than file one account's
   tokens under another's name). Encrypt and write to the vault atomically.
5. **Clear** the live items, **restore** the target's items.
6. Check the identity of the restored state, update `state.json`, append to the audit log,
   delete the journal, relaunch.

## Security

- DPAPI `CurrentUser` with an extra per-install entropy value. Secrets are wiped from memory
  after use (`zeroize`).
- No `show`/`get-token` command. JSON output contains profile names and identities only.
- `add`, `learn` and `remove` need an interactive terminal; agents get `needs_human`.
- Limit: a program running as you can still decrypt DPAPI data or run `gh auth token`
  itself. This matches what the apps themselves offer: most CLIs keep tokens in plain files
  or in Credential Manager. A stronger wall (a broker running as a separate Windows account)
  is possible later but out of scope.

## What can't be swapped

| case | examples | outcome |
|---|---|---|
| Device-bound tokens (Microsoft WAM / PRT, TPM) | Office, new Teams, OneDrive, az with WAM | can't be copied; use native multi-account (az) or the app's own switcher |
| Electron `safeStorage` (DPAPI key in `Local State`) | Discord, Notion, Slack, GitHub Desktop, VS Code | works on this PC and Windows user; a profile can't be moved to another PC |
| Rotating refresh tokens with reuse detection | Claude Code, Codex, many OAuth apps | works because the live state is saved before every switch; never run the same profile from two places |
| Chromium app-bound encryption | Chrome / Edge cookies | not supported; use browser profiles |
| Store (MSIX) apps | `%LOCALAPPDATA%\Packages\…` | possible but fragile; not targeted |

## Layout

```
crates/acswi-core      engine, definitions, vault, state stores, processes (Rust)
crates/acswi-cli       `acswi` command
apps/acswi-desktop     Tauri v2 window + tray (no frontend framework)
defs/                 built-in app definitions (embedded in the binaries)
```
