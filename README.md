<p align="center">
  <img src="https://github.com/OpenRootdev/acswi/raw/assets/logo.svg" width="104" alt="Account Switcher logo">
</p>

<h1 align="center">Account Switcher</h1>

<p align="center">
  Sign in to each of your accounts once, then switch any app between them in one click.<br>
  <sub>For people with several accounts in the same CLI or desktop app on Windows: work and personal, clients, test accounts.</sub>
</p>

<p align="center">
  <img src="https://github.com/OpenRootdev/acswi/raw/assets/preview.gif" width="760" alt="Account Switcher switching accounts in its window and with the acswi command in a terminal">
</p>

<p align="center">
  <a href="https://github.com/OpenRootdev/acswi/releases/latest/download/acswi-windows-x64.zip"><b>Download for Windows</b></a> &nbsp;·&nbsp; <a href="#get-started">Get started</a> &nbsp;·&nbsp; <a href="docs/usage.md">Docs</a>
</p>

<p align="center">
  <sub>Works with</sub><br>
  <picture><source media="(prefers-color-scheme: dark)" srcset="https://github.com/OpenRootdev/acswi/raw/assets/brands/github-dark.svg"><img src="https://github.com/OpenRootdev/acswi/raw/assets/brands/github.svg" height="16" alt=""></picture>&nbsp;GitHub CLI &amp; Desktop &nbsp;·&nbsp; <img src="https://github.com/OpenRootdev/acswi/raw/assets/brands/claude.svg" height="16" alt="">&nbsp;Claude Code &nbsp;·&nbsp; <picture><source media="(prefers-color-scheme: dark)" srcset="https://github.com/OpenRootdev/acswi/raw/assets/brands/openai-dark.svg"><img src="https://github.com/OpenRootdev/acswi/raw/assets/brands/openai.svg" height="16" alt=""></picture>&nbsp;Codex &nbsp;·&nbsp; <img src="https://github.com/OpenRootdev/acswi/raw/assets/brands/discord.svg" height="16" alt="">&nbsp;Discord
</p>

<p align="center">
  <a href="https://toolindex.org/tools/acswi/"><img src="https://toolindex.org/badges/acswi.svg" alt="Listed in ToolIndex"></a>
</p>

## What it does

Account Switcher (<code>acswi</code>) keeps a saved sign-in for every account you use in an app. Pick one in the window, the tray or the terminal, and the app is signed in to it, with no password or browser sign-in.

- Works with command-line tools and desktop apps. GitHub CLI, Azure CLI, Claude Code, Codex, Cursor, GitHub Desktop and Discord are built in; any other app can be added with a short file.
- Saved sign-ins stay on your PC, encrypted for your Windows user.
- Scripts and AI agents can switch accounts by name, and never see a password or token.

> [!NOTE]
> Early version. Switching GitHub CLI, Azure CLI, Discord and Cursor is tested on real accounts; the other built-in apps so far only in automated tests.

## How it's different

Most account switchers cover one app, usually Codex or Claude Code. [TcNo Account Switcher](https://github.com/TCNOco/TcNo-Acc-Switcher) covers many, mostly game launchers like Steam, Battle.net and Epic, and is the better pick for those. acswi is made for the tools you work with:

- One place for command-line tools and desktop apps.
- When an app already keeps several accounts (GitHub CLI, Azure CLI), acswi uses that instead of copying files.
- `acswi learn` watches an app while you sign in and writes its definition for you.

## Get started

1. Download <a href="https://github.com/OpenRootdev/acswi/releases/latest/download/acswi-windows-x64.zip">acswi-windows-x64.zip</a> and unzip it into a folder on your <code>PATH</code>.
2. Open <code>acswi-desktop</code> for the window and tray menu, or type <code>acswi</code> in a terminal.
3. Add a second account to an app. acswi saves the one you're signed in to now, then lets you sign in the new one. After that, <code>acswi</code> lists your accounts and you type a number to switch:

```bash
acswi add codex
acswi
```

## What you need

- Windows 11. Windows 10 with the WebView2 runtime should work but is not tested.
- The apps you want to switch, each signed in once through acswi.
- To build it yourself instead: Rust and the Visual Studio C++ build tools (<a href="docs/usage.md#build-from-source">how</a>).

## What's inside

- [`apps/acswi-desktop/`](apps/acswi-desktop/) · The window and tray menu.
- [`crates/acswi-cli/`](crates/acswi-cli/) · The <code>acswi</code> command.
- [`crates/acswi-core/`](crates/acswi-core/) · Saves, encrypts and restores sign-ins; shared by both.
- [`defs/`](defs/) · The built-in apps, one short file each.
- [`docs/`](docs/) · Commands, adding an app, and how switching works.

<sub>Preview images live on the <a href="https://github.com/OpenRootdev/acswi/tree/assets"><code>assets</code></a> branch, so <code>main</code> holds only the tool.</sub>

---

<sub>MIT licence · <a href="https://github.com/OpenRootdev">OpenRootdev</a></sub>
