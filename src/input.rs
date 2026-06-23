use std::env;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    backend::CrosstermBackend,
    layout::Rect,
    Terminal,
};

use crate::app::*;
use crate::config::*;
use crate::types::*;
use crate::ui::{ui, centered_rect};

// =============================================================================
// Keyboard Inputs Router
// =============================================================================

/// The workspace rectangle (between the 1-row header and the 2-row status/legend
/// footer), matching the layout in `ui()`.
fn workspace_area<B: ratatui::backend::Backend>(terminal: &Terminal<B>) -> Rect {
    let (w, h) = match terminal.size() {
        Ok(s) => (s.width, s.height),
        Err(_) => (80, 24),
    };
    Rect::new(0, 1, w, h.saturating_sub(3))
}

/// True if a cell falls within (or one cell either side of) a 1px seam.
fn hit(seam: Rect, col: u16, row: u16) -> bool {
    let x0 = seam.x.saturating_sub(1);
    let x1 = seam.x + seam.width;
    let y0 = seam.y;
    let y1 = seam.y + seam.height;
    col >= x0 && col <= x1 && row >= y0 && row < y1
}

pub fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, mut app: App) -> io::Result<()> {
    // Event-driven refresh via notify, with a slow periodic fallback if the
    // platform watcher can't be set up.
    let mut watcher = crate::watcher::FsWatcher::new();
    let mut dirty = false;
    let mut dirty_since = std::time::Instant::now();
    let mut last_fallback = std::time::Instant::now();
    const DEBOUNCE: Duration = Duration::from_millis(150);
    const FALLBACK: Duration = Duration::from_secs(4);

    // Active divider drag: (split path, split area, direction).
    let mut drag: Option<(Vec<u8>, Rect, crate::layout::Dir)> = None;

    loop {
        // Drain output from any running background process
        app.drain_process_output();

        // Drain progress from any running background filesystem job
        app.drain_fs_job();

        // Drain loaded previews
        if app.drain_previews() {
            dirty = true;
            dirty_since = std::time::Instant::now();
        }

        let idle = matches!(app.dialog, Dialog::None) && !app.is_fs_busy();

        // Keep the watcher pointed at the two visible panel directories.
        if let Some(w) = watcher.as_mut() {
            w.sync_paths(&app.watch_dirs());
            if w.drain() {
                dirty = true;
                dirty_since = std::time::Instant::now();
            }
        }

        // Refresh after a quiet period (coalesces bursts of external writes).
        if idle && dirty && dirty_since.elapsed() >= DEBOUNCE {
            app.refresh_panels();
            dirty = false;
        }

        // Fallback poll: primary path when there is no watcher, and a safety net
        // even when there is (catches anything the backend may miss).
        if idle && last_fallback.elapsed() >= FALLBACK {
            if watcher.is_none() || !dirty {
                app.refresh_panels();
            }
            last_fallback = std::time::Instant::now();
        }

        terminal.draw(|f| ui(f, &mut app))?;

        // Use poll so we can drain process output even when no key is pressed
        if !event::poll(Duration::from_millis(50))? {
            continue;
        }
        let event = event::read()?;

        // Handle mouse scroll in dialogs
        if let Event::Mouse(mouse) = &event {
            if let Dialog::InternalEditor { scroll_row, lines, .. } = &mut app.dialog {
                match mouse.kind {
                    event::MouseEventKind::ScrollUp => {
                        *scroll_row = scroll_row.saturating_sub(3);
                        continue;
                    }
                    event::MouseEventKind::ScrollDown => {
                        *scroll_row = (*scroll_row + 3).min(lines.len().saturating_sub(1));
                        continue;
                    }
                    _ => {}
                }
            }
            if let Dialog::TerminalOverlay { output_lines, scroll_offset, .. } = &mut app.dialog {
                let display_height = {
                    let area = match terminal.size() {
                        Ok(size) => centered_rect(85, 80, Rect::new(0, 0, size.width, size.height)),
                        Err(_) => centered_rect(85, 80, Rect::new(0, 0, 80, 24)),
                    };
                    area.height.saturating_sub(6) as usize
                };
                match mouse.kind {
                    event::MouseEventKind::ScrollUp => {
                        *scroll_offset = scroll_offset.saturating_sub(3);
                        continue;
                    }
                    event::MouseEventKind::ScrollDown => {
                        let max = output_lines.len().saturating_sub(display_height);
                        *scroll_offset = (*scroll_offset + 3).min(max);
                        continue;
                    }
                    _ => {}
                }
            }

            // Main-view mouse: click to focus/select, click-again to open,
            // wheel to scroll, drag a seam to resize, click header to open menu.
            if matches!(app.dialog, Dialog::None) && !app.is_fs_busy() {
                use crossterm::event::{MouseButton, MouseEventKind};
                let (col, row) = (mouse.column, mouse.row);
                match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) => {
                        if row == 0 {
                            // Header row → open the menu bar.
                            app.dialog = Dialog::Menu { active_menu: 0, active_item: None };
                            continue;
                        }
                        // Start a divider drag if the click landed on a seam.
                        let ws = workspace_area(terminal);
                        if let Some(d) = app
                            .root
                            .dividers(ws)
                            .into_iter()
                            .find(|d| hit(d.rect, col, row))
                        {
                            drag = Some((d.path, d.area, d.dir));
                            continue;
                        }
                        // Otherwise focus the pane and select the item under the cursor.
                        if let Some((id, rect)) = app.pane_at(col, row) {
                            if app.tree_mode && id == app.root.first_leaf() {
                                app.click_tree(rect, row);
                            } else {
                                let activate = app.click_pane(id, rect, row);
                                if activate {
                                    app.handle_enter_or_right(terminal, true);
                                }
                            }
                        }
                        continue;
                    }
                    MouseEventKind::Drag(MouseButton::Left) => {
                        if let Some((path, area, dir)) = &drag {
                            let ratio = match dir {
                                crate::layout::Dir::Horizontal => {
                                    col.saturating_sub(area.x) as f32 / area.width.max(1) as f32
                                }
                                crate::layout::Dir::Vertical => {
                                    row.saturating_sub(area.y) as f32 / area.height.max(1) as f32
                                }
                            };
                            app.root.set_ratio(path, ratio);
                        }
                        continue;
                    }
                    MouseEventKind::Up(MouseButton::Left) => {
                        drag = None;
                        continue;
                    }
                    MouseEventKind::ScrollDown => {
                        if let Some((id, _)) = app.pane_at(col, row) {
                            app.scroll_pane(id, true);
                        }
                        continue;
                    }
                    MouseEventKind::ScrollUp => {
                        if let Some((id, _)) = app.pane_at(col, row) {
                            app.scroll_pane(id, false);
                        }
                        continue;
                    }
                    _ => {}
                }
            }
        }

        let key = match event {
            Event::Key(key) => key,
            _ => continue,
        };

        if key.kind == event::KeyEventKind::Release {
            continue;
        }

        // While a background fs job runs, the UI is modal: only allow cancel.
        if app.is_fs_busy() {
            match key.code {
                KeyCode::Esc | KeyCode::Char('c') | KeyCode::Char('C') => app.cancel_fs_job(),
                _ => {}
            }
            continue;
        }

        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
            && !matches!(app.dialog, Dialog::TerminalOverlay { .. }) {
                app.should_quit = true;
            }

        let active_dir = app.get_active_panel().path.clone();

        let is_view_file_unfocused = if let Dialog::ViewFile { focused: false, .. } = &app.dialog {
            key.code != KeyCode::Esc && key.code != KeyCode::F(3) && key.code != KeyCode::Tab
        } else {
            false
        };

        if is_view_file_unfocused {
            handle_main_keys(&mut app, key, terminal);
        } else {
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
                Dialog::InputTouch { input } => match key.code {
                    KeyCode::Enter => {
                        let text = input.text.clone();
                        app.dialog = Dialog::None;
                        app.execute_touch(text);
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
                Dialog::Properties { .. } => match key.code {
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char(' ') => {
                        app.dialog = Dialog::None;
                    }
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
                            if *active_row < 10 {
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
                                    let editors = ["nvim", "vim", "nano", "micro", "helix", "emacs", "vi", "code"];
                                    let cur = editors.iter().position(|&e| e == app.config.default_editor).unwrap_or(editors.len());
                                    app.config.default_editor = editors[(cur + 1) % editors.len()].to_string();
                                }
                                5 => {
                                    app.config.editor_mode = match app.config.editor_mode.as_str() {
                                        "internal" => "external".to_string(),
                                        _ => "internal".to_string(),
                                    };
                                }
                                6 => {
                                    let names = crate::theme::Theme::all_names();
                                    let cur = names.iter().position(|&n| n == app.config.theme).unwrap_or(0);
                                    app.config.theme = names[(cur + 1) % names.len()].to_string();
                                    app.apply_config();
                                }
                                7 => {
                                    app.config.border_type = match app.config.border_type.as_str() {
                                        "plain" => "rounded".to_string(),
                                        "rounded" => "thick".to_string(),
                                        "thick" => "double".to_string(),
                                        _ => "plain".to_string(),
                                    };
                                    app.apply_config();
                                }
                                8 => {
                                    app.config.use_nerd_fonts = !app.config.use_nerd_fonts;
                                    app.apply_config();
                                }
                                9 => {
                                    app.config.split_editor = !app.config.split_editor;
                                }
                                10 => {
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
                    KeyCode::Char('d') | KeyCode::Char('D')
                        if !app.config.bookmarks.is_empty() => {
                            app.config.bookmarks.remove(*selected_idx);
                            let _ = save_config(&app.config);
                            if *selected_idx > 0 && *selected_idx >= app.config.bookmarks.len() {
                                *selected_idx = app.config.bookmarks.len() - 1;
                            }
                            app.status_message = "Removed bookmark".to_string();
                        }
                    _ => {}
                }
                Dialog::TerminalOverlay { input, output_lines, scroll_offset, command_history, history_index } => {
                    let display_height = {
                        let area = match terminal.size() {
                            Ok(size) => centered_rect(85, 80, Rect::new(0, 0, size.width, size.height)),
                            Err(_) => centered_rect(85, 80, Rect::new(0, 0, 80, 24)),
                        };
                        area.height.saturating_sub(6) as usize
                    };

                    // Collect panel state we may need, to avoid borrowing app inside the match
                    let panel_path = active_dir.clone();
                    let mut cd_target: Option<PathBuf> = None;
                    let mut need_clear = false;

                    match key.code {
                        KeyCode::Esc => {
                            app.kill_running_process();
                            app.dialog = Dialog::None;
                        }
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            // Ctrl+C kills the running process but stays in overlay
                            if app.running_process.is_some() {
                                app.kill_running_process();
                            } else {
                                app.dialog = Dialog::None;
                            }
                        }
                        KeyCode::Enter => {
                            let text = input.text.clone();
                            if !text.is_empty() {
                                if command_history.last() != Some(&text) {
                                    command_history.push(text.clone());
                                }
                                *history_index = None;

                                let trimmed = text.trim();

                                if trimmed == "clear" || trimmed == "cls" {
                                    output_lines.clear();
                                    *scroll_offset = 0;
                                } else if trimmed == "cd" || trimmed == "cd ~" {
                                    if let Some(home) = env::var("HOME").ok().or_else(|| env::var("USERPROFILE").ok()) {
                                        let home_path = PathBuf::from(&home);
                                        output_lines.push(format!("❯ {}", text));
                                        output_lines.push(format!("→ {}", home_path.display()));
                                        cd_target = Some(home_path);
                                    }
                                } else if let Some(rest) = trimmed.strip_prefix("cd ") {
                                    let target = rest.trim();
                                    let target = if (target.starts_with('\'') && target.ends_with('\''))
                                        || (target.starts_with('"') && target.ends_with('"')) {
                                        if target.len() >= 2 { &target[1..target.len()-1] } else { target }
                                    } else { target };

                                    let resolved = if target == "~" {
                                        env::var("HOME").ok().map(PathBuf::from)
                                    } else if target.starts_with("~/") || target.starts_with("~\\") {
                                        env::var("HOME").ok().map(|h| PathBuf::from(h).join(&target[2..]))
                                    } else if target == "-" {
                                        panel_path.parent().map(|p| p.to_path_buf())
                                    } else {
                                        let p = PathBuf::from(target);
                                        if p.is_absolute() { Some(p) }
                                        else { Some(panel_path.join(p)) }
                                    };

                                    output_lines.push(format!("❯ {}", text));
                                    if let Some(p) = resolved {
                                        if p.is_dir() {
                                            output_lines.push(format!("→ {}", p.display()));
                                            cd_target = Some(p);
                                        } else {
                                            output_lines.push(format!("Not a directory: {}", p.display()));
                                        }
                                    }
                                } else if app.running_process.is_some() {
                                    // Don't clobber a streaming child (would orphan it
                                    // and leak its reader threads).
                                    output_lines.push(format!("❯ {}", text));
                                    output_lines.push(
                                        "⚠ A command is still running — press Ctrl+C to stop it first."
                                            .to_string(),
                                    );
                                } else {
                                    output_lines.push(format!("❯ {}", text));

                                    let expanded_cmd = if let Ok(home) = env::var("HOME") {
                                        text.replace("~/", &format!("{}/", home))
                                    } else {
                                        text.clone()
                                    };

                                    let (new_lines, needs_clear, child) = App::execute_overlay_command(&panel_path, &expanded_cmd);
                                    output_lines.extend(new_lines);
                                    need_clear = needs_clear;
                                    if let Some(child) = child {
                                        app.running_process = Some(App::start_streaming(child));
                                    }
                                }

                                // Auto-scroll to bottom
                                if output_lines.len() > display_height {
                                    *scroll_offset = output_lines.len() - display_height;
                                } else {
                                    *scroll_offset = 0;
                                }

                                input.text.clear();
                                input.cursor_position = 0;
                            }
                        }
                        KeyCode::Up if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if !command_history.is_empty() {
                                let idx = match *history_index {
                                    Some(i) => i.saturating_sub(1),
                                    None => command_history.len() - 1,
                                };
                                *history_index = Some(idx);
                                input.text = command_history[idx].clone();
                                input.cursor_position = input.text.chars().count();
                            }
                        }
                        KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if let Some(idx) = *history_index {
                                if idx + 1 < command_history.len() {
                                    let new_idx = idx + 1;
                                    *history_index = Some(new_idx);
                                    input.text = command_history[new_idx].clone();
                                    input.cursor_position = input.text.chars().count();
                                } else {
                                    *history_index = None;
                                    input.text.clear();
                                    input.cursor_position = 0;
                                }
                            }
                        }
                        KeyCode::Up => {
                            *scroll_offset = scroll_offset.saturating_sub(1);
                        }
                        KeyCode::Down => {
                            if *scroll_offset + display_height < output_lines.len() {
                                *scroll_offset += 1;
                            }
                        }
                        KeyCode::PageUp => {
                            *scroll_offset = scroll_offset.saturating_sub(display_height / 2);
                        }
                        KeyCode::PageDown => {
                            let max = output_lines.len().saturating_sub(display_height);
                            *scroll_offset = (*scroll_offset + display_height / 2).min(max);
                        }
                        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            output_lines.clear();
                            *scroll_offset = 0;
                        }
                        KeyCode::Home => input.home(),
                        KeyCode::End => input.end(),
                        KeyCode::Char(c) => input.insert(c),
                        KeyCode::Backspace => input.backspace(),
                        KeyCode::Delete => input.delete(),
                        KeyCode::Left => input.move_left(),
                        KeyCode::Right => input.move_right(),
                        _ => {}
                    }

                    // Deferred mutations on app (after dialog borrow is released)
                    if let Some(path) = cd_target {
                        let _ = app.get_active_panel_mut().set_path(path);
                    }
                    if need_clear {
                        let _ = terminal.clear();
                    }
                }
                Dialog::InternalEditor { file_path, lines, cursor_row, cursor_col, scroll_row, scroll_col, modified } => {
                    let (editor_width, editor_height) = {
                        let area = if app.config.split_editor && app.leaf_rects.len() > 1 {
                            app.leaf_rects.iter()
                                .find(|(id, _)| *id == app.partner)
                                .map(|(_, r)| *r)
                                .unwrap_or_else(|| {
                                    let rect = match terminal.size() {
                                        Ok(s) => Rect::new(0, 0, s.width, s.height),
                                        Err(_) => Rect::new(0, 0, 80, 24),
                                    };
                                    centered_rect(95, 90, rect)
                                })
                        } else {
                            let rect = match terminal.size() {
                                Ok(s) => Rect::new(0, 0, s.width, s.height),
                                Err(_) => Rect::new(0, 0, 80, 24),
                            };
                            centered_rect(95, 90, rect)
                        };
                        let line_num_width = format!("{}", lines.len()).len().max(3);
                        let h = area.height.saturating_sub(2) as usize;
                        let w = area.width.saturating_sub(2).saturating_sub(line_num_width as u16 + 2) as usize;
                        (w, h)
                    };

                    let mut save_file = false;
                    let mut close_editor = false;

                    match key.code {
                        KeyCode::Esc => {
                            // Don't silently discard unsaved work: require an
                            // explicit Ctrl+Q (discard) or Ctrl+S (save) first.
                            if *modified {
                                app.status_message =
                                    "Unsaved changes — Ctrl+S to save, Ctrl+Q to discard".to_string();
                            } else {
                                close_editor = true;
                            }
                        }
                        KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            save_file = true;
                        }
                        KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            // Explicit discard-and-close.
                            if *modified {
                                app.status_message = "Discarded unsaved changes".to_string();
                            }
                            close_editor = true;
                        }
                        KeyCode::Up => {
                            if *cursor_row > 0 {
                                *cursor_row -= 1;
                                let line_len = lines[*cursor_row].chars().count();
                                if *cursor_col > line_len { *cursor_col = line_len; }
                            }
                        }
                        KeyCode::Down => {
                            if *cursor_row + 1 < lines.len() {
                                *cursor_row += 1;
                                let line_len = lines[*cursor_row].chars().count();
                                if *cursor_col > line_len { *cursor_col = line_len; }
                            }
                        }
                        KeyCode::Left => {
                            if *cursor_col > 0 {
                                *cursor_col -= 1;
                            } else if *cursor_row > 0 {
                                *cursor_row -= 1;
                                *cursor_col = lines[*cursor_row].chars().count();
                            }
                        }
                        KeyCode::Right => {
                            let line_len = lines[*cursor_row].chars().count();
                            if *cursor_col < line_len {
                                *cursor_col += 1;
                            } else if *cursor_row + 1 < lines.len() {
                                *cursor_row += 1;
                                *cursor_col = 0;
                            }
                        }
                        KeyCode::Home => { *cursor_col = 0; }
                        KeyCode::End => {
                            *cursor_col = lines[*cursor_row].chars().count();
                        }
                        KeyCode::PageUp => {
                            *cursor_row = cursor_row.saturating_sub(editor_height);
                            let line_len = lines[*cursor_row].chars().count();
                            if *cursor_col > line_len { *cursor_col = line_len; }
                        }
                        KeyCode::PageDown => {
                            *cursor_row = (*cursor_row + editor_height).min(lines.len().saturating_sub(1));
                            let line_len = lines[*cursor_row].chars().count();
                            if *cursor_col > line_len { *cursor_col = line_len; }
                        }
                        KeyCode::Enter => {
                            let current = &lines[*cursor_row];
                            let byte_idx = current.char_indices().map(|(i, _)| i).nth(*cursor_col).unwrap_or(current.len());
                            let rest = current[byte_idx..].to_string();
                            lines[*cursor_row] = current[..byte_idx].to_string();
                            *cursor_row += 1;
                            *cursor_col = 0;
                            lines.insert(*cursor_row, rest);
                            *modified = true;
                        }
                        KeyCode::Backspace => {
                            if *cursor_col > 0 {
                                let line = &lines[*cursor_row];
                                let byte_idx = line.char_indices().map(|(i, _)| i).nth(*cursor_col - 1).unwrap_or(0);
                                let next_byte = line.char_indices().map(|(i, _)| i).nth(*cursor_col).unwrap_or(line.len());
                                let mut new_line = line[..byte_idx].to_string();
                                new_line.push_str(&line[next_byte..]);
                                lines[*cursor_row] = new_line;
                                *cursor_col -= 1;
                                *modified = true;
                            } else if *cursor_row > 0 {
                                let removed = lines.remove(*cursor_row);
                                *cursor_row -= 1;
                                *cursor_col = lines[*cursor_row].chars().count();
                                lines[*cursor_row].push_str(&removed);
                                *modified = true;
                            }
                        }
                        KeyCode::Delete => {
                            let line_len = lines[*cursor_row].chars().count();
                            if *cursor_col < line_len {
                                let line = &lines[*cursor_row];
                                let byte_idx = line.char_indices().map(|(i, _)| i).nth(*cursor_col).unwrap_or(line.len());
                                let next_byte = line.char_indices().map(|(i, _)| i).nth(*cursor_col + 1).unwrap_or(line.len());
                                let mut new_line = line[..byte_idx].to_string();
                                new_line.push_str(&line[next_byte..]);
                                lines[*cursor_row] = new_line;
                                *modified = true;
                            } else if *cursor_row + 1 < lines.len() {
                                let next = lines.remove(*cursor_row + 1);
                                lines[*cursor_row].push_str(&next);
                                *modified = true;
                            }
                        }
                        KeyCode::Tab => {
                            let line = &lines[*cursor_row];
                            let byte_idx = line.char_indices().map(|(i, _)| i).nth(*cursor_col).unwrap_or(line.len());
                            let mut new_line = line[..byte_idx].to_string();
                            new_line.push_str("    ");
                            new_line.push_str(&line[byte_idx..]);
                            lines[*cursor_row] = new_line;
                            *cursor_col += 4;
                            *modified = true;
                        }
                        KeyCode::Char(c) => {
                            let line = &lines[*cursor_row];
                            let byte_idx = line.char_indices().map(|(i, _)| i).nth(*cursor_col).unwrap_or(line.len());
                            let mut new_line = line[..byte_idx].to_string();
                            new_line.push(c);
                            new_line.push_str(&line[byte_idx..]);
                            lines[*cursor_row] = new_line;
                            *cursor_col += 1;
                            *modified = true;
                        }
                        _ => {}
                    }

                    // Adjust scroll to follow cursor
                    if *cursor_row < *scroll_row {
                        *scroll_row = *cursor_row;
                    }
                    if *cursor_row >= *scroll_row + editor_height {
                        *scroll_row = *cursor_row - editor_height + 1;
                    }
                    if *cursor_col < *scroll_col {
                        *scroll_col = *cursor_col;
                    }
                    if *cursor_col >= *scroll_col + editor_width {
                        *scroll_col = *cursor_col - editor_width + 1;
                    }

                    // Deferred save
                    if save_file {
                        let path = file_path.clone();
                        let content = if let Dialog::InternalEditor { lines, modified, .. } = &mut app.dialog {
                            let c = lines.join("\n") + "\n";
                            *modified = false;
                            Some(c)
                        } else { None };
                        if let Some(c) = content {
                            match std::fs::write(&path, c) {
                                Ok(_) => app.status_message = format!("Saved: {}", path.display()),
                                Err(e) => app.status_message = format!("Save failed: {}", e),
                            }
                        }
                    }
                    if close_editor {
                        app.dialog = Dialog::None;
                        app.refresh_panels();
                    }
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
                Dialog::ViewFile { scroll_offset, content, focused, .. } => {
                    let lines_count = content.lines().count();
                    let visible_lines = {
                        let area = if app.config.split_editor && app.leaf_rects.len() > 1 {
                            app.leaf_rects.iter()
                                .find(|(id, _)| *id == app.partner)
                                .map(|(_, r)| *r)
                                .unwrap_or_else(|| {
                                    let rect = match terminal.size() {
                                        Ok(s) => Rect::new(0, 0, s.width, s.height),
                                        Err(_) => Rect::new(0, 0, 80, 24),
                                    };
                                    centered_rect(90, 90, rect)
                                })
                        } else {
                            let rect = match terminal.size() {
                                Ok(s) => Rect::new(0, 0, s.width, s.height),
                                Err(_) => Rect::new(0, 0, 80, 24),
                            };
                            centered_rect(90, 90, rect)
                        };
                        area.height.saturating_sub(2) as usize
                    };
                    match key.code {
                        KeyCode::Esc | KeyCode::F(3) => {
                            app.dialog = Dialog::None;
                        }
                        KeyCode::Tab => {
                            *focused = !*focused;
                        }
                        _ if *focused => {
                            match key.code {
                                KeyCode::Char('q') => {
                                    app.dialog = Dialog::None;
                                }
                                KeyCode::Up | KeyCode::Char('k') => {
                                    *scroll_offset = scroll_offset.saturating_sub(1);
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    if *scroll_offset + visible_lines < lines_count {
                                        *scroll_offset += 1;
                                    }
                                }
                                KeyCode::PageUp => {
                                    *scroll_offset = scroll_offset.saturating_sub(visible_lines);
                                }
                                KeyCode::PageDown => {
                                    if *scroll_offset + visible_lines < lines_count {
                                        *scroll_offset += visible_lines;
                                    } else {
                                        *scroll_offset = lines_count.saturating_sub(visible_lines);
                                    }
                                }
                                _ => {}
                            }
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
            app.kill_running_process();
            if let Some(ref path) = app.write_last_dir {
                let active_dir = app.get_active_panel().path.clone();
                let _ = std::fs::write(path, active_dir.to_string_lossy().as_bytes());
            }
            break;
        }
    }
    Ok(())
}



fn handle_main_keys<B: ratatui::backend::Backend>(app: &mut App, key: KeyEvent, terminal: &mut Terminal<B>) {
    let prev_state = app.focus_snapshot();

    let keys = &app.keymap;

    if (key.code == KeyCode::Up && (key.modifiers.contains(KeyModifiers::SHIFT) || key.modifiers.contains(KeyModifiers::ALT)))
        || (key.code == KeyCode::Char('p') && key.modifiers.contains(KeyModifiers::SHIFT))
    {
        app.preview_scroll_offset = app.preview_scroll_offset.saturating_sub(1);
    } else if (key.code == KeyCode::Down && (key.modifiers.contains(KeyModifiers::SHIFT) || key.modifiers.contains(KeyModifiers::ALT)))
        || (key.code == KeyCode::Char('n') && key.modifiers.contains(KeyModifiers::SHIFT))
    {
        app.preview_scroll_offset = app.preview_scroll_offset.saturating_add(1);
    } else if key.code == KeyCode::PageUp && key.modifiers.contains(KeyModifiers::SHIFT) {
        app.preview_scroll_offset = app.preview_scroll_offset.saturating_sub(5);
    } else if key.code == KeyCode::PageDown && key.modifiers.contains(KeyModifiers::SHIFT) {
        app.preview_scroll_offset = app.preview_scroll_offset.saturating_add(5);
    } else if matches_key(&key, &keys.quit) {
        if app.config.confirm_quit {
            app.dialog = Dialog::ConfirmQuit;
        } else {
            app.should_quit = true;
        }
    } else if matches_key(&key, &keys.help) {
        app.dialog = Dialog::Help { active_tab: 0 };
    } else if matches_key(&key, &keys.view) {
        if let Some(item) = app.get_active_panel().get_selected_item().cloned()
            && !item.is_dir {
                app.open_viewer(item.path);
            }
    } else if matches_key(&key, &keys.edit) {
        app.open_editor(terminal);
    } else if matches_key(&key, &keys.copy) {
        app.initiate_copy();
    } else if matches_key(&key, &keys.move_item) {
        app.initiate_move();
    } else if matches_key(&key, &keys.mkdir) {
        if key.modifiers.contains(KeyModifiers::SHIFT) || key.modifiers.contains(KeyModifiers::CONTROL) {
            app.initiate_touch();
        } else {
            app.initiate_mkdir();
        }
    } else if matches_key(&key, &keys.delete) {
        app.initiate_delete();
    } else if key.code == KeyCode::F(2) || (key.code == KeyCode::Char('i') && key.modifiers.contains(KeyModifiers::CONTROL)) {
        app.initiate_properties();
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
            app.focus_first_pane();
            app.init_tree();
            app.update_right_panel_from_tree();
        } else {
            // On exit, move the focused (tree) pane to the directory the
            // partner content pane was showing.
            app.sync_focus_to_partner();
        }
        app.status_message = format!("Tree View: {}", if app.tree_mode { "ON" } else { "OFF" });
    } else if key.code == KeyCode::Char('o') && key.modifiers.contains(KeyModifiers::CONTROL) {
        app.dialog = Dialog::TerminalOverlay {
            input: InputField::new(String::new()),
            output_lines: vec![
                "Terminal Overlay — type commands below, Esc to close.".to_string(),
                "Scroll: ↑/↓/PgUp/PgDn/Mouse  History: Ctrl+↑/Ctrl+↓".to_string(),
                "".to_string(),
            ],
            scroll_offset: 0,
            command_history: Vec::new(),
            history_index: None,
        };
    } else if key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL) {
        app.dialog = Dialog::Settings { active_row: 0 };
    } else if matches_key(&key, &keys.fuzzy_find) {
        app.trigger_fuzzy_find();
    } else if matches_key(&key, &keys.live_grep) {
        app.trigger_live_grep();
    } else if matches_key(&key, &keys.zoxide_jump) {
        app.trigger_zoxide_jump();
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
    } else if key.code == KeyCode::Char('y') && key.modifiers.contains(KeyModifiers::CONTROL) {
        let path_str = app.get_active_panel().path.to_string_lossy().to_string();
        let copied = copy_to_clipboard(&path_str);
        if copied {
            app.status_message = format!("Path copied: {}", path_str);
        } else {
            app.status_message = "Failed to copy path (no clipboard tool found)".to_string();
        }
    } else if key.code == KeyCode::Char('u') && key.modifiers.contains(KeyModifiers::CONTROL) {
        // Ctrl+U — swap left and right panels
        app.swap_panels();
        app.status_message = "Panels swapped".to_string();
    } else if key.code == KeyCode::Char('=') && key.modifiers.is_empty() {
        // = — sync inactive panel to active panel's directory
        let active_path = app.get_active_panel().path.to_string_lossy().to_string();
        app.sync_panels();
        app.status_message = format!("Other panel synced to: {}", active_path);
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
    } else if key.code == KeyCode::Char('|') {
        // Split the focused pane into a left | right pair.
        app.split_focus(crate::layout::Dir::Horizontal);
    } else if key.code == KeyCode::Char('-') && key.modifiers.is_empty() {
        // Split the focused pane into a top / bottom pair.
        app.split_focus(crate::layout::Dir::Vertical);
    } else if key.code == KeyCode::Char('w') && key.modifiers.contains(KeyModifiers::CONTROL) {
        app.close_focus();
    } else if key.code == KeyCode::Right && key.modifiers.contains(KeyModifiers::CONTROL) {
        app.resize_focus(true, crate::layout::Dir::Horizontal);
    } else if key.code == KeyCode::Left && key.modifiers.contains(KeyModifiers::CONTROL) {
        app.resize_focus(false, crate::layout::Dir::Horizontal);
    } else if key.code == KeyCode::Down && key.modifiers.contains(KeyModifiers::CONTROL) {
        app.resize_focus(true, crate::layout::Dir::Vertical);
    } else if key.code == KeyCode::Up && key.modifiers.contains(KeyModifiers::CONTROL) {
        app.resize_focus(false, crate::layout::Dir::Vertical);
    } else if matches_key(&key, &keys.select_item) {
        if let Some(item) = app.get_active_panel().get_selected_item().cloned()
            && item.name != ".." {
                let panel = app.get_active_panel_mut();
                if panel.marked.contains(&item.path) {
                    panel.marked.remove(&item.path);
                } else {
                    panel.marked.insert(item.path.clone());
                }
                panel.select_next();
            }
    } else if matches_key(&key, &keys.up) {
        if app.tree_mode && app.is_tree_pane_focused() {
            if app.tree_selected > 0 {
                app.tree_selected -= 1;
                app.update_right_panel_from_tree();
            }
        } else {
            app.get_active_panel_mut().select_prev();
        }
    } else if matches_key(&key, &keys.down) {
        if app.tree_mode && app.is_tree_pane_focused() {
            if app.tree_selected + 1 < app.tree_nodes.len() {
                app.tree_selected += 1;
                app.update_right_panel_from_tree();
            }
        } else {
            app.get_active_panel_mut().select_next();
        }
    } else if matches_key(&key, &keys.left) {
        if app.tree_mode && app.is_tree_pane_focused() {
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
        if app.tree_mode && app.is_tree_pane_focused() {
            app.toggle_tree_node();
        } else {
            let is_enter = key.code == KeyCode::Enter;
            app.handle_enter_or_right(terminal, is_enter);
        }
    } else if matches_key(&key, &keys.tab) {
        app.toggle_panel();
    } else if matches_key(&key, &keys.tab_prev) {
        app.focus_prev();
    } else if matches_key(&key, &keys.pin_target) {
        app.toggle_target_pin();
    } else {
        // Fallback controls
        match key.code {
            KeyCode::PageUp => {
                if app.tree_mode && app.is_tree_pane_focused() {
                    app.tree_selected = app.tree_selected.saturating_sub(10);
                    app.update_right_panel_from_tree();
                } else {
                    app.get_active_panel_mut().page_up();
                }
            }
            KeyCode::PageDown => {
                if app.tree_mode && app.is_tree_pane_focused() {
                    if !app.tree_nodes.is_empty() {
                        app.tree_selected = std::cmp::min(app.tree_selected + 10, app.tree_nodes.len() - 1);
                        app.update_right_panel_from_tree();
                    }
                } else {
                    app.get_active_panel_mut().page_down();
                }
            }
            KeyCode::Home => {
                if app.tree_mode && app.is_tree_pane_focused() {
                    app.tree_selected = 0;
                    app.update_right_panel_from_tree();
                } else {
                    app.get_active_panel_mut().select_first();
                }
            }
            KeyCode::End => {
                if app.tree_mode && app.is_tree_pane_focused() {
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
                app.reset_git_query_all();
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

    let new_state = app.focus_snapshot();
    if prev_state != new_state {
        app.preview_scroll_offset = 0;
        let selected_item = app.get_active_panel().get_selected_item().cloned();
        if let Some(item) = selected_item {
            if let Dialog::ViewFile { path, content, scroll_offset, .. } = &mut app.dialog {
                *path = item.path.clone();
                *content = "\n  Loading preview...".to_string();
                *scroll_offset = 0;
                app.trigger_async_preview(item.path.clone(), 120, 40);
            }
        }
    }
}

// =============================================================================
// Clipboard Helper
// =============================================================================

/// Copies the given text to the system clipboard.
/// Uses `pbcopy` on macOS, and tries `xclip` then `xsel` on Linux.
/// Returns true if the copy succeeded.
fn copy_to_clipboard(text: &str) -> bool {
    use std::io::Write;
    use std::process::{Command, Stdio};

    #[cfg(target_os = "macos")]
    {
        if let Ok(mut child) = Command::new("pbcopy")
            .stdin(Stdio::piped())
            .spawn()
        {
            if let Some(stdin) = child.stdin.take() {
                let mut stdin = stdin;
                let _ = stdin.write_all(text.as_bytes());
            }
            return child.wait().map(|s| s.success()).unwrap_or(false);
        }
        false
    }

    #[cfg(not(target_os = "macos"))]
    {
        // Try xclip first, then xsel
        for (cmd, args) in &[
            ("xclip", vec!["-selection", "clipboard"]),
            ("xsel", vec!["--clipboard", "--input"]),
        ] {
            if let Ok(mut child) = Command::new(cmd)
                .args(args)
                .stdin(Stdio::piped())
                .spawn()
            {
                if let Some(stdin) = child.stdin.take() {
                    let mut stdin = stdin;
                    let _ = stdin.write_all(text.as_bytes());
                }
                if child.wait().map(|s| s.success()).unwrap_or(false) {
                    return true;
                }
            }
        }
        false
    }
}
