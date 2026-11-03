use std::io::{BufRead, IsTerminal, Write};
use std::process::ExitCode;

use acswi_core::engine::{Acswi, Actor, AddStep, AppView, SwitchOptions};
use acswi_core::{learn, Code, Error, Strategy};
use anstream::{eprintln, println};
use anstyle::{AnsiColor, Style};
use clap::{Parser, Subcommand};
use serde::Serialize;

const BOLD: Style = Style::new().bold();
const DIM: Style = Style::new().dimmed();
const GREEN: Style = AnsiColor::Green.on_default();
const YELLOW: Style = AnsiColor::Yellow.on_default();
const RED: Style = AnsiColor::Red.on_default();

/// Switch which account an app uses, without signing in again.
///
///   acswi                  pick an account from a list
///   acswi discord          Discord's accounts; pick one
///   acswi discord work     switch Discord to "work"
///   acswi add discord      sign in another Discord account and save it
#[derive(Parser)]
#[command(name = "acswi", version, verbatim_doc_comment, disable_help_subcommand = true)]
struct Cli {
    /// Print machine-readable JSON (for scripts and AI agents)
    #[arg(long, global = true)]
    json: bool,

    /// Who is making the change, for the audit log
    #[arg(long, global = true, value_enum, hide = true, env = "ACSWI_ACTOR")]
    actor: Option<ActorArg>,

    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum ActorArg {
    Cli,
    Ui,
    Agent,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show apps, their accounts and which one is active
    #[command(visible_alias = "ls")]
    List {
        /// Only this app
        app: Option<String>,
        /// Include apps that are not installed and have no accounts
        #[arg(long)]
        all: bool,
    },
    /// Switch an app to another saved account
    #[command(visible_alias = "switch")]
    Use {
        app: String,
        profile: String,
        /// Close the app if it is open (and reopen it afterwards)
        #[arg(long)]
        close: bool,
        /// Do not reopen the app after closing it
        #[arg(long)]
        no_relaunch: bool,
        /// Save the current sign-in even if it does not match the active profile
        #[arg(long)]
        force: bool,
    },
    /// Sign in a new account and save it as a profile
    Add {
        app: String,
        /// Name for the new account (asked for if left out)
        profile: Option<String>,
        /// Close the app if it is open
        #[arg(long)]
        close: bool,
        /// Finish an add that was started earlier (after signing in)
        #[arg(long, conflicts_with = "cancel")]
        finish: bool,
        /// Cancel an unfinished add and go back to the previous account
        #[arg(long)]
        cancel: bool,
        /// Extra options for the app's sign-in command, after `--`
        /// (e.g. `acswi add az work -- --tenant contoso.onmicrosoft.com`)
        #[arg(last = true)]
        login_args: Vec<String>,
    },
    /// Save the account the app is signed in to now (or refresh the active profile's copy)
    Save { app: String, profile: Option<String> },
    /// Rename a profile (or name an account the app already knows)
    Rename { app: String, old: String, new: String },
    /// Delete a saved profile
    #[command(visible_alias = "rm")]
    Remove {
        app: String,
        profile: String,
        /// Do not ask for confirmation
        #[arg(long, short)]
        yes: bool,
    },
    /// Show who each app is signed in as right now
    Status { app: Option<String> },
    /// List the apps acswi knows how to switch
    Apps,
    /// Teach acswi a new app by watching what changes when you sign in
    Learn {
        /// Short id for the new app, e.g. "slack"
        id: String,
        /// Display name (defaults to the id)
        #[arg(long)]
        name: Option<String>,
        /// Text to look for in folder names (defaults to the id)
        #[arg(long)]
        hint: Option<String>,
    },
    /// Check acswi's setup and finish any interrupted switch
    Doctor,
    /// `acswi <app>` lists an app, `acswi <app> <profile>` switches it
    #[command(external_subcommand)]
    Shortcut(Vec<String>),
}

#[derive(Serialize)]
struct Ok<T: Serialize> {
    ok: bool,
    data: T,
}

#[derive(Serialize)]
struct Err<'a> {
    ok: bool,
    error: &'a Error,
}

struct Ctx {
    acswi: Acswi,
    json: bool,
    actor: Actor,
}

impl Ctx {
    fn interactive(&self) -> bool {
        !self.json && std::io::stdin().is_terminal()
    }

    fn require_human(&self, what: &str) -> Result<(), Error> {
        if self.interactive() {
            Ok(())
        } else {
            Err(Error::new(
                Code::NeedsHuman,
                format!("{what} needs a person at the keyboard. Run it in a terminal, or use the acswi app."),
            ))
        }
    }

    fn emit<T: Serialize>(&self, data: T, human: impl FnOnce(&T)) {
        if self.json {
            println!("{}", serde_json::to_string_pretty(&Ok { ok: true, data }).unwrap());
        } else {
            human(&data);
        }
    }

    fn opts(&self, close: bool, relaunch: bool, force: bool) -> SwitchOptions {
        SwitchOptions { close, relaunch, force, actor: self.actor }
    }
}

fn main() -> ExitCode {
    let mut cli = Cli::parse();
    // Flags typed after a shortcut (`acswi gh work --json`) arrive as plain arguments.
    let mut shortcut_close = false;
    if let Some(Cmd::Shortcut(args)) = &mut cli.command {
        args.retain(|a| match a.as_str() {
            "--json" => {
                cli.json = true;
                false
            }
            "--close" => {
                shortcut_close = true;
                false
            }
            _ => true,
        });
    }
    let json = cli.json;
    let actor = match cli.actor {
        Some(ActorArg::Ui) => Actor::Ui,
        Some(ActorArg::Agent) => Actor::Agent,
        Some(ActorArg::Cli) => Actor::Cli,
        // Without a terminal, assume a script or agent is calling.
        None if !std::io::stdin().is_terminal() => Actor::Agent,
        None => Actor::Cli,
    };
    let result = Acswi::open().and_then(|acswi| {
        let ctx = Ctx { acswi, json, actor };
        run(&ctx, cli.command, shortcut_close)
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&Err { ok: false, error: &e }).unwrap());
            } else {
                eprintln!("{RED}error{RED:#}: {}", e.message);
            }
            ExitCode::from(match e.code {
                Code::AppRunning => 3,
                Code::NeedsHuman => 4,
                Code::UnknownApp | Code::UnknownProfile => 5,
                Code::UnsavedSession | Code::IdentityMismatch => 6,
                _ => 1,
            })
        }
    }
}

fn run(ctx: &Ctx, cmd: Option<Cmd>, shortcut_close: bool) -> Result<(), Error> {
    match cmd {
        None if ctx.interactive() => menu(ctx, None),
        None => list(ctx, None, false),
        Some(Cmd::List { app, all }) => list(ctx, app.as_deref(), all),
        Some(Cmd::Use { app, profile, close, no_relaunch, force }) => use_profile(ctx, &app, &profile, close, !no_relaunch, force),
        Some(Cmd::Add { app, profile, close, finish, cancel, login_args }) => {
            add(ctx, &app, profile.as_deref(), close, finish, cancel, &login_args)
        }
        Some(Cmd::Save { app, profile }) => save(ctx, &app, profile.as_deref()),
        Some(Cmd::Rename { app, old, new }) => {
            ctx.acswi.rename(&app, &old, &new)?;
            ctx.emit(serde_json::json!({ "app": app, "from": old, "to": new }), |_| {
                println!("Renamed {BOLD}{old}{BOLD:#} to {BOLD}{new}{BOLD:#}.")
            });
            Ok(())
        }
        Some(Cmd::Remove { app, profile, yes }) => {
            if !yes {
                ctx.require_human("Removing a profile without --yes")?;
                if !confirm(&format!("Delete the saved {app} profile {profile:?}? Its sign-in cannot be recovered. [y/N] ")) {
                    println!("Nothing was deleted.");
                    return Ok(());
                }
            }
            ctx.acswi.remove(&app, &profile, ctx.actor)?;
            ctx.emit(serde_json::json!({ "app": app, "removed": profile }), |_| println!("Deleted {profile}."));
            Ok(())
        }
        Some(Cmd::Status { app }) => status(ctx, app.as_deref()),
        Some(Cmd::Apps) => apps(ctx),
        Some(Cmd::Learn { id, name, hint }) => learn_app(ctx, &id, name, hint),
        Some(Cmd::Doctor) => doctor(ctx),
        Some(Cmd::Shortcut(args)) => match args.as_slice() {
            [app] if ctx.interactive() => menu(ctx, Some(app)),
            [app] => list(ctx, Some(app), true),
            [app, profile] => use_profile(ctx, app, profile, shortcut_close, true, false),
            _ => Err(Error::new(Code::InvalidName, "Usage: acswi <app> [profile]. Run `acswi --help` for all commands.")),
        },
    }
}

// ------------------------------------------------------------------ list / status

fn list(ctx: &Ctx, app: Option<&str>, all: bool) -> Result<(), Error> {
    let views = ctx.acswi.overview(app)?;
    let shown: Vec<AppView> = views
        .into_iter()
        .filter(|v| all || app.is_some() || v.installed || !v.profiles.is_empty())
        .collect();
    ctx.emit(shown, |views| {
        if views.is_empty() {
            println!("No apps found. Run `acswi apps` to see what acswi can switch.");
        }
        for (i, v) in views.iter().enumerate() {
            if i > 0 {
                println!();
            }
            print_app(v, None);
        }
    });
    Ok(())
}

/// Prints one app and its accounts; with `first`, accounts are numbered from it for the menu.
fn print_app(v: &AppView, first: Option<usize>) {
    let mut tags = Vec::new();
    if !v.installed {
        tags.push(format!("{YELLOW}not installed{YELLOW:#}"));
    }
    if v.running {
        tags.push("open".to_string());
    }
    let tags = if tags.is_empty() { String::new() } else { format!("  {DIM}·{DIM:#} {}", tags.join(" · ")) };
    println!("{BOLD}{}{BOLD:#}  {DIM}{}{DIM:#}{tags}", v.name, v.id);
    if let Some(w) = &v.warning {
        println!("  {YELLOW}!{YELLOW:#} {w}");
    }
    if let Some(p) = &v.pending_add {
        println!("  {YELLOW}!{YELLOW:#} adding {p:?} is not finished: acswi add {} --finish (or --cancel)", v.id);
    }
    if v.profiles.is_empty() {
        let hint = if first.is_some() { "type a to add one".to_string() } else { format!("acswi add {}", v.id) };
        println!("  {DIM}no accounts yet · {hint}{DIM:#}");
        return;
    }
    let width = v.profiles.iter().map(|p| p.name.len()).max().unwrap_or(0);
    for (i, p) in v.profiles.iter().enumerate() {
        let num = first.map(|f| format!("{DIM}{:>2}{DIM:#} ", f + i)).unwrap_or_default();
        let mark = if p.active { format!("{GREEN}●{GREEN:#}") } else { " ".into() };
        let ident = match &p.identity {
            Some(i) if i != &p.name => i.as_str(),
            _ => "",
        };
        println!("  {num}{mark} {:<width$}  {DIM}{ident}{DIM:#}", p.name);
    }
}

/// `acswi` or `acswi <app>` in a terminal: numbered accounts, then switch or add by typing.
fn menu(ctx: &Ctx, app: Option<&str>) -> Result<(), Error> {
    let views: Vec<AppView> = ctx
        .acswi
        .overview(app)?
        .into_iter()
        .filter(|v| app.is_some() || v.installed || !v.profiles.is_empty())
        .collect();
    if views.is_empty() {
        println!("No apps found. Run `acswi apps` to see what acswi can switch.");
        return Ok(());
    }
    let mut choices: Vec<(&str, &str)> = Vec::new();
    for (i, v) in views.iter().enumerate() {
        if i > 0 {
            println!();
        }
        print_app(v, Some(choices.len() + 1));
        choices.extend(v.profiles.iter().map(|p| (v.id.as_str(), p.name.as_str())));
    }
    println!();
    let prompt = match choices.len() {
        0 => "Type a to add an account, or press Enter to quit: ".to_string(),
        1 => "Type 1 to switch, a to add an account, or press Enter to quit: ".to_string(),
        n => format!("Type 1-{n} to switch, a to add an account, or press Enter to quit: "),
    };
    loop {
        let answer = ask(&prompt);
        if answer.is_empty() {
            return Ok(());
        }
        if answer.eq_ignore_ascii_case("a") {
            let target = match views.as_slice() {
                [only] => only.id.clone(),
                _ => match pick_app(&views) {
                    Some(id) => id,
                    None => return Ok(()),
                },
            };
            return add(ctx, &target, None, false, false, false, &[]);
        }
        match answer.parse::<usize>().ok().and_then(|n| n.checked_sub(1)).and_then(|i| choices.get(i)) {
            Some((app, profile)) => return use_profile(ctx, app, profile, false, true, false),
            None => println!("{DIM}Type one of the numbers above, a, or press Enter.{DIM:#}"),
        }
    }
}

fn pick_app(views: &[AppView]) -> Option<String> {
    let installed: Vec<&AppView> = views.iter().filter(|v| v.installed).collect();
    for (i, v) in installed.iter().enumerate() {
        println!("  {DIM}{:>2}{DIM:#} {}", i + 1, v.name);
    }
    loop {
        let answer = ask("Add an account to which app? (number, or Enter to go back): ");
        if answer.is_empty() {
            return None;
        }
        if let Some(v) = answer.parse::<usize>().ok().and_then(|n| n.checked_sub(1)).and_then(|i| installed.get(i)) {
            return Some(v.id.clone());
        }
    }
}

fn status(ctx: &Ctx, app: Option<&str>) -> Result<(), Error> {
    let ids: Vec<String> = match app {
        Some(a) => vec![ctx.acswi.def(a)?.id.clone()],
        None => ctx
            .acswi
            .overview(None)?
            .into_iter()
            .filter(|v| v.installed && (!v.profiles.is_empty() || v.strategy == Strategy::Native))
            .map(|v| v.id)
            .collect(),
    };
    let mut out = Vec::new();
    for id in ids {
        out.push(ctx.acswi.status(&id)?);
    }
    ctx.emit(out, |rows| {
        for s in rows {
            let who = s.live_identity.as_deref().unwrap_or(if s.signed_in { "signed in" } else { "signed out" });
            let prof = s.active.as_deref().map(|a| format!("  {DIM}profile{DIM:#} {a}")).unwrap_or_default();
            let warn = if s.in_sync {
                String::new()
            } else {
                format!("  {YELLOW}expected {}{YELLOW:#}", s.expected_identity.as_deref().unwrap_or("?"))
            };
            let open = if s.running.is_empty() { String::new() } else { format!("  {DIM}(open){DIM:#}") };
            println!("{BOLD}{:<16}{BOLD:#} {who}{prof}{warn}{open}", s.app);
        }
    });
    Ok(())
}

fn apps(ctx: &Ctx) -> Result<(), Error> {
    #[derive(Serialize)]
    struct Row {
        id: String,
        name: String,
        strategy: Strategy,
        installed: bool,
        source: String,
    }
    let rows: Vec<Row> = ctx
        .acswi
        .defs
        .apps
        .values()
        .map(|d| Row {
            id: d.id.clone(),
            name: d.name.clone(),
            strategy: d.strategy,
            installed: d.installed(),
            source: d.source.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "built-in".into()),
        })
        .collect();
    let errors = ctx.acswi.defs.errors.clone();
    ctx.emit(serde_json::json!({ "apps": rows, "errors": errors }), |_| {
        for r in &rows {
            let how = match r.strategy {
                Strategy::Native => "uses the app's own account list",
                Strategy::Swap => "swaps saved sign-ins",
                Strategy::Env => "one folder per account",
            };
            let inst = if r.installed { String::new() } else { format!("  {YELLOW}not installed{YELLOW:#}") };
            println!("{BOLD}{:<16}{BOLD:#} {:<16} {DIM}{how} · {}{DIM:#}{inst}", r.id, r.name, r.source);
        }
        for e in &errors {
            println!("{RED}!{RED:#} {}", e.message);
        }
        println!(
            "\n{DIM}Add your own: acswi learn <id>, or put a .toml file in {}{DIM:#}",
            ctx.acswi.paths.user_defs.display()
        );
    });
    Ok(())
}

// ------------------------------------------------------------------ use / save / add

fn use_profile(ctx: &Ctx, app: &str, profile: &str, close: bool, relaunch: bool, force: bool) -> Result<(), Error> {
    let mut close = close;
    let report = loop {
        match ctx.acswi.switch(app, profile, &ctx.opts(close, relaunch, force)) {
            Ok(r) => break r,
            Err(e) if e.code == Code::AppRunning && !close && ctx.interactive() => {
                let name = ctx.acswi.def(app)?.name.clone();
                if !confirm(&format!("{name} is open. Close it and switch? [Y/n] ")) {
                    return Err(e);
                }
                close = true;
            }
            Err(e) if e.code == Code::UnsavedSession && ctx.interactive() => {
                save_current_interactively(ctx, app)?;
            }
            Err(e) => return Err(e),
        }
    };
    ctx.emit(&report, |r| {
        let name = ctx.acswi.def(&r.app).map(|d| d.name.clone()).unwrap_or_default();
        if !r.changed {
            println!("{name} is already using {BOLD}{}{BOLD:#}.", r.to);
            return;
        }
        let who = r.identity.as_deref().filter(|i| *i != r.to).map(|i| format!(" {DIM}({i}){DIM:#}")).unwrap_or_default();
        println!("{GREEN}✓{GREEN:#} {name} now uses {BOLD}{}{BOLD:#}{who}", r.to);
        if r.relaunched {
            println!("  {DIM}{name} was reopened.{DIM:#}");
        }
    });
    Ok(())
}

fn save_current_interactively(ctx: &Ctx, app: &str) -> Result<(), Error> {
    let def = ctx.acswi.def(app)?.clone();
    let who = def.identity.as_ref().and_then(|s| acswi_core::native::identity(s).ok().flatten());
    let suggestion = who.as_deref().map(acswi_core::paths::slug).unwrap_or_else(|| "main".into());
    println!(
        "{} is signed in{} to an account acswi hasn't saved yet. acswi saves it first, so you can switch back to it.",
        def.name,
        who.map(|w| format!(" as {w}")).unwrap_or_default()
    );
    let name = ask(&format!("Name for the account signed in now [{suggestion}]: "));
    let name = if name.is_empty() { suggestion } else { name };
    ctx.acswi.save(&def.id, &name, ctx.actor)?;
    println!("{GREEN}✓{GREEN:#} Saved as {BOLD}{name}{BOLD:#}.");
    Ok(())
}

fn save(ctx: &Ctx, app: &str, profile: Option<&str>) -> Result<(), Error> {
    match profile {
        Some(p) => {
            ctx.acswi.save(app, p, ctx.actor)?;
            ctx.emit(serde_json::json!({ "app": app, "saved": p }), |_| {
                println!("{GREEN}✓{GREEN:#} Saved the current {app} account as {BOLD}{p}{BOLD:#}.")
            });
        }
        None => {
            let refreshed = ctx.acswi.refresh(app)?;
            ctx.emit(serde_json::json!({ "app": app, "refreshed": refreshed }), |r| match r["refreshed"].as_str() {
                Some(p) => println!("{GREEN}✓{GREEN:#} Updated the saved copy of {BOLD}{p}{BOLD:#}."),
                None => println!("Nothing to update. Give the current account a name: acswi save {app} <name>"),
            });
        }
    }
    Ok(())
}

fn add(
    ctx: &Ctx,
    app: &str,
    profile: Option<&str>,
    close: bool,
    finish: bool,
    cancel: bool,
    login_args: &[String],
) -> Result<(), Error> {
    let opts = ctx.opts(close, true, false);
    if cancel {
        let back = ctx.acswi.add_cancel(app, &opts)?;
        ctx.emit(serde_json::json!({ "app": app, "cancelled": true, "active": back }), |_| {
            println!("Cancelled.{}", back.as_deref().map(|b| format!(" Back to {b}.")).unwrap_or_default())
        });
        return Ok(());
    }
    if finish {
        let p = ctx.acswi.add_finish(app, &opts)?;
        ctx.emit(serde_json::json!({ "app": app, "added": p }), |_| {
            println!("{GREEN}✓{GREEN:#} Added {BOLD}{p}{BOLD:#}; it is now active.")
        });
        return Ok(());
    }
    ctx.require_human("Signing in a new account")?;
    let def = ctx.acswi.def(app)?.clone();
    // Save the account that is signed in now before asking about the new one, so the two
    // questions come in the order they happen.
    if ctx.acswi.overview(Some(app))?.first().is_some_and(|v| v.unsaved) {
        save_current_interactively(ctx, app)?;
        println!("Next, acswi signs {} out so you can sign in another account.", def.name);
    }
    let asked;
    let profile = match profile {
        Some(p) => p,
        None => {
            asked = loop {
                let name = ask(&format!("Name for the new {} account you'll sign in (e.g. work): ", def.name));
                if acswi_core::paths::valid_name(&name) {
                    break name;
                }
                println!("{DIM}Use letters, digits, dots, dashes or underscores.{DIM:#}");
            };
            asked.as_str()
        }
    };
    let mut opts = opts;
    let step = loop {
        match ctx.acswi.add_begin(app, profile, &opts) {
            Ok(s) => break s,
            Err(e) if e.code == Code::AppRunning && !opts.close => {
                if !confirm(&format!("{} is open. Close it to sign in a new account? [Y/n] ", def.name)) {
                    return Err(e);
                }
                opts.close = true;
            }
            Err(e) if e.code == Code::UnsavedSession => save_current_interactively(ctx, app)?,
            Err(e) => return Err(e),
        }
    };
    match step {
        AddStep::Done { profile } => println!("{GREEN}✓{GREEN:#} Added {profile}."),
        AddStep::RunLogin { mut command, env } => {
            command.extend(login_args.iter().cloned());
            println!("{DIM}Signing in with: {}{DIM:#}", command.join(" "));
            let ok = acswi_core::process::run_interactive(&command, &env)?;
            if !ok {
                let back = ctx.acswi.add_cancel(app, &opts)?;
                return Err(Error::new(
                    Code::CommandFailed,
                    format!("Sign-in did not finish.{}", back.map(|b| format!(" Back to {b}.")).unwrap_or_default()),
                ));
            }
            finish_add(ctx, app, profile, &opts)?;
        }
        AddStep::SignInApp => {
            println!("{} is opening with no account signed in.", def.name);
            let answer = ask("Sign in there. When you see your account, come back and press Enter (or type c to cancel): ");
            if answer.eq_ignore_ascii_case("c") {
                let back = ctx.acswi.add_cancel(app, &opts)?;
                println!("Cancelled.{}", back.map(|b| format!(" Back to {b}.")).unwrap_or_default());
                return Ok(());
            }
            finish_add(ctx, app, profile, &opts)?;
        }
    }
    Ok(())
}

/// Finishes an add, accepting that it may already have been finished elsewhere (the desktop app).
fn finish_add(ctx: &Ctx, app: &str, profile: &str, opts: &SwitchOptions) -> Result<(), Error> {
    match ctx.acswi.add_finish(app, opts) {
        Ok(p) => println!("{GREEN}✓{GREEN:#} Added {BOLD}{p}{BOLD:#}; it is now active."),
        Err(e) if e.code == Code::UnknownProfile => {
            let added = ctx.acswi.overview(Some(app))?.iter().any(|v| v.profiles.iter().any(|p| p.name == profile));
            if !added {
                return Err(e);
            }
            println!("{GREEN}✓{GREEN:#} {BOLD}{profile}{BOLD:#} was already added (finished in the desktop app).");
        }
        Err(e) => return Err(e),
    }
    Ok(())
}

// ------------------------------------------------------------------ learn / doctor

fn learn_app(ctx: &Ctx, id: &str, name: Option<String>, hint: Option<String>) -> Result<(), Error> {
    ctx.require_human("Learning a new app")?;
    if !acswi_core::paths::valid_name(id) {
        return Err(Error::new(Code::InvalidName, format!("{id:?} is not a valid id")));
    }
    let name = name.unwrap_or_else(|| id.to_string());
    let hint = hint.unwrap_or_else(|| id.to_string());
    let roots = learn::candidate_roots(&hint);
    let registry = learn::registry_roots(&hint);
    if roots.is_empty() && registry.is_empty() {
        println!("No folders or registry keys named like {hint:?}. Try --hint with part of the app's folder name.");
        println!("acswi will still watch Credential Manager.");
    } else {
        println!("Watching:");
        for r in &roots {
            println!("  {DIM}{}{DIM:#}", r.display());
        }
        for k in &registry {
            println!("  {DIM}HKCU\\{k}{DIM:#}");
        }
    }
    // Each step checks that the app really is closed or open, so pressing Enter early
    // can't produce an empty comparison.
    let running = || learn::app_processes(&roots, &hint);
    let wait_closed = |step: &str| loop {
        println!("\n{BOLD}{step}{BOLD:#} Close {name} completely (also from the system tray).");
        ask("   Press Enter when it is closed. ");
        let (open, _) = running();
        if open.is_empty() {
            break;
        }
        println!("   {YELLOW}{name} is still running{YELLOW:#} ({}).", open.join(", "));
    };
    wait_closed("1.");
    let before = learn::take(&roots, &registry)?;
    let (processes, launch) = loop {
        println!("\n{BOLD}2.{BOLD:#} Open {name} and sign in. If it is already signed in, sign out and sign in again.");
        let answer = ask("   Press Enter when you see your account, with the app still open (or type c to continue anyway): ");
        let found = running();
        if !found.0.is_empty() || answer.eq_ignore_ascii_case("c") {
            break found;
        }
        println!("   {YELLOW}{name} doesn't seem to be running.{YELLOW:#} If it is, type c to continue.");
    };
    wait_closed("3.");
    let after = learn::take(&roots, &registry)?;
    let mut report = learn::compare(&before, &after);
    report.processes = processes;
    report.launch = launch;
    if report.findings.is_empty() {
        println!(
            "\n{YELLOW}Nothing that looks like sign-in data changed{YELLOW:#} ({} changed files, all caches or logs).",
            report.noise_files
        );
        println!("The app may keep its sign-in somewhere else (try --hint), or in a place acswi cannot swap (see docs/how-it-works.md).");
        return Ok(());
    }
    let text = learn::draft(id, &name, &report);
    println!("\n{BOLD}Draft definition{BOLD:#}\n{DIM}──────────────{DIM:#}\n{text}");
    let path = ctx.acswi.paths.user_defs.join(format!("{id}.toml"));
    if confirm(&format!("Save it to {}? [Y/n] ", path.display())) {
        std::fs::create_dir_all(&ctx.acswi.paths.user_defs).map_err(|e| Error::io("apps folder", e))?;
        std::fs::write(&path, text).map_err(|e| Error::io(path.display(), e))?;
        println!("{GREEN}✓{GREEN:#} Saved. Next: open {name}, sign in, then run `acswi save {id} <name>`.");
        println!("  {DIM}Edit the file to remove anything that is not sign-in data.{DIM:#}");
    }
    Ok(())
}

fn doctor(ctx: &Ctx) -> Result<(), Error> {
    #[derive(Serialize)]
    struct Check {
        name: String,
        ok: bool,
        detail: String,
    }
    let mut checks = Vec::new();
    let recovered = ctx.acswi.recover()?;
    checks.push(Check {
        name: "interrupted switch".into(),
        ok: true,
        detail: recovered.unwrap_or_else(|| "none".into()),
    });
    let sealer = acswi_core::seal::Sealer::new(&ctx.acswi.paths);
    let crypto = sealer.seal(b"acswi", b"doctor").and_then(|c| sealer.open(&c, b"doctor"));
    checks.push(Check {
        name: "encryption".into(),
        ok: crypto.is_ok(),
        detail: crypto.map(|_| format!("works ({})", sealer.describe())).unwrap_or_else(|e| e.message),
    });
    checks.push(Check {
        name: "data folder".into(),
        ok: true,
        detail: ctx.acswi.paths.root.display().to_string(),
    });
    checks.push(Check {
        name: "your app definitions".into(),
        ok: ctx.acswi.defs.errors.is_empty(),
        detail: if ctx.acswi.defs.errors.is_empty() {
            ctx.acswi.paths.user_defs.display().to_string()
        } else {
            ctx.acswi.defs.errors.iter().map(|e| e.message.clone()).collect::<Vec<_>>().join("; ")
        },
    });
    for v in ctx.acswi.overview(None)? {
        if !v.installed && v.profiles.is_empty() {
            continue;
        }
        let ok = v.warning.is_none() && v.pending_add.is_none();
        let detail = v
            .warning
            .clone()
            .or(v.pending_add.as_ref().map(|p| format!("unfinished add of {p:?}")))
            .unwrap_or_else(|| format!("{} account(s)", v.profiles.len()));
        checks.push(Check { name: v.name, ok, detail });
    }
    ctx.emit(&checks, |rows| {
        for c in rows.iter() {
            let mark = if c.ok { format!("{GREEN}✓{GREEN:#}") } else { format!("{RED}✗{RED:#}") };
            println!("{mark} {:<22} {DIM}{}{DIM:#}", c.name, c.detail);
        }
    });
    Ok(())
}

// ------------------------------------------------------------------ prompts

fn ask(prompt: &str) -> String {
    print!("{prompt}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    line.trim().to_string()
}

fn confirm(prompt: &str) -> bool {
    let a = ask(prompt).to_ascii_lowercase();
    let default_yes = prompt.contains("[Y/n]");
    if a.is_empty() { default_yes } else { a.starts_with('y') }
}
