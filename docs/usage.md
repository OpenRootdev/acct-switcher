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
acswi remove discord old      delete a saved account
acswi apps                    which apps acswi knows
acswi learn slack             teach acswi a new app (see adding-apps.md)
acswi doctor                  check the setup, finish an interrupted switch
```

If the app is open, acswi asks before closing it (`--close` skips the question) and reopens it
afterwards.

An unfinished add (for example when the sign-in window was closed) shows up in `acswi`; finish or
undo it with `acswi add <app> --finish` or `acswi add <app> --cancel`.

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
