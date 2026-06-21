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

pub fn is_executable(path: &Path) -> bool {
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

pub fn has_subdirectories(path: &Path) -> bool {
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

pub fn read_dir(path: &Path) -> io::Result<Vec<FileItem>> {
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

pub fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> io::Result<()> {
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
        if self.cursor_position < char_len {
            if let Some(byte_idx) = self.text.char_indices().map(|(i, _)| i).nth(self.cursor_position) {
                self.text.remove(byte_idx);
            }
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
