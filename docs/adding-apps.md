# Adding an app

acct knows an app from a short TOML file. The built-in ones are in [`defs/`](../defs). Put your own
in `%APPDATA%\acct\apps\` (the desktop app's … menu opens the folder); a file with the same `id`
as a built-in one replaces it.

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
| `credential` | `pattern` (Credential Manager target, `*`/`?` allowed) | encrypted copy |
| `registry` | `key` under HKCU | encrypted copy |
| `json_key` | `path`, `keys = ["/oauthAccount"]` (JSON pointers) | only those keys; the rest of the file is left alone |

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

strategy = "env"            # one config folder per profile, picked by a user variable
[env]
var = "TOOL_CONFIG_DIR"     # only programs started after the switch see it
```

## `acct learn`

If you don't know where an app keeps its sign-in:

```
acct learn slack
```

acct looks for folders named like the app in AppData, your home folder and Microsoft Store app
folders (`%LOCALAPPDATA%\Packages`), and for registry keys named like it under `HKCU\Software`.
It asks you to close the app, sign in, and close it again (and checks that you did), then shows
what changed: files, Electron session folders, registry keys and Credential Manager entries.
Caches, logs and extension folders are ignored. It writes a draft definition for you to check before
saving. Remove anything that is not sign-in data. A `Cache` folder in a profile only costs space;
a settings file means settings switch with the account.
