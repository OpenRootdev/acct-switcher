# Adding an app

acswi knows an app from a short TOML file. The built-in ones are in [`defs/`](../defs). Put your own
in `%APPDATA%\acswi\apps\` on Windows or `~/.config/acswi/apps/` on Linux (the desktop app's …
menu opens the folder); a file with the same `id` as a built-in one replaces it.

```toml
id = "slack"
name = "Slack"
strategy = "swap"                       # swap | native | env
detect = ['%LOCALAPPDATA%\slack\slack.exe']
processes = ["slack.exe"]                # closed before swapping
launch = ['%LOCALAPPDATA%\slack\slack.exe']
# login = ["tool", "login"]             # CLI tools: how to sign in a new account
# login_env = { TOOL_NO_PROMPT = "1" }  # environment for the sign-in command only

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

## State kinds

| kind | fields | saved as |
|---|---|---|
| `file` | `path` | encrypted copy |
| `dir` | `path`, `mode = "encrypt"` (default) or `"move"`, `exclude = ["*.log"]` | `encrypt`: files packed into the encrypted vault, excluded files stay shared. `move`: the folder is moved aside as-is (for big app folders) |
| `credential` | `pattern` (Credential Manager target, `*`/`?` allowed). Windows only | encrypted copy |
| `registry` | `key` under HKCU. Windows only | encrypted copy |
| `json_key` | `path`, `keys = ["/oauthAccount"]` (JSON pointers) | only those keys; the rest of the file is left alone. A `*` in the last part (`"/mcpOAuth/notion\|*"`) takes every matching key of that object |

## Expiry

An optional `[expiry]` table tells acswi when a saved sign-in stops working, so it can warn
before it does (see `acswi check` in [usage.md](usage.md#sign-ins-that-expire)):

```toml
[expiry]
file = "~/.tool/credentials.json"        # a date stored with the sign-in...
pointer = "/oauth/refreshTokenExpiresAt" # seconds or milliseconds since 1970, or an RFC 3339 date
idle_days = 90                           # ...and/or: the provider signs out accounts unused this long
```

acswi reads only that date from the file, never a token. For the active account it reads it live;
for the others it uses the date recorded when it last saved them. With both, the earlier one wins.

## Other strategies

```toml
strategy = "native"         # the app keeps several accounts itself
[native]
list = ["tool", "accounts", "--json"]    # or list_file = "~/.tool/accounts.json"
items = "/accounts"                      # pointer to the array ("" = root)
account = "/email"                       # label per item
active = "/current"                      # boolean per item
value = "/id"                            # passed to switch as {value} (default: label)
switch = ["tool", "use", "{value}"]

strategy = "env"            # Windows only: one config folder per profile, picked by a user variable
[env]
var = "TOOL_CONFIG_DIR"     # only programs started after the switch see it
```

## Windows and Linux

Paths and process names usually differ between systems. An `[os.linux]` (or `[os.windows]`) table
replaces fields for that system only: `detect`, `processes`, `launch`, `login`, `login_env`,
`identity`, `state`, `native`, `env` and `notes`. `platforms` limits a definition to some systems;
on the others it is not shown.

```toml
detect = ['%LOCALAPPDATA%\slack\slack.exe']
processes = ["slack.exe"]
# platforms = ["windows", "linux"]      # default: every system

[os.linux]
detect = ["slack"]
processes = ["slack"]
launch = ["slack"]

[[os.linux.state]]
kind = "dir"
path = "~/.config/Slack/Local Storage"
mode = "move"
```

On Linux a process name is the executable's file name (`slack`, not `slack.exe`); a pattern with
`/` matches the full path, e.g. `'*/.local/share/claude/versions/*'`.

## `acswi learn`

If you don't know where an app keeps its sign-in:

```
acswi learn slack
```

acswi looks for folders named like the app in AppData, your home folder and Microsoft Store app
folders (`%LOCALAPPDATA%\Packages`), and for registry keys named like it under `HKCU\Software`. On
Linux it looks in `~/.config`, `~/.local/share`, dot-folders in your home folder, and Flatpak and
Snap folders. It asks you to close the app, sign in, and close it again (and checks that you did),
then shows what changed: files, Electron session folders and, on Windows, registry keys and
Credential Manager entries.
Caches, logs and extension folders are ignored. It writes a draft definition for you to check before
saving. Remove anything that is not sign-in data. A `Cache` folder in a profile only costs space;
a settings file means settings switch with the account.
