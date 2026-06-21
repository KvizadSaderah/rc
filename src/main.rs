use std::collections::HashSet;
use std::env;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Position, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};

// =============================================================================
// Persistent Configuration & Customizable Keybindings
// =============================================================================

#[derive(Clone, Debug, Copy)]
struct Theme {
    active_border: Color,
    inactive_border: Color,
    active_selection_bg: Color,
    inactive_selection_bg: Color,
    header_bg: Color,
    status_bg: Color,
    folder_fg: Color,
    symlink_fg: Color,
    executable_fg: Color,
    file_fg: Color,
    text_highlight: Color,
    accent: Color,
}

impl Theme {
    fn get_theme(_name: &str) -> Self {
        // Eve Online: deep space black + cold cyan tactical UI
        Self {
            active_border:         Color::Rgb(0, 210, 220),   // cold cyan
            inactive_border:       Color::Rgb(20, 55, 65),    // dim teal
            active_selection_bg:   Color::Rgb(0, 60, 80),     // deep teal selection
            inactive_selection_bg: Color::Rgb(5, 20, 28),     // nearly black
            header_bg:             Color::Rgb(3, 10, 18),     // deep space
            status_bg:             Color::Rgb(2, 7, 13),      // near black
            folder_fg:             Color::Rgb(0, 210, 220),   // cyan folders
            symlink_fg:            Color::Rgb(255, 140, 0),   // amber warning
            executable_fg:         Color::Rgb(80, 255, 160),  // signal green
            file_fg:               Color::Rgb(140, 190, 200), // cold steel
            text_highlight:        Color::Rgb(255, 180, 0),   // amber highlight
            accent:                Color::Rgb(0, 210, 220),   // same cyan
        }
    }
}

#[derive(Clone, Debug)]
struct Config {
    show_hidden: bool,
    sort_by: String,       // "name", "size", "time"
    keybindings: String,   // "standard", "vim"
    default_editor: String,
    confirm_quit: bool,
    bookmarks: Vec<PathBuf>,
    theme: String,
}

#[derive(Clone, Debug)]
struct Keymap {
    quit: Vec<(KeyCode, KeyModifiers)>,
    help: Vec<(KeyCode, KeyModifiers)>,
    view: Vec<(KeyCode, KeyModifiers)>,
    edit: Vec<(KeyCode, KeyModifiers)>,
    copy: Vec<(KeyCode, KeyModifiers)>,
    move_item: Vec<(KeyCode, KeyModifiers)>,
    mkdir: Vec<(KeyCode, KeyModifiers)>,
    delete: Vec<(KeyCode, KeyModifiers)>,
    menu: Vec<(KeyCode, KeyModifiers)>,
    toggle_hidden: Vec<(KeyCode, KeyModifiers)>,
    toggle_preview: Vec<(KeyCode, KeyModifiers)>,
    select_item: Vec<(KeyCode, KeyModifiers)>,
    up: Vec<(KeyCode, KeyModifiers)>,
    down: Vec<(KeyCode, KeyModifiers)>,
    left: Vec<(KeyCode, KeyModifiers)>,
    right: Vec<(KeyCode, KeyModifiers)>,
    tab: Vec<(KeyCode, KeyModifiers)>,
}

impl Keymap {
    fn default_standard() -> Self {
        Self {
            quit: vec![(KeyCode::Esc, KeyModifiers::empty()), (KeyCode::Char('q'), KeyModifiers::empty()), (KeyCode::F(10), KeyModifiers::empty())],
            help: vec![(KeyCode::F(1), KeyModifiers::empty()), (KeyCode::Char('?'), KeyModifiers::empty())],
            view: vec![(KeyCode::F(3), KeyModifiers::empty()), (KeyCode::Char('v'), KeyModifiers::empty())],
            edit: vec![(KeyCode::F(4), KeyModifiers::empty()), (KeyCode::Char('e'), KeyModifiers::empty())],
            copy: vec![(KeyCode::F(5), KeyModifiers::empty()), (KeyCode::Char('c'), KeyModifiers::empty())],
            move_item: vec![(KeyCode::F(6), KeyModifiers::empty()), (KeyCode::Char('m'), KeyModifiers::empty())],
            mkdir: vec![(KeyCode::F(7), KeyModifiers::empty()), (KeyCode::Char('n'), KeyModifiers::empty())],
            delete: vec![(KeyCode::F(8), KeyModifiers::empty()), (KeyCode::Delete, KeyModifiers::empty()), (KeyCode::Char('d'), KeyModifiers::empty())],
            menu: vec![(KeyCode::F(9), KeyModifiers::empty())],
            toggle_hidden: vec![(KeyCode::Char('.'), KeyModifiers::empty())],
            toggle_preview: vec![(KeyCode::Char('p'), KeyModifiers::CONTROL)],
            select_item: vec![(KeyCode::Char(' '), KeyModifiers::empty()), (KeyCode::Insert, KeyModifiers::empty())],
            up: vec![(KeyCode::Up, KeyModifiers::empty())],
            down: vec![(KeyCode::Down, KeyModifiers::empty())],
            left: vec![(KeyCode::Left, KeyModifiers::empty()), (KeyCode::Backspace, KeyModifiers::empty())],
            right: vec![(KeyCode::Right, KeyModifiers::empty()), (KeyCode::Enter, KeyModifiers::empty())],
            tab: vec![(KeyCode::Tab, KeyModifiers::empty())],
        }
    }

    fn default_vim() -> Self {
        let mut k = Self::default_standard();
        k.up.push((KeyCode::Char('k'), KeyModifiers::empty()));
        k.down.push((KeyCode::Char('j'), KeyModifiers::empty()));
        k.left.push((KeyCode::Char('h'), KeyModifiers::empty()));
        k.right.push((KeyCode::Char('l'), KeyModifiers::empty()));
        k.toggle_preview.push((KeyCode::Char('p'), KeyModifiers::empty()));
        k
    }
}

fn get_config_path() -> Option<PathBuf> {
    let base = env::var("HOME")
        .ok()
        .or_else(|| env::var("USERPROFILE").ok())?;
    Some(PathBuf::from(base).join(".config/rust-commander/config.ini"))
}

fn load_config() -> Config {
    let mut config = Config {
        show_hidden: false,
        sort_by: "name".to_string(),
        keybindings: "standard".to_string(),
        default_editor: env::var("EDITOR").unwrap_or_else(|_| "nano".to_string()),
        confirm_quit: true,
        bookmarks: Vec::new(),
        theme: "eve".to_string(),
    };

    if let Some(path) = get_config_path() {
        if let Ok(content) = fs::read_to_string(&path) {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with(';') || trimmed.starts_with('#') || trimmed.is_empty() {
                    continue;
                }
                let parts: Vec<&str> = line.splitn(2, '=').map(|s| s.trim()).collect();
                if parts.len() == 2 {
                    match parts[0] {
                        "show_hidden" => config.show_hidden = parts[1] == "true",
                        "sort_by" => config.sort_by = parts[1].to_string(),
                        "keybindings" => config.keybindings = parts[1].to_string(),
                        "default_editor" => config.default_editor = parts[1].to_string(),
                        "confirm_quit" => config.confirm_quit = parts[1] == "true",
                        "theme" => config.theme = parts[1].to_string(),
                        "bookmarks" => {
                            config.bookmarks = parts[1]
                                .split(',')
                                .map(|s| PathBuf::from(s.trim()))
                                .filter(|p| !p.as_os_str().is_empty())
                                .collect();
                        }
                        _ => {}
                    }
                }
            }
        } else {
            let _ = save_config(&config);
        }
    }
    config
}

fn save_config(config: &Config) -> io::Result<()> {
    if let Some(path) = get_config_path() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bookmarks_str = config.bookmarks.iter()
            .map(|p| p.to_string_lossy())
            .collect::<Vec<_>>()
            .join(",");
        let content = format!(
            "; Rust Commander Configuration File\n\n\
             show_hidden = {}\n\
             sort_by = {}\n\
             keybindings = {}\n\
             default_editor = {}\n\
             confirm_quit = {}\n\
             theme = {}\n\
             bookmarks = {}\n\n\
             [keys]\n\
             ; Customize shortcuts here. Format: command = key, modifiers+key\n\
             ; Examples:\n\
             ; quit = q, f10\n\
             ; view = f3, v\n\
             ; edit = f4, e\n\
             ; copy = f5, c\n\
             ; move = f6, m\n\
             ; delete = f8, d\n\
             ; toggle_hidden = .\n\
             ; toggle_preview = ctrl+p\n\
             ; select_item = space\n",
            config.show_hidden, config.sort_by, config.keybindings, config.default_editor, config.confirm_quit, config.theme, bookmarks_str
        );
        fs::write(path, content)?;
    }
    Ok(())
}

fn parse_keys(val: &str) -> Vec<(KeyCode, KeyModifiers)> {
    let mut result = Vec::new();
    for token in val.split(',') {
        let parts: Vec<&str> = token.trim().split('+').map(|s| s.trim()).collect();
        let mut modifiers = KeyModifiers::empty();
        let mut key_code = None;

        for part in parts {
            let part_lower = part.to_lowercase();
            match part_lower.as_str() {
                "ctrl" | "control" => modifiers.insert(KeyModifiers::CONTROL),
                "alt" | "option" => modifiers.insert(KeyModifiers::ALT),
                "shift" => modifiers.insert(KeyModifiers::SHIFT),
                "up" => key_code = Some(KeyCode::Up),
                "down" => key_code = Some(KeyCode::Down),
                "left" => key_code = Some(KeyCode::Left),
                "right" => key_code = Some(KeyCode::Right),
                "enter" => key_code = Some(KeyCode::Enter),
                "backspace" => key_code = Some(KeyCode::Backspace),
                "delete" => key_code = Some(KeyCode::Delete),
                "space" => key_code = Some(KeyCode::Char(' ')),
                "tab" => key_code = Some(KeyCode::Tab),
                "esc" | "escape" => key_code = Some(KeyCode::Esc),
                "insert" => key_code = Some(KeyCode::Insert),
                _ if part_lower.starts_with('f') => {
                    if let Ok(num) = part_lower[1..].parse::<u8>() {
                        key_code = Some(KeyCode::F(num));
                    }
                }
                _ if part_lower.len() == 1 => {
                    if let Some(c) = part_lower.chars().next() {
                        key_code = Some(KeyCode::Char(c));
                    }
                }
                _ => {}
            }
        }
        if let Some(code) = key_code {
            result.push((code, modifiers));
        }
    }
    result
}

fn load_keymap(config: &Config) -> Keymap {
    let mut k = match config.keybindings.as_str() {
        "vim" => Keymap::default_vim(),
        _ => Keymap::default_standard(),
    };

    if let Some(path) = get_config_path() {
        if let Ok(content) = fs::read_to_string(&path) {
            let mut keys_section = false;
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
                    continue;
                }
                if trimmed == "[keys]" {
                    keys_section = true;
                    continue;
                }
                if trimmed.starts_with('[') {
                    keys_section = false;
                    continue;
                }
                if keys_section {
                    let parts: Vec<&str> = line.splitn(2, '=').map(|s| s.trim()).collect();
                    if parts.len() == 2 {
                        let parsed = parse_keys(parts[1]);
                        if !parsed.is_empty() {
                            match parts[0] {
                                "quit" => k.quit = parsed,
                                "help" => k.help = parsed,
                                "view" => k.view = parsed,
                                "edit" => k.edit = parsed,
                                "copy" => k.copy = parsed,
                                "move" => k.move_item = parsed,
                                "mkdir" => k.mkdir = parsed,
                                "delete" => k.delete = parsed,
                                "menu" => k.menu = parsed,
                                "toggle_hidden" => k.toggle_hidden = parsed,
                                "toggle_preview" => k.toggle_preview = parsed,
                                "select_item" => k.select_item = parsed,
                                "up" => k.up = parsed,
                                "down" => k.down = parsed,
                                "left" => k.left = parsed,
                                "right" => k.right = parsed,
                                "tab" => k.tab = parsed,
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
    }
    k
}

fn matches_key(event: &KeyEvent, keys: &[(KeyCode, KeyModifiers)]) -> bool {
    keys.iter().any(|(code, mods)| {
        event.code == *code && event.modifiers.contains(*mods)
    })
}

fn command_exists(cmd_str: &str, current_dir: &std::path::Path) -> bool {
    let trimmed = cmd_str.trim();
    if trimmed.is_empty() { return false; }
    
    let mut parts = trimmed.split_whitespace();
    let mut first_word = parts.next().unwrap_or("");
    while first_word.contains('=') {
        first_word = parts.next().unwrap_or("");
    }
    
    if first_word.is_empty() { return false; }
    
    if first_word.contains('/') || first_word.contains('\\') {
        let p = PathBuf::from(first_word);
        if p.is_absolute() {
            return p.exists();
        } else {
            return current_dir.join(p).exists();
        }
    }
    
    let builtins = [
        "cd", "echo", "pwd", "exit", "logout", "alias", "unalias", 
        "export", "set", "unset", "history", "type", "which", "read", 
        "source", "exec", "help", "local", "declare", "typeset"
    ];
    if builtins.contains(&first_word) {
        return true;
    }
    
    if let Ok(path_var) = env::var("PATH") {
        let separator = if cfg!(windows) { ';' } else { ':' };
        for path_dir in path_var.split(separator) {
            let bin_path = PathBuf::from(path_dir).join(first_word);
            if cfg!(windows) {
                for ext in &[".exe", ".cmd", ".bat", ".com"] {
                    if bin_path.with_extension(ext).is_file() {
                        return true;
                    }
                }
            }
            if bin_path.is_file() {
                return true;
            }
        }
    }
    
    false
}

// =============================================================================
// Data Models & Filesystem IO Helpers
// =============================================================================

#[derive(Clone, Debug)]
struct FileItem {
    name: String,
    path: PathBuf,
    is_dir: bool,
    is_symlink: bool,
    is_exec: bool,
    size: u64,
    modified: Option<SystemTime>,
}

#[derive(Clone, Debug)]
struct TreeNode {
    path: PathBuf,
    name: String,
    depth: usize,
    is_expanded: bool,
    has_subdirs: bool,
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        if let Ok(metadata) = fs::metadata(path) {
            return metadata.permissions().mode() & 0o111 != 0;
        }
    }
    if let Some(ext) = path.extension() {
        let ext_str = ext.to_string_lossy().to_lowercase();
        return ext_str == "exe" || ext_str == "bat" || ext_str == "sh" || ext_str == "cmd";
    }
    false
}

fn has_subdirectories(path: &Path) -> bool {
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.is_dir() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if name_str != "." && name_str != ".." && !name_str.starts_with('.') {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn read_dir(path: &Path) -> io::Result<Vec<FileItem>> {
    let mut items = Vec::new();

    // Add parent link ".." if we are not at the root
    if let Some(parent) = path.parent() {
        items.push(FileItem {
            name: "..".to_string(),
            path: parent.to_path_buf(),
            is_dir: true,
            is_symlink: false,
            is_exec: false,
            size: 0,
            modified: None,
        });
    }

    if let Ok(entries) = fs::read_dir(path) {
        let mut file_entries = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let metadata = entry.metadata().ok();
            let file_type = entry.file_type();
            
            let is_dir = metadata.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let is_symlink = file_type.as_ref().map(|ft| ft.is_symlink()).unwrap_or(false);
            let is_exec = !is_dir && is_executable(&path);
            let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
            let modified = metadata.as_ref().and_then(|m| m.modified().ok());
            let name = entry.file_name().to_string_lossy().into_owned();

            file_entries.push(FileItem {
                name,
                path,
                is_dir,
                is_symlink,
                is_exec,
                size,
                modified,
            });
        }
        items.extend(file_entries);
    }

    Ok(items)
}

fn format_size(size: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if size >= GB {
        format!("{:.1} GB", size as f64 / GB as f64)
    } else if size >= MB {
        format!("{:.1} MB", size as f64 / MB as f64)
    } else if size >= KB {
        format!("{:.1} KB", size as f64 / KB as f64)
    } else {
        format!("{} B", size)
    }
}

fn format_time(time: Option<SystemTime>) -> String {
    match time {
        Some(t) => {
            let datetime: chrono::DateTime<chrono::Local> = t.into();
            datetime.format("%Y-%m-%d %H:%M:%S").to_string()
        }
        None => "-".to_string(),
    }
}

fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> io::Result<()> {
    fs::create_dir_all(&dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name()))?;
        } else {
            fs::copy(entry.path(), dst.as_ref().join(entry.file_name()))?;
        }
    }
    Ok(())
}

fn read_file_preview(path: &Path) -> String {
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) => return format!("Error opening file: {}", e),
    };

    let mut buffer = vec![0; 25 * 1024]; // Read first 25 KB for live preview
    let bytes_read = match file.read(&mut buffer) {
        Ok(n) => n,
        Err(e) => return format!("Error reading file: {}", e),
    };

    buffer.truncate(bytes_read);

    match String::from_utf8(buffer.clone()) {
        Ok(text) => text,
        Err(_) => {
            let mut hex_view = String::new();
            hex_view.push_str("Binary file (Hex Dump):\n\n");
            for (i, chunk) in buffer.chunks(16).enumerate() {
                hex_view.push_str(&format!("{:08x}: ", i * 16));
                for byte in chunk {
                    hex_view.push_str(&format!("{:02x} ", byte));
                }
                if chunk.len() < 16 {
                    for _ in 0..(16 - chunk.len()) {
                        hex_view.push_str("   ");
                    }
                }
                hex_view.push_str(" | ");
                for &byte in chunk {
                    if byte.is_ascii_graphic() || byte == b' ' {
                        hex_view.push(byte as char);
                    } else {
                        hex_view.push('.');
                    }
                }
                hex_view.push('\n');
                if hex_view.len() > 10000 {
                    hex_view.push_str("\n... preview truncated ...");
                    break;
                }
            }
            hex_view
        }
    }
}

fn read_dir_preview(path: &Path) -> String {
    let mut preview = String::new();
    preview.push_str(&format!("Directory Contents: {}\n\n", path.to_string_lossy()));
    
    match fs::read_dir(path) {
        Ok(entries) => {
            let mut count = 0;
            for entry in entries.flatten().take(40) {
                let name = entry.file_name().to_string_lossy().into_owned();
                let is_dir = entry.metadata().map(|m| m.is_dir()).unwrap_or(false);
                if is_dir {
                    preview.push_str(&format!("  📁 {}/\n", name));
                } else {
                    preview.push_str(&format!("  📄 {}\n", name));
                }
                count += 1;
            }
            if count >= 40 {
                preview.push_str("\n  ... and more items ...");
            }
        }
        Err(e) => {
            preview.push_str(&format!("Error reading directory: {}", e));
        }
    }
    preview
}

// Extension Specific Tailored Styling — theme-aware
fn get_extension_style(item: &FileItem, theme: &Theme) -> Style {
    if item.is_dir {
        return Style::default().fg(theme.folder_fg).bold();
    }
    if item.is_symlink {
        return Style::default().fg(theme.symlink_fg);
    }
    if item.is_exec {
        return Style::default().fg(theme.executable_fg);
    }
    
    if let Some(ext) = item.path.extension() {
        let ext_str = ext.to_string_lossy().to_lowercase();
        match ext_str.as_str() {
            // Source Code & Configs (Light Blue)
            "rs" | "py" | "js" | "ts" | "go" | "c" | "cpp" | "h" | "java" | "html" | "css" | "json" | "toml" | "yaml" | "yml" | "sh" | "ini" | "sql" => {
                Style::default().fg(Color::Rgb(14, 165, 233))
            }
            // Archives (Coral / Red)
            "zip" | "tar" | "gz" | "rar" | "7z" | "bz2" | "xz" | "tgz" => {
                Style::default().fg(Color::Rgb(248, 113, 113))
            }
            // Media (Lavender / Purple)
            "png" | "jpg" | "jpeg" | "gif" | "svg" | "mp3" | "mp4" | "wav" | "mkv" | "mov" | "webm" | "ogg" => {
                Style::default().fg(Color::Rgb(232, 121, 249))
            }
            // Documents (Yellow / Sand)
            "md" | "txt" | "pdf" | "epub" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" => {
                Style::default().fg(theme.text_highlight)
            }
            _ => Style::default().fg(theme.file_fg),
        }
    } else {
        Style::default().fg(theme.file_fg)
    }
}

// =============================================================================
// UI Dialog States & Components
// =============================================================================

struct InputField {
    text: String,
    cursor_position: usize, // character index, NOT byte index!
}

impl InputField {
    fn new(initial: String) -> Self {
        let len = initial.chars().count();
        Self {
            text: initial,
            cursor_position: len,
        }
    }

    fn insert(&mut self, c: char) {
        let byte_idx = self.text.char_indices().map(|(i, _)| i).nth(self.cursor_position).unwrap_or(self.text.len());
        self.text.insert(byte_idx, c);
        self.cursor_position += 1;
    }

    fn backspace(&mut self) {
        if self.cursor_position > 0 {
            self.cursor_position -= 1;
            if let Some(byte_idx) = self.text.char_indices().map(|(i, _)| i).nth(self.cursor_position) {
                self.text.remove(byte_idx);
            }
        }
    }

    fn delete(&mut self) {
        let char_len = self.text.chars().count();
        if self.cursor_position < char_len {
            if let Some(byte_idx) = self.text.char_indices().map(|(i, _)| i).nth(self.cursor_position) {
                self.text.remove(byte_idx);
            }
        }
    }

    fn move_left(&mut self) {
        if self.cursor_position > 0 {
            self.cursor_position -= 1;
        }
    }

    fn move_right(&mut self) {
        let char_len = self.text.chars().count();
        if self.cursor_position < char_len {
            self.cursor_position += 1;
        }
    }

    /// Returns the visual column offset for terminal cursor placement.
    /// For ASCII text this equals cursor_position; for Unicode
    /// characters each char is assumed 1 terminal column wide.
    fn visual_cursor_col(&self) -> u16 {
        self.text.chars().take(self.cursor_position).count() as u16
    }
}

struct PreviewCache {
    path: PathBuf,
    width: u16,
    height: u16,
    content: String,
}

enum Dialog {
    None,
    ConfirmDelete {
        item_name: String,
        item_path: PathBuf,
    },
    InputMkdir {
        input: InputField,
    },
    ConfirmCopy {
        source_path: PathBuf,
        input: InputField,
    },
    ConfirmMove {
        source_path: PathBuf,
        input: InputField,
    },
    ViewFile {
        path: PathBuf,
        content: String,
        scroll_offset: usize,
    },
    CommandLine {
        input: InputField,
    },
    Filter {
        input: InputField,
    },
    Settings {
        active_row: usize,
    },
    Menu {
        active_menu: usize,          // 0: Left, 1: File, 2: Command, 3: Options, 4: Right
        active_item: Option<usize>,  // Some(idx) if dropdown menu is open
    },
    Help {
        active_tab: usize, // 0=Navigation, 1=File Ops, 2=Shell/View, 3=Tips
    },
    Error {
        message: String,
    },
    CommandOutput {
        command: String,
        output: String,
        scroll_offset: usize,
    },
    ConfirmQuit,
    InputEditor {
        input: InputField,
    },
    Bookmarks {
        selected_idx: usize,
    },
    TerminalOverlay {
        input: InputField,
        output_lines: Vec<String>,
        scroll_offset: usize,
    },
}

// =============================================================================
// File Panel Core State
// =============================================================================

#[derive(Copy, Clone, PartialEq, Eq)]
enum ActivePanel {
    Left,
    Right,
}

struct Panel {
    path: PathBuf,
    items: Vec<FileItem>,
    selected: usize,
    scroll_state: ListState,
    show_hidden: bool,
    sort_by: String,
    filter: Option<String>,
    marked: HashSet<PathBuf>, // Multi-selection Set (Tagged items)
    git_branch: Option<String>,
    git_statuses: std::collections::HashMap<PathBuf, String>,
}

impl Panel {
    fn new(path: PathBuf, show_hidden: bool, sort_by: String) -> Self {
        let canonical_path = path.canonicalize().unwrap_or(path);
        let mut panel = Self {
            path: canonical_path,
            items: Vec::new(),
            selected: 0,
            scroll_state: ListState::default(),
            show_hidden,
            sort_by,
            filter: None,
            marked: HashSet::new(),
            git_branch: None,
            git_statuses: std::collections::HashMap::new(),
        };
        panel.refresh();
        panel
    }

    fn refresh(&mut self) {
        let prev_selected_name = self.get_selected_item().map(|item| item.name.clone());
        let raw_items = read_dir(&self.path).unwrap_or_default();

        // 1. Filter out hidden or non-matched files
        self.items = raw_items
            .into_iter()
            .filter(|item| {
                if item.name == ".." {
                    return true;
                }
                if !self.show_hidden && item.name.starts_with('.') {
                    return false;
                }
                if let Some(ref f) = self.filter {
                    if !item.name.to_lowercase().contains(&f.to_lowercase()) {
                        return false;
                    }
                }
                true
            })
            .collect();

        // 2. Sort results
        let sort_criteria = self.sort_by.clone();
        self.items.sort_by(|a, b| {
            if a.name == ".." {
                return std::cmp::Ordering::Less;
            }
            if b.name == ".." {
                return std::cmp::Ordering::Greater;
            }

            if a.is_dir != b.is_dir {
                b.is_dir.cmp(&a.is_dir) // Directories first
            } else {
                match sort_criteria.as_str() {
                    "size" => b.size.cmp(&a.size),
                    "time" => b.modified.cmp(&a.modified),
                    _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                }
            }
        });

        // Clean invalid marked paths (paths no longer present in folder)
        let current_paths: HashSet<&PathBuf> = self.items.iter().map(|item| &item.path).collect();
        self.marked.retain(|path| current_paths.contains(path));

        // 3. Restore scroll list selection index
        if self.items.is_empty() {
            self.selected = 0;
            self.scroll_state.select(None);
        } else {
            if let Some(ref name) = prev_selected_name {
                if let Some(pos) = self.items.iter().position(|item| &item.name == name) {
                    self.selected = pos;
                }
            }
            if self.selected >= self.items.len() {
                self.selected = self.items.len() - 1;
            }
            self.scroll_state.select(Some(self.selected));
        }

        // Query Git status if we are in a Git workspace
        self.git_branch = None;
        self.git_statuses.clear();

        if let Ok(out) = std::process::Command::new("git")
            .arg("rev-parse")
            .arg("--is-inside-work-tree")
            .current_dir(&self.path)
            .output() {
            if out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "true" {
                if let Ok(branch_out) = std::process::Command::new("git")
                    .arg("branch")
                    .arg("--show-current")
                    .current_dir(&self.path)
                    .output() {
                    let b_name = String::from_utf8_lossy(&branch_out.stdout).trim().to_string();
                    if !b_name.is_empty() {
                        self.git_branch = Some(b_name);
                    } else if let Ok(rev_out) = std::process::Command::new("git")
                        .arg("rev-parse")
                        .arg("--short")
                        .arg("HEAD")
                        .current_dir(&self.path)
                        .output() {
                        self.git_branch = Some(format!("detached@{}", String::from_utf8_lossy(&rev_out.stdout).trim()));
                    }
                }

                if let Ok(status_out) = std::process::Command::new("git")
                    .arg("status")
                    .arg("--porcelain")
                    .current_dir(&self.path)
                    .output() {
                    let status_str = String::from_utf8_lossy(&status_out.stdout);
                    if let Ok(root_out) = std::process::Command::new("git")
                        .arg("rev-parse")
                        .arg("--show-toplevel")
                        .current_dir(&self.path)
                        .output() {
                        let repo_root = PathBuf::from(String::from_utf8_lossy(&root_out.stdout).trim());
                        for line in status_str.lines() {
                            if line.len() > 3 {
                                let code = line[..2].trim().to_string();
                                let rel_path = &line[3..];
                                let actual_rel_path = if let Some(idx) = rel_path.find(" -> ") {
                                    &rel_path[idx + 4..]
                                } else {
                                    rel_path
                                };
                                let cleaned_rel = actual_rel_path.trim_matches('"');
                                let abs_path = repo_root.join(cleaned_rel);
                                if let Ok(canon_path) = abs_path.canonicalize() {
                                    self.git_statuses.insert(canon_path, code);
                                } else {
                                    self.git_statuses.insert(abs_path, code);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn set_path(&mut self, new_path: PathBuf) -> Result<(), String> {
        let resolved = new_path.canonicalize().unwrap_or(new_path);
        match read_dir(&resolved) {
            Ok(_) => {
                self.path = resolved;
                self.filter = None;
                self.marked.clear(); // Reset selections on dir transition
                self.selected = 0;
                self.refresh();
                Ok(())
            }
            Err(e) => Err(format!("Cannot open directory: {}", e)),
        }
    }

    fn select_next(&mut self) {
        if self.items.is_empty() { return; }
        self.selected = (self.selected + 1) % self.items.len();
        self.scroll_state.select(Some(self.selected));
    }

    fn select_prev(&mut self) {
        if self.items.is_empty() { return; }
        if self.selected == 0 {
            self.selected = self.items.len() - 1;
        } else {
            self.selected -= 1;
        }
        self.scroll_state.select(Some(self.selected));
    }

    fn select_first(&mut self) {
        if self.items.is_empty() { return; }
        self.selected = 0;
        self.scroll_state.select(Some(self.selected));
    }

    fn select_last(&mut self) {
        if self.items.is_empty() { return; }
        self.selected = self.items.len() - 1;
        self.scroll_state.select(Some(self.selected));
    }

    fn page_down(&mut self) {
        if self.items.is_empty() { return; }
        self.selected = (self.selected + 10).min(self.items.len() - 1);
        self.scroll_state.select(Some(self.selected));
    }

    fn page_up(&mut self) {
        if self.items.is_empty() { return; }
        self.selected = self.selected.saturating_sub(10);
        self.scroll_state.select(Some(self.selected));
    }

    fn get_selected_item(&self) -> Option<&FileItem> {
        self.items.get(self.selected)
    }
}

// =============================================================================
// App Controller State
// =============================================================================

struct App {
    left_panel: Panel,
    right_panel: Panel,
    active_panel: ActivePanel,
    dialog: Dialog,
    status_message: String,
    should_quit: bool,
    config: Config,
    keymap: Keymap,
    preview_mode: bool,
    tree_mode: bool,
    tree_nodes: Vec<TreeNode>,
    tree_selected: usize,
    preview_cache: Option<PreviewCache>,
}

impl App {
    fn new() -> Self {
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

    fn apply_config(&mut self) {
        self.left_panel.show_hidden = self.config.show_hidden;
        self.left_panel.sort_by = self.config.sort_by.clone();

        self.right_panel.show_hidden = self.config.show_hidden;
        self.right_panel.sort_by = self.config.sort_by.clone();

        self.keymap = load_keymap(&self.config);

        self.refresh_panels();
    }

    fn get_active_panel_mut(&mut self) -> &mut Panel {
        match self.active_panel {
            ActivePanel::Left => &mut self.left_panel,
            ActivePanel::Right => &mut self.right_panel,
        }
    }

    fn get_active_panel(&self) -> &Panel {
        match self.active_panel {
            ActivePanel::Left => &self.left_panel,
            ActivePanel::Right => &self.right_panel,
        }
    }

    fn get_inactive_panel(&self) -> &Panel {
        match self.active_panel {
            ActivePanel::Left => &self.right_panel,
            ActivePanel::Right => &self.left_panel,
        }
    }

    fn refresh_panels(&mut self) {
        if self.tree_mode {
            self.init_tree();
            self.update_right_panel_from_tree();
        } else {
            self.left_panel.refresh();
            self.right_panel.refresh();
        }
    }

    fn toggle_panel(&mut self) {
        self.active_panel = match self.active_panel {
            ActivePanel::Left => ActivePanel::Right,
            ActivePanel::Right => ActivePanel::Left,
        };
    }

    fn init_tree(&mut self) {
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

    fn toggle_tree_node(&mut self) {
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

    fn update_right_panel_from_tree(&mut self) {
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
    fn execute_overlay_command(active_dir: &std::path::Path, cmd: &str) -> (Vec<String>, bool) {
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

    fn handle_enter(&mut self) {
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
            } else {
                self.open_viewer(item.path);
            }
        }
    }

    fn handle_backspace(&mut self) {
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

    fn get_preview_content(&mut self, path: PathBuf, cols: u16, rows: u16) -> String {
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

    fn open_viewer(&mut self, path: PathBuf) {
        let content = read_file_preview(&path);
        self.dialog = Dialog::ViewFile {
            path,
            content,
            scroll_offset: 0,
        };
    }

    fn open_editor(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) {
        let selected_path = self.get_active_panel().get_selected_item().map(|item| item.path.clone());
        if let Some(path) = selected_path {
            if path.is_file() {
                match edit_file(&path, &self.config.default_editor) {
                    Ok(_) => {
                        self.status_message = format!("Edited file: {}", path.file_name().unwrap().to_string_lossy());
                        self.refresh_panels();
                        let _ = terminal.clear();
                    }
                    Err(e) => {
                        let _ = terminal.clear();
                        self.dialog = Dialog::Error {
                            message: format!("Failed to edit file: {}", e),
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

    fn execute_shell_command(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, cmd: String) {
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
                let p = PathBuf::from(target_dir_unquoted);
                if p.is_absolute() {
                    Some(p)
                } else {
                    Some(active_dir.join(p))
                }
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

    fn initiate_copy(&mut self) {
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

    fn execute_copy(&mut self, source: PathBuf, destination: String) {
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

    fn initiate_move(&mut self) {
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

    fn execute_move(&mut self, source: PathBuf, destination: String) {
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

    fn initiate_mkdir(&mut self) {
        self.dialog = Dialog::InputMkdir {
            input: InputField::new(String::new()),
        };
    }

    fn execute_mkdir(&mut self, dir_name: String) {
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

    fn initiate_delete(&mut self) {
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

    fn execute_delete(&mut self, path: PathBuf) {
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
fn edit_file(path: &Path, editor_bin: &str) -> io::Result<()> {
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
fn execute_menu_action(app: &mut App, menu_idx: usize, item_idx: usize, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) {
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
                    app.open_editor(terminal);
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

// =============================================================================
// Keyboard Inputs Router
// =============================================================================

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, mut app: App) -> io::Result<()> {
    loop {
        terminal.draw(|f| ui(f, &mut app))?;

        if let Event::Key(key) = event::read()? {
            if key.kind == event::KeyEventKind::Release {
                continue;
            }

            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                app.should_quit = true;
            }

            let active_dir = match app.active_panel {
                ActivePanel::Left => app.left_panel.path.clone(),
                ActivePanel::Right => app.right_panel.path.clone(),
            };

            // Route key events depending on dialog state
            match &mut app.dialog {
                Dialog::None => handle_main_keys(&mut app, key, terminal),
                Dialog::ConfirmDelete { item_path, .. } => {
                    let path = item_path.clone();
                    match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                            app.dialog = Dialog::None;
                            app.execute_delete(path);
                        }
                        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                            app.dialog = Dialog::None;
                            app.status_message = "Delete operation cancelled".to_string();
                        }
                        _ => {}
                    }
                }
                Dialog::InputMkdir { input } => match key.code {
                    KeyCode::Enter => {
                        let text = input.text.clone();
                        app.dialog = Dialog::None;
                        app.execute_mkdir(text);
                    }
                    KeyCode::Esc => {
                        app.dialog = Dialog::None;
                        app.status_message = "Cancelled".to_string();
                    }
                    KeyCode::Char(c) => input.insert(c),
                    KeyCode::Backspace => input.backspace(),
                    KeyCode::Delete => input.delete(),
                    KeyCode::Left => input.move_left(),
                    KeyCode::Right => input.move_right(),
                    _ => {}
                },
                Dialog::ConfirmCopy { source_path, input } => {
                    let path = source_path.clone();
                    match key.code {
                        KeyCode::Enter => {
                            let dest = input.text.clone();
                            app.dialog = Dialog::None;
                            app.execute_copy(path, dest);
                        }
                        KeyCode::Esc => {
                            app.dialog = Dialog::None;
                            app.status_message = "Cancelled".to_string();
                        }
                        KeyCode::Char(c) => input.insert(c),
                        KeyCode::Backspace => input.backspace(),
                        KeyCode::Delete => input.delete(),
                        KeyCode::Left => input.move_left(),
                        KeyCode::Right => input.move_right(),
                        _ => {}
                    }
                }
                Dialog::ConfirmMove { source_path, input } => {
                    let path = source_path.clone();
                    match key.code {
                        KeyCode::Enter => {
                            let dest = input.text.clone();
                            app.dialog = Dialog::None;
                            app.execute_move(path, dest);
                        }
                        KeyCode::Esc => {
                            app.dialog = Dialog::None;
                            app.status_message = "Cancelled".to_string();
                        }
                        KeyCode::Char(c) => input.insert(c),
                        KeyCode::Backspace => input.backspace(),
                        KeyCode::Delete => input.delete(),
                        KeyCode::Left => input.move_left(),
                        KeyCode::Right => input.move_right(),
                        _ => {}
                    }
                }
                Dialog::CommandLine { input } => match key.code {
                    KeyCode::Enter => {
                        let cmd = input.text.clone();
                        app.dialog = Dialog::None;
                        app.execute_shell_command(terminal, cmd);
                    }
                    KeyCode::Esc => {
                        app.dialog = Dialog::None;
                    }
                    KeyCode::Char(c) => input.insert(c),
                    KeyCode::Backspace => input.backspace(),
                    KeyCode::Delete => input.delete(),
                    KeyCode::Left => input.move_left(),
                    KeyCode::Right => input.move_right(),
                    _ => {}
                },
                Dialog::Filter { input } => {
                    match key.code {
                        KeyCode::Enter => {
                            app.dialog = Dialog::None;
                        }
                        KeyCode::Esc => {
                            app.dialog = Dialog::None;
                            let panel = app.get_active_panel_mut();
                            panel.filter = None;
                            panel.refresh();
                        }
                        KeyCode::Char(c) => {
                            input.insert(c);
                            let text = input.text.clone();
                            let panel = app.get_active_panel_mut();
                            panel.filter = Some(text);
                            panel.refresh();
                        }
                        KeyCode::Backspace => {
                            input.backspace();
                            let text = input.text.clone();
                            let panel = app.get_active_panel_mut();
                            if text.is_empty() {
                                panel.filter = None;
                            } else {
                                panel.filter = Some(text);
                            }
                            panel.refresh();
                        }
                        KeyCode::Delete => {
                            input.delete();
                            let text = input.text.clone();
                            let panel = app.get_active_panel_mut();
                            if text.is_empty() {
                                panel.filter = None;
                            } else {
                                panel.filter = Some(text);
                            }
                            panel.refresh();
                        }
                        KeyCode::Left => input.move_left(),
                        KeyCode::Right => input.move_right(),
                        _ => {}
                    }
                }
                Dialog::Settings { active_row } => {
                    match key.code {
                        KeyCode::Esc => {
                            app.dialog = Dialog::None;
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            if *active_row > 0 {
                                *active_row -= 1;
                            }
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if *active_row < 5 {
                                *active_row += 1;
                            }
                        }
                        KeyCode::Char(' ') | KeyCode::Enter => {
                            match *active_row {
                                0 => {
                                    app.config.show_hidden = !app.config.show_hidden;
                                }
                                1 => {
                                    app.config.sort_by = match app.config.sort_by.as_str() {
                                        "name" => "size".to_string(),
                                        "size" => "time".to_string(),
                                        _ => "name".to_string(),
                                    };
                                }
                                2 => {
                                    app.config.keybindings = match app.config.keybindings.as_str() {
                                        "standard" => "vim".to_string(),
                                        _ => "standard".to_string(),
                                    };
                                }
                                3 => {
                                    app.config.confirm_quit = !app.config.confirm_quit;
                                }
                                4 => {
                                    app.dialog = Dialog::InputEditor {
                                        input: InputField::new(app.config.default_editor.clone()),
                                    };
                                }
                                5 => {
                                    let _ = save_config(&app.config);
                                    app.apply_config();
                                    app.dialog = Dialog::None;
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
                Dialog::ConfirmQuit => match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                        app.should_quit = true;
                    }
                    _ => {
                        app.dialog = Dialog::None;
                    }
                }
                Dialog::InputEditor { input } => match key.code {
                    KeyCode::Enter => {
                        let text = input.text.clone();
                        app.config.default_editor = text;
                        app.dialog = Dialog::Settings { active_row: 4 };
                    }
                    KeyCode::Esc => {
                        app.dialog = Dialog::Settings { active_row: 4 };
                    }
                    KeyCode::Char(c) => input.insert(c),
                    KeyCode::Backspace => input.backspace(),
                    KeyCode::Delete => input.delete(),
                    KeyCode::Left => input.move_left(),
                    KeyCode::Right => input.move_right(),
                    _ => {}
                }
                Dialog::Bookmarks { selected_idx } => match key.code {
                    KeyCode::Esc => {
                        app.dialog = Dialog::None;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        if *selected_idx > 0 {
                            *selected_idx -= 1;
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if !app.config.bookmarks.is_empty() && *selected_idx + 1 < app.config.bookmarks.len() {
                            *selected_idx += 1;
                        }
                    }
                    KeyCode::Enter => {
                        if let Some(path) = app.config.bookmarks.get(*selected_idx).cloned() {
                            if path.exists() {
                                let _ = app.get_active_panel_mut().set_path(path);
                                app.dialog = Dialog::None;
                                app.status_message = "Jumped to bookmark".to_string();
                            } else {
                                app.status_message = "Error: Bookmarked path does not exist".to_string();
                            }
                        }
                    }
                    KeyCode::Char('a') | KeyCode::Char('A') => {
                        let active_path = app.get_active_panel().path.clone();
                        if !app.config.bookmarks.contains(&active_path) {
                            app.config.bookmarks.push(active_path);
                            let _ = save_config(&app.config);
                            app.status_message = "Added folder to bookmarks".to_string();
                        } else {
                            app.status_message = "Folder is already bookmarked".to_string();
                        }
                    }
                    KeyCode::Char('d') | KeyCode::Char('D') => {
                        if !app.config.bookmarks.is_empty() {
                            app.config.bookmarks.remove(*selected_idx);
                            let _ = save_config(&app.config);
                            if *selected_idx > 0 && *selected_idx >= app.config.bookmarks.len() {
                                *selected_idx = app.config.bookmarks.len() - 1;
                            }
                            app.status_message = "Removed bookmark".to_string();
                        }
                    }
                    _ => {}
                }
                Dialog::TerminalOverlay { input, output_lines, scroll_offset } => match key.code {
                    KeyCode::Esc => {
                        app.dialog = Dialog::None;
                    }
                    KeyCode::Enter => {
                        let text = input.text.clone();
                        if !text.is_empty() {
                            if text == "clear" {
                                output_lines.clear();
                                *scroll_offset = 0;
                            } else {
                                output_lines.push(format!("❯ {}", text));
                                let (new_lines, needs_clear) = App::execute_overlay_command(&active_dir, &text);
                                output_lines.extend(new_lines);

                                // If a TUI program was run, the terminal needs a full clear
                                // before ratatui redraws, otherwise we get render artifacts.
                                if needs_clear {
                                    let _ = terminal.clear();
                                }

                                // Auto-scroll to bottom
                                let area = match terminal.size() {
                                    Ok(size) => centered_rect(85, 75, Rect::new(0, 0, size.width, size.height)),
                                    Err(_) => centered_rect(85, 75, Rect::new(0, 0, 80, 24)),
                                };
                                let display_height = area.height.saturating_sub(4) as usize;

                                if output_lines.len() > display_height {
                                    *scroll_offset = output_lines.len() - display_height;
                                } else {
                                    *scroll_offset = 0;
                                }
                            }
                            input.text.clear();
                            input.cursor_position = 0;
                        }
                    }
                    KeyCode::PageUp => {
                        *scroll_offset = scroll_offset.saturating_sub(5);
                    }
                    KeyCode::PageDown => {
                        let area = match terminal.size() {
                            Ok(size) => centered_rect(85, 75, Rect::new(0, 0, size.width, size.height)),
                            Err(_) => centered_rect(85, 75, Rect::new(0, 0, 80, 24)),
                        };
                        let display_height = area.height.saturating_sub(4) as usize;
                        if *scroll_offset + display_height < output_lines.len() {
                            *scroll_offset = (*scroll_offset + 5).min(output_lines.len().saturating_sub(display_height));
                        }
                    }
                    KeyCode::Up => {
                        *scroll_offset = scroll_offset.saturating_sub(1);
                    }
                    KeyCode::Down => {
                        let area = match terminal.size() {
                            Ok(size) => centered_rect(85, 75, Rect::new(0, 0, size.width, size.height)),
                            Err(_) => centered_rect(85, 75, Rect::new(0, 0, 80, 24)),
                        };
                        let display_height = area.height.saturating_sub(4) as usize;
                        if *scroll_offset + display_height < output_lines.len() {
                            *scroll_offset += 1;
                        }
                    }
                    KeyCode::Char(c) => input.insert(c),
                    KeyCode::Backspace => input.backspace(),
                    KeyCode::Delete => input.delete(),
                    KeyCode::Left => input.move_left(),
                    KeyCode::Right => input.move_right(),
                    _ => {}
                }
                Dialog::Menu { active_menu, active_item } => {
                    match active_item {
                        None => {
                            match key.code {
                                KeyCode::Esc => {
                                    app.dialog = Dialog::None;
                                }
                                KeyCode::Left | KeyCode::Char('h') => {
                                    *active_menu = (*active_menu + 4) % 5;
                                }
                                KeyCode::Right | KeyCode::Char('l') => {
                                    *active_menu = (*active_menu + 1) % 5;
                                }
                                KeyCode::Down | KeyCode::Char('j') | KeyCode::Enter | KeyCode::Char(' ') => {
                                    *active_item = Some(0);
                                }
                                _ => {}
                            }
                        }
                        Some(item_idx) => {
                            let max_items = match *active_menu {
                                0 | 4 => 4,
                                1 => 6,
                                2 => 4,
                                3 => 3,
                                _ => 0,
                            };
                            match key.code {
                                KeyCode::Esc => {
                                    *active_item = None;
                                }
                                KeyCode::Up | KeyCode::Char('k') => {
                                    if *item_idx == 0 {
                                        *item_idx = max_items - 1;
                                    } else {
                                        *item_idx -= 1;
                                    }
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    *item_idx = (*item_idx + 1) % max_items;
                                }
                                KeyCode::Left | KeyCode::Char('h') => {
                                    *active_menu = (*active_menu + 4) % 5;
                                    *active_item = Some(0);
                                }
                                KeyCode::Right | KeyCode::Char('l') => {
                                    *active_menu = (*active_menu + 1) % 5;
                                    *active_item = Some(0);
                                }
                                KeyCode::Enter | KeyCode::Char(' ') => {
                                    let m_idx = *active_menu;
                                    let i_idx = *item_idx;
                                    execute_menu_action(&mut app, m_idx, i_idx, terminal);
                                }
                                _ => {}
                            }
                        }
                    }
                }
                Dialog::ViewFile { scroll_offset, content, .. } => {
                    let lines_count = content.lines().count();
                    match key.code {
                        KeyCode::Up | KeyCode::Char('k') => {
                            *scroll_offset = scroll_offset.saturating_sub(1);
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if *scroll_offset + 5 < lines_count {
                                *scroll_offset += 1;
                            }
                        }
                        KeyCode::PageUp => {
                            *scroll_offset = scroll_offset.saturating_sub(15);
                        }
                        KeyCode::PageDown => {
                            if *scroll_offset + 15 < lines_count {
                                *scroll_offset += 15;
                            } else {
                                *scroll_offset = lines_count.saturating_sub(5);
                            }
                        }
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::F(3) => {
                            app.dialog = Dialog::None;
                        }
                        _ => {}
                    }
                }
                Dialog::CommandOutput { scroll_offset, output, .. } => {
                    let lines_count = output.lines().count();
                    match key.code {
                        KeyCode::Up | KeyCode::Char('k') => {
                            *scroll_offset = scroll_offset.saturating_sub(1);
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if *scroll_offset + 5 < lines_count {
                                *scroll_offset += 1;
                            }
                        }
                        KeyCode::PageUp => {
                            *scroll_offset = scroll_offset.saturating_sub(15);
                        }
                        KeyCode::PageDown => {
                            if *scroll_offset + 15 < lines_count {
                                *scroll_offset += 15;
                            } else {
                                *scroll_offset = lines_count.saturating_sub(5);
                            }
                        }
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char(' ') | KeyCode::Enter => {
                            app.dialog = Dialog::None;
                        }
                        _ => {}
                    }
                }
                Dialog::Help { active_tab } => {
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::F(1) => {
                            app.dialog = Dialog::None;
                        }
                        KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
                            *active_tab = (*active_tab + 1) % 4;
                        }
                        KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                            *active_tab = (*active_tab + 3) % 4;
                        }
                        KeyCode::Char('1') => { *active_tab = 0; }
                        KeyCode::Char('2') => { *active_tab = 1; }
                        KeyCode::Char('3') => { *active_tab = 2; }
                        KeyCode::Char('4') => { *active_tab = 3; }
                        _ => {}
                    }
                }
                Dialog::Error { .. } => {
                    if key.code == KeyCode::Esc || key.code == KeyCode::Enter || key.code == KeyCode::Char(' ') {
                        app.dialog = Dialog::None;
                    }
                }
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

fn handle_main_keys(app: &mut App, key: KeyEvent, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) {
    let keys = &app.keymap;

    if matches_key(&key, &keys.quit) {
        if app.config.confirm_quit {
            app.dialog = Dialog::ConfirmQuit;
        } else {
            app.should_quit = true;
        }
    } else if matches_key(&key, &keys.help) {
        app.dialog = Dialog::Help { active_tab: 0 };
    } else if matches_key(&key, &keys.view) {
        if let Some(item) = app.get_active_panel().get_selected_item().cloned() {
            if !item.is_dir {
                app.open_viewer(item.path);
            }
        }
    } else if matches_key(&key, &keys.edit) {
        app.open_editor(terminal);
    } else if matches_key(&key, &keys.copy) {
        app.initiate_copy();
    } else if matches_key(&key, &keys.move_item) {
        app.initiate_move();
    } else if matches_key(&key, &keys.mkdir) {
        app.initiate_mkdir();
    } else if matches_key(&key, &keys.delete) {
        app.initiate_delete();
    } else if matches_key(&key, &keys.menu) {
        app.dialog = Dialog::Menu {
            active_menu: 0,
            active_item: None,
        };
    } else if matches_key(&key, &keys.toggle_hidden) {
        app.config.show_hidden = !app.config.show_hidden;
        app.apply_config();
        app.status_message = format!("Hidden files: {}", if app.config.show_hidden { "Shown" } else { "Hidden" });
    } else if matches_key(&key, &keys.toggle_preview) {
        app.preview_mode = !app.preview_mode;
        app.status_message = format!("Quick View Pane: {}", if app.preview_mode { "ON" } else { "OFF" });
    } else if key.code == KeyCode::Char('t') && key.modifiers.contains(KeyModifiers::CONTROL) {
        app.tree_mode = !app.tree_mode;
        if app.tree_mode {
            app.init_tree();
            app.active_panel = ActivePanel::Left;
            app.update_right_panel_from_tree();
        }
        app.status_message = format!("Tree View: {}", if app.tree_mode { "ON" } else { "OFF" });
    } else if key.code == KeyCode::Char('o') && key.modifiers.contains(KeyModifiers::CONTROL) {
        app.dialog = Dialog::TerminalOverlay {
            input: InputField::new(String::new()),
            output_lines: vec![
                "Welcome to Rust Commander Terminal Overlay.".to_string(),
                "Type shell commands and press [Enter] to execute. Press [Esc] to exit.".to_string(),
                "Use [PageUp]/[PageDown] to scroll command output history.".to_string(),
                "".to_string(),
            ],
            scroll_offset: 0,
        };
    } else if key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL) {
        app.dialog = Dialog::Settings { active_row: 0 };
    } else if key.code == KeyCode::Char('b') && key.modifiers.contains(KeyModifiers::CONTROL) {
        app.dialog = Dialog::Bookmarks { selected_idx: 0 };
    } else if key.code == KeyCode::Char('d') && key.modifiers.contains(KeyModifiers::CONTROL) {
        let active_path = app.get_active_panel().path.clone();
        if !app.config.bookmarks.contains(&active_path) {
            app.config.bookmarks.push(active_path);
            let _ = save_config(&app.config);
            app.status_message = "Added folder to bookmarks".to_string();
        } else {
            app.status_message = "Folder is already bookmarked".to_string();
        }
    } else if key.code == KeyCode::Char('a') && key.modifiers.contains(KeyModifiers::CONTROL) {
        let panel = app.get_active_panel_mut();
        let all_paths: Vec<PathBuf> = panel.items.iter()
            .filter(|i| i.name != "..")
            .map(|i| i.path.clone())
            .collect();
        if panel.marked.len() == all_paths.len() {
            panel.marked.clear();
            app.status_message = "Deselected all".to_string();
        } else {
            for p in all_paths {
                panel.marked.insert(p);
            }
            app.status_message = format!("Selected {} items", panel.marked.len());
        }
    } else if matches_key(&key, &keys.select_item) {
        if let Some(item) = app.get_active_panel().get_selected_item().cloned() {
            if item.name != ".." {
                let panel = app.get_active_panel_mut();
                if panel.marked.contains(&item.path) {
                    panel.marked.remove(&item.path);
                } else {
                    panel.marked.insert(item.path.clone());
                }
                panel.select_next();
            }
        }
    } else if matches_key(&key, &keys.up) {
        if app.tree_mode && app.active_panel == ActivePanel::Left {
            if app.tree_selected > 0 {
                app.tree_selected -= 1;
                app.update_right_panel_from_tree();
            }
        } else {
            app.get_active_panel_mut().select_prev();
        }
    } else if matches_key(&key, &keys.down) {
        if app.tree_mode && app.active_panel == ActivePanel::Left {
            if app.tree_selected + 1 < app.tree_nodes.len() {
                app.tree_selected += 1;
                app.update_right_panel_from_tree();
            }
        } else {
            app.get_active_panel_mut().select_next();
        }
    } else if matches_key(&key, &keys.left) {
        if app.tree_mode && app.active_panel == ActivePanel::Left {
            let idx = app.tree_selected;
            if idx < app.tree_nodes.len() {
                if app.tree_nodes[idx].is_expanded {
                    app.toggle_tree_node();
                } else if app.tree_nodes[idx].depth > 0 {
                    let current_depth = app.tree_nodes[idx].depth;
                    let mut parent_idx = idx;
                    while parent_idx > 0 {
                        parent_idx -= 1;
                        if app.tree_nodes[parent_idx].depth < current_depth {
                            app.tree_selected = parent_idx;
                            app.update_right_panel_from_tree();
                            break;
                        }
                    }
                }
            }
        } else {
            app.handle_backspace();
        }
    } else if matches_key(&key, &keys.right) {
        if app.tree_mode && app.active_panel == ActivePanel::Left {
            app.toggle_tree_node();
        } else {
            app.handle_enter();
        }
    } else if matches_key(&key, &keys.tab) {
        app.toggle_panel();
    } else {
        // Fallback controls
        match key.code {
            KeyCode::PageUp => {
                if app.tree_mode && app.active_panel == ActivePanel::Left {
                    app.tree_selected = app.tree_selected.saturating_sub(10);
                    app.update_right_panel_from_tree();
                } else {
                    app.get_active_panel_mut().page_up();
                }
            }
            KeyCode::PageDown => {
                if app.tree_mode && app.active_panel == ActivePanel::Left {
                    if !app.tree_nodes.is_empty() {
                        app.tree_selected = std::cmp::min(app.tree_selected + 10, app.tree_nodes.len() - 1);
                        app.update_right_panel_from_tree();
                    }
                } else {
                    app.get_active_panel_mut().page_down();
                }
            }
            KeyCode::Home => {
                if app.tree_mode && app.active_panel == ActivePanel::Left {
                    app.tree_selected = 0;
                    app.update_right_panel_from_tree();
                } else {
                    app.get_active_panel_mut().select_first();
                }
            }
            KeyCode::End => {
                if app.tree_mode && app.active_panel == ActivePanel::Left {
                    if !app.tree_nodes.is_empty() {
                        app.tree_selected = app.tree_nodes.len() - 1;
                        app.update_right_panel_from_tree();
                    }
                } else {
                    app.get_active_panel_mut().select_last();
                }
            }
            KeyCode::Char(':') => {
                app.dialog = Dialog::CommandLine {
                    input: InputField::new(String::new()),
                };
            }
            KeyCode::Char('/') => {
                app.dialog = Dialog::Filter {
                    input: InputField::new(String::new()),
                };
            }
            KeyCode::Char('r') => {
                app.refresh_panels();
                app.status_message = "Panels reloaded".to_string();
            }
            KeyCode::Char('~') => {
                if let Ok(home) = env::var("HOME").or_else(|_| env::var("USERPROFILE")) {
                    let home_path = PathBuf::from(&home);
                    if home_path.is_dir() {
                        let _ = app.get_active_panel_mut().set_path(home_path);
                        app.status_message = "Jumped to home directory".to_string();
                    }
                }
            }
            _ => {}
        }
    }
}

// =============================================================================
// UI Drawing Layouts & Formatting
// =============================================================================

fn ui(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Top Header Bar
            Constraint::Min(3),    // Middle Workspace area
            Constraint::Length(1), // Bottom Status Line
            Constraint::Length(1), // Bottom Hotkey Legend Bar
        ])
        .split(f.area());



    // 2. Middle Workspace Layout (Twin split panels, or active panel + preview, or Tree view)
    let panels_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Percentage(50),
        ])
        .split(chunks[1]);

    let theme = Theme::get_theme(&app.config.theme.clone());

    // 1. Top Header Bar (Interactive menu tabs) — theme aware
    let header_rect = chunks[0];
    
    let active_menu_idx = match app.dialog {
        Dialog::Menu { active_menu, .. } => Some(active_menu),
        _ => None,
    };

    let active_tab_style = Style::default().bg(theme.active_border).fg(Color::Black).bold();
    let idle_tab_style   = Style::default().fg(Color::White);
    let left_style  = if active_menu_idx == Some(0) { active_tab_style } else { idle_tab_style };
    let file_style  = if active_menu_idx == Some(1) { active_tab_style } else { idle_tab_style };
    let cmd_style   = if active_menu_idx == Some(2) { active_tab_style } else { idle_tab_style };
    let opt_style   = if active_menu_idx == Some(3) { active_tab_style } else { idle_tab_style };
    let right_style = if active_menu_idx == Some(4) { active_tab_style } else { idle_tab_style };

    let menu_spans = vec![
        Span::raw("  "),
        Span::styled(" Left ",    left_style),
        Span::raw("   "),
        Span::styled(" File ",    file_style),
        Span::raw("   "),
        Span::styled(" Command ", cmd_style),
        Span::raw("   "),
        Span::styled(" Options ", opt_style),
        Span::raw("   "),
        Span::styled(" Right ",   right_style),
        Span::raw("   │ "),
        Span::styled("RUST COMMANDER", Style::default().fg(theme.accent).bold()),
        Span::styled(format!(" [Bindings: {}]", app.config.keybindings), Style::default().fg(Color::DarkGray)),
    ];
    f.render_widget(Paragraph::new(Line::from(menu_spans)).bg(theme.header_bg), header_rect);

    if app.tree_mode {
        draw_tree_panel(f, panels_layout[0], app, app.active_panel == ActivePanel::Left, &theme);
        draw_beautiful_contents_panel(f, panels_layout[1], app, app.active_panel == ActivePanel::Right, &theme);
    } else if app.preview_mode {
        match app.active_panel {
            ActivePanel::Left => {
                draw_panel(f, panels_layout[0], &mut app.left_panel, true, &theme);
                let selected_item = app.left_panel.get_selected_item().cloned();
                draw_live_preview(f, panels_layout[1], selected_item, app);
            }
            ActivePanel::Right => {
                let selected_item = app.right_panel.get_selected_item().cloned();
                draw_live_preview(f, panels_layout[0], selected_item, app);
                draw_panel(f, panels_layout[1], &mut app.right_panel, true, &theme);
            }
        }
    } else {
        draw_panel(f, panels_layout[0], &mut app.left_panel, app.active_panel == ActivePanel::Left, &theme);
        draw_panel(f, panels_layout[1], &mut app.right_panel, app.active_panel == ActivePanel::Right, &theme);
    }

    // 3. Bottom Status Line
    let status_rect = chunks[2];
    match &app.dialog {
        Dialog::CommandLine { input } => {
            let line = Line::from(vec![
                Span::styled("Run Command: ", Style::default().fg(Color::Yellow).bold()),
                Span::raw(input.text.as_str()),
            ]);
            f.render_widget(Paragraph::new(line).bg(Color::Rgb(30, 41, 59)), status_rect);
            f.set_cursor_position(Position::new(
                status_rect.x + 13 + input.visual_cursor_col(),
                status_rect.y,
            ));
        }
        Dialog::Filter { input } => {
            let line = Line::from(vec![
                Span::styled("Filter: ", Style::default().fg(Color::Cyan).bold()),
                Span::raw(input.text.as_str()),
            ]);
            f.render_widget(Paragraph::new(line).bg(Color::Rgb(30, 41, 59)), status_rect);
            f.set_cursor_position(Position::new(
                status_rect.x + 8 + input.visual_cursor_col(),
                status_rect.y,
            ));
        }
        _ => {
            let status_para = Paragraph::new(app.status_message.as_str())
                .style(Style::default().bg(Color::Rgb(15, 23, 42)).fg(Color::Rgb(241, 245, 249)));
            f.render_widget(status_para, status_rect);
        }
    }

    // 4. Hotkey Legend Bar — theme-aware
    let legend_rect = chunks[3];
    let fn_key = Style::default().bg(theme.active_selection_bg).fg(Color::White).bold();
    let ctrl_key = Style::default().bg(theme.inactive_selection_bg).fg(Color::White).bold();
    let label_style = Style::default().bg(theme.header_bg).fg(Color::Rgb(203, 213, 225));
    let quit_key = Style::default().bg(Color::Rgb(220, 38, 38)).fg(Color::White).bold();

    let legend_spans = vec![
        Span::styled(" F1 ",     fn_key),
        Span::styled("Help ",    label_style),
        Span::styled(" F3 ",     fn_key),
        Span::styled("View ",    label_style),
        Span::styled(" F4 ",     fn_key),
        Span::styled("Edit ",    label_style),
        Span::styled(" F5 ",     fn_key),
        Span::styled("Copy ",    label_style),
        Span::styled(" F6 ",     fn_key),
        Span::styled("Move ",    label_style),
        Span::styled(" F7 ",     fn_key),
        Span::styled("MkDir ",   label_style),
        Span::styled(" F8 ",     fn_key),
        Span::styled("Delete ",  label_style),
        Span::styled(" F9 ",     fn_key),
        Span::styled("Menu ",    label_style),
        Span::styled(" Tab ",    ctrl_key),
        Span::styled("Switch ",  label_style),
        Span::styled(" Space ",  ctrl_key),
        Span::styled("Tag ",     label_style),
        Span::styled(" . ",      ctrl_key),
        Span::styled("Hidden ",  label_style),
        Span::styled(" Ctrl+P ", ctrl_key),
        Span::styled("Preview ", label_style),
        Span::styled(" Ctrl+T ", ctrl_key),
        Span::styled("Tree ",    label_style),
        Span::styled(" Ctrl+B ", ctrl_key),
        Span::styled("Bookmarks ", label_style),
        Span::styled(" Ctrl+O ", ctrl_key),
        Span::styled("Shell ",   label_style),
        Span::styled(" Ctrl+S ", ctrl_key),
        Span::styled("Settings ", label_style),
        Span::styled(" F10 ",    quit_key),
        Span::styled("Quit ",    label_style),
    ];
    
    let legend_para = Paragraph::new(Line::from(legend_spans)).bg(theme.header_bg);
    f.render_widget(legend_para, legend_rect);


    // =========================================================================
    // Dialog Popups Rendering Overlays
    // =========================================================================

    match &app.dialog {
        Dialog::None | Dialog::CommandLine { .. } | Dialog::Filter { .. } => {}
        Dialog::Menu { active_menu, active_item } => {
            if let Some(item_idx) = active_item {
                draw_menu_dropdown(f, *active_menu, *item_idx);
            }
        }
        Dialog::ConfirmDelete { item_name, .. } => {
            let area = centered_rect(50, 20, f.area());
            f.render_widget(Clear, area);
            let block = Block::default()
                .title(" Confirm Delete ")
                .title_alignment(Alignment::Center)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(239, 68, 68)))
                .bg(Color::Rgb(17, 24, 39));
            
            let text = vec![
                Line::from(""),
                Line::from(vec![
                    Span::raw("Are you sure you want to delete '"),
                    Span::styled(item_name, Style::default().fg(Color::Yellow).bold()),
                    Span::raw("'?"),
                ]).alignment(Alignment::Center),
                Line::from(""),
                Line::from("Press [Y] or [Enter] to delete, [N] or [Esc] to cancel.").alignment(Alignment::Center),
            ];
            
            let para = Paragraph::new(text).block(block);
            f.render_widget(para, area);
        }
        Dialog::InputMkdir { input } => {
            let area = centered_rect(60, 20, f.area());
            f.render_widget(Clear, area);
            let block = Block::default()
                .title(" Create Directory ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(56, 189, 248)))
                .bg(Color::Rgb(17, 24, 39));
            
            let label = Paragraph::new("Enter directory name:").block(Block::default());
            let input_text = Paragraph::new(input.text.as_str())
                .style(Style::default().bg(Color::Rgb(55, 65, 81)).fg(Color::White))
                .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)));

            let sub_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Length(3),
                    Constraint::Min(0),
                ])
                .margin(1)
                .split(area);

            f.render_widget(Clear, area);
            f.render_widget(block, area);
            f.render_widget(label, sub_chunks[0]);
            f.render_widget(input_text, sub_chunks[1]);
            
            f.set_cursor_position(Position::new(
                sub_chunks[1].x + 1 + input.visual_cursor_col(),
                sub_chunks[1].y + 1,
            ));
        }
        Dialog::ConfirmCopy { source_path, input } => {
            let area = centered_rect(65, 20, f.area());
            f.render_widget(Clear, area);
            let name = if source_path.as_os_str().is_empty() {
                "Selected items (bulk)".to_string()
            } else {
                source_path.file_name().unwrap_or_default().to_string_lossy().to_string()
            };
            let block = Block::default()
                .title(format!(" Copy: {} ", name))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(56, 189, 248)))
                .bg(Color::Rgb(17, 24, 39));
            
            let label = Paragraph::new("Copy to location:").block(Block::default());
            let input_text = Paragraph::new(input.text.as_str())
                .style(Style::default().bg(Color::Rgb(55, 65, 81)).fg(Color::White))
                .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)));

            let sub_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Length(3),
                    Constraint::Min(0),
                ])
                .margin(1)
                .split(area);

            f.render_widget(Clear, area);
            f.render_widget(block, area);
            f.render_widget(label, sub_chunks[0]);
            f.render_widget(input_text, sub_chunks[1]);
            
            f.set_cursor_position(Position::new(
                sub_chunks[1].x + 1 + input.visual_cursor_col(),
                sub_chunks[1].y + 1,
            ));
        }
        Dialog::ConfirmMove { source_path, input } => {
            let area = centered_rect(65, 20, f.area());
            f.render_widget(Clear, area);
            let name = if source_path.as_os_str().is_empty() {
                "Selected items (bulk)".to_string()
            } else {
                source_path.file_name().unwrap_or_default().to_string_lossy().to_string()
            };
            let block = Block::default()
                .title(format!(" Move: {} ", name))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(56, 189, 248)))
                .bg(Color::Rgb(17, 24, 39));
            
            let label = Paragraph::new("Move/rename to location:").block(Block::default());
            let input_text = Paragraph::new(input.text.as_str())
                .style(Style::default().bg(Color::Rgb(55, 65, 81)).fg(Color::White))
                .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)));

            let sub_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Length(3),
                    Constraint::Min(0),
                ])
                .margin(1)
                .split(area);

            f.render_widget(Clear, area);
            f.render_widget(block, area);
            f.render_widget(label, sub_chunks[0]);
            f.render_widget(input_text, sub_chunks[1]);
            
            f.set_cursor_position(Position::new(
                sub_chunks[1].x + 1 + input.visual_cursor_col(),
                sub_chunks[1].y + 1,
            ));
        }
        Dialog::ViewFile { path, content, scroll_offset } => {
            let area = centered_rect(90, 90, f.area());
            f.render_widget(Clear, area);
            let filename = path.file_name().unwrap_or_default().to_string_lossy();
            
            let block = Block::default()
                .title(format!(" Viewer: {} (Esc to Close) ", filename))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(79, 70, 229)))
                .bg(Color::Rgb(15, 23, 42));

            let lines: Vec<&str> = content.lines().collect();
            let visible_lines = area.height.saturating_sub(2) as usize;
            let display_lines = lines
                .iter()
                .skip(*scroll_offset)
                .take(visible_lines)
                .map(|&s| Line::from(s))
                .collect::<Vec<Line>>();

            let para = Paragraph::new(display_lines)
                .block(block)
                .wrap(Wrap { trim: false });
            f.render_widget(para, area);
        }
        Dialog::Settings { active_row } => {
            let area = centered_rect(70, 65, f.area());
            f.render_widget(Clear, area);
            let block = Block::default()
                .title(" Settings Configuration ")
                .title_alignment(Alignment::Center)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(6, 182, 212)))
                .bg(Color::Rgb(17, 24, 39));

            let r0_style = if *active_row == 0 { Style::default().fg(Color::Yellow).bold() } else { Style::default() };
            let r1_style = if *active_row == 1 { Style::default().fg(Color::Yellow).bold() } else { Style::default() };
            let r2_style = if *active_row == 2 { Style::default().fg(Color::Yellow).bold() } else { Style::default() };
            let r3_style = if *active_row == 3 { Style::default().fg(Color::Yellow).bold() } else { Style::default() };
            let r4_style = if *active_row == 4 { Style::default().fg(Color::Yellow).bold() } else { Style::default() };
            let r5_style = if *active_row == 5 { Style::default().fg(Color::Green).bold() } else { Style::default() };

            let r0_check = if app.config.show_hidden { "[X] Show" } else { "[ ] Hide" };
            let r1_val = format!("< {} >", app.config.sort_by.to_uppercase());
            let r2_val = format!("< {} >", app.config.keybindings.to_uppercase());
            let r3_check = if app.config.confirm_quit { "[X] Enabled" } else { "[ ] Disabled" };
            let r4_val = format!("[ {} ] (Press Space/Enter to Edit)", app.config.default_editor);

            let text = vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled("  Show Hidden Files:   ", r0_style),
                    Span::styled(r0_check, Style::default().fg(Color::Cyan)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("  Sorting Criteria:    ", r1_style),
                    Span::styled(r1_val, Style::default().fg(Color::Cyan)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("  Keybindings Mode:    ", r2_style),
                    Span::styled(r2_val, Style::default().fg(Color::Cyan)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("  Quit Confirmation:   ", r3_style),
                    Span::styled(r3_check, Style::default().fg(Color::Cyan)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("  Default Editor:      ", r4_style),
                    Span::styled(r4_val, Style::default().fg(Color::Cyan)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("     [ SAVE & CLOSE CONFIGURATION ]     ", r5_style),
                ]).alignment(Alignment::Center),
                Line::from(""),
                Line::from("Press [Up/Down] to navigate, [Space/Enter] to change, [Esc] to exit").alignment(Alignment::Center),
            ];

            let para = Paragraph::new(text).block(block);
            f.render_widget(para, area);
        }
        Dialog::ConfirmQuit => {
            let area = centered_rect(44, 30, f.area());
            f.render_widget(Clear, area);
            let block = Block::default()
                .title(" ⏻ EXIT CONFIRMATION ")
                .title_alignment(Alignment::Center)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(220, 60, 60)))
                .bg(Color::Rgb(3, 10, 18));

            let text = vec![
                Line::from(""),
                Line::from(Span::styled(
                    "Terminate session?",
                    Style::default().fg(Color::Rgb(220, 60, 60)).bold(),
                )).alignment(Alignment::Center),
                Line::from(""),
                Line::from(Span::styled(
                    "All unsaved state will be lost.",
                    Style::default().fg(Color::Rgb(140, 190, 200)),
                )).alignment(Alignment::Center),
                Line::from(""),
                Line::from(""),
                Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(" Y / Enter ", Style::default().bg(Color::Rgb(220, 60, 60)).fg(Color::White).bold()),
                    Span::styled("  Confirm   ", Style::default().fg(Color::Rgb(140, 190, 200))),
                ]).alignment(Alignment::Center),
                Line::from(""),
                Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(" Esc / N   ", Style::default().bg(Color::Rgb(0, 60, 80)).fg(Color::Rgb(0, 210, 220)).bold()),
                    Span::styled("  Cancel    ", Style::default().fg(Color::Rgb(140, 190, 200))),
                ]).alignment(Alignment::Center),
                Line::from(""),
            ];

            let para = Paragraph::new(text).block(block);
            f.render_widget(para, area);
        }
        Dialog::InputEditor { input } => {
            let area = centered_rect(60, 20, f.area());
            f.render_widget(Clear, area);
            let block = Block::default()
                .title(" Configure Default Editor ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(56, 189, 248)))
                .bg(Color::Rgb(17, 24, 39));
            
            let label = Paragraph::new("Enter default text editor name/path:").block(Block::default());
            let input_text = Paragraph::new(input.text.as_str())
                .style(Style::default().bg(Color::Rgb(55, 65, 81)).fg(Color::White))
                .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)));

            let sub_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Length(3),
                    Constraint::Min(0),
                ])
                .margin(1)
                .split(area);

            f.render_widget(Clear, area);
            f.render_widget(block, area);
            f.render_widget(label, sub_chunks[0]);
            f.render_widget(input_text, sub_chunks[1]);
            
            f.set_cursor_position(Position::new(
                sub_chunks[1].x + 1 + input.visual_cursor_col(),
                sub_chunks[1].y + 1,
            ));
        }
        Dialog::Help { active_tab } => {
            let area = centered_rect(72, 78, f.area());
            f.render_widget(Clear, area);

            let tab_titles = ["[1] Navigation", "[2] File Ops", "[3] Shell & View", "[4] Tips"];
            let tab_bar_line = Line::from(
                tab_titles.iter().enumerate().map(|(i, &t)| {
                    if i == *active_tab {
                        Span::styled(format!(" {} ", t), Style::default().bg(Color::Rgb(79, 70, 229)).fg(Color::White).bold())
                    } else {
                        Span::styled(format!(" {} ", t), Style::default().fg(Color::DarkGray))
                    }
                }).collect::<Vec<_>>()
            );

            let outer_block = Block::default()
                .title(Line::from(vec![
                    Span::styled(" ❓ RC Help ", Style::default().fg(Color::Cyan).bold()),
                ]))
                .title_alignment(Alignment::Center)
                .title_bottom(Line::from("  Tab/←/→: switch  1-4: jump  Esc/q: close  ").alignment(Alignment::Center))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(79, 70, 229)))
                .bg(Color::Rgb(13, 17, 28));

            let inner = outer_block.inner(area);
            f.render_widget(outer_block, area);

            let sections = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Min(0)])
                .split(inner);

            f.render_widget(Paragraph::new(tab_bar_line).bg(Color::Rgb(17, 24, 39)), sections[0]);
            f.render_widget(
                Paragraph::new("─".repeat(sections[1].width as usize)).fg(Color::Rgb(30, 41, 59)),
                sections[1]
            );

            let content_area = sections[2];
            let key = |s: &'static str| Span::styled(format!(" {:<13}", s), Style::default().fg(Color::Rgb(250, 204, 21)).bold());
            let desc = |s: &'static str| Span::styled(format!("  {}", s), Style::default().fg(Color::Rgb(203, 213, 225)));
            let head = |s: &'static str| Line::from(Span::styled(
                format!("  ── {} ", s),
                Style::default().fg(Color::Rgb(34, 211, 238)).bold()
            ));
            let row = |k: &'static str, d: &'static str| Line::from(vec![key(k), desc(d)]);

            let content: Vec<Line> = match *active_tab {
                0 => vec![
                    Line::from(""),
                    head("Panel Navigation"),
                    row("Tab",           "Switch active panel (Left ↔ Right)"),
                    row("↑ / k",         "Move cursor up"),
                    row("↓ / j",         "Move cursor down"),
                    row("Enter",         "Open directory or file viewer"),
                    row("Backspace",     "Go to parent directory"),
                    row("~",            "Jump to Home directory"),
                    row("g / Home",      "Jump to top of list"),
                    row("G / End",       "Jump to bottom of list"),
                    row("PgUp / PgDn",   "Scroll page up / down"),
                    Line::from(""),
                    head("Selection & Marking"),
                    row("Space",         "Tag/mark file for bulk operation"),
                    row(".",             "Toggle hidden files visibility"),
                    row("/",             "Filter current directory by name"),
                    row("Ctrl+A",        "Select all items in panel"),
                ],
                1 => vec![
                    Line::from(""),
                    head("File Operations"),
                    row("F5 / c",        "Copy selection to opposite panel"),
                    row("F6 / m",        "Move / Rename selection"),
                    row("F7 / n",        "Create new directory (mkdir)"),
                    row("F8 / Delete",   "Delete selection (with confirm)"),
                    Line::from(""),
                    head("Viewing & Editing"),
                    row("F3 / v",        "Full-screen text/binary viewer"),
                    row("F4 / e",        "Open in editor (configurable)"),
                    row("Enter",         "Open file with viewer"),
                    Line::from(""),
                    head("Sorting"),
                    row("Ctrl+S in menu","Sort by Name / Size / Date"),
                    Line::from(""),
                    head("Bookmarks"),
                    row("Ctrl+B",        "Open bookmarks manager"),
                    row("Ctrl+D",        "Add current dir to bookmarks"),
                ],
                2 => vec![
                    Line::from(""),
                    head("Terminal Overlay  (Ctrl+O)"),
                    row("Ctrl+O",        "Open interactive terminal shell"),
                    row("Enter",         "Execute typed command"),
                    row("↑ / ↓",         "Scroll output history"),
                    row("PgUp / PgDn",   "Fast scroll output"),
                    row("clear",         "Clear terminal output"),
                    row("Esc",           "Close terminal overlay"),
                    Line::from(""),
                    head("TUI Apps (auto-detected)"),
                    Line::from(Span::styled(
                        "  lazygit, vim, nvim, nano, htop, btop, mc, ranger,",
                        Style::default().fg(Color::Rgb(134, 239, 172))
                    )),
                    Line::from(Span::styled(
                        "  tig, fzf, top, less, man → launched in full-screen mode",
                        Style::default().fg(Color::Rgb(134, 239, 172))
                    )),
                    Line::from(""),
                    head("Preview & Tree"),
                    row("Ctrl+P",        "Toggle live file preview panel"),
                    row("Ctrl+T",        "Toggle directory tree view"),
                ],
                _ => vec![
                    Line::from(""),
                    head("Menu & Settings"),
                    row("F9 / F1",       "Open top menu bar / Help"),
                    row("Ctrl+S / o",    "Open settings configuration"),
                    row(":",             "Run shell command inline"),
                    row("?",            "This help dialog"),
                    Line::from(""),
                    head("Vim Keybindings"),
                    Line::from(Span::styled(
                        "  Enable in Settings → Keybindings → vim",
                        Style::default().fg(Color::Rgb(203, 213, 225))
                    )),
                    Line::from(Span::styled(
                        "  h/j/k/l = ←/↓/↑/→,  g/G = Home/End",
                        Style::default().fg(Color::Rgb(203, 213, 225))
                    )),
                    Line::from(""),
                    head("Config File"),
                    Line::from(Span::styled(
                        "  ~/.config/rc/config  (auto-saved on exit)",
                        Style::default().fg(Color::Rgb(203, 213, 225))
                    )),
                ],
            };

            let para = Paragraph::new(content)
                .style(Style::default().bg(Color::Rgb(13, 17, 28)))
                .wrap(Wrap { trim: false });
            f.render_widget(para, content_area);
        }
        Dialog::Error { message } => {

            let area = centered_rect(50, 25, f.area());
            f.render_widget(Clear, area);
            let block = Block::default()
                .title(" System Error ")
                .title_alignment(Alignment::Center)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red))
                .bg(Color::Rgb(31, 41, 55));
            
            let text = vec![
                Line::from(""),
                Line::from(Span::styled("An error occurred during operation:", Style::default().bold().fg(Color::Red))).alignment(Alignment::Center),
                Line::from(""),
                Line::from(message.as_str()).alignment(Alignment::Center),
                Line::from(""),
                Line::from("Press any key to dismiss.").alignment(Alignment::Center),
            ];

            let para = Paragraph::new(text).block(block).wrap(Wrap { trim: true });
            f.render_widget(para, area);
        }
        Dialog::CommandOutput { command, output, scroll_offset } => {
            let area = centered_rect(75, 60, f.area());
            f.render_widget(Clear, area);
            let block = Block::default()
                .title(" Command Output ")
                .title_alignment(Alignment::Center)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .bg(Color::Rgb(17, 24, 39));
            f.render_widget(block.clone(), area);

            let inner_area = block.inner(area);
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1), // Command line
                    Constraint::Length(1), // Separator
                    Constraint::Min(3),    // Scrollable content
                    Constraint::Length(1), // Help footer
                ])
                .split(inner_area);

            // 1. Command heading
            let cmd_line = Line::from(vec![
                Span::styled("Command: ", Style::default().bold().fg(Color::Yellow)),
                Span::styled(command, Style::default().fg(Color::White)),
            ]);
            f.render_widget(Paragraph::new(cmd_line), chunks[0]);

            // 2. Separator
            f.render_widget(Paragraph::new("─".repeat(chunks[1].width as usize)).fg(Color::DarkGray), chunks[1]);

            // 3. Scrollable output
            let lines: Vec<Line> = output
                .lines()
                .skip(*scroll_offset)
                .take(chunks[2].height as usize)
                .map(|line| {
                    if line.starts_with("Error:") || line.starts_with("rm:") || line.to_lowercase().contains("error") || line.to_lowercase().contains("failed") {
                        Line::from(Span::styled(line, Style::default().fg(Color::Red)))
                    } else {
                        Line::from(Span::raw(line))
                    }
                })
                .collect();
            f.render_widget(Paragraph::new(lines), chunks[2]);

            // 4. Help footer
            let footer = Line::from("Press [Esc] / [Space] / [Enter] to return").alignment(Alignment::Center).fg(Color::DarkGray);
            f.render_widget(Paragraph::new(footer), chunks[3]);
        }
        Dialog::Bookmarks { selected_idx } => {
            let area = centered_rect(60, 40, f.area());
            f.render_widget(Clear, area);
            let block = Block::default()
                .title(" Bookmarks Manager ")
                .title_alignment(Alignment::Center)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(168, 85, 247)))
                .bg(Color::Rgb(17, 24, 39));

            let list_items: Vec<ListItem> = if app.config.bookmarks.is_empty() {
                vec![ListItem::new("  No bookmarks saved yet. Press [A] to add current directory.")]
            } else {
                app.config.bookmarks.iter().enumerate().map(|(idx, path)| {
                    let is_selected = idx == *selected_idx;
                    let prefix = if is_selected { "▶ " } else { "  " };
                    let style = if is_selected {
                        Style::default().bg(Color::Rgb(79, 70, 229)).fg(Color::White).bold()
                    } else {
                        Style::default().fg(Color::Rgb(226, 232, 240))
                    };
                    ListItem::new(format!("{}📂 {}", prefix, path.display())).style(style)
                }).collect()
            };

            let list = List::new(list_items)
                .block(block)
                .highlight_style(Style::default());

            let footer = Paragraph::new(
                "Press [Enter] to Jump  •  [A] Add Current  •  [D] Delete  •  [Esc] Close"
            )
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::DarkGray));

            let sub_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(3),
                    Constraint::Length(1),
                ])
                .margin(1)
                .split(area);

            f.render_widget(list, sub_chunks[0]);
            f.render_widget(footer, sub_chunks[1]);
        }
        Dialog::TerminalOverlay { input, output_lines, scroll_offset } => {
            let area = centered_rect(85, 75, f.area());
            f.render_widget(Clear, area);
            
            let display_height = area.height.saturating_sub(4) as usize;
            let title = if output_lines.len() > display_height {
                let start_idx = *scroll_offset + 1;
                let end_idx = (*scroll_offset + display_height).min(output_lines.len());
                format!(" Interactive Terminal Overlay (Lines {}-{}/{}) ", start_idx, end_idx, output_lines.len())
            } else {
                " Interactive Terminal Overlay ".to_string()
            };
            
            let block = Block::default()
                .title(Span::styled(title, Style::default().fg(Color::Rgb(56, 189, 248)).bold()))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(56, 189, 248)))
                .bg(Color::Rgb(15, 23, 42));
            f.render_widget(block.clone(), area);

            let inner = block.inner(area);
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(3),    // Scrollable command output
                    Constraint::Length(1), // Separator line
                    Constraint::Length(1), // Shell prompt and input line
                ])
                .split(inner);

            let lines: Vec<Line> = output_lines.iter()
                .skip(*scroll_offset)
                .take(display_height)
                .map(|line| {
                    if line.starts_with("❯ ") {
                        Line::from(Span::styled(line, Style::default().fg(Color::Green).bold()))
                    } else if line.starts_with("stderr:") || line.starts_with("Failed to") || line.contains("Error") {
                        Line::from(Span::styled(line, Style::default().fg(Color::Red)))
                    } else if line.starts_with("[Command exited") {
                        Line::from(Span::styled(line, Style::default().fg(Color::Yellow)))
                    } else {
                        Line::from(Span::raw(line))
                    }
                })
                .collect();

            f.render_widget(Paragraph::new(lines), chunks[0]);
            f.render_widget(Paragraph::new("─".repeat(chunks[1].width as usize)).fg(Color::DarkGray), chunks[1]);

            let prompt = "rc-shell ❯ ";
            let prompt_len = prompt.chars().count() as u16;
            let input_para = Paragraph::new(Line::from(vec![
                Span::styled(prompt, Style::default().fg(Color::Cyan).bold()),
                Span::raw(input.text.as_str()),
            ]));
            f.render_widget(input_para, chunks[2]);

            f.set_cursor_position(Position::new(
                chunks[2].x + prompt_len + input.visual_cursor_col(),
                chunks[2].y,
            ));
        }
    }
}

// Drops down overlay block under the active top tab
fn draw_menu_dropdown(f: &mut Frame, active_menu: usize, item_idx: usize) {
    let items = match active_menu {
        0 => vec![
            "Toggle Hidden Files",
            "Sort by Name",
            "Sort by Size",
            "Sort by Time",
        ],
        1 => vec![
            "View (F3)",
            "Edit (F4)",
            "Copy (F5)",
            "Move / Rename (F6)",
            "Create Directory (F7)",
            "Delete Selected (F8)",
        ],
        2 => vec![
            "Run Command (:)",
            "Filter Files (/)",
            "Quick Preview (Ctrl+P)",
            "Home Jump (~)",
        ],
        3 => vec![
            "Settings (Ctrl+S)",
            "Help Manual (F1)",
            "Exit Application (F10)",
        ],
        4 => vec![
            "Toggle Hidden Files",
            "Sort by Name",
            "Sort by Size",
            "Sort by Time",
        ],
        _ => Vec::new(),
    };

    let x = match active_menu {
        0 => 2,
        1 => 11,
        2 => 20,
        3 => 32,
        4 => 44,
        _ => 2,
    };

    let width = 26;
    let height = items.len() as u16 + 2;
    let area = Rect::new(x, 1, width, height);

    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(6, 182, 212)))
        .bg(Color::Rgb(17, 24, 39));

    let list_items: Vec<ListItem> = items.iter().enumerate().map(|(idx, item)| {
        let style = if idx == item_idx {
            Style::default().bg(Color::Rgb(79, 70, 229)).fg(Color::White).bold()
        } else {
            Style::default().fg(Color::Rgb(226, 232, 240))
        };
        ListItem::new(Line::from(format!(" {}", item))).style(style)
    }).collect();

    let list = List::new(list_items).block(block);
    f.render_widget(list, area);
}

fn draw_live_preview(f: &mut Frame, area: Rect, selected: Option<FileItem>, app: &mut App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(14, 116, 144)))
        .bg(Color::Rgb(15, 23, 42));

    let content_lines = if let Some(item) = selected {
        let title_span = Span::styled(
            format!(" Live Preview: {} ", item.name),
            Style::default().fg(Color::Rgb(14, 116, 144)).bold(),
        );
        let active_block = block.title(title_span);
        
        let body = if item.name == ".." {
            "↩ Go up to parent folder".to_string()
        } else if item.is_dir {
            read_dir_preview(&item.path)
        } else {
            app.get_preview_content(item.path, area.width.saturating_sub(2), area.height.saturating_sub(2))
        };

        (active_block, parse_ansi_text(&body))
    } else {
        (block.title(" Live Preview "), parse_ansi_text("No item selected"))
    };

    let lines: Vec<Line> = content_lines.1
        .into_iter()
        .take(area.height.saturating_sub(2) as usize)
        .collect();

    let para = Paragraph::new(lines).block(content_lines.0).wrap(Wrap { trim: false });
    f.render_widget(para, area);
}

fn draw_panel(f: &mut Frame, area: Rect, panel: &mut Panel, is_active: bool, theme: &Theme) {
    let title_prefix = if is_active { "▶ " } else { "  " };
    let border_color = if is_active { theme.active_border } else { theme.inactive_border };

    let mut details = format!(" [Sort: {}]", panel.sort_by.to_uppercase());
    if let Some(ref filter) = panel.filter {
        details.push_str(&format!(" [Filter: {}]", filter));
    }
    if !panel.marked.is_empty() {
        details.push_str(&format!(" [Marked: {}]", panel.marked.len()));
    }

    let mut title_spans = vec![
        Span::styled(format!("{}📁 {}", title_prefix, panel.path.to_string_lossy()), Style::default().fg(if is_active { Color::White } else { Color::Gray }).bold()),
    ];
    if let Some(ref branch) = panel.git_branch {
        title_spans.push(Span::styled(format!(" [git:{}]", branch), Style::default().fg(theme.active_border).bold()));
    }
    title_spans.push(Span::styled(format!("{} ", details), Style::default().fg(if is_active { Color::White } else { Color::Gray })));

    let block = Block::default()
        .title(Line::from(title_spans))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .bg(theme.status_bg);

    if panel.items.is_empty() {
        let empty_msg = Paragraph::new("Empty Directory / Filters cleared no items")
            .block(block)
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(empty_msg, area);
        return;
    }

    let list_items: Vec<ListItem> = panel.items.iter().enumerate().map(|(idx, item)| {
        let is_selected = Some(idx) == panel.scroll_state.selected();
        let is_marked = panel.marked.contains(&item.path);

        let icon = if item.name == ".." {
            "↩ "
        } else if item.is_dir {
            "📁 "
        } else if item.is_symlink {
            "🔗 "
        } else if item.is_exec {
            "⚙️ "
        } else {
            "📄 "
        };

        let marker = if is_marked { "✔ " } else { "" };

        let mut item_style = Style::default();
        if is_active && is_selected {
            item_style = item_style.bg(theme.active_selection_bg).fg(Color::White).bold();
        } else if !is_active && is_selected {
            item_style = item_style.bg(theme.inactive_selection_bg).fg(theme.file_fg);
        } else if is_marked {
            item_style = item_style.fg(theme.text_highlight).bold();
        } else {
            item_style = get_extension_style(item, theme);
        }

        let git_badge = if let Ok(canon) = item.path.canonicalize() {
            panel.git_statuses.get(&canon)
        } else {
            panel.git_statuses.get(&item.path)
        };

        let git_str = match git_badge.map(|s| s.as_str()) {
            Some("M") => " [M]",
            Some("A") => " [A]",
            Some("D") => " [D]",
            Some("??") => " [?]",
            Some(other) => other,
            None => "",
        };
        let raw_name = format!("{}{}{}", marker, icon, item.name);
        
        let size_str = if item.is_dir || item.name == ".." {
            " <DIR> ".to_string()
        } else {
            format_size(item.size)
        };

        let time_str = format_time(item.modified);

        let width = area.width.saturating_sub(2);
        let time_w = 19;
        let size_w = 10;
        let name_w = width.saturating_sub(time_w + size_w + 3) as usize;

        let final_name_str = if raw_name.len() + git_str.len() > name_w {
            let max_raw = name_w.saturating_sub(git_str.len() + 3);
            if raw_name.len() > max_raw {
                format!("{}...", &raw_name[..max_raw])
            } else {
                raw_name
            }
        } else {
            raw_name
        };

        let padded_len = name_w.saturating_sub(final_name_str.len() + git_str.len());
        let padding = " ".repeat(padded_len);

        let git_span = match git_badge.map(|s| s.as_str()) {
            Some("M") => Span::styled(" [M]", Style::default().fg(Color::Yellow).bold()),
            Some("A") => Span::styled(" [A]", Style::default().fg(Color::Green).bold()),
            Some("D") => Span::styled(" [D]", Style::default().fg(Color::Red).bold()),
            Some("??") => Span::styled(" [?]", Style::default().fg(Color::DarkGray)),
            Some(other) => Span::styled(format!(" [{}]", other), Style::default().fg(Color::DarkGray)),
            None => Span::raw(""),
        };

        let line = Line::from(vec![
            Span::raw(final_name_str),
            git_span,
            Span::raw(padding),
            Span::raw(" │ "),
            Span::raw(format!("{:>width$}", size_str, width = size_w as usize)),
            Span::raw(" │ "),
            Span::raw(time_str),
        ]);

        ListItem::new(line).style(item_style)
    }).collect();

    let list = List::new(list_items)
        .block(block)
        .highlight_style(Style::default());
    
    f.render_stateful_widget(list, area, &mut panel.scroll_state);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn draw_tree_panel(f: &mut Frame, area: Rect, app: &mut App, is_active: bool, theme: &Theme) {
    let title_prefix = if is_active { "▶ " } else { "  " };
    let border_color = if is_active { theme.active_border } else { theme.inactive_border };

    let title_span = Span::styled(
        format!("{}🌳 Directory Tree ", title_prefix),
        Style::default().fg(if is_active { Color::White } else { Color::Gray }).bold(),
    );

    let block = Block::default()
        .title(title_span)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .bg(theme.status_bg);

    if app.tree_nodes.is_empty() {
        let empty_msg = Paragraph::new("No directories found")
            .block(block)
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(empty_msg, area);
        return;
    }

    let list_items: Vec<ListItem> = app.tree_nodes.iter().enumerate().map(|(idx, node)| {
        let is_selected = idx == app.tree_selected;
        
        let indent = "  ".repeat(node.depth);
        let folder_icon = if node.is_expanded { "📂 " } else { "📁 " };
        let toggle_icon = if !node.has_subdirs {
            "  "
        } else if node.is_expanded {
            "▼ "
        } else {
            "▶ "
        };

        let mut item_style = Style::default();
        if is_active && is_selected {
            item_style = item_style.bg(theme.active_selection_bg).fg(Color::White).bold();
        } else if !is_active && is_selected {
            item_style = item_style.bg(theme.inactive_selection_bg).fg(theme.file_fg);
        } else {
            item_style = item_style.fg(theme.folder_fg);
        }

        let line = Line::from(vec![
            Span::raw(indent),
            Span::styled(toggle_icon, Style::default().fg(theme.text_highlight)),
            Span::styled(folder_icon, Style::default().fg(theme.folder_fg)),
            Span::raw(&node.name),
        ]);

        ListItem::new(line).style(item_style)
    }).collect();

    let mut state = ListState::default();
    state.select(Some(app.tree_selected));

    let list = List::new(list_items)
        .block(block)
        .highlight_style(Style::default());

    f.render_stateful_widget(list, area, &mut state);
}

fn draw_beautiful_contents_panel(f: &mut Frame, area: Rect, app: &mut App, is_active: bool, theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(60),
            Constraint::Percentage(40),
        ])
        .split(area);

    draw_panel(f, chunks[0], &mut app.right_panel, is_active, theme);

    let selected_item = app.right_panel.get_selected_item().cloned();
    draw_live_preview(f, chunks[1], selected_item, app);
}

fn parse_ansi_text(text: &str) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut current_style = Style::default();

    for line_str in text.lines() {
        let mut spans = Vec::new();
        let mut current_text = String::new();
        let mut chars = line_str.chars().peekable();

        while let Some(&c) = chars.peek() {
            if c == '\x1b' {
                if !current_text.is_empty() {
                    spans.push(Span::styled(current_text.clone(), current_style));
                    current_text.clear();
                }

                chars.next(); // consume '\x1b'
                if chars.peek() == Some(&'[') {
                    chars.next(); // consume '['
                    let mut params_str = String::new();
                    while let Some(&pc) = chars.peek() {
                        chars.next();
                        if pc == 'm' {
                            break;
                        }
                        params_str.push(pc);
                    }

                    if params_str.is_empty() {
                        current_style = Style::default();
                    } else {
                        let parts: Vec<&str> = params_str.split(';').collect();
                        let mut i = 0;
                        while i < parts.len() {
                            if let Ok(code) = parts[i].parse::<u8>() {
                                match code {
                                    0 => current_style = Style::default(),
                                    1 => current_style = current_style.add_modifier(ratatui::style::Modifier::BOLD),
                                    3 => current_style = current_style.add_modifier(ratatui::style::Modifier::ITALIC),
                                    4 => current_style = current_style.add_modifier(ratatui::style::Modifier::UNDERLINED),
                                    22 => current_style = current_style.remove_modifier(ratatui::style::Modifier::BOLD),
                                    23 => current_style = current_style.remove_modifier(ratatui::style::Modifier::ITALIC),
                                    24 => current_style = current_style.remove_modifier(ratatui::style::Modifier::UNDERLINED),
                                    c @ 30..=37 => {
                                        let color = map_ansi_color_code(c - 30, false);
                                        current_style = current_style.fg(color);
                                    }
                                    c @ 90..=97 => {
                                        let color = map_ansi_color_code(c - 90, true);
                                        current_style = current_style.fg(color);
                                    }
                                    39 => current_style.fg = None,
                                    c @ 40..=47 => {
                                        let color = map_ansi_color_code(c - 40, false);
                                        current_style = current_style.bg(color);
                                    }
                                    c @ 100..=107 => {
                                        let color = map_ansi_color_code(c - 100, true);
                                        current_style = current_style.bg(color);
                                    }
                                    49 => current_style.bg = None,
                                    38 => {
                                        if i + 1 < parts.len() {
                                            if parts[i + 1] == "5" {
                                                if i + 2 < parts.len() {
                                                    if let Ok(idx) = parts[i + 2].parse::<u8>() {
                                                        current_style = current_style.fg(Color::Indexed(idx));
                                                    }
                                                    i += 2;
                                                }
                                            } else if parts[i + 1] == "2" {
                                                if i + 4 < parts.len() {
                                                    if let (Ok(r), Ok(g), Ok(b)) = (
                                                        parts[i + 2].parse::<u8>(),
                                                        parts[i + 3].parse::<u8>(),
                                                        parts[i + 4].parse::<u8>(),
                                                    ) {
                                                        current_style = current_style.fg(Color::Rgb(r, g, b));
                                                    }
                                                    i += 4;
                                                }
                                            }
                                        }
                                    }
                                    48 => {
                                        if i + 1 < parts.len() {
                                            if parts[i + 1] == "5" {
                                                if i + 2 < parts.len() {
                                                    if let Ok(idx) = parts[i + 2].parse::<u8>() {
                                                        current_style = current_style.bg(Color::Indexed(idx));
                                                    }
                                                    i += 2;
                                                }
                                            } else if parts[i + 1] == "2" {
                                                if i + 4 < parts.len() {
                                                    if let (Ok(r), Ok(g), Ok(b)) = (
                                                        parts[i + 2].parse::<u8>(),
                                                        parts[i + 3].parse::<u8>(),
                                                        parts[i + 4].parse::<u8>(),
                                                    ) {
                                                        current_style = current_style.bg(Color::Rgb(r, g, b));
                                                    }
                                                    i += 4;
                                                }
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            i += 1;
                        }
                    }
                }
            } else {
                current_text.push(c);
                chars.next();
            }
        }

        if !current_text.is_empty() {
            spans.push(Span::styled(current_text, current_style));
        }

        lines.push(Line::from(spans));
    }

    lines
}

fn map_ansi_color_code(code: u8, bright: bool) -> Color {
    match (code, bright) {
        (0, false) => Color::Black,
        (0, true) => Color::DarkGray,
        (1, false) => Color::Red,
        (1, true) => Color::LightRed,
        (2, false) => Color::Green,
        (2, true) => Color::LightGreen,
        (3, false) => Color::Yellow,
        (3, true) => Color::LightYellow,
        (4, false) => Color::Blue,
        (4, true) => Color::LightBlue,
        (5, false) => Color::Magenta,
        (5, true) => Color::LightMagenta,
        (6, false) => Color::Cyan,
        (6, true) => Color::LightCyan,
        (7, false) => Color::Gray,
        (7, true) => Color::White,
        _ => Color::Reset,
    }
}

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
         \x1b[36m  F1\x1b[0m  Help    \x1b[36mF3\x1b[0m  View    \x1b[36mF4\x1b[0m  Edit    \x1b[36mF5\x1b[0m  Copy\n\
         \x1b[36m  F6\x1b[0m  Move    \x1b[36mF7\x1b[0m  MkDir   \x1b[36mF8\x1b[0m  Delete  \x1b[36mF9\x1b[0m  Menu\n\
         \x1b[36m  Ctrl+S\x1b[0m  Settings    \x1b[36mCtrl+O\x1b[0m  Shell   \x1b[36mCtrl+B\x1b[0m  Bookmarks\n\
         \x1b[36m  Ctrl+T\x1b[0m  Tree view   \x1b[36mCtrl+P\x1b[0m  Preview \x1b[36m~\x1b[0m       Home\n\
         \n\
         \x1b[1mCONFIG:\x1b[0m  ~/.config/rust-commander/config.ini\n\
         \x1b[1mREPO:\x1b[0m    https://github.com/{REPO}"
    );
}

fn self_update() -> Result<(), Box<dyn std::error::Error>> {
    println!("\x1b[1;36m▶ Checking for updates...\x1b[0m");

    let output = Command::new("curl")
        .args(["-s", &format!("https://api.github.com/repos/{REPO}/releases/latest")])
        .output()?;

    let body = String::from_utf8_lossy(&output.stdout);

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
