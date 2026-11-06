# How acswi works

## Built-in apps

| app | how acswi switches it |
|---|---|
| GitHub CLI (`gh`) | its own account list (`gh auth switch`) |
| Azure CLI (`az`) | its own account list (`az account set`); profile = user |
| Claude Code (`claude`) | swaps the `claudeAiOauth` entry in `~/.claude/.credentials.json` and the `oauthAccount` entry in `~/.claude.json`; MCP server sign-ins in the same file stay put |
| Codex CLI (`codex`) | swaps `~/.codex/auth.json` |
| GitHub Desktop (Windows only) | swaps its Credential Manager entries and its `Local Storage` folder |
| Discord | swaps its `Local Storage` folder |
| Cursor | swaps `User\globalStorage` (sign-in and chat history) and its session folders; settings stay shared |

## A switch, step by step

1. acswi takes a lock, so the CLI, the app and agents never switch at the same time.
2. If the app is running it is asked to close (a normal window close on Windows, `SIGTERM` on
   Linux), then force-closed after 6 seconds (many Electron apps only hide to the tray).
3. The live sign-in is **saved back into the active profile first**. Apps that rotate refresh
   tokens (Claude Code, Codex, most OAuth apps) keep working because the newest token is what
   gets stored. If the app reports a different account than the profile expects, acswi stops
   and changes nothing.
4. The live state is cleared and the target profile is restored. Each step is journaled; if
   the PC dies halfway, the next acswi command finishes or undoes it.
5. The app is reopened if it was open.

## Security

- Saved sign-ins are encrypted for your user on this computer, together with a random
  per-install value in the data folder's `entropy` file.
  - Windows: DPAPI. Copying the vault to another PC or user does not decrypt it.
  - Linux: XChaCha20-Poly1305 with a random key kept in your desktop keyring (GNOME Keyring,
    KWallet). When the keyring is locked or missing on first use, as in WSL or over SSH, the key
    goes in a `key` file that only your user can read. That is how most CLIs keep their own
    tokens on Linux.
  - Files acswi restores keep their permissions; new ones are readable by your user only.
- Secrets are wiped from memory after use. `state.json` and `audit.log` hold names and
  account labels only.
- acswi never prints a token, and the JSON output has no secret fields.
- Limit: anything running as you can still decrypt the vault or ask a CLI for its token
  (`gh auth token`). That is the same protection the apps themselves give their tokens. It
  is not a sandbox against your own processes. To keep an AI agent away from tokens, also deny
  it those commands, e.g. in Claude Code settings:
  `"deny": ["Read(~/AppData/Local/acswi/**)", "Read(~/.local/share/acswi/**)", "Bash(gh auth token*)"]`.

## Limits

| case | examples | what works |
|---|---|---|
| Device-bound tokens (Microsoft WAM / Primary Refresh Token) | Office, new Teams, OneDrive, az on Windows | can't be copied. acswi only selects among accounts the system already holds (az); use the app's own account switcher for Office/Teams |
| Electron `safeStorage` | Discord, Slack, Notion, GitHub Desktop, VS Code | works on this computer under this user (on Linux the app's key is in the keyring); profiles can't move to another computer |
| Rotating refresh tokens | Claude Code, Codex, many OAuth apps | fine while acswi does the switching. Don't restore an old copy by hand, and don't use the same profile on two PCs |
| Chrome/Edge app-bound cookies | browsers | not supported; use browser profiles |
| Store (MSIX) apps | apps under `%LOCALAPPDATA%\Packages` | possible with a definition, not tested |
| Apps open while switching | any | acswi always closes them first; a running app would overwrite the swap |

Claude desktop app sessions sign in through the desktop app, not `~/.claude/.credentials.json`,
so switching Claude Code does not affect them.

Azure CLI: `az login`'s interactive tenant picker crashes when a tenant needs MFA
([Azure/azure-cli#33555](https://github.com/Azure/azure-cli/issues/33555)). acswi turns the picker
off while adding an account; for an account whose tenant needs MFA, name the tenant:
`acswi add az work -- --tenant <tenant id>`.

Design notes: [DESIGN.md](DESIGN.md).
