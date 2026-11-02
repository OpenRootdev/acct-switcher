# Using acct

In a terminal, `acct` on its own lists every app with numbered accounts. Type a number to switch,
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

`acct discord` does the same for one app. Everything also works as a direct command:

```
acct                         pick an account from a list
acct gh work                 switch GitHub CLI to "work"   (same as: acct use gh work)
acct add discord             sign in another Discord account (asks for its name)
acct add discord personal    the same, named "personal"
acct add az work -- --tenant contoso.onmicrosoft.com
                             options after -- go to the app's sign-in command
acct save claude me          save the account Claude Code is signed in to now as "me"
acct save claude             update the saved copy of the active account
acct rename az someone@contoso.onmicrosoft.com work
acct status                  who each app is signed in as right now
acct remove discord old      delete a saved account
acct apps                    which apps acct knows
acct learn slack             teach acct a new app (see adding-apps.md)
acct doctor                  check the setup, finish an interrupted switch
```

If the app is open, acct asks before closing it (`--close` skips the question) and reopens it
afterwards.

An unfinished add (for example when the sign-in window was closed) shows up in `acct`; finish or
undo it with `acct add <app> --finish` or `acct add <app> --cancel`.

## The desktop app

`acct-desktop` opens a small window with every app and its accounts. Click **Use** to switch,
**Add account** to sign in a new one, and the **…** menu on an account to rename it, update its
saved copy or remove it. Closing the window keeps acct in the tray, where each app has a submenu
of its accounts. Start it with `--hidden` to go straight to the tray.

## For scripts and AI agents

Without a terminal (or with `--json`), `acct` and `acct <app>` print the list instead of asking.
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
token, and `add`/`learn` refuse to run without a terminal. Every switch is written to
`%LOCALAPPDATA%\acct\audit.log` with who made it (`cli`, `ui` or `agent`).

## Build from source

Needs Rust (stable, MSVC) and the Visual Studio C++ build tools. WebView2 is part of Windows 11.

```
cargo build --release
```

Produces `target\release\acct.exe` (CLI) and `target\release\acct-desktop.exe` (window and tray).
Put both in a folder on your `PATH`.

Data lives in `%LOCALAPPDATA%\acct` (`ACCT_HOME` overrides it, e.g. for tests).

Tests: `cargo test -p acct-core` (uses real DPAPI and Credential Manager under a temporary folder;
it creates and deletes `acct-selftest*` credentials).
