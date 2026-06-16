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

/// Whether a `/proc` comm name identifies a known terminal emulator.
///
/// The `/proc/<pid>/status` `Name` field is truncated to 15 chars
/// (`TASK_COMM_LEN`), so `gnome-terminal-server` (21 chars) arrives as
/// `gnome-terminal-` (15). Match on exact equality, OR — when the name is
/// exactly 15 chars (i.e. possibly truncated) — when a known terminal name
/// starts with it.
fn is_known_terminal(name: &str) -> bool {
    KNOWN_TERMINALS
        .iter()
        .any(|t| name == *t || (name.len() == 15 && t.starts_with(name)))
}

/// Whether a `/proc` comm name identifies the tmux server/client process. The
/// comm name is `"tmux: server"` / `"tmux: client"`, so the colon distinguishes
/// it from unrelated names like `tmuxinator`.
fn is_tmux(name: &str) -> bool {
    name.starts_with("tmux:")
}

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

        // Check if this is a known terminal (truncation-aware — see is_known_terminal).
        if is_known_terminal(&name) {
            return Some(pid);
        }

        // Tmux-aware: if we hit the tmux server, find the client PID
        if is_tmux(&name) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_exact_terminal_names() {
        assert!(is_known_terminal("kitty"));
        assert!(is_known_terminal("alacritty"));
        assert!(is_known_terminal("gnome-terminal-server"));
        assert!(is_known_terminal("xfce4-terminal"));
        assert!(is_known_terminal("st"));
    }

    #[test]
    fn matches_15char_truncated_gnome_terminal_server() {
        // /proc Name is truncated to 15 chars (TASK_COMM_LEN), so
        // "gnome-terminal-server" (21) becomes "gnome-terminal-" (15). The
        // truncation-aware match must still recognize it. This is the exact
        // regression the 15-char clause fixes (process_tree.rs:49-51).
        assert_eq!("gnome-terminal-".len(), 15);
        assert!(is_known_terminal("gnome-terminal-"));
    }

    #[test]
    fn rejects_non_terminals() {
        assert!(!is_known_terminal("bash"));
        assert!(!is_known_terminal("zsh"));
        assert!(!is_known_terminal("node"));
        assert!(!is_known_terminal("claude-focus"));
        assert!(!is_known_terminal(""));
    }

    #[test]
    fn truncation_clause_only_fires_at_15_chars() {
        // A non-15-char prefix of a known terminal must NOT match — only a
        // genuinely truncated 15-char comm name may use the prefix rule.
        // (Note: "gnome-terminal" itself is an exact KNOWN_TERMINALS entry, so
        // it legitimately matches; use prefixes that are NOT exact entries.)
        assert!(!is_known_terminal("gnome")); // 5-char prefix
        assert!(!is_known_terminal("gnome-termina")); // 13-char prefix, not an exact entry
        assert!(!is_known_terminal("alacritt")); // 8-char prefix of "alacritty"
    }

    #[test]
    fn detects_tmux_server_and_clients() {
        assert!(is_tmux("tmux: server"));
        assert!(is_tmux("tmux: client"));
    }

    #[test]
    fn rejects_non_tmux_names() {
        assert!(!is_tmux("tmux")); // bare name has no colon — not the server/client comm
        assert!(!is_tmux("tmuxinator"));
        assert!(!is_tmux("bash"));
        assert!(!is_tmux(""));
    }
}
