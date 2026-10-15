//! A click on a session brings up the window it runs in: walk up from the session's process (claude → shell →
//! terminal / IDE) to the first process that owns visible windows, and put the right one in front. An IDE has one
//! window per project, so among that process's windows the one whose title names the session's folder wins
//! (IntelliJ: "statics_be – console_60" for a session in …/statics_be); without a match, the first one.
//! Best effort: it finds the window, not a particular tab or tool window inside it — except Terminal and iTerm2 on
//! macOS, where the tab is found by the session's tty.

/// The process and its parents, nearest first (at most 12 steps; a parent loop can't hang it), with their executables.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn ancestors(pid: u32) -> Vec<(u32, Option<std::path::PathBuf>)> {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let mut out: Vec<(u32, Option<std::path::PathBuf>)> = vec![];
    let mut cur = Some(sysinfo::Pid::from_u32(pid));
    while let Some(p) = cur {
        if out.len() >= 12 || out.iter().any(|(q, _)| *q == p.as_u32()) {
            break;
        }
        let proc = sys.process(p);
        out.push((p.as_u32(), proc.and_then(|x| x.exe()).map(|e| e.to_path_buf())));
        cur = proc.and_then(|x| x.parent());
    }
    out
}

/// The .app bundle an executable lives in ("/Applications/IntelliJ IDEA.app/Contents/MacOS/idea" → the .app).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn app_bundle(exe: &std::path::Path) -> Option<std::path::PathBuf> {
    let s = exe.to_str()?;
    let end = s.find(".app/Contents/MacOS/")?;
    Some(std::path::PathBuf::from(&s[..end + 4]))
}

/// The folder names of the session's path, the deepest first ("E:\w\statics_be\api" → api, statics_be, w),
/// lowercase; drive letters and one-letter names are skipped.
fn folder_names(cwd: &str) -> Vec<String> {
    cwd.split(['/', '\\'])
        .filter(|p| p.len() > 1 && !p.ends_with(':'))
        .rev()
        .map(|p| p.to_lowercase())
        .collect()
}

/// The best of a process's windows for the session: the deepest folder name its title contains, else the first.
fn pick<'a, T>(windows: &'a [(T, String)], cwd: &str) -> Option<&'a T> {
    for name in folder_names(cwd) {
        if let Some((w, _)) = windows.iter().find(|(_, title)| title.to_lowercase().contains(&name)) {
            return Some(w);
        }
    }
    windows.first().map(|(w, _)| w)
}

#[cfg(windows)]
pub fn focus(pid: u32, cwd: &str) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindow, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
        IsWindowVisible, SetForegroundWindow, ShowWindow, GW_OWNER, SW_RESTORE,
    };

    struct Search {
        pid: u32,
        found: Vec<(HWND, String)>,
    }
    // EnumWindows calls this for every top-level window; it collects the visible, titled, unowned ones of `pid`
    unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = &mut *(lparam as *mut Search);
        let mut owner_pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut owner_pid);
        let len = GetWindowTextLengthW(hwnd);
        if owner_pid == search.pid && IsWindowVisible(hwnd) != 0 && len > 0 && GetWindow(hwnd, GW_OWNER).is_null() {
            let mut buf = vec![0u16; len as usize + 1];
            let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
            search.found.push((hwnd, String::from_utf16_lossy(&buf[..n.max(0) as usize])));
        }
        1
    }

    for (p, _) in ancestors(pid) {
        let mut search = Search { pid: p, found: vec![] };
        // SAFETY: `search` outlives the call; the callback only writes into it
        unsafe { EnumWindows(Some(each), &mut search as *mut Search as LPARAM) };
        if let Some(&hwnd) = pick(&search.found, cwd) {
            unsafe {
                if IsIconic(hwnd) != 0 {
                    ShowWindow(hwnd, SW_RESTORE);
                }
                // allowed: the widget is the foreground window right after your click
                SetForegroundWindow(hwnd);
            }
            return Ok(());
        }
    }
    Err("no window found".into())
}

/// One `ps` column of a process ("" columns and dead processes → None). Unlike sysinfo, ps also reads processes of
/// other users: Terminal and iTerm2 start the shell through /usr/bin/login, which runs as root, and sysinfo gave it
/// no parent — the walk up stopped there and the click did nothing.
#[cfg(target_os = "macos")]
fn ps(pid: u32, field: &str) -> Option<String> {
    let out = std::process::Command::new("ps")
        .args(["-o", &format!("{field}="), "-p", &pid.to_string()])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// `ancestors` for macOS, through ps (comm = the executable's full path for apps).
#[cfg(target_os = "macos")]
fn mac_ancestors(pid: u32) -> Vec<(u32, Option<std::path::PathBuf>)> {
    let mut out: Vec<(u32, Option<std::path::PathBuf>)> = vec![];
    let mut cur = pid;
    while cur > 1 && out.len() < 12 && !out.iter().any(|(q, _)| *q == cur) {
        out.push((cur, ps(cur, "comm").map(std::path::PathBuf::from)));
        match ps(cur, "ppid").and_then(|s| s.parse().ok()) {
            Some(parent) => cur = parent,
            None => break,
        }
    }
    out
}

/// AppleScript that selects the tab running on `tty` in Terminal or iTerm2 and brings its window up (prints "ok" when
/// it found it); None for other apps.
#[cfg(target_os = "macos")]
fn tab_script(app: &std::path::Path, tty: &str) -> Option<String> {
    match app.file_name()?.to_str()? {
        "Terminal.app" => Some(format!(
            "tell application id \"com.apple.Terminal\"\n repeat with w in windows\n  repeat with t in tabs of w\n   if tty of t is \"{tty}\" then\n    set miniaturized of w to false\n    set selected of t to true\n    set index of w to 1\n    activate\n    return \"ok\"\n   end if\n  end repeat\n end repeat\nend tell"
        )),
        "iTerm.app" => Some(format!(
            "tell application id \"com.googlecode.iterm2\"\n repeat with w in windows\n  repeat with t in tabs of w\n   repeat with s in sessions of t\n    if tty of s is \"{tty}\" then\n     select w\n     tell t to select\n     tell s to select\n     activate\n     return \"ok\"\n    end if\n   end repeat\n  end repeat\n end repeat\nend tell"
        )),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
pub fn focus(pid: u32, cwd: &str) -> Result<(), String> {
    // The first ancestor inside an .app bundle (Terminal, iTerm2, an IDE…) is the app.
    let (p, app) = mac_ancestors(pid)
        .into_iter()
        .find_map(|(p, exe)| exe.as_deref().and_then(app_bundle).map(|app| (p, app)))
        .ok_or("no app found")?;
    // Terminal / iTerm2: the exact tab, found by the session's terminal (ttys001) — asks for Automation once
    let tty = ps(pid, "tty").filter(|t| t.starts_with("ttys") && t.chars().all(|c| c.is_ascii_alphanumeric()));
    if let Some(script) = tty.and_then(|t| tab_script(&app, &format!("/dev/{t}"))) {
        let found = std::process::Command::new("osascript")
            .args(["-e", &script])
            .stderr(std::process::Stdio::null())
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "ok")
            .unwrap_or(false);
        if found {
            return Ok(());
        }
    }
    // Otherwise (an IDE, or the tab wasn't found): `open` on the bundle brings the running instance to the front — no
    // permission needed. Then its window naming the session's folder is raised via System Events, which needs
    // Automation + Accessibility (macOS asks once); without them the app still comes up.
    let opened = std::process::Command::new("open")
        .arg(&app)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let raise = folder_names(cwd)
        .iter()
        .map(|n| {
            let n = n.replace('\\', "").replace('"', "");
            format!("try\n perform action \"AXRaise\" of (first window whose name contains \"{n}\")\n return\n end try\n")
        })
        .collect::<String>();
    let script = format!(
        "tell application \"System Events\"\n set p to first process whose unix id is {p}\n set frontmost of p to true\n tell p\n{raise} end tell\nend tell"
    );
    let raised = std::process::Command::new("osascript")
        .args(["-e", &script])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if opened || raised {
        Ok(())
    } else {
        Err("no window found".into())
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn focus(pid: u32, cwd: &str) -> Result<(), String> {
    // X11 with xdotool (or wmctrl): the process's windows, the one naming the folder first (Wayland can't raise others)
    use std::process::Command;
    for (p, _) in ancestors(pid) {
        let ids = Command::new("xdotool").args(["search", "--onlyvisible", "--pid", &p.to_string()]).output();
        let windows: Vec<(String, String)> = match ids {
            Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(|id| {
                    let title = Command::new("xdotool")
                        .args(["getwindowname", id])
                        .output()
                        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                        .unwrap_or_default();
                    (id.to_string(), title)
                })
                .collect(),
            _ => vec![],
        };
        if let Some(id) = pick(&windows, cwd) {
            let ok = Command::new("xdotool").args(["windowactivate", id.as_str()]).status().map(|s| s.success()).unwrap_or(false);
            if ok {
                return Ok(());
            }
        }
        let ok = Command::new("sh")
            .arg("-c")
            .arg(format!("w=$(wmctrl -lp 2>/dev/null | awk '$3=={p}{{print $1; exit}}') && [ -n \"$w\" ] && wmctrl -ia \"$w\""))
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return Ok(());
        }
    }
    Err("no window found (install xdotool or wmctrl)".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_naming_the_deepest_folder_wins() {
        let windows = vec![
            (1, "react-electron – App.jsx".to_string()),
            (2, "statics_be – console_60 [STATICS_PROD]".to_string()),
            (3, "new – something".to_string()),
        ];
        assert_eq!(pick(&windows, r"E:\work\new\new\new\statics_be"), Some(&2));
        assert_eq!(pick(&windows, "/home/me/react-electron/src"), Some(&1));
        // no folder in any title: the first window
        assert_eq!(pick(&windows, "/opt/other"), Some(&1));
        assert_eq!(pick::<i32>(&[], "/x"), None);
    }

    #[test]
    fn the_app_bundle_of_an_executable() {
        let app = |s: &str| app_bundle(std::path::Path::new(s)).map(|p| p.to_string_lossy().into_owned());
        assert_eq!(app("/Applications/IntelliJ IDEA.app/Contents/MacOS/idea").as_deref(), Some("/Applications/IntelliJ IDEA.app"));
        assert_eq!(app("/bin/zsh"), None);
    }

    #[test]
    fn folder_names_skip_drives_and_single_letters() {
        assert_eq!(folder_names(r"E:\work\a\statics_be"), vec!["statics_be", "work"]);
    }
}
