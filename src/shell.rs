// =============================================================================
// Shared shell-command plumbing
//
// Both the ':' command line and the Ctrl+O terminal overlay used to carry their
// own hardcoded lists of "interactive" / "TUI" programs and their own copies of
// the terminal suspend/resume dance. The two lists had drifted apart, so a
// program treated as interactive in one path would hang or misbehave in the
// other. This module is the single source of truth for:
//   * which programs need a controlling TTY,
//   * suspending/restoring the alternate screen around a foreground child, and
//   * spawning a piped child safely (stdin = null) so an unexpected TUI exits
//     instead of hanging the UI on a read that never returns.
// =============================================================================

use std::env;
use std::io;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

/// Programs that require a controlling terminal: full-screen TUIs, pagers,
/// editors, REPLs, multiplexers and interactive network/debug tools. These run
/// in the foreground with the real TTY; everything else is captured/streamed.
const TTY_PROGRAMS: &[&str] = &[
    // editors
    "vim", "vi", "nvim", "neovim", "nano", "emacs", "helix", "hx", "micro", "kak", "kakoune",
    // pagers / viewers
    "less", "more", "most", "man",
    // system monitors
    "top", "htop", "btop", "btm", "glances", "atop",
    // file managers / TUIs
    "mc", "ranger", "nnn", "vifm", "lf", "yazi", "broot",
    // git / docker / k8s TUIs
    "tig", "lazygit", "gitui", "lazydocker", "k9s",
    // REPLs / shells
    "python", "python3", "ipython", "node", "irb", "lua", "ghci",
    "bash", "zsh", "fish", "sh",
    // pickers / multiplexers / prompts
    "fzf", "tmux", "screen", "watch", "dialog", "whiptail",
    // interactive network / remote
    "ssh", "sftp", "telnet", "ftp", "nc", "ncftp",
    // debuggers
    "gdb", "lldb",
];

/// The leading program name of a command, stripped of path components and of
/// leading `VAR=value` environment assignments and quotes.
pub fn first_program(cmd: &str) -> String {
    let mut parts = cmd.split_whitespace();
    let mut word = parts.next().unwrap_or("");
    while word.contains('=') {
        word = parts.next().unwrap_or("");
    }
    let word = word.trim_matches(|c| c == '\'' || c == '"');
    Path::new(word)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(word)
        .to_string()
}

/// Best-effort: does this command need a controlling terminal?
pub fn needs_tty(cmd: &str) -> bool {
    TTY_PROGRAMS.contains(&first_program(cmd).as_str())
}

/// Is this our own binary (used to block recursive `rc` launches)?
pub fn is_self(cmd: &str) -> bool {
    matches!(first_program(cmd).as_str(), "rc" | "rust-commander")
}

pub fn user_shell() -> String {
    #[cfg(windows)]
    {
        "cmd.exe".to_string()
    }
    #[cfg(not(windows))]
    {
        env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

fn shell_flag() -> &'static str {
    if cfg!(windows) {
        "/c"
    } else {
        "-c"
    }
}

/// Build a `Command` that runs `cmd` through the user's shell in `dir`.
/// Stdio is left at defaults: `.status()` inherits, `.output()` captures.
pub fn build(cmd: &str, dir: &Path) -> Command {
    let mut c = Command::new(user_shell());
    c.arg(shell_flag()).arg(cmd).current_dir(dir);
    c
}

/// Hand the real terminal to a child program (leave alt-screen, raw off).
pub fn suspend() {
    let _ = disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        crossterm::cursor::Show
    );
}

/// Reclaim the terminal for the TUI (raw on, alt-screen, cursor hidden).
pub fn resume() {
    let _ = enable_raw_mode();
    let _ = execute!(
        io::stdout(),
        EnterAlternateScreen,
        EnableMouseCapture,
        crossterm::cursor::Hide
    );
}

/// Run a command in the foreground with full TTY access (TUIs / REPLs),
/// suspending and restoring the alternate screen around it.
pub fn run_foreground(cmd: &str, dir: &Path) -> io::Result<ExitStatus> {
    suspend();
    let result = build(cmd, dir)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();
    resume();
    result
}

/// Spawn a command with piped stdout/stderr for streaming. stdin is null so a
/// program that unexpectedly wants a TTY exits promptly instead of hanging.
pub fn spawn_piped(cmd: &str, dir: &Path) -> io::Result<Child> {
    build(cmd, dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_first_program() {
        assert_eq!(first_program("ls -la"), "ls");
        assert_eq!(first_program("/usr/bin/vim file"), "vim");
        assert_eq!(first_program("FOO=1 BAR=2 git status"), "git");
        assert_eq!(first_program("'vim' arg"), "vim");
        assert_eq!(first_program(""), "");
    }

    #[test]
    fn test_needs_tty() {
        assert!(needs_tty("vim file.txt"));
        assert!(needs_tty("htop"));
        assert!(needs_tty("/usr/local/bin/nvim"));
        assert!(needs_tty("python3"));
        assert!(!needs_tty("ls -la"));
        assert!(!needs_tty("git status"));
        assert!(!needs_tty("echo hi"));
        assert!(!needs_tty("cargo build"));
    }

    #[test]
    fn test_is_self() {
        assert!(is_self("rc"));
        assert!(is_self("/usr/bin/rc"));
        assert!(is_self("rust-commander"));
        assert!(!is_self("rg pattern"));
    }
}
