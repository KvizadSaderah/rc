use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use crate::config::*;
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
        let parent_dir = current_dir.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| current_dir.clone());
        
        Self {
            left_panel: Panel::new(current_dir, config.show_hidden, config.sort_by.clone()),
            right_panel: Panel::new(parent_dir, config.show_hidden, config.sort_by.clone()),
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
                if let Ok(meta) = entry.metadata() {
                    if meta.is_dir() {
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
                    if let Ok(meta) = entry.metadata() {
                        if meta.is_dir() {
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
    pub fn execute_overlay_command(active_dir: &std::path::Path, cmd: &str) -> (Vec<String>, bool) {
        let mut lines = Vec::new();

        // Guard: prevent launching rc inside rc (recursion)
        let self_names = ["rc", "rust-commander"];
        let first_word = cmd.split_whitespace().next().unwrap_or("");
        let first_word_base = std::path::Path::new(first_word)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(first_word);

        if self_names.contains(&first_word_base) {
            lines.push("⚠ Cannot launch rc inside rc (would cause recursion).".to_string());
            lines.push("  Use a separate terminal window instead.".to_string());
            return (lines, false);
        }

        // Detect TUI / full-screen programs that need terminal control.
        // These cannot be captured via .output() — they must be run via .spawn().
        let tui_programs = [
            "lazygit", "vim", "nvim", "nano", "emacs", "htop", "btop",
            "less", "more", "man", "top", "mc", "ranger", "nnn", "tig",
            "fzf", "zsh", "bash", "fish", "python", "irb", "node", "lua",
        ];
        let is_tui = tui_programs.contains(&first_word_base);

        if is_tui {
            lines.push(format!("[Launching {}...]", first_word_base));

            // Step 1: fully surrender the terminal to the child TUI app
            let _ = crossterm::terminal::disable_raw_mode();
            let _ = crossterm::execute!(
                std::io::stdout(),
                crossterm::terminal::LeaveAlternateScreen,
                crossterm::cursor::Show
            );

            // Step 2: run the TUI program with inherited stdin/stdout/stderr
            let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
            let _ = std::process::Command::new(&shell)
                .arg("-c")
                .arg(cmd)
                .current_dir(active_dir)
                .stdin(std::process::Stdio::inherit())
                .stdout(std::process::Stdio::inherit())
                .stderr(std::process::Stdio::inherit())
                .status();

            // Step 3: re-enable raw mode and enter alternate screen
            // We must do this BEFORE ratatui draws, and signal caller to clear.
            let _ = crossterm::terminal::enable_raw_mode();
            let _ = crossterm::execute!(
                std::io::stdout(),
                crossterm::terminal::EnterAlternateScreen,
                crossterm::cursor::Hide
            );

            lines.push(format!("[{} exited — press Esc to return to rc]", first_word_base));
            // needs_clear = true so caller calls terminal.clear()
            return (lines, true);
        }

        let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        match std::process::Command::new(shell)
            .arg("-c")
            .arg(cmd)
            .current_dir(active_dir)
            .output() {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    lines.push(line.to_string());
                }
                let stderr = String::from_utf8_lossy(&output.stderr);
                for line in stderr.lines() {
                    lines.push(format!("stderr: {}", line));
                }
                if !output.status.success() {
                    if let Some(code) = output.status.code() {
                        lines.push(format!("[Command exited with status code: {}]", code));
                    } else {
                        lines.push("[Command terminated by signal]".to_string());
                    }
                }
            }
            Err(e) => {
                lines.push(format!("Failed to execute command: {}", e));
            }
        }
        (lines, false)
    }

    pub fn handle_enter(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) {
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

        // Determine if command should be run interactively in the foreground
        let is_interactive = {
            let first_word = trimmed.split_whitespace()
                .find(|w| !w.contains('='))
                .unwrap_or("")
                .trim_matches(|c| c == '\'' || c == '"');
            let interactive_bins = [
                "vim", "vi", "nano", "emacs", "neovim", "nvim", "top", 
                "htop", "less", "more", "ssh", "sftp", "man", "tail", "watch"
            ];
            interactive_bins.contains(&first_word)
        };

        #[cfg(unix)]
        let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        #[cfg(windows)]
        let shell = "cmd.exe".to_string();

        if is_interactive {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture, crossterm::cursor::Show);
            
            println!("$ {}\n", trimmed);
            
            let status = std::process::Command::new(shell)
                .current_dir(&active_dir)
                .arg(if cfg!(windows) { "/c" } else { "-c" })
                .arg(trimmed)
                .status();

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

            let _ = enable_raw_mode();
            let _ = execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture, crossterm::cursor::Hide);
            let _ = terminal.clear();
            self.refresh_panels();
        } else {
            // Run silently and capture output
            let output = std::process::Command::new(shell)
                .current_dir(&active_dir)
                .arg(if cfg!(windows) { "/c" } else { "-c" })
                .arg(trimmed)
                .output();

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
        
        let result = if !marked_paths.is_empty() {
            // Bulk selections copy
            let mut err = None;
            for path in &marked_paths {
                let name = path.file_name().unwrap_or_default();
                let target = dest_dir.join(name);
                let res = if path.is_dir() {
                    copy_dir_all(path, &target)
                } else {
                    fs::copy(path, &target).map(|_| ())
                };
                if let Err(e) = res {
                    err = Some(e);
                    break;
                }
            }
            if let Some(e) = err { Err(e) } else { Ok(()) }
        } else {
            // Single selection copy
            let name = source.file_name().unwrap_or_default();
            let target = dest_dir.join(name);
            if source.is_dir() {
                copy_dir_all(&source, &target)
            } else {
                fs::copy(&source, &target).map(|_| ())
            }
        };

        match result {
            Ok(_) => {
                let count = if !marked_paths.is_empty() { marked_paths.len() } else { 1 };
                self.status_message = format!("Successfully copied {} item(s)", count);
                self.get_active_panel_mut().marked.clear();
                self.refresh_panels();
            }
            Err(e) => {
                self.dialog = Dialog::Error {
                    message: format!("Copy failed: {}", e),
                };
            }
        }
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

        let result = if !marked_paths.is_empty() {
            // Bulk move
            let mut err = None;
            for path in &marked_paths {
                let name = path.file_name().unwrap_or_default();
                let target = dest_dir.join(name);
                if let Err(e) = fs::rename(path, &target) {
                    err = Some(e);
                    break;
                }
            }
            if let Some(e) = err { Err(e) } else { Ok(()) }
        } else {
            // Single rename / move
            let name = source.file_name().unwrap_or_default();
            let target = dest_dir.join(name);
            fs::rename(&source, &target)
        };

        match result {
            Ok(_) => {
                let count = if !marked_paths.is_empty() { marked_paths.len() } else { 1 };
                self.status_message = format!("Successfully moved/renamed {} item(s)", count);
                self.get_active_panel_mut().marked.clear();
                self.refresh_panels();
            }
            Err(e) => {
                self.dialog = Dialog::Error {
                    message: format!("Move/Rename failed: {}", e),
                };
            }
        }
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

        let result = if !marked_paths.is_empty() {
            // Bulk delete
            let mut err = None;
            for p in &marked_paths {
                let res = if p.is_dir() {
                    fs::remove_dir_all(p)
                } else {
                    fs::remove_file(p)
                };
                if let Err(e) = res {
                    err = Some(e);
                    break;
                }
            }
            if let Some(e) = err { Err(e) } else { Ok(()) }
        } else {
            // Single delete
            if path.is_dir() {
                fs::remove_dir_all(&path)
            } else {
                fs::remove_file(&path)
            }
        };

        match result {
            Ok(_) => {
                let count = if !marked_paths.is_empty() { marked_paths.len() } else { 1 };
                self.status_message = format!("Deleted {} item(s)", count);
                self.get_active_panel_mut().marked.clear();
                self.refresh_panels();
            }
            Err(e) => {
                self.dialog = Dialog::Error {
                    message: format!("Failed to delete: {}", e),
                };
            }
        }
    }
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
