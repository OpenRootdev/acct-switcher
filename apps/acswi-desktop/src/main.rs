#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::os::windows::process::CommandExt;
use std::process::Command;

use acswi_core::engine::{Acswi, Actor, AddStep, AppView, SwitchOptions, SwitchReport};
use acswi_core::{Code, Error};
use serde::Serialize;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WindowEvent, Wry};

const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
const SEP: char = '\u{1f}';

fn opts(close: bool) -> SwitchOptions {
    SwitchOptions { close, relaunch: true, force: false, actor: Actor::Ui }
}

#[derive(Clone, Serialize)]
struct Changed {
    app: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Error>,
}

#[derive(Clone, Serialize)]
struct ConfirmClose {
    app: String,
    name: String,
    profile: String,
}

fn changed(handle: &AppHandle, app: &str, message: Option<String>, error: Option<Error>) {
    let _ = handle.emit("acswi://changed", Changed { app: app.into(), message, error });
    refresh_tray(handle.clone());
}

// ------------------------------------------------------------------ commands

#[tauri::command(async)]
fn overview(all: bool) -> Result<Vec<AppView>, Error> {
    Ok(Acswi::open()?
        .overview(None)?
        .into_iter()
        .filter(|v| all || v.installed || !v.profiles.is_empty())
        .collect())
}

#[tauri::command(async)]
fn switch_profile(handle: AppHandle, app: String, profile: String, close: bool) -> Result<SwitchReport, Error> {
    let report = Acswi::open()?.switch(&app, &profile, &opts(close))?;
    refresh_tray(handle);
    Ok(report)
}

#[tauri::command(async)]
fn save_current(handle: AppHandle, app: String, profile: String) -> Result<(), Error> {
    Acswi::open()?.save(&app, &profile, Actor::Ui)?;
    refresh_tray(handle);
    Ok(())
}

#[tauri::command(async)]
fn refresh_saved(app: String) -> Result<Option<String>, Error> {
    Acswi::open()?.refresh(&app)
}

#[tauri::command(async)]
fn rename_profile(handle: AppHandle, app: String, old: String, new: String) -> Result<(), Error> {
    Acswi::open()?.rename(&app, &old, &new)?;
    refresh_tray(handle);
    Ok(())
}

#[tauri::command(async)]
fn remove_profile(handle: AppHandle, app: String, profile: String) -> Result<(), Error> {
    Acswi::open()?.remove(&app, &profile, Actor::Ui)?;
    refresh_tray(handle);
    Ok(())
}

/// Starts adding an account. Sign-in commands (gh auth login, …) need a terminal, so they
/// run in their own console window; when it closes the account is saved automatically and
/// `acswi://changed` is emitted.
#[tauri::command(async)]
fn add_begin(handle: AppHandle, app: String, profile: String, close: bool) -> Result<AddStep, Error> {
    let acswi = Acswi::open()?;
    let step = acswi.add_begin(&app, &profile, &opts(close))?;
    if let AddStep::RunLogin { command, env } = &step {
        let (command, env) = (command.clone(), env.clone());
        std::thread::spawn(move || {
            let result = run_in_console(&command, &env);
            let acswi = match Acswi::open() {
                Ok(a) => a,
                Err(e) => return changed(&handle, &app, None, Some(e)),
            };
            match result {
                Ok(true) => match acswi.add_finish(&app, &opts(true)) {
                    Ok(p) => changed(&handle, &app, Some(format!("Added {p}")), None),
                    Err(e) => {
                        let _ = acswi.add_cancel(&app, &opts(true));
                        changed(&handle, &app, None, Some(e))
                    }
                },
                Ok(false) | Err(_) => {
                    let _ = acswi.add_cancel(&app, &opts(true));
                    let err = result.err().unwrap_or_else(|| Error::new(Code::CommandFailed, "Sign-in was not completed"));
                    changed(&handle, &app, None, Some(err))
                }
            }
        });
    }
    Ok(step)
}

#[tauri::command(async)]
fn add_finish(handle: AppHandle, app: String) -> Result<String, Error> {
    let p = Acswi::open()?.add_finish(&app, &opts(true))?;
    refresh_tray(handle);
    Ok(p)
}

#[tauri::command(async)]
fn add_cancel(handle: AppHandle, app: String) -> Result<Option<String>, Error> {
    let back = Acswi::open()?.add_cancel(&app, &opts(true))?;
    refresh_tray(handle);
    Ok(back)
}

#[tauri::command]
fn open_apps_folder() -> Result<(), Error> {
    let acswi = Acswi::open()?;
    let dir = &acswi.paths.user_defs;
    std::fs::create_dir_all(dir).map_err(|e| Error::io(dir.display(), e))?;
    Command::new("explorer.exe").arg(dir).spawn().map_err(|e| Error::io("explorer", e))?;
    Ok(())
}

fn run_in_console(command: &[String], env: &[(String, String)]) -> Result<bool, Error> {
    let Some((prog, args)) = command.split_first() else { return Ok(true) };
    let path = acswi_core::paths::which(prog)
        .ok_or_else(|| Error::new(Code::NotInstalled, format!("{prog} was not found on PATH")))?;
    let status = Command::new(path)
        .args(args)
        .envs(env.iter().map(|(k, v)| (k, v)))
        .creation_flags(CREATE_NEW_CONSOLE)
        .status()
        .map_err(|e| Error::new(Code::CommandFailed, format!("{prog}: {e}")))?;
    Ok(status.success())
}

// ------------------------------------------------------------------ window + tray

fn show_main(handle: &AppHandle) {
    if let Some(w) = handle.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn build_menu(handle: &AppHandle, views: &[AppView]) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::new(handle)?;
    menu.append(&MenuItem::with_id(handle, "open", "Open Account Switcher", true, None::<&str>)?)?;
    menu.append(&PredefinedMenuItem::separator(handle)?)?;
    let mut any = false;
    for v in views.iter().filter(|v| v.installed && !v.profiles.is_empty()) {
        let sub = Submenu::new(handle, &v.name, true)?;
        for p in &v.profiles {
            let id = format!("use{SEP}{}{SEP}{}", v.id, p.name);
            sub.append(&CheckMenuItem::with_id(handle, id, &p.name, true, p.active, None::<&str>)?)?;
        }
        menu.append(&sub)?;
        any = true;
    }
    if !any {
        menu.append(&MenuItem::with_id(handle, "none", "No saved accounts yet", false, None::<&str>)?)?;
    }
    menu.append(&PredefinedMenuItem::separator(handle)?)?;
    menu.append(&MenuItem::with_id(handle, "quit", "Quit", true, None::<&str>)?)?;
    Ok(menu)
}

/// Rebuilds the tray menu in the background (listing apps can take a second).
fn refresh_tray(handle: AppHandle) {
    std::thread::spawn(move || {
        let views = Acswi::open().and_then(|a| a.overview(None)).unwrap_or_default();
        if let (Ok(menu), Some(tray)) = (build_menu(&handle, &views), handle.tray_by_id("main")) {
            let _ = tray.set_menu(Some(menu));
        }
    });
}

fn tray_switch(handle: AppHandle, app: String, profile: String) {
    std::thread::spawn(move || {
        let result = Acswi::open().and_then(|a| a.switch(&app, &profile, &opts(false)));
        match result {
            Ok(r) => changed(&handle, &app, Some(format!("Switched to {}", r.to)), None),
            Err(e) if e.code == Code::AppRunning => {
                let name = Acswi::open().ok().and_then(|a| a.def(&app).ok().map(|d| d.name.clone())).unwrap_or(app.clone());
                show_main(&handle);
                let _ = handle.emit("acswi://confirm-close", ConfirmClose { app, name, profile });
                refresh_tray(handle);
            }
            Err(e) => {
                show_main(&handle);
                changed(&handle, &app, None, Some(e));
            }
        }
    });
}

fn main() {
    let hidden = std::env::args().any(|a| a == "--hidden");
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|handle, _args, _cwd| show_main(handle)))
        .invoke_handler(tauri::generate_handler![
            overview,
            switch_profile,
            save_current,
            refresh_saved,
            rename_profile,
            remove_profile,
            add_begin,
            add_finish,
            add_cancel,
            open_apps_folder,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let menu = build_menu(&handle, &[])?;
            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().cloned().expect("icon"))
                .tooltip("Account Switcher")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|handle, event| {
                    let id = event.id().as_ref().to_string();
                    match id.as_str() {
                        "open" => show_main(handle),
                        "quit" => handle.exit(0),
                        _ => {
                            let parts: Vec<&str> = id.split(SEP).collect();
                            if let ["use", app, profile] = parts.as_slice() {
                                tray_switch(handle.clone(), app.to_string(), profile.to_string());
                            }
                        }
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        show_main(tray.app_handle());
                    }
                })
                .build(app)?;
            refresh_tray(handle.clone());
            if !hidden {
                show_main(&handle);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window keeps acswi in the tray.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("acswi failed to start");
}
