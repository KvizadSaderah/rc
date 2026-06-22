use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub struct Config {
    pub show_hidden: bool,
    pub sort_by: String,       // "name", "size", "time"
    pub keybindings: String,   // "standard", "vim"
    pub default_editor: String,
    pub editor_mode: String,   // "external" (fullscreen vim/nano), "internal" (built-in editor)
    pub confirm_quit: bool,
    pub bookmarks: Vec<PathBuf>,
    pub theme: String,
    pub border_type: String,   // "plain", "rounded", "thick", "double"
    pub use_trash: bool,       // send deletes to OS trash instead of permanent removal
}

#[derive(Clone, Debug)]
pub struct Keymap {
    pub quit: Vec<(KeyCode, KeyModifiers)>,
    pub help: Vec<(KeyCode, KeyModifiers)>,
    pub view: Vec<(KeyCode, KeyModifiers)>,
    pub edit: Vec<(KeyCode, KeyModifiers)>,
    pub copy: Vec<(KeyCode, KeyModifiers)>,
    pub move_item: Vec<(KeyCode, KeyModifiers)>,
    pub mkdir: Vec<(KeyCode, KeyModifiers)>,
    pub delete: Vec<(KeyCode, KeyModifiers)>,
    pub menu: Vec<(KeyCode, KeyModifiers)>,
    pub toggle_hidden: Vec<(KeyCode, KeyModifiers)>,
    pub toggle_preview: Vec<(KeyCode, KeyModifiers)>,
    pub select_item: Vec<(KeyCode, KeyModifiers)>,
    pub up: Vec<(KeyCode, KeyModifiers)>,
    pub down: Vec<(KeyCode, KeyModifiers)>,
    pub left: Vec<(KeyCode, KeyModifiers)>,
    pub right: Vec<(KeyCode, KeyModifiers)>,
    pub tab: Vec<(KeyCode, KeyModifiers)>,
}

impl Keymap {
    pub fn default_standard() -> Self {
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

    pub fn default_vim() -> Self {
        let mut k = Self::default_standard();
        k.up.push((KeyCode::Char('k'), KeyModifiers::empty()));
        k.down.push((KeyCode::Char('j'), KeyModifiers::empty()));
        k.left.push((KeyCode::Char('h'), KeyModifiers::empty()));
        k.right.push((KeyCode::Char('l'), KeyModifiers::empty()));
        k.toggle_preview.push((KeyCode::Char('p'), KeyModifiers::empty()));
        k
    }
}

pub fn get_config_path() -> Option<PathBuf> {
    let base = env::var("HOME")
        .ok()
        .or_else(|| env::var("USERPROFILE").ok())?;
    Some(PathBuf::from(base).join(".config/rust-commander/config.ini"))
}

pub fn load_config() -> Config {
    let mut config = Config {
        show_hidden: false,
        sort_by: "name".to_string(),
        keybindings: "standard".to_string(),
        default_editor: env::var("EDITOR").unwrap_or_else(|_| "nvim".to_string()),
        editor_mode: "internal".to_string(),
        confirm_quit: true,
        bookmarks: Vec::new(),
        theme: "eve".to_string(),
        border_type: "plain".to_string(),
        use_trash: true,
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
                        "editor_mode" => config.editor_mode = parts[1].to_string(),
                        "confirm_quit" => config.confirm_quit = parts[1] == "true",
                        "theme" => config.theme = parts[1].to_string(),
                        "border_type" => config.border_type = parts[1].to_string(),
                        "use_trash" => config.use_trash = parts[1] == "true",
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

pub fn save_config(config: &Config) -> io::Result<()> {
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
             editor_mode = {}\n\
             confirm_quit = {}\n\
             theme = {}\n\
             border_type = {}\n\
             use_trash = {}\n\
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
            config.show_hidden, config.sort_by, config.keybindings, config.default_editor, config.editor_mode, config.confirm_quit, config.theme, config.border_type, config.use_trash, bookmarks_str
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

pub fn load_keymap(config: &Config) -> Keymap {
    let mut k = match config.keybindings.as_str() {
        "vim" => Keymap::default_vim(),
        _ => Keymap::default_standard(),
    };

    if let Some(path) = get_config_path()
        && let Ok(content) = fs::read_to_string(&path) {
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
    k
}

pub fn matches_key(event: &KeyEvent, keys: &[(KeyCode, KeyModifiers)]) -> bool {
    keys.iter().any(|(code, mods)| {
        event.code == *code && event.modifiers.contains(*mods)
    })
}

pub fn command_exists(cmd_str: &str, current_dir: &std::path::Path) -> bool {
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
