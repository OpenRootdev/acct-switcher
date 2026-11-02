//! Finding, closing and starting app processes.

use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;
use windows_sys::Win32::Foundation::{CloseHandle, HWND, INVALID_HANDLE_VALUE, LPARAM};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, TerminateProcess, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE,
};

use crate::error::{Code, Error, Result};
use crate::paths::{expand, which};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

#[derive(Debug, Clone, Serialize)]
pub struct Proc {
    pub pid: u32,
    pub name: String,
    pub path: String,
}

fn snapshot() -> Vec<Proc> {
    let mut out = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut entry);
        while ok != 0 {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
            out.push(Proc { pid: entry.th32ProcessID, path: image_path(entry.th32ProcessID), name });
            ok = Process32NextW(snap, &mut entry);
        }
        CloseHandle(snap);
    }
    out
}

fn image_path(pid: u32) -> String {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return String::new();
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
        CloseHandle(h);
        if ok == 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..len as usize])
    }
}

/// `*` wildcard match, case-insensitive.
fn wildcard(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut pi, mut ti, mut star, mut mark) = (0, 0, None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == t[ti] || p[pi] == '?') {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

fn matches(spec: &str, p: &Proc) -> bool {
    if spec.contains(['\\', '/']) {
        let spec = expand(spec).to_string_lossy().into_owned();
        !p.path.is_empty() && wildcard(&spec, &p.path)
    } else {
        wildcard(spec, &p.name)
    }
}

/// Running processes that match any of the definition's process specs.
pub fn find(specs: &[String]) -> Vec<Proc> {
    if specs.is_empty() {
        return Vec::new();
    }
    let me = std::process::id();
    snapshot().into_iter().filter(|p| p.pid != me && specs.iter().any(|s| matches(s, p))).collect()
}

unsafe extern "system" fn post_close(hwnd: HWND, lparam: LPARAM) -> i32 {
    let pids = &*(lparam as *const Vec<u32>);
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if pids.contains(&pid) && IsWindowVisible(hwnd) != 0 {
        PostMessageW(hwnd, WM_CLOSE, 0, 0);
    }
    1
}

/// Asks the app to close (like clicking X), waits, then ends whatever is left.
/// Returns how many processes had to be force-closed.
pub fn close(specs: &[String], grace: Duration) -> Result<usize> {
    let running = find(specs);
    if running.is_empty() {
        return Ok(0);
    }
    let pids: Vec<u32> = running.iter().map(|p| p.pid).collect();
    unsafe {
        EnumWindows(Some(post_close), &pids as *const Vec<u32> as LPARAM);
    }
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        if find(specs).is_empty() {
            return Ok(0);
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    let left = find(specs);
    for p in &left {
        unsafe {
            let h = OpenProcess(PROCESS_TERMINATE, 0, p.pid);
            if !h.is_null() {
                TerminateProcess(h, 1);
                CloseHandle(h);
            }
        }
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if find(specs).is_empty() {
            // Give Windows a moment to release file handles.
            std::thread::sleep(Duration::from_millis(300));
            return Ok(left.len());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(Error::new(Code::AppRunning, "The app did not close").with(serde_json::to_value(find(specs)).unwrap()))
}

/// Starts a GUI app detached from acct.
pub fn launch(cmd: &[String]) -> Result<()> {
    let Some((prog, args)) = cmd.split_first() else { return Ok(()) };
    let path = which(prog).unwrap_or_else(|| expand(prog));
    Command::new(&path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .map(|_| ())
        .map_err(|e| Error::new(Code::CommandFailed, format!("could not start {}: {e}", path.display())))
}

pub struct Output {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Runs a command without a console window and captures its output.
pub fn run(cmd: &[String]) -> Result<Output> {
    let Some((prog, args)) = cmd.split_first() else {
        return Err(Error::new(Code::InvalidDefinition, "empty command"));
    };
    let path = which(prog).ok_or_else(|| Error::new(Code::NotInstalled, format!("{prog} was not found on PATH")))?;
    let out = Command::new(&path)
        .args(args)
        .stdin(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| Error::new(Code::CommandFailed, format!("{prog}: {e}")))?;
    Ok(Output {
        ok: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// Runs an interactive command (sign-in) attached to the current terminal.
pub fn run_interactive(cmd: &[String], env: &[(String, String)]) -> Result<bool> {
    let Some((prog, args)) = cmd.split_first() else { return Ok(true) };
    let path = which(prog).ok_or_else(|| Error::new(Code::NotInstalled, format!("{prog} was not found on PATH")))?;
    let status = Command::new(&path)
        .args(args)
        .envs(env.iter().map(|(k, v)| (k, v)))
        .status()
        .map_err(|e| Error::new(Code::CommandFailed, format!("{prog}: {e}")))?;
    Ok(status.success())
}

#[cfg(test)]
mod tests {
    use super::wildcard;

    #[test]
    fn wildcards() {
        assert!(wildcard("*\\.local\\bin\\claude.exe", "C:\\Users\\k\\.local\\bin\\Claude.exe"));
        assert!(!wildcard("*\\.local\\bin\\claude.exe", "C:\\Program Files\\Claude\\claude.exe"));
        assert!(wildcard("discord.exe", "Discord.exe"));
        assert!(wildcard("git*.exe", "GitHubDesktop.exe"));
    }
}
