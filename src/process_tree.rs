use std::collections::HashSet;
use std::fs;
use std::process::Command;

const KNOWN_TERMINALS: &[&str] = &[
    "gnome-terminal-server",
    "gnome-terminal",
    "kitty",
    "alacritty",
    "wezterm-gui",
    "wezterm",
    "foot",
    "konsole",
    "xterm",
    "tilix",
    "terminator",
    "xfce4-terminal",
    "mate-terminal",
    "lxterminal",
    "sakura",
    "st",
    "urxvt",
    "rxvt",
];

/// Walk the process tree from the current process upward to find the terminal PID.
/// Tmux-aware: if we encounter a tmux server, we find the client PID and continue from there.
pub fn find_terminal_pid() -> Option<u32> {
    let start_pid = std::process::id();
    let mut visited = HashSet::new();
    walk_tree(start_pid, &mut visited)
}

fn walk_tree(start_pid: u32, visited: &mut HashSet<u32>) -> Option<u32> {
    let mut pid = start_pid;

    loop {
        if !visited.insert(pid) {
            // Already visited this PID — cycle detected
            return None;
        }

        let name = read_proc_name(pid)?;

        // Check if this is a known terminal.
        // /proc Name field is truncated to 15 chars (TASK_COMM_LEN), so
        // "gnome-terminal-server" becomes "gnome-terminal-". Handle this
        // by also checking if a known terminal starts with the truncated name.
        if KNOWN_TERMINALS.iter().any(|t| {
            name == *t || (name.len() == 15 && t.starts_with(&name))
        }) {
            return Some(pid);
        }

        // Tmux-aware: if we hit the tmux server, find the client PID
        if name == "tmux: server" || name.starts_with("tmux:") {
            if let Some(client_pid) = find_tmux_client_pid() {
                if let Some(result) = walk_tree(client_pid, visited) {
                    return Some(result);
                }
            }
            // If tmux path didn't find a terminal, continue up from the server's parent
        }

        // Move to parent
        let ppid = read_ppid(pid)?;
        if ppid == 0 || ppid == pid {
            return None;
        }
        pid = ppid;
    }
}

fn read_proc_name(pid: u32) -> Option<String> {
    let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    for line in status.lines() {
        if let Some(name) = line.strip_prefix("Name:\t") {
            return Some(name.trim().to_string());
        }
    }
    None
}

fn read_ppid(pid: u32) -> Option<u32> {
    let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    for line in status.lines() {
        if let Some(ppid_str) = line.strip_prefix("PPid:\t") {
            return ppid_str.trim().parse().ok();
        }
    }
    None
}

fn find_tmux_client_pid() -> Option<u32> {
    // Try session-aware lookup first via $TMUX_PANE
    if let Ok(pane) = std::env::var("TMUX_PANE") {
        if let Some(pid) = find_tmux_client_for_pane(&pane) {
            return Some(pid);
        }
    }

    // Fallback: first client from any session
    let output = Command::new("tmux")
        .args(["list-clients", "-F", "#{client_pid}"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().next()?.trim().parse().ok()
}

fn find_tmux_client_for_pane(pane: &str) -> Option<u32> {
    // Get the session ID for this pane
    let session_output = Command::new("tmux")
        .args(["display-message", "-p", "-t", pane, "#{session_id}"])
        .output()
        .ok()?;

    if !session_output.status.success() {
        return None;
    }

    let session_id = String::from_utf8_lossy(&session_output.stdout)
        .trim()
        .to_string();

    if session_id.is_empty() {
        return None;
    }

    // List clients attached to this specific session
    let client_output = Command::new("tmux")
        .args(["list-clients", "-t", &session_id, "-F", "#{client_pid}"])
        .output()
        .ok()?;

    if !client_output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&client_output.stdout);
    stdout.lines().next()?.trim().parse().ok()
}
