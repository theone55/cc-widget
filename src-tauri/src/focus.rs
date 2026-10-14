//! A click on a session brings up the window it runs in: walk up from the session's process (claude → shell →
//! terminal / IDE) to the first process that owns visible windows, and put the right one in front. An IDE has one
//! window per project, so among that process's windows the one whose title names the session's folder wins
//! (IntelliJ: "statics_be – console_60" for a session in …/statics_be); without a match, the first one.
//! Best effort: it finds the window, not a particular tab or tool window inside it.

/// The process and its parents, nearest first (at most 12 steps; a parent loop can't hang it).
fn ancestors(pid: u32) -> Vec<u32> {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let mut out = vec![];
    let mut cur = Some(sysinfo::Pid::from_u32(pid));
    while let Some(p) = cur {
        if out.len() >= 12 || out.contains(&p.as_u32()) {
            break;
        }
        out.push(p.as_u32());
        cur = sys.process(p).and_then(|proc| proc.parent());
    }
    out
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

    for p in ancestors(pid) {
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

#[cfg(target_os = "macos")]
pub fn focus(pid: u32, cwd: &str) -> Result<(), String> {
    // the first ancestor that is an app System Events knows (Terminal, iTerm2, an IDE…) comes to the front, and its
    // window naming the session's folder is raised
    let names = folder_names(cwd);
    for p in ancestors(pid) {
        let raise = names
            .iter()
            .map(|n| {
                let n = n.replace('\\', "").replace('"', "");
                format!("try\n perform action \"AXRaise\" of (first window whose name contains \"{n}\")\n return\n end try\n")
            })
            .collect::<String>();
        let script = format!(
            "tell application \"System Events\"\n set p to first process whose unix id is {p}\n set frontmost of p to true\n tell p\n{raise} end tell\nend tell"
        );
        let ok = std::process::Command::new("osascript")
            .args(["-e", &script])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return Ok(());
        }
    }
    Err("no window found".into())
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn focus(pid: u32, cwd: &str) -> Result<(), String> {
    // X11 with xdotool (or wmctrl): the process's windows, the one naming the folder first (Wayland can't raise others)
    use std::process::Command;
    for p in ancestors(pid) {
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
    fn folder_names_skip_drives_and_single_letters() {
        assert_eq!(folder_names(r"E:\work\a\statics_be"), vec!["statics_be", "work"]);
    }
}
