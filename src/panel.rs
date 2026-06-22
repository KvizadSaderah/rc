use std::collections::HashSet;
use std::path::PathBuf;

use ratatui::widgets::ListState;

use crate::types::{FileItem, read_dir};

// =============================================================================
// File Panel Core State
// =============================================================================

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ActivePanel {
    Left,
    Right,
}

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
    pub git_statuses: std::collections::HashMap<PathBuf, String>,
    pub last_git_query: Option<std::time::Instant>,
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
            git_statuses: std::collections::HashMap::new(),
            last_git_query: None,
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
                    && !item.name.to_lowercase().contains(&f.to_lowercase()) {
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

        // Query Git status if we are in a Git workspace and enough time has elapsed
        let should_query_git = match self.last_git_query {
            None => true,
            Some(inst) => inst.elapsed() >= std::time::Duration::from_secs(10),
        };

        if should_query_git {
            self.git_branch = None;
            self.git_statuses.clear();
            self.last_git_query = Some(std::time::Instant::now());

            if let Ok(out) = std::process::Command::new("git")
                .arg("rev-parse")
                .arg("--is-inside-work-tree")
                .current_dir(&self.path)
                .output()
                && out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "true" {
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

    pub fn set_path(&mut self, new_path: PathBuf) -> Result<(), String> {
        let resolved = new_path.canonicalize().unwrap_or(new_path);
        match read_dir(&resolved) {
            Ok(_) => {
                self.path = resolved;
                self.filter = None;
                self.marked.clear(); // Reset selections on dir transition
                self.selected = 0;
                self.last_git_query = None;
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
    fn test_set_path_rejects_file() {
        let (root, mut p) = fixture("reject");
        let file = root.join("c.txt");
        let before = p.path.clone();
        assert!(p.set_path(file).is_err());
        assert_eq!(p.path, before, "navigation into a file must not change path");
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
