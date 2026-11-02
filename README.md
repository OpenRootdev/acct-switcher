<p align="center">
  <img src="https://github.com/Ker102/acct-switcher/raw/assets/logo.svg" width="104" alt="acct logo">
</p>

<h1 align="center">acct</h1>

<p align="center">
  Sign in to each of your accounts once, then switch any app between them in one click.<br>
  <sub>For people with several accounts in the same CLI or desktop app on Windows: work and personal, clients, test accounts.</sub>
</p>

<p align="center">
  <img src="https://github.com/Ker102/acct-switcher/raw/assets/preview.gif" width="760" alt="The acct window switching Codex from a personal to a work account with one click">
</p>

<p align="center">
  <a href="https://github.com/Ker102/acct-switcher/releases/latest/download/acct-windows-x64.zip"><b>Download for Windows</b></a> &nbsp;·&nbsp; <a href="#get-started">Get started</a> &nbsp;·&nbsp; <a href="docs/usage.md">Docs</a>
</p>

<p align="center">
  <sub>Works with</sub><br>
  <picture><source media="(prefers-color-scheme: dark)" srcset="https://github.com/Ker102/acct-switcher/raw/assets/brands/github-dark.svg"><img src="https://github.com/Ker102/acct-switcher/raw/assets/brands/github.svg" height="16" alt=""></picture>&nbsp;GitHub CLI &amp; Desktop &nbsp;·&nbsp; <img src="https://github.com/Ker102/acct-switcher/raw/assets/brands/claude.svg" height="16" alt="">&nbsp;Claude Code &nbsp;·&nbsp; <picture><source media="(prefers-color-scheme: dark)" srcset="https://github.com/Ker102/acct-switcher/raw/assets/brands/openai-dark.svg"><img src="https://github.com/Ker102/acct-switcher/raw/assets/brands/openai.svg" height="16" alt=""></picture>&nbsp;Codex &nbsp;·&nbsp; <img src="https://github.com/Ker102/acct-switcher/raw/assets/brands/discord.svg" height="16" alt="">&nbsp;Discord
</p>

## What it does

acct keeps a saved sign-in for every account you use in an app. Pick one in the window, the tray or the terminal, and the app is signed in to it, with no password or browser sign-in.

- Works with command-line tools and desktop apps. GitHub CLI, Azure CLI, Claude Code, Codex, Cursor, GitHub Desktop and Discord are built in; any other app can be added with a short file.
- Saved sign-ins stay on your PC, encrypted for your Windows user.
- Scripts and AI agents can switch accounts by name, and never see a password or token.

> [!NOTE]
> Early version. Switching GitHub CLI, Azure CLI and Discord is tested on real accounts; the other built-in apps so far only in automated tests.

## How it's different

Most account switchers cover one app, usually Codex or Claude Code. [TcNo Account Switcher](https://github.com/TCNOco/TcNo-Acc-Switcher) covers many, mostly game launchers like Steam, Battle.net and Epic, and is the better pick for those. acct is made for the tools you work with:

- One place for command-line tools and desktop apps.
- When an app already keeps several accounts (GitHub CLI, Azure CLI), acct uses that instead of copying files.
- `acct learn` watches an app while you sign in and writes its definition for you.

## Get started

1. Download <a href="https://github.com/Ker102/acct-switcher/releases/latest/download/acct-windows-x64.zip">acct-windows-x64.zip</a> and unzip it into a folder on your <code>PATH</code>.
2. Open <code>acct-desktop</code> for the window and tray menu, or type <code>acct</code> in a terminal.
3. Add a second account to an app. acct saves the one you're signed in to now, then lets you sign in the new one. After that, <code>acct</code> lists your accounts and you type a number to switch:

```bash
acct add codex
acct
```

## What you need

- Windows 11. Windows 10 with the WebView2 runtime should work but is not tested.
- The apps you want to switch, each signed in once through acct.
- To build it yourself instead: Rust and the Visual Studio C++ build tools (<a href="docs/usage.md#build-from-source">how</a>).

## What's inside

- [`apps/acct-desktop/`](apps/acct-desktop/) · The window and tray menu.
- [`crates/acct-cli/`](crates/acct-cli/) · The <code>acct</code> command.
- [`crates/acct-core/`](crates/acct-core/) · Saves, encrypts and restores sign-ins; shared by both.
- [`defs/`](defs/) · The built-in apps, one short file each.
- [`docs/`](docs/) · Commands, adding an app, and how switching works.

<sub>Preview images live on the <a href="https://github.com/Ker102/acct-switcher/tree/assets"><code>assets</code></a> branch, so <code>main</code> holds only the tool.</sub>

---

<sub>MIT licence · <a href="https://github.com/Ker102">Ker102</a></sub>
