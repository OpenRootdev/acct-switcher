# acct

Keep several signed-in accounts for an app or CLI on one Windows PC and switch which one
it uses, without signing in again. One account at a time per app; this is not for running
two accounts side by side.

- **`acct`**: a small command-line tool. Readable for people, `--json` for scripts and AI agents.
- **acct desktop app**: a window and a tray menu. Click an account to switch.
- **Generic**: every app is described by a short TOML file. acct ships with a few; add your
  own by hand or with `acct learn`.

## Use it

```
acct                         show every app and its accounts
acct gh work                 switch GitHub CLI to "work"   (same as: acct use gh work)
acct add discord personal    sign in a new Discord account and save it as "personal"
acct save claude me          save the account Claude Code is signed in to now as "me"
acct save claude             update the saved copy of the active account
acct rename az KristoferJussmann@corp.onmicrosoft.com work
acct status                  who each app is signed in as right now
acct remove discord old      delete a saved account
acct apps                    which apps acct knows
acct learn slack             teach acct a new app (see below)
acct doctor                  check the setup, finish an interrupted switch
```

If the app is open, acct asks before closing it (`--close` skips the question) and reopens it
afterwards.

### For scripts and AI agents

Add `--json` to any command. Output is always `{"ok": true, "data": …}` or
`{"ok": false, "error": {"code": "…", "message": "…"}}`.

| exit | code | meaning |
|---|---|---|
| 0 | | done |
| 3 | `app_running` | the app is open; retry with `--close` if closing it is fine |
| 4 | `needs_human` | signing in needs a person (`add`, `learn`) |
| 5 | `unknown_app`, `unknown_profile` | `details.profiles` lists the valid names |
| 6 | `unsaved_session`, `identity_mismatch` | the live account is not the one acct expected; nothing was changed |
| 1 | anything else | see `error.message` |

Agents switch by profile name (`acct use gh work --json`). There is no command that prints a
token, and `add`/`learn` refuse to run without a terminal.

## What ships

| app | how acct switches it |
|---|---|
| GitHub CLI (`gh`) | its own account list (`gh auth switch`) |
| Azure CLI (`az`) | its own account list (`az account set`); profile = user |
| Claude Code (`claude`) | swaps `~/.claude/.credentials.json` and the `oauthAccount` entry in `~/.claude.json` |
| Codex CLI (`codex`) | swaps `~/.codex/auth.json` |
| GitHub Desktop | swaps its Credential Manager entries and its `Local Storage` folder |
| Discord | swaps its `Local Storage` folder |

## Add an app

Put a `.toml` file in `%APPDATA%\acct\apps\` (the desktop app's … menu opens the folder).
Your files override the built-in ones with the same `id`.

```toml
id = "slack"
name = "Slack"
strategy = "swap"                       # swap | native | env
detect = ['%LOCALAPPDATA%\slack\slack.exe']
processes = ["slack.exe"]                # closed before swapping
launch = ['%LOCALAPPDATA%\slack\slack.exe']
# login = ["tool", "login"]             # CLI tools: how to sign in a new account

[identity]                               # optional; who is signed in (never a secret)
command = ["tool", "whoami", "--json"]
pointer = "/email"

[[state]]
kind = "dir"
path = '%APPDATA%\Slack\Network'
mode = "move"

[[state]]
kind = "dir"
path = '%APPDATA%\Slack\Local Storage'
mode = "move"
```

State kinds:

| kind | fields | saved as |
|---|---|---|
| `file` | `path` | encrypted copy |
| `dir` | `path`, `mode = "encrypt"` (default) or `"move"`, `exclude = ["*.log"]` | `encrypt`: files packed into the encrypted vault, excluded files stay shared. `move`: the folder is moved aside as-is (for big app folders) |
| `credential` | `pattern` (Credential Manager target, `*`/`?` allowed) | encrypted copy |
| `registry` | `key` under HKCU | encrypted copy |
| `json_key` | `path`, `keys = ["/oauthAccount"]` (JSON pointers) | only those keys; the rest of the file is left alone |

Other strategies:

```toml
strategy = "native"         # the app keeps several accounts itself
[native]
list = ["tool", "accounts", "--json"]    # or list_file = "~/.tool/accounts.json"
items = "/accounts"                      # pointer to the array ("" = root)
account = "/email"                       # label per item
active = "/current"                      # boolean per item
value = "/id"                            # passed to switch as {value} (default: label)
switch = ["tool", "use", "{value}"]

strategy = "env"            # one config folder per profile, picked by a user variable
[env]
var = "TOOL_CONFIG_DIR"     # only programs started after the switch see it
```

### `acct learn`

If you don't know where an app keeps its sign-in:

```
acct learn slack
```

acct looks for folders named like the app in AppData and your home folder, asks you to close
the app, sign in, and close it again, then shows what changed (files, Electron session
folders, Credential Manager entries). It writes a draft definition for you to check before
saving. Remove anything that is not sign-in data. A `Cache` folder in a profile only costs space;
a settings file means settings switch with the account.

## How a switch works

1. acct takes a lock, so the CLI, the app and agents never switch at the same time.
2. If the app is running it is asked to close, then force-closed after 6 seconds (many
   Electron apps only hide to the tray).
3. The live sign-in is **saved back into the active profile first**. Apps that rotate refresh
   tokens (Claude Code, Codex, most OAuth apps) keep working because the newest token is what
   gets stored. If the app reports a different account than the profile expects, acct stops
   and changes nothing.
4. The live state is cleared and the target profile is restored. Each step is journaled; if
   the PC dies halfway, the next acct command finishes or undoes it.
5. The app is reopened if it was open.

## Security

- Saved sign-ins are encrypted with Windows DPAPI for your user on this PC, plus a random
  per-install key in `%LOCALAPPDATA%\acct\entropy`. Copying the vault to another PC or user
  does not decrypt it.
- Secrets are wiped from memory after use. `state.json` and `audit.log` hold names and
  account labels only.
- acct never prints a token, and the JSON output has no secret fields.
- Limit: anything running as you can still decrypt DPAPI data or ask a CLI for its token
  (`gh auth token`). That is the same protection the apps themselves give their tokens. It
  is not a sandbox against your own processes. To keep an AI agent away from tokens, also deny
  it those commands, e.g. in Claude Code settings:
  `"deny": ["Read(~/AppData/Local/acct/**)", "Bash(gh auth token*)"]`.

## Limits

| case | examples | what works |
|---|---|---|
| Device-bound tokens (Microsoft WAM / Primary Refresh Token) | Office, new Teams, OneDrive, az on Windows | can't be copied. acct only selects among accounts the system already holds (az); use the app's own account switcher for Office/Teams |
| Electron `safeStorage` | Discord, Slack, Notion, GitHub Desktop, VS Code | works on this PC under this Windows user; profiles can't move to another PC |
| Rotating refresh tokens | Claude Code, Codex, many OAuth apps | fine while acct does the switching. Don't restore an old copy by hand, and don't use the same profile on two PCs |
| Chrome/Edge app-bound cookies | browsers | not supported; use browser profiles |
| Store (MSIX) apps | apps under `%LOCALAPPDATA%\Packages` | possible with a definition, not tested |
| Apps open while switching | any | acct always closes them first; a running app would overwrite the swap |

Claude desktop app sessions sign in through the desktop app, not `~/.claude/.credentials.json`,
so switching Claude Code does not affect them.

## Build

Needs Rust (stable, MSVC) and the Visual Studio C++ build tools. WebView2 is part of Windows 11.

```
cargo build --release
```

Produces `target\release\acct.exe` (CLI) and `target\release\acct-desktop.exe` (window and tray;
start with `--hidden` to go straight to the tray). Put both on your `PATH` or in a folder of
your choice.

Data lives in `%LOCALAPPDATA%\acct` (`ACCT_HOME` overrides it, e.g. for tests).

Tests: `cargo test -p acct-core` (uses real DPAPI and Credential Manager under a
temporary folder; it creates and deletes `acct-selftest*` credentials).

Design notes: [docs/DESIGN.md](docs/DESIGN.md).
