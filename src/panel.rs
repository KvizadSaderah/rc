use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;

use ratatui::widgets::ListState;

use crate::types::{FileItem, read_dir};

// =============================================================================
// File Panel Core State
// =============================================================================

pub struct Panel {
    pub path: PathBuf,
    pub items: Vec<FileItem>,
    pub selected: usize,
    pub scroll_state: ListState,
    pub show_hidden: bool,
    pub sort_by: String,
    pub filter: Option<String>,
    pub marked: HashSet<PathBuf>, // Multi-selection Set (Tagged items)
    pub git_branch: Option<String>,
    pub git_statuses: HashMap<PathBuf, String>,
    pub last_git_query: Option<std::time::Instant>,
    git_rx: Option<mpsc::Receiver<GitInfo>>,
}

/// Branch name (or `detached@<sha>`) and per-path porcelain status codes.
type GitInfo = (Option<String>, HashMap<PathBuf, String>);

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).current_dir(dir).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

fn query_git(dir: &Path) -> GitInfo {
    let mut statuses = HashMap::new();
    if git(dir, &["rev-parse", "--is-inside-work-tree"]).as_deref() != Some("true") {
        return (None, statuses);
    }
    let branch = match git(dir, &["branch", "--show-current"]) {
        Some(b) if !b.is_empty() => Some(b),
        _ => git(dir, &["rev-parse", "--short", "HEAD"]).map(|sha| format!("detached@{sha}")),
    };
    // --show-toplevel is already a real path, matching the canonical panel paths.
    if let (Some(status), Some(root)) = (
        git(dir, &["status", "--porcelain"]),
        git(dir, &["rev-parse", "--show-toplevel"]),
    ) {
        let root = PathBuf::from(root);
        for line in status.lines().filter(|l| l.len() > 3) {
            let code = line[..2].trim().to_string();
            let rel = &line[3..];
            let rel = rel.find(" -> ").map_or(rel, |i| &rel[i + 4..]);
            statuses.insert(root.join(rel.trim_matches('"')), code);
        }
    }
    (branch, statuses)
}

fn fuzzy_match(text: &str, query: &str) -> bool {
    let query_chars: Vec<char> = query.to_lowercase().chars().collect();
    if query_chars.is_empty() {
        return true;
    }
    let text_chars: Vec<char> = text.to_lowercase().chars().collect();

    let mut q_idx = 0;
    for &c in &text_chars {
        if c == query_chars[q_idx] {
            q_idx += 1;
            if q_idx == query_chars.len() {
                return true;
            }
        }
    }
    false
}

impl Panel {
    pub fn new(path: PathBuf, show_hidden: bool, sort_by: String) -> Self {
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
            git_statuses: HashMap::new(),
            last_git_query: None,
            git_rx: None,
        };
        panel.refresh();
        panel
    }

    pub fn refresh(&mut self) {
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
                if let Some(ref f) = self.filter
                    && !fuzzy_match(&item.name, f) {
                        return false;
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
            if let Some(ref name) = prev_selected_name
                && let Some(pos) = self.items.iter().position(|item| &item.name == name) {
                    self.selected = pos;
                }
            if self.selected >= self.items.len() {
                self.selected = self.items.len() - 1;
            }
            self.scroll_state.select(Some(self.selected));
        }

        // Query Git status in the background: `git status` can take seconds in
        // large repos and must not block the UI thread.
        let should_query_git = match self.last_git_query {
            None => true,
            Some(inst) => inst.elapsed() >= std::time::Duration::from_secs(10),
        };
        if should_query_git && self.git_rx.is_none() {
            self.last_git_query = Some(std::time::Instant::now());
            let (tx, rx) = mpsc::channel();
            let dir = self.path.clone();
            std::thread::spawn(move || {
                let _ = tx.send(query_git(&dir));
            });
            self.git_rx = Some(rx);
        }
    }

    /// Apply a finished background git query. Returns true if one arrived.
    pub fn poll_git(&mut self) -> bool {
        let Some(rx) = &self.git_rx else { return false };
        match rx.try_recv() {
            Ok(info) => {
                (self.git_branch, self.git_statuses) = info;
                self.git_rx = None;
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.git_rx = None;
                false
            }
        }
    }

    pub fn set_path(&mut self, new_path: PathBuf) -> Result<(), String> {
        let resolved = new_path.canonicalize().unwrap_or(new_path);
        match read_dir(&resolved) {
            Ok(_) => {
                self.path = resolved;
                self.filter = None;
                self.marked.clear(); // Reset selections on dir transition
                self.selected = 0;
                // Drop any in-flight query for the old directory.
                self.last_git_query = None;
                self.git_rx = None;
                self.git_branch = None;
                self.git_statuses.clear();
                self.refresh();
                Ok(())
            }
            Err(e) => Err(format!("Cannot open directory: {}", e)),
        }
    }

    pub fn select_next(&mut self) {
        if self.items.is_empty() { return; }
        self.selected = (self.selected + 1) % self.items.len();
        self.scroll_state.select(Some(self.selected));
    }

    pub fn select_prev(&mut self) {
        if self.items.is_empty() { return; }
        if self.selected == 0 {
            self.selected = self.items.len() - 1;
        } else {
            self.selected -= 1;
        }
        self.scroll_state.select(Some(self.selected));
    }

    pub fn select_first(&mut self) {
        if self.items.is_empty() { return; }
        self.selected = 0;
        self.scroll_state.select(Some(self.selected));
    }

    pub fn select_last(&mut self) {
        if self.items.is_empty() { return; }
        self.selected = self.items.len() - 1;
        self.scroll_state.select(Some(self.selected));
    }

    pub fn page_down(&mut self) {
        if self.items.is_empty() { return; }
        self.selected = (self.selected + 10).min(self.items.len() - 1);
        self.scroll_state.select(Some(self.selected));
    }

    pub fn page_up(&mut self) {
        if self.items.is_empty() { return; }
        self.selected = self.selected.saturating_sub(10);
        self.scroll_state.select(Some(self.selected));
    }

    pub fn get_selected_item(&self) -> Option<&FileItem> {
        self.items.get(self.selected)
    }

    pub fn select_item_by_path(&mut self, path: &Path) {
        if let Some(pos) = self.items.iter().position(|item| item.path == path) {
            self.selected = pos;
            self.scroll_state.select(Some(self.selected));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn fixture(tag: &str) -> (PathBuf, Panel) {
        let root = std::env::temp_dir()
            .join(format!("rc_panel_{}_{}", tag, chrono::Utc::now().timestamp_micros()));
        fs::create_dir_all(root.join("adir")).unwrap();
        fs::create_dir_all(root.join("bdir")).unwrap();
        fs::write(root.join("c.txt"), b"x").unwrap();
        fs::write(root.join("d.txt"), b"y").unwrap();
        let panel = Panel::new(root.clone(), false, "name".to_string());
        (root, panel)
    }

    #[test]
    fn test_select_wraparound() {
        let (root, mut p) = fixture("sel");
        let n = p.items.len();
        assert!(n >= 4);

        p.selected = 0;
        p.select_prev(); // wrap to last
        assert_eq!(p.selected, n - 1);
        p.select_next(); // wrap to first
        assert_eq!(p.selected, 0);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_page_clamps() {
        let (root, mut p) = fixture("page");
        let n = p.items.len();
        p.select_last();
        assert_eq!(p.selected, n - 1);
        p.page_down(); // already at end, stays clamped
        assert_eq!(p.selected, n - 1);
        p.select_first();
        p.page_up(); // at start, saturating
        assert_eq!(p.selected, 0);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_dirs_sorted_before_files() {
        let (root, p) = fixture("sort");
        // First entry is "..", then directories, then files.
        let names: Vec<&str> = p.items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names[0], "..");
        let first_file = names.iter().position(|n| n.ends_with(".txt")).unwrap();
        let last_dir = names.iter().rposition(|n| *n == "adir" || *n == "bdir").unwrap();
        assert!(last_dir < first_file, "all dirs must precede files");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_filter_narrows_listing() {
        let (root, mut p) = fixture("filter");
        p.filter = Some("c.txt".to_string());
        p.refresh();
        assert!(p.items.iter().any(|i| i.name == "c.txt"));
        assert!(!p.items.iter().any(|i| i.name == "d.txt"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_fuzzy_filter() {
        let (root, mut p) = fixture("fuzzy_filter");
        p.filter = Some("cx".to_string());
        p.refresh();
        assert!(p.items.iter().any(|i| i.name == "c.txt"));
        assert!(!p.items.iter().any(|i| i.name == "d.txt"));
        
        p.filter = Some("ad".to_string());
        p.refresh();
        assert!(p.items.iter().any(|i| i.name == "adir"));
        assert!(!p.items.iter().any(|i| i.name == "c.txt"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_fuzzy_match_unit() {
        assert!(fuzzy_match("Cargo.toml", "ct"));
        assert!(fuzzy_match("ui.rs", "urs"));
        assert!(fuzzy_match("main.rs", "mr"));
        assert!(!fuzzy_match("main.rs", "rm"));
        assert!(fuzzy_match("main.rs", ""));
    }

    #[test]
    fn test_set_path_rejects_file() {
        let (root, mut p) = fixture("reject");
        let file = root.join("c.txt");
        let before = p.path.clone();
        assert!(p.set_path(file).is_err());
        assert_eq!(p.path, before, "navigation into a file must not change path");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_git_query_runs_in_background() {
        let (root, mut p) = fixture("git");
        let inited = Command::new("git").arg("init").arg("-q").current_dir(&root).status();
        if !inited.map(|s| s.success()).unwrap_or(false) {
            let _ = fs::remove_dir_all(&root);
            return; // git not installed
        }
        // The fixture's initial query predates `git init`; let it land first.
        while p.git_rx.is_some() {
            p.poll_git();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        p.last_git_query = None;
        p.refresh();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !p.poll_git() {
            assert!(std::time::Instant::now() < deadline, "git query never finished");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(p.git_branch.is_some());
        assert_eq!(p.git_statuses.get(&p.path.join("c.txt")).map(String::as_str), Some("??"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_set_path_clears_marks_and_filter() {
        let (root, mut p) = fixture("clear");
        p.marked.insert(root.join("c.txt"));
        p.filter = Some("c".to_string());
        p.set_path(root.join("adir")).unwrap();
        assert!(p.marked.is_empty());
        assert!(p.filter.is_none());
        let _ = fs::remove_dir_all(&root);
    }
}
