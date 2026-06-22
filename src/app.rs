use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::sync::mpsc;
use std::path::{Path, PathBuf};

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use crate::config::*;
use crate::fileops::{self, JobState, OpKind};
use crate::panel::*;
use crate::types::*;

// =============================================================================
// App Controller State
// =============================================================================

pub struct App {
    pub left_panel: Panel,
    pub right_panel: Panel,
    pub active_panel: ActivePanel,
    pub dialog: Dialog,
    pub status_message: String,
    pub should_quit: bool,
    pub config: Config,
    pub keymap: Keymap,
    pub preview_mode: bool,
    pub tree_mode: bool,
    pub tree_nodes: Vec<TreeNode>,
    pub tree_selected: usize,
    pub preview_cache: Option<PreviewCache>,
    pub running_process: Option<RunningProcess>,
    pub preview_scroll_offset: usize,
    pub fs_job: Option<JobState>,
}

impl App {
    pub fn new() -> Self {
        let config = load_config();
        let keymap = load_keymap(&config);
        
        let mut current_dir = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        if !current_dir.exists() {
            while !current_dir.exists() {
                if let Some(parent) = current_dir.parent() {
                    current_dir = parent.to_path_buf();
                } else {
                    current_dir = env::var("HOME").ok().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
                    break;
                }
            }
        }
        Self {
            left_panel: Panel::new(current_dir.clone(), config.show_hidden, config.sort_by.clone()),
            right_panel: Panel::new(current_dir, config.show_hidden, config.sort_by.clone()),
            active_panel: ActivePanel::Left,
            dialog: Dialog::None,
            status_message: String::new(),
            should_quit: false,
            config,
            keymap,
            preview_mode: false,
            tree_mode: false,
            tree_nodes: Vec::new(),
            tree_selected: 0,
            preview_cache: None,
            running_process: None,
            preview_scroll_offset: 0,
            fs_job: None,
        }
    }

    /// True while a background copy/move/delete job is running.
    pub fn is_fs_busy(&self) -> bool {
        self.fs_job.is_some()
    }

    /// Poll the background fs job; on completion refresh panels, clear marks,
    /// and surface a status message (including any collected per-item errors).
    pub fn drain_fs_job(&mut self) {
        let just_finished = match self.fs_job {
            Some(ref mut job) => job.poll(),
            None => false,
        };
        if !just_finished {
            return;
        }
        if let Some(job) = self.fs_job.take() {
            let err_count = job.errors.len();
            if job.is_cancelled() {
                self.status_message = format!("{} cancelled", job.kind.verb());
            } else if err_count > 0 {
                let first = job.errors.first().cloned().unwrap_or_default();
                self.status_message = format!(
                    "{} finished with {} error(s): {}",
                    job.kind.past(), err_count, first
                );
                self.dialog = Dialog::Error {
                    message: format!(
                        "{} completed with {} error(s):\n\n{}",
                        job.kind.past(), err_count,
                        job.errors.join("\n")
                    ),
                };
            } else {
                self.status_message = format!("{} {} item(s)", job.kind.past(), job.total_files);
            }
            self.get_active_panel_mut().marked.clear();
            self.refresh_panels();
        }
    }

    /// Cancel the running fs job, if any.
    pub fn cancel_fs_job(&mut self) {
        if let Some(ref job) = self.fs_job {
            job.cancel();
            self.status_message = "Cancelling…".to_string();
        }
    }

    pub fn apply_config(&mut self) {
        self.left_panel.show_hidden = self.config.show_hidden;
        self.left_panel.sort_by = self.config.sort_by.clone();

        self.right_panel.show_hidden = self.config.show_hidden;
        self.right_panel.sort_by = self.config.sort_by.clone();

        self.keymap = load_keymap(&self.config);

        self.refresh_panels();
    }

    pub fn get_active_panel_mut(&mut self) -> &mut Panel {
        match self.active_panel {
            ActivePanel::Left => &mut self.left_panel,
            ActivePanel::Right => &mut self.right_panel,
        }
    }

    pub fn get_active_panel(&self) -> &Panel {
        match self.active_panel {
            ActivePanel::Left => &self.left_panel,
            ActivePanel::Right => &self.right_panel,
        }
    }

    pub fn get_inactive_panel(&self) -> &Panel {
        match self.active_panel {
            ActivePanel::Left => &self.right_panel,
            ActivePanel::Right => &self.left_panel,
        }
    }

    pub fn refresh_panels(&mut self) {
        if self.tree_mode {
            self.init_tree();
            self.update_right_panel_from_tree();
        } else {
            self.left_panel.refresh();
            self.right_panel.refresh();
        }
    }

    pub fn toggle_panel(&mut self) {
        self.active_panel = match self.active_panel {
            ActivePanel::Left => ActivePanel::Right,
            ActivePanel::Right => ActivePanel::Left,
        };
    }

    /// Sync inactive panel to the same directory as the active panel.
    pub fn sync_panels(&mut self) {
        let active_path = self.get_active_panel().path.clone();
        let inactive = match self.active_panel {
            ActivePanel::Left => &mut self.right_panel,
            ActivePanel::Right => &mut self.left_panel,
        };
        let _ = inactive.set_path(active_path);
    }

    /// Swap left and right panel directories.
    pub fn swap_panels(&mut self) {
        let left_path = self.left_panel.path.clone();
        let right_path = self.right_panel.path.clone();
        let _ = self.left_panel.set_path(right_path);
        let _ = self.right_panel.set_path(left_path);
    }

    pub fn init_tree(&mut self) {
        let root_path = self.left_panel.path.clone();
        self.tree_nodes.clear();
        self.tree_selected = 0;

        let root_name = root_path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root_path.to_string_lossy().into_owned());
        
        let has_subdirs = has_subdirectories(&root_path);
        self.tree_nodes.push(TreeNode {
            path: root_path.clone(),
            name: root_name,
            depth: 0,
            is_expanded: true,
            has_subdirs,
        });

        if let Ok(entries) = fs::read_dir(&root_path) {
            let mut subdirs = Vec::new();
            for entry in entries.flatten() {
                // Skip symlinks so the tree can never descend into a loop.
                let is_symlink = entry.file_type().map(|ft| ft.is_symlink()).unwrap_or(false);
                if let Ok(meta) = entry.metadata() {
                    if meta.is_dir() && !is_symlink {
                        let name = entry.file_name().to_string_lossy().into_owned();
                        if !name.starts_with('.') {
                            let path = entry.path();
                            let has_sub = has_subdirectories(&path);
                            subdirs.push(TreeNode {
                                path,
                                name,
                                depth: 1,
                                is_expanded: false,
                                has_subdirs: has_sub,
                            });
                        }
                    }
                }
            }
            subdirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            self.tree_nodes.extend(subdirs);
        }
    }

    pub fn toggle_tree_node(&mut self) {
        if self.tree_nodes.is_empty() { return; }
        let idx = self.tree_selected;
        let node = &self.tree_nodes[idx];
        if !node.has_subdirs { return; }

        if node.is_expanded {
            let depth = node.depth;
            let mut remove_count = 0;
            for next_node in self.tree_nodes.iter().skip(idx + 1) {
                if next_node.depth > depth {
                    remove_count += 1;
                } else {
                    break;
                }
            }
            for _ in 0..remove_count {
                self.tree_nodes.remove(idx + 1);
            }
            self.tree_nodes[idx].is_expanded = false;
        } else {
            let path = node.path.clone();
            let depth = node.depth;
            let mut subdirs = Vec::new();
            if let Ok(entries) = fs::read_dir(&path) {
                for entry in entries.flatten() {
                    let is_symlink = entry.file_type().map(|ft| ft.is_symlink()).unwrap_or(false);
                    if let Ok(meta) = entry.metadata() {
                        if meta.is_dir() && !is_symlink {
                            let name = entry.file_name().to_string_lossy().into_owned();
                            if !name.starts_with('.') {
                                let sub_path = entry.path();
                                let has_sub = has_subdirectories(&sub_path);
                                subdirs.push(TreeNode {
                                    path: sub_path,
                                    name,
                                    depth: depth + 1,
                                    is_expanded: false,
                                    has_subdirs: has_sub,
                                });
                            }
                        }
                    }
                }
            }
            subdirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            
            for (offset, item) in subdirs.into_iter().enumerate() {
                self.tree_nodes.insert(idx + 1 + offset, item);
            }
            self.tree_nodes[idx].is_expanded = true;
        }

        self.update_right_panel_from_tree();
    }

    pub fn update_right_panel_from_tree(&mut self) {
        if let Some(node) = self.tree_nodes.get(self.tree_selected) {
            let path = node.path.clone();
            self.right_panel.path = path;
            self.right_panel.refresh();
            self.right_panel.selected = 0;
            self.right_panel.scroll_state.select(Some(0));
        }
    }

    // Returns (output_lines, needs_terminal_clear)
    // needs_terminal_clear is true when a full-screen TUI program was run
    // and ratatui must call terminal.clear() before the next draw.
    pub fn execute_overlay_command(active_dir: &std::path::Path, cmd: &str) -> (Vec<String>, bool, Option<std::process::Child>) {
        let mut lines = Vec::new();

        // Guard: prevent launching rc inside rc (recursion).
        if crate::shell::is_self(cmd) {
            lines.push("⚠ Cannot launch rc inside rc (would cause recursion).".to_string());
            lines.push("  Use a separate terminal window instead.".to_string());
            return (lines, false, None);
        }

        // Full-screen / interactive programs need the real TTY — run them in
        // the foreground (suspending the alt-screen) instead of capturing them.
        if crate::shell::needs_tty(cmd) {
            let prog = crate::shell::first_program(cmd);
            lines.push(format!("[Launching {}...]", prog));
            let _ = crate::shell::run_foreground(cmd, active_dir);
            // needs_clear = true so the caller calls terminal.clear() before redraw.
            lines.push(format!("[{} exited — press Esc to return to rc]", prog));
            return (lines, true, None);
        }

        // Everything else streams via a piped child (stdin = null so an
        // unexpected TUI can't hang the overlay waiting on input).
        match crate::shell::spawn_piped(cmd, active_dir) {
            Ok(child) => (lines, false, Some(child)),
            Err(e) => {
                lines.push(format!("Failed to execute command: {}", e));
                (lines, false, None)
            }
        }
    }

    // Wrap a spawned child process into a RunningProcess with threaded readers
    pub fn start_streaming(child: std::process::Child) -> RunningProcess {
        let (tx, rx) = mpsc::channel::<String>();

        let mut child = child;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        // Reader thread for stdout
        if let Some(stdout) = stdout {
            let tx_out = tx.clone();
            std::thread::spawn(move || {
                let reader = io::BufReader::new(stdout);
                for line in reader.lines() {
                    match line {
                        Ok(l) => { let _ = tx_out.send(l); }
                        Err(_) => break,
                    }
                }
            });
        }

        // Reader thread for stderr
        if let Some(stderr) = stderr {
            let tx_err = tx;
            std::thread::spawn(move || {
                let reader = io::BufReader::new(stderr);
                for line in reader.lines() {
                    match line {
                        Ok(l) => { let _ = tx_err.send(format!("stderr: {}", l)); }
                        Err(_) => break,
                    }
                }
            });
        }

        RunningProcess { child, receiver: rx, done: false }
    }

    // Drain available output from a running process (non-blocking)
    pub fn drain_process_output(&mut self) {
        let mut new_lines: Vec<String> = Vec::new();
        let mut process_done = false;

        if let Some(ref mut proc) = self.running_process {
            // Drain all available lines (non-blocking)
            loop {
                match proc.receiver.try_recv() {
                    Ok(line) => new_lines.push(line),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        proc.done = true;
                        break;
                    }
                }
            }

            // Check if child process has exited
            if !proc.done {
                match proc.child.try_wait() {
                    Ok(Some(status)) => {
                        proc.done = true;
                        // Drain remaining lines after exit
                        while let Ok(line) = proc.receiver.try_recv() {
                            new_lines.push(line);
                        }
                        if !status.success() {
                            if let Some(code) = status.code() {
                                new_lines.push(format!("[Command exited with status code: {}]", code));
                            } else {
                                new_lines.push("[Command terminated by signal]".to_string());
                            }
                        }
                    }
                    Ok(None) => {} // still running
                    Err(_) => { proc.done = true; }
                }
            }

            if proc.done {
                process_done = true;
            }
        }

        // Apply new lines to TerminalOverlay dialog
        if !new_lines.is_empty() {
            if let Dialog::TerminalOverlay { output_lines, scroll_offset, .. } = &mut self.dialog {
                output_lines.extend(new_lines);
                // Cap buffer at 10000 lines to prevent memory explosion
                const MAX_LINES: usize = 10_000;
                if output_lines.len() > MAX_LINES {
                    let drain_count = output_lines.len() - MAX_LINES;
                    output_lines.drain(..drain_count);
                }
                // Auto-scroll to bottom
                let display_height = 20; // approximate
                if output_lines.len() > display_height {
                    *scroll_offset = output_lines.len() - display_height;
                }
            }
        }

        if process_done {
            self.running_process = None;
        }
    }

    // Kill a running process
    pub fn kill_running_process(&mut self) {
        if let Some(ref mut proc) = self.running_process {
            let _ = proc.child.kill();
            let _ = proc.child.wait();
            // Drain any remaining output
            while let Ok(line) = proc.receiver.try_recv() {
                if let Dialog::TerminalOverlay { output_lines, .. } = &mut self.dialog {
                    output_lines.push(line);
                }
            }
            if let Dialog::TerminalOverlay { output_lines, .. } = &mut self.dialog {
                output_lines.push("[Process killed]".to_string());
            }
        }
        self.running_process = None;
    }

    pub fn handle_enter_or_right(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, is_enter: bool) {
        let selected_item = {
            let panel = self.get_active_panel();
            panel.get_selected_item().cloned()
        };

        if let Some(item) = selected_item {
            if item.is_dir {
                let panel = self.get_active_panel_mut();
                if let Err(e) = panel.set_path(item.path.clone()) {
                    self.dialog = Dialog::Error { message: e };
                } else {
                    self.status_message = format!("Entered: {}", item.name);
                }
            } else if item.path.is_file() {
                if is_enter {
                    // Open in external editor (nvim/vim/nano — whatever is configured)
                    match edit_file(&item.path, &self.config.default_editor) {
                        Ok(_) => {
                            self.status_message = format!("Edited: {}", item.name);
                            self.refresh_panels();
                            let _ = terminal.clear();
                        }
                        Err(e) => {
                            let _ = terminal.clear();
                            self.dialog = Dialog::Error {
                                message: format!("Failed to open editor: {}", e),
                            };
                        }
                    }
                } else {
                    self.status_message = "Press Enter to edit/open file".to_string();
                }
            }
        }
    }

    pub fn handle_backspace(&mut self) {
        let active_path = self.get_active_panel().path.clone();
        if let Some(parent) = active_path.parent() {
            let panel = self.get_active_panel_mut();
            if let Err(e) = panel.set_path(parent.to_path_buf()) {
                self.dialog = Dialog::Error { message: e };
            } else {
                self.status_message = "Moved up a directory".to_string();
            }
        }
    }

    pub fn get_preview_content(&mut self, path: PathBuf, cols: u16, rows: u16) -> String {
        if let Some(ref cache) = self.preview_cache {
            if cache.path == path && cache.width == cols && cache.height == rows {
                return cache.content.clone();
            }
        }
        
        let content = self.generate_preview_content(&path, cols, rows);
        self.preview_cache = Some(PreviewCache {
            path: path.clone(),
            width: cols,
            height: rows,
            content: content.clone(),
        });
        content
    }

    fn generate_preview_content(&self, path: &Path, cols: u16, rows: u16) -> String {
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
        
        // 1. Image previews via chafa
        if ["png", "jpg", "jpeg", "gif", "webp", "bmp"].contains(&ext.as_str()) {
            if let Ok(out) = std::process::Command::new("chafa")
                .arg(format!("--size={}x{}", cols, rows))
                .arg(path)
                .output() {
                if out.status.success() {
                    return String::from_utf8_lossy(&out.stdout).into_owned();
                }
            }
            return format!(
                "\n  [ Image Preview ]\n  File: {}\n\n  Tip: Install 'chafa' (e.g. 'brew install chafa')\n  for beautiful inline terminal image previews!",
                path.file_name().unwrap_or_default().to_string_lossy()
            );
        }

        // 2. Code previews via bat
        let is_text = ["rs", "py", "js", "ts", "json", "toml", "md", "sh", "txt", "cfg", "ini", "yaml", "yml", "xml", "html", "css", "c", "cpp", "h", "go", "java"].contains(&ext.as_str());
        if is_text {
            if let Ok(out) = std::process::Command::new("bat")
                .arg("--color=always")
                .arg("--style=plain")
                .arg(format!("--terminal-width={}", cols))
                .arg(path)
                .output() {
                if out.status.success() {
                    let raw_str = String::from_utf8_lossy(&out.stdout);
                    let lines: Vec<&str> = raw_str.lines().take(rows as usize).collect();
                    return lines.join("\n");
                }
            }
        }

        read_file_preview(path)
    }

    // =========================================================================
    // Core Actions (View, Edit, Copy, Move, Mkdir, Delete) — Bulk selections aware!
    // =========================================================================

    pub fn open_viewer(&mut self, path: PathBuf) {
        let content = read_file_preview(&path);
        self.dialog = Dialog::ViewFile {
            path,
            content,
            scroll_offset: 0,
        };
    }

    pub fn open_editor(&mut self) {
        let selected_path = self.get_active_panel().get_selected_item().map(|item| item.path.clone());
        if let Some(path) = selected_path {
            if path.is_file() {
                // Always open built-in internal editor (F4)
                // Use Enter for external editor (nvim/vim/nano)
                match fs::read_to_string(&path) {
                    Ok(content) => {
                        let lines: Vec<String> = if content.is_empty() {
                            vec![String::new()]
                        } else {
                            content.lines().map(|l| l.to_string()).collect()
                        };
                        self.dialog = Dialog::InternalEditor {
                            file_path: path,
                            lines,
                            cursor_row: 0,
                            cursor_col: 0,
                            scroll_row: 0,
                            scroll_col: 0,
                            modified: false,
                        };
                    }
                    Err(e) => {
                        self.dialog = Dialog::Error {
                            message: format!("Cannot read file: {}", e),
                        };
                    }
                }
            } else {
                self.dialog = Dialog::Error {
                    message: "Cannot edit directories".to_string(),
                };
            }
        }
    }

    pub fn execute_shell_command(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, cmd: String) {
        let trimmed = cmd.trim();
        if trimmed.is_empty() { return; }

        // Intercept quit commands
        if trimmed == "q" || trimmed == "quit" || trimmed == "exit" 
            || trimmed == ":q" || trimmed == ":quit" || trimmed == ":q!" 
            || trimmed == "q!" || trimmed == ":wq" || trimmed == "wq" {
            self.should_quit = true;
            return;
        }

        // Intercept help commands
        if trimmed == "help" || trimmed == ":help" || trimmed == "h" || trimmed == ":h" || trimmed == "?" || trimmed == ":?" {
            self.dialog = Dialog::Help { active_tab: 0 };
            return;
        }

        // Intercept directory changes (cd)
        let active_dir = self.get_active_panel().path.clone();
        if trimmed == "cd" {
            if let Some(home) = env::var("HOME").ok().or_else(|| env::var("USERPROFILE").ok()).map(PathBuf::from) {
                let _ = self.get_active_panel_mut().set_path(home);
                self.status_message = format!("Changed directory to {}", self.get_active_panel().path.display());
            }
            return;
        } else if trimmed.starts_with("cd ") {
            let target_dir = trimmed["cd ".len()..].trim();
            // Remove quotes if present
            let target_dir_unquoted = if (target_dir.starts_with('\'') && target_dir.ends_with('\''))
                || (target_dir.starts_with('"') && target_dir.ends_with('"')) {
                if target_dir.len() >= 2 {
                    &target_dir[1..target_dir.len() - 1]
                } else {
                    target_dir
                }
            } else {
                target_dir
            };
            let path_to_set = {
                let expanded = if target_dir_unquoted == "~" {
                    env::var("HOME").ok().or_else(|| env::var("USERPROFILE").ok())
                        .map(PathBuf::from)
                } else if target_dir_unquoted.starts_with("~/") || target_dir_unquoted.starts_with("~\\") {
                    env::var("HOME").ok().or_else(|| env::var("USERPROFILE").ok())
                        .map(|h| PathBuf::from(h).join(&target_dir_unquoted[2..]))
                } else {
                    let p = PathBuf::from(target_dir_unquoted);
                    if p.is_absolute() {
                        Some(p)
                    } else {
                        Some(active_dir.join(p))
                    }
                };
                expanded
            };
            if let Some(p) = path_to_set {
                if p.is_dir() {
                    let _ = self.get_active_panel_mut().set_path(p);
                    self.status_message = format!("Changed directory to {}", self.get_active_panel().path.display());
                } else {
                    self.status_message = format!("Not a directory: {}", p.display());
                }
            }
            return;
        }

        // Verify if the command exists before switching screen modes and running it (relative to active panel's path)
        if !command_exists(trimmed, &active_dir) {
            let first_word = trimmed.split_whitespace()
                .find(|w| !w.contains('='))
                .unwrap_or("")
                .trim_matches(|c| c == '\'' || c == '"');
            self.dialog = Dialog::Error {
                message: format!("Command not found: '{}'", first_word),
            };
            return;
        }

        // Interactive / full-screen programs run in the foreground with the
        // real TTY (shared detection with the Ctrl+O overlay); everything else
        // runs captured.
        if crate::shell::needs_tty(trimmed) {
            crate::shell::suspend();

            println!("$ {}\n", trimmed);

            let status = crate::shell::build(trimmed, &active_dir).status();

            match status {
                Ok(s) => {
                    println!("\n[Command exited with status {}]", s);
                }
                Err(e) => {
                    println!("\n[Failed to execute command: {}]", e);
                }
            }

            print!("Press Enter to return to Rust Commander...");
            let _ = io::stdout().flush();
            let mut input = String::new();
            let _ = io::stdin().read_line(&mut input);

            crate::shell::resume();
            let _ = terminal.clear();
            self.refresh_panels();
        } else {
            // Run silently and capture output
            let output = crate::shell::build(trimmed, &active_dir).output();

            match output {
                Ok(out) => {
                    let stdout_str = String::from_utf8_lossy(&out.stdout);
                    let stderr_str = String::from_utf8_lossy(&out.stderr);
                    let success = out.status.success();

                    // If it was successful AND has no terminal output, do not pause or flicker!
                    if success && stdout_str.trim().is_empty() && stderr_str.trim().is_empty() {
                        self.status_message = format!("Executed: {}", trimmed);
                        self.refresh_panels();
                    } else {
                        // Show output inside scrollable modal dialog
                        let mut combined_output = String::new();
                        if !stdout_str.is_empty() {
                            combined_output.push_str(&stdout_str);
                        }
                        if !stderr_str.is_empty() {
                            if !combined_output.is_empty() {
                                combined_output.push('\n');
                            }
                            combined_output.push_str(&stderr_str);
                        }
                        if !success {
                            if !combined_output.is_empty() {
                                combined_output.push('\n');
                            }
                            combined_output.push_str(&format!("[Command exited with exit status: {}]", out.status));
                        }

                        self.dialog = Dialog::CommandOutput {
                            command: trimmed.to_string(),
                            output: combined_output,
                            scroll_offset: 0,
                        };
                        self.refresh_panels();
                    }
                }
                Err(e) => {
                    self.dialog = Dialog::Error {
                        message: format!("Failed to run command: {}", e),
                    };
                }
            }
        }
    }

    pub fn initiate_copy(&mut self) {
        let marked_paths: Vec<PathBuf> = self.get_active_panel().marked.iter().cloned().collect();
        if !marked_paths.is_empty() {
            // Bulk copy
            let dest_dir = self.get_inactive_panel().path.clone();
            self.dialog = Dialog::ConfirmCopy {
                source_path: PathBuf::new(), // Emtpy indicates marked paths copy
                input: InputField::new(dest_dir.to_string_lossy().to_string()),
            };
        } else if let Some(item) = self.get_active_panel().get_selected_item() {
            if item.name == ".." { return; }
            let dest_dir = self.get_inactive_panel().path.clone();
            let dest_path = dest_dir.join(&item.name);
            self.dialog = Dialog::ConfirmCopy {
                source_path: item.path.clone(),
                input: InputField::new(dest_path.to_string_lossy().to_string()),
            };
        }
    }

    pub fn execute_copy(&mut self, source: PathBuf, destination: String) {
        let dest_dir = PathBuf::from(destination);
        let marked_paths: Vec<PathBuf> = self.get_active_panel().marked.iter().cloned().collect();

        let items: Vec<(PathBuf, PathBuf)> = if !marked_paths.is_empty() {
            // Bulk copy: each marked item lands inside the destination directory.
            marked_paths
                .iter()
                .map(|p| (p.clone(), dest_dir.join(p.file_name().unwrap_or_default())))
                .collect()
        } else {
            // Single copy: `destination` is the full target path (allows rename).
            vec![(source, dest_dir)]
        };

        self.fs_job = Some(fileops::spawn(OpKind::Copy, items));
        self.status_message = "Copying…".to_string();
    }

    pub fn initiate_move(&mut self) {
        let marked_paths: Vec<PathBuf> = self.get_active_panel().marked.iter().cloned().collect();
        if !marked_paths.is_empty() {
            let dest_dir = self.get_inactive_panel().path.clone();
            self.dialog = Dialog::ConfirmMove {
                source_path: PathBuf::new(), // Empty indicates marked paths move
                input: InputField::new(dest_dir.to_string_lossy().to_string()),
            };
        } else if let Some(item) = self.get_active_panel().get_selected_item() {
            if item.name == ".." { return; }
            let dest_dir = self.get_inactive_panel().path.clone();
            let dest_path = dest_dir.join(&item.name);
            self.dialog = Dialog::ConfirmMove {
                source_path: item.path.clone(),
                input: InputField::new(dest_path.to_string_lossy().to_string()),
            };
        }
    }

    pub fn execute_move(&mut self, source: PathBuf, destination: String) {
        let dest_dir = PathBuf::from(destination);
        let marked_paths: Vec<PathBuf> = self.get_active_panel().marked.iter().cloned().collect();

        let items: Vec<(PathBuf, PathBuf)> = if !marked_paths.is_empty() {
            // Bulk move: each marked item lands inside the destination directory.
            marked_paths
                .iter()
                .map(|p| (p.clone(), dest_dir.join(p.file_name().unwrap_or_default())))
                .collect()
        } else {
            // Single rename / move:
            //   destination is an existing directory → move into it (append name)
            //   otherwise treat destination as the full target path (rename)
            let target = if dest_dir.is_dir() {
                dest_dir.join(source.file_name().unwrap_or_default())
            } else {
                if let Some(parent) = dest_dir.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                dest_dir
            };
            vec![(source, target)]
        };

        self.fs_job = Some(fileops::spawn(OpKind::Move, items));
        self.status_message = "Moving…".to_string();
    }

    pub fn initiate_mkdir(&mut self) {
        self.dialog = Dialog::InputMkdir {
            input: InputField::new(String::new()),
        };
    }

    pub fn execute_mkdir(&mut self, dir_name: String) {
        if dir_name.trim().is_empty() {
            self.dialog = Dialog::Error { message: "Directory name cannot be empty".to_string() };
            return;
        }

        let new_dir_path = self.get_active_panel().path.join(&dir_name);
        match fs::create_dir_all(&new_dir_path) {
            Ok(_) => {
                self.status_message = format!("Created directory '{}'", dir_name);
                self.refresh_panels();
            }
            Err(e) => {
                self.dialog = Dialog::Error {
                    message: format!("Failed to create directory: {}", e),
                };
            }
        }
    }

    pub fn initiate_touch(&mut self) {
        self.dialog = Dialog::InputTouch {
            input: InputField::new(String::new()),
        };
    }

    pub fn execute_touch(&mut self, file_name: String) {
        let trimmed = file_name.trim();
        if trimmed.is_empty() {
            self.dialog = Dialog::Error { message: "File name cannot be empty".to_string() };
            return;
        }

        let active_dir = self.get_active_panel().path.clone();
        let target_path = active_dir.join(trimmed);
        if target_path.exists() {
            self.dialog = Dialog::Error {
                message: format!("Error: File or directory '{}' already exists", trimmed),
            };
            return;
        }

        match fs::File::create(&target_path) {
            Ok(_) => {
                self.status_message = format!("Created file '{}'", trimmed);
                self.refresh_panels();
            }
            Err(e) => {
                self.dialog = Dialog::Error {
                    message: format!("Failed to create file: {}", e),
                };
            }
        }
    }

    pub fn initiate_delete(&mut self) {
        let marked_paths: Vec<PathBuf> = self.get_active_panel().marked.iter().cloned().collect();
        if !marked_paths.is_empty() {
            self.dialog = Dialog::ConfirmDelete {
                item_name: format!("{} selected items", marked_paths.len()),
                item_path: PathBuf::new(), // Empty indicates marked paths deletion
            };
        } else if let Some(item) = self.get_active_panel().get_selected_item() {
            if item.name == ".." { return; }
            self.dialog = Dialog::ConfirmDelete {
                item_name: item.name.clone(),
                item_path: item.path.clone(),
            };
        }
    }

    pub fn execute_delete(&mut self, path: PathBuf) {
        let marked_paths: Vec<PathBuf> = self.get_active_panel().marked.iter().cloned().collect();

        let items: Vec<(PathBuf, PathBuf)> = if !marked_paths.is_empty() {
            marked_paths.into_iter().map(|p| (p, PathBuf::new())).collect()
        } else {
            vec![(path, PathBuf::new())]
        };

        self.fs_job = Some(fileops::spawn(OpKind::Delete, items));
        self.status_message = "Deleting…".to_string();
    }

    pub fn initiate_properties(&mut self) {
        if let Some(item) = self.get_active_panel().get_selected_item() {
            if item.name == ".." {
                return;
            }
            let name = item.name.clone();
            let path = item.path.clone();
            
            let metadata = match path.symlink_metadata() {
                Ok(m) => m,
                Err(e) => {
                    self.dialog = Dialog::Error {
                        message: format!("Failed to read metadata: {}", e),
                    };
                    return;
                }
            };
            
            let size_str = if metadata.is_dir() {
                "Directory".to_string()
            } else {
                format_size(metadata.len())
            };
            
            #[cfg(unix)]
            let (permissions_str, owner_str) = {
                use std::os::unix::fs::MetadataExt;
                let mode = metadata.mode();
                let uid = metadata.uid();
                let gid = metadata.gid();
                
                let is_dir = metadata.is_dir();
                let is_symlink = metadata.file_type().is_symlink();
                
                let perm = format_permissions(mode, is_dir, is_symlink);
                let owner = format!("UID: {}, GID: {}", uid, gid);
                (perm, owner)
            };
            
            #[cfg(not(unix))]
            let (permissions_str, owner_str) = {
                let perm = if metadata.permissions().readonly() {
                    "r--------".to_string()
                } else {
                    "rw-------".to_string()
                };
                let owner = "N/A".to_string();
                (perm, owner)
            };
            
            let path_str = if metadata.file_type().is_symlink() {
                match fs::read_link(&path) {
                    Ok(target) => format!("{} -> {}", path.display(), target.display()),
                    Err(_) => path.display().to_string(),
                }
            } else {
                path.display().to_string()
            };

            let format_time = |t: std::time::SystemTime| -> String {
                let datetime: chrono::DateTime<chrono::Local> = t.into();
                datetime.format("%Y-%m-%d %H:%M:%S").to_string()
            };
            
            let modified_str = metadata.modified().map(format_time).unwrap_or_else(|_| "N/A".to_string());
            let created_str = metadata.created().map(format_time).unwrap_or_else(|_| "N/A".to_string());
            
            self.dialog = Dialog::Properties {
                name,
                path_str,
                size_str,
                permissions_str,
                modified_str,
                created_str,
                owner_str,
            };
        }
    }
}

fn format_permissions(mode: u32, is_dir: bool, is_symlink: bool) -> String {
    let mut s = String::with_capacity(10);
    if is_dir {
        s.push('d');
    } else if is_symlink {
        s.push('l');
    } else {
        s.push('-');
    }
    
    let chars = ['r', 'w', 'x'];
    for i in (0..3).rev() {
        let shift = i * 3;
        let bits = (mode >> shift) & 0o7;
        s.push(if bits & 4 != 0 { chars[0] } else { '-' });
        s.push(if bits & 2 != 0 { chars[1] } else { '-' });
        s.push(if bits & 1 != 0 { chars[2] } else { '-' });
    }
    s
}


// Suspension editor helper
pub fn edit_file(path: &Path, editor_bin: &str) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture, crossterm::cursor::Show)?;
    
    let mut child = std::process::Command::new(editor_bin)
        .arg(path)
        .spawn()?;
        
    let status = child.wait()?;
    
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture, crossterm::cursor::Hide)?;
    
    if !status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("Editor '{}' exited with non-zero status", editor_bin),
        ));
    }
    
    Ok(())
}

// Menu dropdown actions router
pub fn execute_menu_action(app: &mut App, menu_idx: usize, item_idx: usize, _terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) {
    app.dialog = Dialog::None; // Close menu state
    match menu_idx {
        0 => { // Left Panel Config
            match item_idx {
                0 => {
                    app.left_panel.show_hidden = !app.left_panel.show_hidden;
                    app.left_panel.refresh();
                }
                1 => {
                    app.left_panel.sort_by = "name".to_string();
                    app.left_panel.refresh();
                }
                2 => {
                    app.left_panel.sort_by = "size".to_string();
                    app.left_panel.refresh();
                }
                3 => {
                    app.left_panel.sort_by = "time".to_string();
                    app.left_panel.refresh();
                }
                _ => {}
            }
        }
        1 => { // File Actions
            match item_idx {
                0 => {
                    if let Some(item) = app.get_active_panel().get_selected_item().cloned() {
                        if !item.is_dir { app.open_viewer(item.path); }
                    }
                }
                1 => {
                    app.open_editor();
                }
                2 => {
                    app.initiate_copy();
                }
                3 => {
                    app.initiate_move();
                }
                4 => {
                    app.initiate_mkdir();
                }
                5 => {
                    app.initiate_delete();
                }
                _ => {}
            }
        }
        2 => { // Command Prompt
            match item_idx {
                0 => {
                    app.dialog = Dialog::CommandLine {
                        input: InputField::new(String::new()),
                    };
                }
                1 => {
                    app.dialog = Dialog::Filter {
                        input: InputField::new(String::new()),
                    };
                }
                2 => {
                    app.preview_mode = !app.preview_mode;
                    app.status_message = format!("Quick View Pane: {}", if app.preview_mode { "ON" } else { "OFF" });
                }
                3 => {
                    if let Some(home) = env::var("HOME").ok().or_else(|| env::var("USERPROFILE").ok()) {
                        let _ = app.get_active_panel_mut().set_path(PathBuf::from(home));
                    }
                }
                _ => {}
            }
        }
        3 => { // Options
            match item_idx {
                0 => {
                    app.dialog = Dialog::Settings { active_row: 0 };
                }
                1 => {
                    app.dialog = Dialog::Help { active_tab: 0 };
                }
                2 => {
                    app.should_quit = true;
                }
                _ => {}
            }
        }
        4 => { // Right Panel Config
            match item_idx {
                0 => {
                    app.right_panel.show_hidden = !app.right_panel.show_hidden;
                    app.right_panel.refresh();
                }
                1 => {
                    app.right_panel.sort_by = "name".to_string();
                    app.right_panel.refresh();
                }
                2 => {
                    app.right_panel.sort_by = "size".to_string();
                    app.right_panel.refresh();
                }
                3 => {
                    app.right_panel.sort_by = "time".to_string();
                    app.right_panel.refresh();
                }
                _ => {}
            }
        }
        _ => {}
    }
}
