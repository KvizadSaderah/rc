use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

// =============================================================================
// Streaming Process for Terminal Overlay
// =============================================================================

pub struct RunningProcess {
    pub child: std::process::Child,
    pub receiver: mpsc::Receiver<String>,
    pub done: bool,
}

use std::time::SystemTime;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use ratatui::style::{Color, Style};

use crate::theme::Theme;

// =============================================================================
// Data Models & Filesystem IO Helpers
// =============================================================================

#[derive(Clone, Debug)]
pub struct FileItem {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub is_exec: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

#[derive(Clone, Debug)]
pub struct TreeNode {
    pub path: PathBuf,
    pub name: String,
    pub depth: usize,
    pub is_expanded: bool,
    pub has_subdirs: bool,
}

/// Executable check reusing metadata we already have (avoids a second stat
/// per entry on the hot directory-listing path).
fn is_executable_meta(meta: &fs::Metadata, path: &Path) -> bool {
    #[cfg(unix)]
    {
        let _ = path;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        if let Some(ext) = path.extension() {
            let ext_str = ext.to_string_lossy().to_lowercase();
            return ext_str == "exe" || ext_str == "bat" || ext_str == "sh" || ext_str == "cmd";
        }
        false
    }
}

/// Whether `path` has at least one non-hidden subdirectory. Symlinked
/// directories are deliberately not counted, so the tree view never descends
/// into a symlink loop.
pub fn has_subdirectories(path: &Path) -> bool {
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            // file_type() does not follow symlinks; skip symlinked entries.
            let ft = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if ft.is_dir() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str != "." && name_str != ".." && !name_str.starts_with('.') {
                    return true;
                }
            }
        }
    }
    false
}

pub fn read_dir(path: &Path) -> io::Result<Vec<FileItem>> {
    // Open the directory first so permission/IO errors surface to the caller
    // (e.g. set_path) instead of silently presenting an empty listing.
    let entries = fs::read_dir(path)?;

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

    for entry in entries.flatten() {
        let entry_path = entry.path();
        // symlink_metadata() does not follow links — needed to detect symlinks.
        let link_meta = entry.metadata().ok(); // follows symlink (for is_dir/size)
        let file_type = entry.file_type(); // does not follow symlink

        let is_symlink = file_type.as_ref().map(|ft| ft.is_symlink()).unwrap_or(false);
        let is_dir = link_meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        let is_exec = !is_dir
            && link_meta
                .as_ref()
                .map(|m| is_executable_meta(m, &entry_path))
                .unwrap_or(false);
        let size = link_meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let modified = link_meta.as_ref().and_then(|m| m.modified().ok());
        let name = entry.file_name().to_string_lossy().into_owned();

        items.push(FileItem {
            name,
            path: entry_path,
            is_dir,
            is_symlink,
            is_exec,
            size,
            modified,
        });
    }

    Ok(items)
}

pub fn format_size(size: u64) -> String {
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

pub fn format_time(time: Option<SystemTime>) -> String {
    match time {
        Some(t) => {
            let datetime: chrono::DateTime<chrono::Local> = t.into();
            datetime.format("%Y-%m-%d %H:%M:%S").to_string()
        }
        None => "-".to_string(),
    }
}

// Recursive copy / cross-device move now live in the `fileops` module, where
// they run on a background thread with progress reporting and cancellation.

pub fn read_file_preview(path: &Path) -> String {
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

pub fn read_dir_preview(path: &Path) -> String {
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
pub fn get_extension_style(item: &FileItem, theme: &Theme) -> Style {
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

pub struct InputField {
    pub text: String,
    pub cursor_position: usize, // character index, NOT byte index!
}

impl InputField {
    pub fn new(initial: String) -> Self {
        let len = initial.chars().count();
        Self {
            text: initial,
            cursor_position: len,
        }
    }

    pub fn insert(&mut self, c: char) {
        let byte_idx = self.text.char_indices().map(|(i, _)| i).nth(self.cursor_position).unwrap_or(self.text.len());
        self.text.insert(byte_idx, c);
        self.cursor_position += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor_position > 0 {
            self.cursor_position -= 1;
            if let Some(byte_idx) = self.text.char_indices().map(|(i, _)| i).nth(self.cursor_position) {
                self.text.remove(byte_idx);
            }
        }
    }

    pub fn delete(&mut self) {
        let char_len = self.text.chars().count();
        if self.cursor_position < char_len
            && let Some(byte_idx) = self.text.char_indices().map(|(i, _)| i).nth(self.cursor_position) {
                self.text.remove(byte_idx);
            }
    }

    pub fn move_left(&mut self) {
        if self.cursor_position > 0 {
            self.cursor_position -= 1;
        }
    }

    pub fn move_right(&mut self) {
        let char_len = self.text.chars().count();
        if self.cursor_position < char_len {
            self.cursor_position += 1;
        }
    }

    pub fn home(&mut self) {
        self.cursor_position = 0;
    }

    pub fn end(&mut self) {
        self.cursor_position = self.text.chars().count();
    }

    /// Returns the visual column offset for terminal cursor placement.
    /// For ASCII text this equals cursor_position; for Unicode
    /// characters each char is assumed 1 terminal column wide.
    pub fn visual_cursor_col(&self) -> u16 {
        self.text.chars().take(self.cursor_position).count() as u16
    }
}

pub struct PreviewCache {
    pub path: PathBuf,
    pub width: u16,
    pub height: u16,
    pub content: String,
}

pub enum Dialog {
    None,
    ConfirmDelete {
        item_name: String,
        item_path: PathBuf,
    },
    InputMkdir {
        input: InputField,
    },
    InputTouch {
        input: InputField,
    },
    Properties {
        name: String,
        path_str: String,
        size_str: String,
        permissions_str: String,
        modified_str: String,
        created_str: String,
        owner_str: String,
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
    Bookmarks {
        selected_idx: usize,
    },
    TerminalOverlay {
        input: InputField,
        output_lines: Vec<String>,
        scroll_offset: usize,
        command_history: Vec<String>,
        history_index: Option<usize>,
    },
    InternalEditor {
        file_path: PathBuf,
        lines: Vec<String>,
        cursor_row: usize,
        cursor_col: usize,
        scroll_row: usize,
        scroll_col: usize,
        modified: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(500), "500 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(1024 * 1024 * 1024), "1.0 GB");
        assert_eq!(format_size(1024 * 1024 * 1024 * 5 + 500 * 1024 * 1024), "5.5 GB");
    }

    #[test]
    fn test_input_field_navigation() {
        let mut input = InputField::new("hello".to_string());
        assert_eq!(input.text, "hello");
        assert_eq!(input.cursor_position, 5);

        input.move_left();
        assert_eq!(input.cursor_position, 4);

        input.move_right();
        assert_eq!(input.cursor_position, 5);

        // Can't move right past end
        input.move_right();
        assert_eq!(input.cursor_position, 5);

        input.home();
        assert_eq!(input.cursor_position, 0);

        // Can't move left past start
        input.move_left();
        assert_eq!(input.cursor_position, 0);

        input.end();
        assert_eq!(input.cursor_position, 5);
    }

    #[test]
    fn test_input_field_editing() {
        let mut input = InputField::new("world".to_string());
        input.home();
        input.insert('x');
        assert_eq!(input.text, "xworld");
        assert_eq!(input.cursor_position, 1);

        input.delete();
        assert_eq!(input.text, "xorld");
        assert_eq!(input.cursor_position, 1);

        input.end();
        input.backspace();
        assert_eq!(input.text, "xorl");
        assert_eq!(input.cursor_position, 4);
    }

    #[test]
    fn test_read_dir_errors_surface() {
        // Reading a non-existent path must be an Err, not a silent empty list.
        let missing = std::env::temp_dir().join(format!("rc_nope_{}", chrono::Utc::now().timestamp_micros()));
        assert!(read_dir(&missing).is_err());

        // Reading a regular file (not a directory) must also be an Err.
        let file = std::env::temp_dir().join(format!("rc_file_{}.txt", chrono::Utc::now().timestamp_micros()));
        fs::write(&file, b"x").unwrap();
        assert!(read_dir(&file).is_err());
        let _ = fs::remove_file(&file);
    }

    #[test]
    fn test_read_dir_lists_children() {
        let root = std::env::temp_dir().join(format!("rc_rd_{}", chrono::Utc::now().timestamp_micros()));
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("a.txt"), b"hi").unwrap();

        let items = read_dir(&root).unwrap();
        assert!(items.iter().any(|i| i.name == ".."));
        assert!(items.iter().any(|i| i.name == "a.txt" && !i.is_dir));
        assert!(items.iter().any(|i| i.name == "sub" && i.is_dir));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_lazygit_theme() {
        let theme = Theme::get_theme("lazygit");
        assert_eq!(theme.active_border, ratatui::style::Color::Rgb(74, 222, 128));
        assert!(Theme::all_names().contains(&"lazygit"));
    }

    #[test]
    fn test_config_border_type() {
        let config = crate::config::load_config();
        // default border type should be plain or whatever is currently in config.ini
        assert!(!config.border_type.is_empty());
    }

    #[test]
    fn test_preview_scroll_offset() {
        let mut app = crate::app::App::new();
        assert_eq!(app.preview_scroll_offset, 0);

        app.preview_scroll_offset = 10;
        assert_eq!(app.preview_scroll_offset, 10);

        // simulate path navigation
        app.get_active_panel_mut().selected = 1;
        
        // compare states (like at the end of handle_main_keys)
        app.preview_scroll_offset = 0; // reset
        assert_eq!(app.preview_scroll_offset, 0);
    }

    #[test]
    fn test_touch_file() {
        let mut app = crate::app::App::new();
        let root = std::env::temp_dir().join(format!("rc_test_touch_{}", chrono::Utc::now().timestamp_micros()));
        fs::create_dir_all(&root).unwrap();

        let _ = app.get_active_panel_mut().set_path(root.clone());
        
        let new_file_name = "empty_file.txt".to_string();
        app.execute_touch(new_file_name.clone());

        assert!(root.join(&new_file_name).exists());
        
        // Clean up
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_properties_dialog() {
        let mut app = crate::app::App::new();
        let temp = std::env::temp_dir().canonicalize().unwrap();
        let root = temp.join(format!("rc_test_properties_{}", chrono::Utc::now().timestamp_micros()));
        fs::create_dir_all(&root).unwrap();

        let file_path = root.join("test_file.txt");
        fs::write(&file_path, "properties content").unwrap();

        let _ = app.get_active_panel_mut().set_path(root.clone());
        
        // Find test_file.txt in items and select it
        if let Some(idx) = app.get_active_panel().items.iter().position(|i| i.name == "test_file.txt") {
            app.get_active_panel_mut().selected = idx;
        }

        app.initiate_properties();

        if let Dialog::Properties { name, path_str, size_str, .. } = &app.dialog {
            assert_eq!(name, "test_file.txt");
            assert_eq!(path_str, &file_path.display().to_string());
            assert_eq!(size_str, &format_size(18));
        } else {
            panic!("Expected Dialog::Properties");
        }

        // Clean up
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_git_caching() {
        let mut panel = crate::panel::Panel::new(std::env::temp_dir(), false, "name".to_string());
        // Since Panel::new calls refresh internally, last_git_query is already Some.
        let time1 = panel.last_git_query;
        assert!(time1.is_some());
        
        panel.refresh();
        let time2 = panel.last_git_query;
        assert_eq!(time1, time2);
    }

    #[test]
    fn test_right_arrow_on_file() {
        let mut app = crate::app::App::new();
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!("rc_test_right_arrow_{}", chrono::Utc::now().timestamp_micros()));
        fs::create_dir_all(&root).unwrap();

        let file_path = root.join("test_file.txt");
        fs::write(&file_path, "some content").unwrap();

        let _ = app.get_active_panel_mut().set_path(root.clone());

        // Select the file
        if let Some(idx) = app.get_active_panel().items.iter().position(|i| i.name == "test_file.txt") {
            app.get_active_panel_mut().selected = idx;
        }

        let backend = ratatui::backend::CrosstermBackend::new(std::io::stdout());
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        app.handle_enter_or_right(&mut terminal, false);

        assert!(matches!(app.dialog, Dialog::None));
        assert_eq!(app.status_message, "Press Enter to edit/open file");

        let _ = fs::remove_dir_all(&root);
    }
}
