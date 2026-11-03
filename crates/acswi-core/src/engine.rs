//! The public API used by the CLI and the desktop app.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::time::{Duration, Instant};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::def::{AppDef, Defs, Strategy};
use crate::error::{bail, Code, Error, Result};
use crate::items::{self, MoveStore};
use crate::native::{self, NativeAccount};
use crate::paths::{valid_name, Paths};
use crate::process::{self, Proc};
use crate::state::{write_atomic, PendingAdd, Profile, State};
use crate::vault::{Snapshot, Vault};

const CLOSE_GRACE: Duration = Duration::from_secs(6);

/// Who asked for a change; written to the audit log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Actor {
    #[default]
    Cli,
    Ui,
    Agent,
}

#[derive(Debug, Clone, Default)]
pub struct SwitchOptions {
    /// Close the app if it is running (otherwise fail with `app_running`).
    pub close: bool,
    /// Start the app again afterwards if it was running.
    pub relaunch: bool,
    /// Save even if the signed-in account does not match the profile.
    pub force: bool,
    pub actor: Actor,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProfileView {
    pub name: String,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
    /// Native accounts the app knows that have no acswi profile yet are listed with
    /// `saved: false`; they can still be switched to by name.
    pub saved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used: Option<chrono::DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppView {
    pub id: String,
    pub name: String,
    pub strategy: Strategy,
    pub installed: bool,
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
    pub profiles: Vec<ProfileView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_add: Option<String>,
    /// Signed in, but that account is not saved as a profile yet (swap apps).
    pub unsaved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SwitchReport {
    pub app: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    pub to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
    pub closed: bool,
    pub relaunched: bool,
    pub changed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusView {
    pub app: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
    /// Who the app says is signed in now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live_identity: Option<String>,
    /// Identity recorded for the active profile.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_identity: Option<String>,
    pub signed_in: bool,
    pub in_sync: bool,
    pub running: Vec<Proc>,
}

/// What the caller must do to finish `add`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "next", rename_all = "snake_case")]
pub enum AddStep {
    /// Run this sign-in command in a terminal, then call `add_finish`.
    RunLogin { command: Vec<String>, env: Vec<(String, String)> },
    /// The app was opened; sign in there, then call `add_finish`.
    SignInApp,
    /// Nothing to do: the profile was created from an existing account.
    Done { profile: String },
}

fn login_env(def: &AppDef, mut env: Vec<(String, String)>) -> Vec<(String, String)> {
    env.extend(def.login_env.iter().map(|(k, v)| (k.clone(), v.clone())));
    env
}

#[derive(Debug, Serialize, Deserialize)]
struct Journal {
    app: String,
    from: Option<String>,
    to: Option<String>,
    phase: Phase,
    started: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    /// Live state is being saved; undo = put moved folders back.
    Saving,
    /// Saved; live state is being replaced; redo = clear and restore again.
    Restoring,
}

pub struct Acswi {
    pub paths: Paths,
    pub defs: Defs,
}

struct Lock(#[allow(dead_code)] File);

impl Acswi {
    pub fn open() -> Result<Acswi> {
        Self::with_paths(Paths::from_env()?)
    }

    pub fn with_paths(paths: Paths) -> Result<Acswi> {
        fs::create_dir_all(&paths.root).map_err(|e| Error::io(paths.root.display(), e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&paths.root, fs::Permissions::from_mode(0o700))
                .map_err(|e| Error::io(paths.root.display(), e))?;
        }
        let defs = Defs::load(&paths.user_defs);
        Ok(Acswi { paths, defs })
    }

    fn lock(&self) -> Result<Lock> {
        let path = self.paths.lock_file();
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| Error::io(path.display(), e))?;
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Lock(file)),
                Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
                Err(_) => bail!(Busy, "Another acswi switch is in progress"),
            }
        }
    }

    fn state(&self) -> Result<State> {
        State::load(&self.paths.state_file())
    }

    fn save_state(&self, s: &State) -> Result<()> {
        s.save(&self.paths.state_file())
    }

    fn audit(&self, actor: Actor, event: serde_json::Value) {
        let mut line = serde_json::json!({ "time": Utc::now(), "actor": actor });
        if let (Some(obj), Some(ev)) = (line.as_object_mut(), event.as_object()) {
            obj.extend(ev.clone());
        }
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(self.paths.audit_file()) {
            let _ = writeln!(f, "{line}");
        }
    }

    pub fn def(&self, app: &str) -> Result<&AppDef> {
        self.defs.get(app)
    }

    // ------------------------------------------------------------------ views

    /// All apps (or one), their profiles and which is active. Cheap: no app commands run
    /// except the native list commands.
    pub fn overview(&self, only: Option<&str>) -> Result<Vec<AppView>> {
        let state = self.state()?;
        let defs: Vec<&AppDef> = match only {
            Some(id) => vec![self.def(id)?],
            None => self.defs.apps.values().collect(),
        };
        // Native list commands take ~1s each (az is Python), so look at apps in parallel.
        Ok(std::thread::scope(|scope| {
            let handles: Vec<_> = defs.iter().map(|def| scope.spawn(|| self.view(def, &state))).collect();
            handles.into_iter().map(|h| h.join().expect("view thread")).collect()
        }))
    }

    fn view(&self, def: &AppDef, state: &State) -> AppView {
        let app_state = state.app(&def.id).cloned().unwrap_or_default();
        let installed = def.installed();
        let mut warning = None;
        let mut active = app_state.active.clone();
        let mut profiles: Vec<ProfileView> = app_state
            .profiles
            .iter()
            .map(|(name, p)| ProfileView {
                name: name.clone(),
                active: false,
                identity: p.identity.clone().or_else(|| p.account.clone()),
                saved: true,
                last_used: p.used,
            })
            .collect();
        if def.strategy == Strategy::Native && installed {
            match def.native.as_ref().map(native::list).transpose() {
                Ok(Some(accounts)) => {
                    active = None;
                    for acc in &accounts {
                        let bound = app_state
                            .profiles
                            .iter()
                            .find(|(_, p)| p.account.as_deref() == Some(&acc.account))
                            .map(|(n, _)| n.clone());
                        if acc.active {
                            active = Some(bound.clone().unwrap_or_else(|| acc.account.clone()));
                        }
                        if bound.is_none() {
                            profiles.push(ProfileView {
                                name: acc.account.clone(),
                                active: false,
                                identity: Some(acc.account.clone()),
                                saved: false,
                                last_used: None,
                            });
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => warning = Some(e.message),
            }
        }
        for p in &mut profiles {
            p.active = active.as_deref() == Some(p.name.as_str());
        }
        let unsaved = def.strategy == Strategy::Swap
            && installed
            && active.is_none()
            && app_state.pending.is_none()
            && self.has_live_state(def).unwrap_or(false);
        AppView {
            id: def.id.clone(),
            name: def.name.clone(),
            strategy: def.strategy,
            installed,
            running: !process::find(&def.processes).is_empty(),
            active,
            profiles,
            pending_add: app_state.pending.map(|p| p.profile),
            unsaved,
            notes: def.notes.clone(),
            warning,
        }
    }

    /// Like `overview` for one app, plus who the app says is signed in right now.
    pub fn status(&self, app: &str) -> Result<StatusView> {
        let def = self.def(app)?;
        let state = self.state()?;
        let view = self.view(def, &state);
        let expected = view.active.as_ref().and_then(|a| {
            state.app(&def.id).and_then(|s| s.profiles.get(a)).and_then(|p| p.identity.clone().or(p.account.clone()))
        });
        let live = self.live_identity(def)?;
        let signed_in = match def.strategy {
            Strategy::Swap => def.state.iter().map(items::present).collect::<Result<Vec<_>>>()?.into_iter().any(|b| b),
            _ => view.active.is_some() || live.is_some(),
        };
        let in_sync = match (&live, &expected) {
            (Some(l), Some(e)) => l.eq_ignore_ascii_case(e),
            _ => true,
        };
        Ok(StatusView {
            app: def.id.clone(),
            active: view.active,
            live_identity: live,
            expected_identity: expected,
            signed_in,
            in_sync,
            running: process::find(&def.processes),
        })
    }

    fn live_identity(&self, def: &AppDef) -> Result<Option<String>> {
        match def.strategy {
            Strategy::Native => Ok(def
                .native
                .as_ref()
                .map(native::list)
                .transpose()?
                .and_then(|l| l.into_iter().find(|a| a.active).map(|a| a.account))),
            _ => match &def.identity {
                Some(spec) => native::identity(spec).or(Ok(None)),
                None => Ok(None),
            },
        }
    }

    // ------------------------------------------------------------------ switch

    pub fn switch(&self, app: &str, profile: &str, opts: &SwitchOptions) -> Result<SwitchReport> {
        let def = self.def(app)?.clone();
        if !def.installed() {
            bail!(NotInstalled, "{} is not installed", def.name);
        }
        let _lock = self.lock()?;
        self.recover_app(&def)?;
        let report = match def.strategy {
            Strategy::Native => self.switch_native(&def, profile)?,
            Strategy::Swap => self.switch_swap(&def, profile, opts)?,
            Strategy::Env => self.switch_env(&def, profile)?,
        };
        if report.changed {
            self.audit(
                opts.actor,
                serde_json::json!({ "action": "use", "app": def.id, "from": report.from, "to": report.to }),
            );
        }
        Ok(report)
    }

    fn native_accounts(&self, def: &AppDef) -> Result<Vec<NativeAccount>> {
        native::list(def.native.as_ref().expect("validated"))
    }

    fn switch_native(&self, def: &AppDef, profile: &str) -> Result<SwitchReport> {
        let mut state = self.state()?;
        let accounts = self.native_accounts(def)?;
        let app_state = state.app_mut(&def.id);
        // A profile name, or directly an account label the app knows.
        let account = match app_state.profiles.get(profile).and_then(|p| p.account.clone()) {
            Some(a) => a,
            None => accounts
                .iter()
                .find(|a| a.account.eq_ignore_ascii_case(profile))
                .map(|a| a.account.clone())
                .ok_or_else(|| self.unknown_profile(def, profile))?,
        };
        let target = accounts.iter().find(|a| a.account == account).ok_or_else(|| {
            Error::new(
                Code::UnknownProfile,
                format!("{} no longer knows the account {account}. Sign in again with `acswi add {} {profile}`.", def.name, def.id),
            )
        })?;
        let from = accounts.iter().find(|a| a.active).map(|a| self.native_name(def, &a.account));
        let changed = !target.active;
        if changed {
            native::switch(def.native.as_ref().unwrap(), &target.value)?;
        }
        let name = self.native_name(def, &account);
        if let Some(p) = state.app_mut(&def.id).profiles.get_mut(&name) {
            p.used = Some(Utc::now());
        }
        self.save_state(&state)?;
        Ok(SwitchReport {
            app: def.id.clone(),
            from,
            to: name,
            identity: Some(account),
            closed: false,
            relaunched: false,
            changed,
        })
    }

    /// The profile name bound to a native account, or the account label itself.
    fn native_name(&self, def: &AppDef, account: &str) -> String {
        self.state()
            .ok()
            .and_then(|s| {
                s.app(&def.id).and_then(|a| {
                    a.profiles.iter().find(|(_, p)| p.account.as_deref() == Some(account)).map(|(n, _)| n.clone())
                })
            })
            .unwrap_or_else(|| account.to_string())
    }

    fn unknown_profile(&self, def: &AppDef, profile: &str) -> Error {
        let names: Vec<String> = self
            .overview(Some(&def.id))
            .ok()
            .and_then(|mut v| v.pop())
            .map(|v| v.profiles.into_iter().map(|p| p.name).collect())
            .unwrap_or_default();
        Error::new(
            Code::UnknownProfile,
            format!("{} has no profile {profile:?}. Known: {}", def.name, if names.is_empty() { "none".into() } else { names.join(", ") }),
        )
        .with(serde_json::json!({ "profiles": names }))
    }

    fn ensure_closed(&self, def: &AppDef, opts: &SwitchOptions) -> Result<bool> {
        let running = process::find(&def.processes);
        if running.is_empty() {
            return Ok(false);
        }
        if !opts.close {
            return Err(Error::new(
                Code::AppRunning,
                format!("{} is running. Close it first, or pass --close to let acswi close it.", def.name),
            )
            .with(serde_json::to_value(&running).unwrap()));
        }
        process::close(&def.processes, CLOSE_GRACE)?;
        Ok(true)
    }

    fn switch_swap(&self, def: &AppDef, profile: &str, opts: &SwitchOptions) -> Result<SwitchReport> {
        let mut state = self.state()?;
        let app_state = state.app_mut(&def.id);
        if let Some(p) = &app_state.pending {
            bail!(AddPending, "Adding {:?} is not finished. Run `acswi add {} {} --finish` or `--cancel`.", p.profile, def.id, p.profile);
        }
        if !app_state.profiles.contains_key(profile) {
            return Err(self.unknown_profile(def, profile));
        }
        let from = app_state.active.clone();
        if from.as_deref() == Some(profile) {
            return Ok(SwitchReport {
                app: def.id.clone(),
                from: from.clone(),
                to: profile.into(),
                identity: app_state.profiles[profile].identity.clone(),
                closed: false,
                relaunched: false,
                changed: false,
            });
        }
        if from.is_none() && self.has_live_state(def)? {
            return Err(self.unsaved(def));
        }
        let was_running = self.ensure_closed(def, opts)?;

        if let Some(from) = &from {
            self.save_live(def, from, Some(profile), opts.force)?;
        } else {
            self.write_journal(&Journal { app: def.id.clone(), from: None, to: Some(profile.into()), phase: Phase::Restoring, started: Utc::now() })?;
        }
        self.restore_profile(def, profile)?;

        let identity = match &def.identity {
            Some(spec) => native::identity(spec).unwrap_or(None),
            None => None,
        };
        let mut state = self.state()?;
        let app_state = state.app_mut(&def.id);
        app_state.active = Some(profile.into());
        if let Some(p) = app_state.profiles.get_mut(profile) {
            p.used = Some(Utc::now());
            if p.identity.is_none() {
                p.identity = identity.clone();
            }
        }
        self.save_state(&state)?;
        self.clear_journal();

        let relaunched = was_running && opts.relaunch && !def.launch.is_empty() && process::launch(&def.launch).is_ok();
        Ok(SwitchReport { app: def.id.clone(), from, to: profile.into(), identity, closed: was_running, relaunched, changed: true })
    }

    fn unsaved(&self, def: &AppDef) -> Error {
        Error::new(
            Code::UnsavedSession,
            format!(
                "{} is signed in but that account is not saved yet. Save it first: `acswi save {} <name>`",
                def.name, def.id
            ),
        )
    }

    fn has_live_state(&self, def: &AppDef) -> Result<bool> {
        for item in &def.state {
            if items::present(item)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Saves the live state into `profile` and clears it from the live location.
    /// `next` is only recorded in the journal.
    fn save_live(&self, def: &AppDef, profile: &str, next: Option<&str>, force: bool) -> Result<()> {
        let mut journal = Journal {
            app: def.id.clone(),
            from: Some(profile.into()),
            to: next.map(Into::into),
            phase: Phase::Saving,
            started: Utc::now(),
        };
        // Refuse to file one account's tokens under another account's profile.
        let recorded = self.state()?.app(&def.id).and_then(|a| a.profiles.get(profile)).and_then(|p| p.identity.clone());
        let live_identity = match &def.identity {
            Some(spec) => native::identity(spec).unwrap_or(None),
            None => None,
        };
        if let (Some(rec), Some(live)) = (&recorded, &live_identity) {
            if !rec.eq_ignore_ascii_case(live) && !force {
                return Err(Error::new(
                    Code::IdentityMismatch,
                    format!(
                        "{} is signed in as {live}, but the active profile {profile:?} belongs to {rec}. Nothing was changed. \
                         Save this account under its own name with `acswi save {} <name>`, or pass --force to overwrite {profile:?}.",
                        def.name, def.id
                    ),
                ));
            }
        }

        self.write_journal(&journal)?;
        let pdir = self.paths.profile_dir(&def.id, profile);
        let store = MoveStore { profile_dir: &pdir };
        let vault = Vault::open(&self.paths)?;
        let mut snap = Snapshot { app: def.id.clone(), saved: Utc::now(), items: Vec::new() };
        let captured: Result<()> = (|| {
            for item in &def.state {
                snap.items.push((item.describe(), items::capture(item, &store)?));
            }
            vault.write(&vault.path(&def.id, profile), &snap)
        })();
        if let Err(e) = captured {
            for item in &def.state {
                let _ = items::rollback_moved(item, &store);
            }
            self.clear_journal();
            return Err(e);
        }
        for item in &def.state {
            items::commit_moved(item, &store)?;
        }
        journal.phase = Phase::Restoring;
        self.write_journal(&journal)?;
        for item in &def.state {
            items::clear(item)?;
        }
        let mut state = self.state()?;
        let app_state = state.app_mut(&def.id);
        if let Some(p) = app_state.profiles.get_mut(profile) {
            p.saved = Some(Utc::now());
            if p.identity.is_none() || force {
                p.identity = live_identity;
            }
        }
        app_state.active = None;
        self.save_state(&state)
    }

    fn restore_profile(&self, def: &AppDef, profile: &str) -> Result<()> {
        let vault = Vault::open(&self.paths)?;
        let snap = vault.read(&def.id, profile)?;
        let pdir = self.paths.profile_dir(&def.id, profile);
        let store = MoveStore { profile_dir: &pdir };
        for item in &def.state {
            let saved = snap.as_ref().and_then(|s| s.get(&item.describe()));
            items::restore(item, saved, &store)?;
        }
        Ok(())
    }

    // ------------------------------------------------------------------ env strategy

    fn switch_env(&self, def: &AppDef, profile: &str) -> Result<SwitchReport> {
        let spec = def.env.as_ref().expect("validated");
        let mut state = self.state()?;
        let app_state = state.app_mut(&def.id);
        if !app_state.profiles.contains_key(profile) {
            return Err(self.unknown_profile(def, profile));
        }
        let from = app_state.active.clone();
        let dir = self.paths.profile_dir(&def.id, profile);
        crate::envvar::set_user(&spec.var, &dir.to_string_lossy())?;
        app_state.active = Some(profile.into());
        if let Some(p) = app_state.profiles.get_mut(profile) {
            p.used = Some(Utc::now());
        }
        self.save_state(&state)?;
        Ok(SwitchReport {
            app: def.id.clone(),
            changed: from.as_deref() != Some(profile),
            from,
            to: profile.into(),
            identity: None,
            closed: false,
            relaunched: false,
        })
    }

    // ------------------------------------------------------------------ add / save

    /// Saves the account that is signed in right now as `profile` (and makes it active).
    /// For native apps, binds the active account to the name.
    pub fn save(&self, app: &str, profile: &str, actor: Actor) -> Result<()> {
        let def = self.def(app)?.clone();
        check_name(profile)?;
        let _lock = self.lock()?;
        let mut state = self.state()?;
        let app_state = state.app_mut(&def.id);
        match def.strategy {
            Strategy::Native => {
                let active = self
                    .native_accounts(&def)?
                    .into_iter()
                    .find(|a| a.active)
                    .ok_or_else(|| Error::new(Code::UnknownProfile, format!("{} has no signed-in account", def.name)))?;
                app_state.profiles.retain(|_, p| p.account.as_deref() != Some(&active.account));
                let p = app_state.profiles.entry(profile.into()).or_default();
                p.account = Some(active.account.clone());
                p.identity = Some(active.account);
                self.save_state(&state)?;
            }
            Strategy::Env => bail!(Unsupported, "{} uses one folder per profile; use `acswi add`", def.name),
            Strategy::Swap => {
                if let Some(active) = app_state.active.clone() {
                    if active != profile {
                        bail!(
                            ProfileExists,
                            "{} is already saved as {active:?}. Use `acswi rename {} {active} {profile}` to rename it.",
                            def.name,
                            def.id
                        );
                    }
                }
                if !self.has_live_state(&def)? {
                    bail!(UnknownProfile, "{} is not signed in, so there is nothing to save", def.name);
                }
                if app_state.profiles.contains_key(profile) && app_state.active.as_deref() != Some(profile) {
                    bail!(ProfileExists, "{profile:?} already exists for {}", def.name);
                }
                self.snapshot_into(&def, profile)?;
                let identity = def.identity.as_ref().and_then(|s| native::identity(s).ok().flatten());
                let mut state = self.state()?;
                let app_state = state.app_mut(&def.id);
                let p = app_state.profiles.entry(profile.into()).or_default();
                p.saved = Some(Utc::now());
                if identity.is_some() {
                    p.identity = identity;
                }
                app_state.active = Some(profile.into());
                self.save_state(&state)?;
            }
        }
        self.audit(actor, serde_json::json!({ "action": "save", "app": def.id, "profile": profile }));
        Ok(())
    }

    /// Copies live state into the vault without clearing it (moved folders are not touched).
    fn snapshot_into(&self, def: &AppDef, profile: &str) -> Result<()> {
        let vault = Vault::open(&self.paths)?;
        let mut snap = Snapshot { app: def.id.clone(), saved: Utc::now(), items: Vec::new() };
        let pdir = self.paths.profile_dir(&def.id, profile);
        let store = MoveStore { profile_dir: &pdir };
        for item in &def.state {
            if let crate::def::StateItem::Dir { mode: crate::def::DirMode::Move, .. } = item {
                // Lives in place while active; moved into the profile store on switch.
                snap.items.push((item.describe(), crate::vault::Saved::Moved(items::present(item)?)));
                continue;
            }
            snap.items.push((item.describe(), items::capture(item, &store)?));
        }
        vault.write(&vault.path(&def.id, profile), &snap)
    }

    /// Refreshes the active profile's saved copy from the live state (e.g. after a token
    /// refresh). Safe to call any time the app is closed.
    pub fn refresh(&self, app: &str) -> Result<Option<String>> {
        let def = self.def(app)?.clone();
        if def.strategy != Strategy::Swap {
            return Ok(None);
        }
        let _lock = self.lock()?;
        let state = self.state()?;
        let Some(active) = state.app(&def.id).and_then(|a| a.active.clone()) else { return Ok(None) };
        self.snapshot_into(&def, &active)?;
        let mut state = self.state()?;
        if let Some(p) = state.app_mut(&def.id).profiles.get_mut(&active) {
            p.saved = Some(Utc::now());
        }
        self.save_state(&state)?;
        Ok(Some(active))
    }

    /// Starts adding a new account as `profile`.
    pub fn add_begin(&self, app: &str, profile: &str, opts: &SwitchOptions) -> Result<AddStep> {
        let def = self.def(app)?.clone();
        check_name(profile)?;
        if !def.installed() {
            bail!(NotInstalled, "{} is not installed", def.name);
        }
        let _lock = self.lock()?;
        self.recover_app(&def)?;
        let mut state = self.state()?;
        let app_state = state.app_mut(&def.id);
        if app_state.profiles.contains_key(profile) {
            bail!(ProfileExists, "{profile:?} already exists for {}", def.name);
        }
        if let Some(p) = &app_state.pending {
            bail!(AddPending, "Adding {:?} is not finished yet. Finish or cancel it first.", p.profile);
        }
        match def.strategy {
            Strategy::Native => {
                let before: Vec<String> = self.native_accounts(&def)?.into_iter().map(|a| a.account).collect();
                app_state.pending = Some(PendingAdd { profile: profile.into(), previous: Some(before.join("\n")), started: Utc::now() });
                self.save_state(&state)?;
                Ok(AddStep::RunLogin { command: def.login.clone(), env: login_env(&def, vec![]) })
            }
            Strategy::Env => {
                let spec = def.env.as_ref().unwrap();
                let dir = self.paths.profile_dir(&def.id, profile);
                fs::create_dir_all(&dir).map_err(|e| Error::io(dir.display(), e))?;
                app_state.pending = Some(PendingAdd { profile: profile.into(), previous: app_state.active.clone(), started: Utc::now() });
                self.save_state(&state)?;
                Ok(AddStep::RunLogin {
                    command: def.login.clone(),
                    env: login_env(&def, vec![(spec.var.clone(), dir.to_string_lossy().into_owned())]),
                })
            }
            Strategy::Swap => {
                let previous = app_state.active.clone();
                if previous.is_none() && self.has_live_state(&def)? {
                    return Err(self.unsaved(&def));
                }
                self.ensure_closed(&def, opts)?;
                if let Some(prev) = &previous {
                    self.save_live(&def, prev, None, opts.force)?;
                    self.clear_journal();
                }
                let mut state = self.state()?;
                state.app_mut(&def.id).pending = Some(PendingAdd { profile: profile.into(), previous, started: Utc::now() });
                self.save_state(&state)?;
                if def.login.is_empty() {
                    process::launch(&def.launch)?;
                    Ok(AddStep::SignInApp)
                } else {
                    Ok(AddStep::RunLogin { command: def.login.clone(), env: login_env(&def, vec![]) })
                }
            }
        }
    }

    /// Captures the newly signed-in account into the pending profile.
    pub fn add_finish(&self, app: &str, opts: &SwitchOptions) -> Result<String> {
        let def = self.def(app)?.clone();
        let _lock = self.lock()?;
        let mut state = self.state()?;
        let pending = state
            .app(&def.id)
            .and_then(|a| a.pending.clone())
            .ok_or_else(|| Error::new(Code::UnknownProfile, format!("No account is being added for {}", def.name)))?;
        let profile = pending.profile.clone();
        match def.strategy {
            Strategy::Native => {
                let before: Vec<&str> = pending.previous.as_deref().unwrap_or("").lines().collect();
                let accounts = self.native_accounts(&def)?;
                let Some(new) = accounts.iter().find(|a| !before.contains(&a.account.as_str())) else {
                    // Signing in to an account the app already had is not an add; keep the
                    // pending add so the person can sign in to a different one and retry.
                    let active = accounts.iter().find(|a| a.active).map(|a| a.account.as_str()).unwrap_or("an existing account");
                    bail!(
                        CommandFailed,
                        "{} is still signed in to {active}, which acswi already has. Sign in to a different account, then finish again (or cancel).",
                        def.name
                    );
                };
                let app_state = state.app_mut(&def.id);
                app_state.profiles.retain(|_, p| p.account.as_deref() != Some(&new.account));
                let mut p = Profile::new();
                p.account = Some(new.account.clone());
                p.identity = Some(new.account.clone());
                p.used = Some(Utc::now());
                app_state.profiles.insert(profile.clone(), p);
                app_state.pending = None;
                self.save_state(&state)?;
            }
            Strategy::Env => {
                let app_state = state.app_mut(&def.id);
                app_state.profiles.insert(profile.clone(), Profile::new());
                app_state.pending = None;
                self.save_state(&state)?;
                drop(_lock);
                self.switch_env(&def, &profile)?;
            }
            Strategy::Swap => {
                if !self.has_live_state(&def)? {
                    bail!(CommandFailed, "{} is not signed in yet. Sign in, then finish again (or cancel).", def.name);
                }
                let was_running = self.ensure_closed(&def, &SwitchOptions { close: true, ..opts.clone() })?;
                state.app_mut(&def.id).profiles.insert(profile.clone(), Profile::new());
                self.save_state(&state)?;
                self.snapshot_into(&def, &profile)?;
                let identity = def.identity.as_ref().and_then(|s| native::identity(s).ok().flatten());
                let mut state = self.state()?;
                let app_state = state.app_mut(&def.id);
                if let Some(p) = app_state.profiles.get_mut(&profile) {
                    p.identity = identity;
                    p.saved = Some(Utc::now());
                    p.used = Some(Utc::now());
                }
                app_state.active = Some(profile.clone());
                app_state.pending = None;
                self.save_state(&state)?;
                if was_running && opts.relaunch {
                    let _ = process::launch(&def.launch);
                }
            }
        }
        self.audit(opts.actor, serde_json::json!({ "action": "add", "app": def.id, "profile": profile }));
        Ok(profile)
    }

    /// Abandons an unfinished add and puts the previous account back.
    pub fn add_cancel(&self, app: &str, opts: &SwitchOptions) -> Result<Option<String>> {
        let def = self.def(app)?.clone();
        let _lock = self.lock()?;
        let mut state = self.state()?;
        let Some(pending) = state.app(&def.id).and_then(|a| a.pending.clone()) else { return Ok(None) };
        state.app_mut(&def.id).pending = None;
        self.save_state(&state)?;
        if def.strategy != Strategy::Swap {
            return Ok(None);
        }
        self.ensure_closed(&def, &SwitchOptions { close: true, ..opts.clone() })?;
        for item in &def.state {
            if !matches!(item, crate::def::StateItem::Dir { mode: crate::def::DirMode::Move, .. }) {
                items::clear(item)?;
            } else {
                // Discard the half-made sign-in folder.
                let live = crate::paths::expand(match item {
                    crate::def::StateItem::Dir { path, .. } => path,
                    _ => unreachable!(),
                });
                if live.exists() {
                    fs::remove_dir_all(&live).map_err(|e| Error::io(live.display(), e))?;
                }
            }
        }
        if let Some(prev) = &pending.previous {
            self.restore_profile(&def, prev)?;
            let mut state = self.state()?;
            state.app_mut(&def.id).active = Some(prev.clone());
            self.save_state(&state)?;
        }
        Ok(pending.previous)
    }

    // ------------------------------------------------------------------ manage

    pub fn rename(&self, app: &str, old: &str, new: &str) -> Result<()> {
        let def = self.def(app)?.clone();
        check_name(new)?;
        let _lock = self.lock()?;
        let mut state = self.state()?;
        if def.strategy == Strategy::Native && !state.app(&def.id).is_some_and(|a| a.profiles.contains_key(old)) {
            // Give a name to an account the app knows but acswi has not saved yet.
            let acc = self
                .native_accounts(&def)?
                .into_iter()
                .find(|a| a.account.eq_ignore_ascii_case(old))
                .ok_or_else(|| self.unknown_profile(&def, old))?;
            let app_state = state.app_mut(&def.id);
            if app_state.profiles.contains_key(new) {
                bail!(ProfileExists, "{new:?} already exists for {}", def.name);
            }
            let mut p = Profile::new();
            p.identity = Some(acc.account.clone());
            p.account = Some(acc.account);
            app_state.profiles.insert(new.into(), p);
            return self.save_state(&state);
        }
        let app_state = state.app_mut(&def.id);
        if app_state.profiles.contains_key(new) {
            bail!(ProfileExists, "{new:?} already exists for {}", def.name);
        }
        let p = app_state.profiles.remove(old).ok_or_else(|| self.unknown_profile(&def, old))?;
        app_state.profiles.insert(new.into(), p);
        if app_state.active.as_deref() == Some(old) {
            app_state.active = Some(new.into());
        }
        let (vo, vn) = (self.paths.vault_blob(&def.id, old), self.paths.vault_blob(&def.id, new));
        if vo.exists() {
            fs::rename(&vo, &vn).map_err(|e| Error::io(vo.display(), e))?;
        }
        let (po, pn) = (self.paths.profile_dir(&def.id, old), self.paths.profile_dir(&def.id, new));
        if po.exists() {
            fs::rename(&po, &pn).map_err(|e| Error::io(po.display(), e))?;
        }
        self.save_state(&state)
    }

    /// Forgets a profile and deletes its saved state. The active profile cannot be removed
    /// (switch away first) so a live sign-in is never lost by accident.
    pub fn remove(&self, app: &str, profile: &str, actor: Actor) -> Result<()> {
        let def = self.def(app)?.clone();
        let _lock = self.lock()?;
        let mut state = self.state()?;
        let app_state = state.app_mut(&def.id);
        if !app_state.profiles.contains_key(profile) {
            return Err(self.unknown_profile(&def, profile));
        }
        if def.strategy != Strategy::Native && app_state.active.as_deref() == Some(profile) {
            bail!(Unsupported, "{profile:?} is in use. Switch to another profile first.");
        }
        app_state.profiles.remove(profile);
        self.save_state(&state)?;
        let _ = fs::remove_file(self.paths.vault_blob(&def.id, profile));
        let dir = self.paths.profile_dir(&def.id, profile);
        if dir.exists() {
            fs::remove_dir_all(&dir).map_err(|e| Error::io(dir.display(), e))?;
        }
        self.audit(actor, serde_json::json!({ "action": "remove", "app": def.id, "profile": profile }));
        Ok(())
    }

    // ------------------------------------------------------------------ recovery

    fn write_journal(&self, j: &Journal) -> Result<()> {
        write_atomic(&self.paths.journal_file(), &serde_json::to_vec_pretty(j).unwrap())
    }

    fn clear_journal(&self) {
        let _ = fs::remove_file(self.paths.journal_file());
    }

    /// Finishes or undoes a switch that was interrupted (crash, power loss).
    /// Returns a description of what was done.
    pub fn recover(&self) -> Result<Option<String>> {
        let _lock = self.lock()?;
        let Some(j) = self.read_journal()? else { return Ok(None) };
        let def = self.def(&j.app)?.clone();
        self.recover_with(&def, j)
    }

    fn read_journal(&self) -> Result<Option<Journal>> {
        match fs::read(self.paths.journal_file()) {
            Ok(b) => Ok(serde_json::from_slice(&b).ok()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::io("journal", e)),
        }
    }

    fn recover_app(&self, def: &AppDef) -> Result<()> {
        if let Some(j) = self.read_journal()? {
            if j.app == def.id {
                self.recover_with(def, j)?;
            }
        }
        Ok(())
    }

    fn recover_with(&self, def: &AppDef, j: Journal) -> Result<Option<String>> {
        let msg = match j.phase {
            Phase::Saving => {
                if let Some(from) = &j.from {
                    let pdir = self.paths.profile_dir(&def.id, from);
                    let store = MoveStore { profile_dir: &pdir };
                    for item in &def.state {
                        items::rollback_moved(item, &store)?;
                    }
                }
                format!("Undid an interrupted switch of {}; {:?} is still active.", def.name, j.from)
            }
            Phase::Restoring => {
                let mut state = self.state()?;
                match &j.to {
                    Some(to) => {
                        for item in &def.state {
                            items::clear(item)?;
                        }
                        self.restore_profile(def, to)?;
                        state.app_mut(&def.id).active = Some(to.clone());
                        self.save_state(&state)?;
                        format!("Finished an interrupted switch of {} to {to:?}.", def.name)
                    }
                    None => {
                        state.app_mut(&def.id).active = None;
                        self.save_state(&state)?;
                        format!("{} was signed out by an interrupted add; its previous account is saved.", def.name)
                    }
                }
            }
        };
        self.clear_journal();
        Ok(Some(msg))
    }
}

fn check_name(name: &str) -> Result<()> {
    if !valid_name(name) {
        bail!(InvalidName, "{name:?} is not a valid profile name (letters, digits, - _ . @; up to 64 characters)");
    }
    Ok(())
}
