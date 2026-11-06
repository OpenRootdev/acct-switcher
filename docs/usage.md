# Using Account Switcher (acswi)

In a terminal, `acswi` on its own lists every app with numbered accounts. Type a number to switch,
or `a` to add an account:

```
Discord  discord
   1 ● personal
   2   work

GitHub CLI  gh
   3 ● sam-dev
   4   sam-contoso

Type 1-4 to switch, a to add an account, or press Enter to quit:
```

`acswi discord` does the same for one app. Everything also works as a direct command:

```
acswi                         pick an account from a list
acswi gh work                 switch GitHub CLI to "work"   (same as: acswi use gh work)
acswi add discord             sign in another Discord account (asks for its name)
acswi add discord personal    the same, named "personal"
acswi add az work -- --tenant contoso.onmicrosoft.com
                             options after -- go to the app's sign-in command
acswi save claude me          save the account Claude Code is signed in to now as "me"
acswi save claude             update the saved copy of the active account
acswi rename az someone@contoso.onmicrosoft.com work
acswi status                  who each app is signed in as right now
acswi check                   when saved sign-ins expire (see below)
acswi mcp                     sign-ins to remote MCP servers (see below)
acswi remove discord old      delete a saved account
acswi apps                    which apps acswi knows
acswi learn slack             teach acswi a new app (see adding-apps.md)
acswi doctor                  check the setup, finish an interrupted switch
```

If the app is open, acswi asks before closing it (`--close` skips the question) and reopens it
afterwards.

An unfinished add (for example when the sign-in window was closed) shows up in `acswi`; finish or
undo it with `acswi add <app> --finish` or `acswi add <app> --cancel`.

## Sign-ins that expire

Sign-ins don't last forever. Some apps store an expiry date next to their tokens, and some
providers sign out accounts that go unused for a while. acswi shows this next to each account in
`acswi`, `acswi status` and the window (`until 23 Nov 2026`, `expires in 3 days`, `expired, sign
in again`), and the tray marks expiring accounts. `acswi check` lists every account:

```
✓ Azure CLI second      in use (signed out after 90 days unused)
✓ Claude Code work      until 23 Nov 2026
! Claude Code personal  expires in 3 days
✗ GitHub CLI old-job    probably signed out after going unused, sign in again

No expiry date known for Codex CLI, Discord.
```

It exits with 7 when an account has expired or expires within 7 days (`--days 14` to look further
ahead), so it can run on a schedule.

| app | what acswi knows |
|---|---|
| Claude Code | the date Claude Code stores with its sign-in; it moves on each time Claude Code refreshes it |
| GitHub CLI | GitHub revokes a token unused for a year, so a saved account not used for a year is probably signed out |
| Azure CLI | Microsoft sign-ins stop working after 90 days without use |
| others | no date is known; they work until the app or provider ends the sign-in |

Dates marked `~` or "about" are estimates from when acswi last switched away from the account.
Using an account resets the count. To renew one that has expired, switch to it, sign in again in
the app, then run `acswi save <app>` so the saved copy is the new sign-in.

## Remote MCP servers

Claude Code and Codex CLI sign in to remote MCP servers (Notion, Figma, Sentry, …) separately
from your own account, one sign-in per server. `acswi mcp` lists the remote servers each one
knows, and `acswi mcp add <client> <server>` gives one server's sign-in its own accounts:

```
acswi mcp                     list remote servers and whether they are signed in
acswi mcp add codex notion    make "notion MCP (Codex CLI)" an app with accounts of its own
acswi save codex-mcp-notion personal
acswi add codex-mcp-notion work    signs out, runs `codex mcp login notion`, saves "work"
acswi codex-mcp-notion personal    switch back
```

`mcp add` writes an app definition (for example `codex-mcp-notion.toml`) into your apps folder,
so the server shows up like any other app, also in the window and tray. Switching it changes
only that server's sign-in: the client's own account and its other servers stay as they are. The
client has to be closed while switching, since it holds the server's tokens while it runs.

Codex keeps these sign-ins in Credential Manager on Windows. On Linux it uses the system keyring,
which acswi can't switch; set `mcp_oauth_credentials_store = "file"` in `~/.codex/config.toml`
and sign in again to use acswi there. Claude Code keeps them in `~/.claude/.credentials.json`
on both.

## The desktop app

`acswi-desktop` opens a small window with every app and its accounts. Click **Use** to switch,
**Add account** to sign in a new one, and the **…** menu on an account to rename it, update its
saved copy or remove it. Closing the window keeps acswi in the tray, where each app has a submenu
of its accounts. Start it with `--hidden` to go straight to the tray.

On Linux the tray icon needs a desktop with AppIndicator support (on GNOME, the AppIndicator
extension); without it, use the window. Sign-in commands open in a terminal window: `$TERMINAL`
if it is set, otherwise the first of GNOME Terminal, Konsole, Xfce Terminal, kitty, Alacritty,
foot, WezTerm and xterm that is installed.

## For scripts and AI agents

Without a terminal (or with `--json`), `acswi` and `acswi <app>` print the list instead of asking.
Add `--json` to any command. Output is always `{"ok": true, "data": …}` or
`{"ok": false, "error": {"code": "…", "message": "…"}}`.

| exit | code | meaning |
|---|---|---|
| 0 | | done |
| 3 | `app_running` | the app is open; retry with `--close` if closing it is fine |
| 4 | `needs_human` | signing in needs a person (`add`, `learn`) |
| 5 | `unknown_app`, `unknown_profile` | `details.profiles` lists the valid names |
| 6 | `unsaved_session`, `identity_mismatch` | the live account is not the one acswi expected; nothing was changed |
| 7 | | `acswi check` only: an account has expired or expires soon (the output is still `"ok": true`) |
| 1 | anything else | see `error.message` |

Agents switch by profile name (`acswi use gh work --json`). There is no command that prints a
token, and `add`/`learn` refuse to run without a terminal. Every switch is written to
`audit.log` in the data folder (see below) with who made it (`cli`, `ui` or `agent`).

## Where acswi keeps its files

| what | Windows | Linux |
|---|---|---|
| saved accounts, state, audit log | `%LOCALAPPDATA%\acswi` | `~/.local/share/acswi` |
| your app definitions | `%APPDATA%\acswi\apps` | `~/.config/acswi/apps` |
| key that encrypts saved accounts | DPAPI, tied to your Windows user | your desktop keyring, or a `key` file in the data folder |

On Linux, `XDG_DATA_HOME` and `XDG_CONFIG_HOME` move those folders. `ACSWI_HOME` puts everything
in one folder (handy for tests). A new Linux install keeps its key in the keyring when the keyring
is unlocked, as it normally is after you log in; otherwise, or with `ACSWI_NO_KEYRING=1`, it uses
the key file, which only your user can read.

The tool used to be called acct. Its `%LOCALAPPDATA%\acct` and `%APPDATA%\acct` folders are
renamed to `acswi` the first time acswi runs, so saved accounts carry over.

## Build from source

On Windows you need Rust (stable, MSVC) and the Visual Studio C++ build tools. WebView2 is part of
Windows 11.

On Linux you need Rust and the WebKitGTK development packages. On Debian and Ubuntu:

```
sudo apt install build-essential pkg-config libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev
```

Then:

```
cargo build --release
```

This produces `acswi` (the command) and `acswi-desktop` (window and tray) in `target/release`,
with `.exe` on Windows. Put both in a folder on your `PATH`. `cargo build --release -p acswi-cli`
builds only the command, which needs none of the WebKitGTK packages.

Tests: `cargo test -p acswi-core -p acswi-cli`. On Windows they use real DPAPI and Credential
Manager under a temporary folder, creating and deleting `acswi-selftest*` credentials. On Linux
they use a key file, never your keyring.
