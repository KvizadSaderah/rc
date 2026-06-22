mod theme;
mod config;
mod types;
mod panel;
mod fileops;
mod watcher;
mod shell;
mod app;
mod input;
mod ui;

use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::Command;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    Terminal,
};

use app::App;
use input::run_app;

// =============================================================================
// CLI: --help, --version, update
// =============================================================================

const VERSION: &str = env!("CARGO_PKG_VERSION");
const REPO: &str = "KvizadSaderah/rc";

fn print_help() {
    println!(
        "\x1b[1;36mRust Commander\x1b[0m v{VERSION}  —  dual-pane TUI file manager\n\
         \n\
         \x1b[1mUSAGE:\x1b[0m\n\
         \x1b[36m  rc\x1b[0m                Launch the file manager\n\
         \x1b[36m  rc update\x1b[0m          Self-update to the latest release\n\
         \x1b[36m  rc -h, --help\x1b[0m      Show this help\n\
         \x1b[36m  rc -V, --version\x1b[0m   Print version\n\
         \n\
         \x1b[1mKEYBOARDS:\x1b[0m\n\
         \x1b[36m  F1\x1b[0m  Help    \x1b[36mF2\x1b[0m  Props   \x1b[36mF3\x1b[0m  View    \x1b[36mF4\x1b[0m  Edit\n\
         \x1b[36m  F5\x1b[0m  Copy    \x1b[36mF6\x1b[0m  Move    \x1b[36mF7\x1b[0m  MkDir   \x1b[36mF8\x1b[0m  Delete\n\
         \x1b[36m  F9\x1b[0m  Menu    \x1b[36mTab\x1b[0m Switch  \x1b[36mSpace\x1b[0m Tag   \x1b[36mCtrl+A\x1b[0m All\n\
         \x1b[36m  Ctrl+S\x1b[0m  Settings    \x1b[36mCtrl+O\x1b[0m  Shell    \x1b[36mCtrl+B\x1b[0m  Bookmarks\n\
         \x1b[36m  Ctrl+T\x1b[0m  Tree view   \x1b[36mCtrl+P\x1b[0m  Preview  \x1b[36mCtrl+D\x1b[0m  Bookmark dir\n\
         \x1b[36m  Ctrl+U\x1b[0m  Swap panels \x1b[36m=\x1b[0m       Sync     \x1b[36m~\x1b[0m       Home\n\
         \x1b[36m  Ctrl+Y\x1b[0m  Copy current panel path to clipboard\n\
         \x1b[2m  Copy/Move/Delete run in the background — Esc or c cancels.\x1b[0m\n\
         \n\
         \x1b[1mCONFIG:\x1b[0m  ~/.config/rust-commander/config.ini\n\
         \x1b[1mREPO:\x1b[0m    https://github.com/{REPO}"
    );
}

fn self_update() -> Result<(), Box<dyn std::error::Error>> {
    println!("\x1b[1;36m▶ Checking for updates...\x1b[0m");

    // Use GITHUB_TOKEN if set to avoid anonymous API rate limits (60 req/hr per IP)
    let token = std::env::var("GITHUB_TOKEN").ok();
    let mut curl_args = vec!["-s".to_string()];
    if let Some(ref t) = token {
        curl_args.push("-H".to_string());
        curl_args.push(format!("Authorization: Bearer {t}"));
    }
    curl_args.push(format!("https://api.github.com/repos/{REPO}/releases/latest"));

    let output = Command::new("curl")
        .args(&curl_args)
        .output()?;

    let body = String::from_utf8_lossy(&output.stdout);

    if body.contains("rate limit exceeded") {
        println!("\x1b[31m✗ GitHub API rate limit exceeded.\x1b[0m");
        println!("  Set GITHUB_TOKEN env var to increase the limit:");
        println!("  \x1b[36mexport GITHUB_TOKEN=your_token\x1b[0m");
        return Ok(());
    }

    let remote_tag = body
        .split("\"tag_name\"")
        .nth(1)
        .and_then(|s| s.split('"').nth(1))
        .unwrap_or("")
        .trim_start_matches('v');

    if remote_tag.is_empty() {
        println!("\x1b[31m✗ Could not fetch release info from GitHub.\x1b[0m");
        return Ok(());
    }

    let current = VERSION.trim_start_matches('v');
    println!("  Current version : v{current}");
    println!("  Latest release  : v{remote_tag}");

    if current == remote_tag {
        println!("\x1b[32m✓ Already up to date.\x1b[0m");
        return Ok(());
    }

    println!("\x1b[33m⟳ Updating to v{remote_tag}...\x1b[0m");

    let os = std::env::consts::OS;
    match os {
        "macos" => {
            let url = format!(
                "https://github.com/{REPO}/releases/download/v{remote_tag}/rc-macos.tar.gz"
            );
            let install_dir = format!("{}/.local/bin", env::var("HOME")?);
            fs::create_dir_all(&install_dir)?;

            let tmp = env::temp_dir().join("rc-update");
            let _ = fs::remove_dir_all(&tmp);
            fs::create_dir_all(&tmp)?;

            let tar_path = tmp.join("rc-macos.tar.gz");
            let status = Command::new("curl")
                .args(["-L", "-s", "-o", &tar_path.to_string_lossy(), &url])
                .status()?;
            if !status.success() {
                println!("\x1b[31m✗ Download failed.\x1b[0m");
                return Ok(());
            }

            let status = Command::new("tar")
                .args(["-xzf", &tar_path.to_string_lossy(), "-C", &tmp.to_string_lossy()])
                .status()?;
            if !status.success() {
                println!("\x1b[31m✗ Extraction failed.\x1b[0m");
                return Ok(());
            }

            let bin_src = tmp.join("rc");
            let bin_dst = PathBuf::from(&install_dir).join("rc");
            if bin_src.exists() {
                fs::copy(&bin_src, &bin_dst)?;
                #[cfg(unix)]
                {
                    let mut perms = fs::metadata(&bin_dst)?.permissions();
                    perms.set_mode(0o755);
                    fs::set_permissions(&bin_dst, perms)?;
                }
                println!("\x1b[32m✓ Updated to v{remote_tag}  →  {}\x1b[0m", bin_dst.display());
            } else {
                println!("\x1b[31m✗ Binary not found in release archive.\x1b[0m");
            }
            let _ = fs::remove_dir_all(&tmp);
        }
        "linux" => {
            println!("  Compiling from source via cargo...");
            let status = Command::new("cargo")
                .args([
                    "install", "--git",
                    &format!("https://github.com/{REPO}.git"),
                    "--root",
                    &format!("{}/.local", env::var("HOME")?),
                ])
                .status()?;
            if status.success() {
                println!("\x1b[32m✓ Updated to v{remote_tag}\x1b[0m");
            } else {
                println!("\x1b[31m✗ Cargo build failed.\x1b[0m");
            }
        }
        _ => {
            println!("\x1b[31m✗ Unsupported platform: {os}\x1b[0m");
        }
    }
    Ok(())
}

// =============================================================================
// Main Entrypoint
// =============================================================================

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();

    if args.len() > 1 {
        match args[1].as_str() {
            "-h" | "--help" | "help" => {
                print_help();
                return Ok(());
            }
            "-V" | "--version" | "version" => {
                println!("rc {VERSION}");
                return Ok(());
            }
            "update" | "self-update" => {
                return self_update();
            }
            other => {
                eprintln!("Unknown argument: {other}");
                eprintln!("Run \x1b[1mrc --help\x1b[0m for usage.");
                std::process::exit(1);
            }
        }
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let app = App::new();
    let res = run_app(&mut terminal, app);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("Application Error: {:?}", err);
    }

    Ok(())
}
