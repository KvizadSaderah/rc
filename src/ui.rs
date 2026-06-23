use std::env;

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Position, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, BorderType, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::app::App;
use crate::panel::Panel;
use crate::theme::Theme;
use crate::types::*;

pub fn get_border_type(border_str: &str) -> BorderType {
    match border_str.to_lowercase().as_str() {
        "rounded" => BorderType::Rounded,
        "thick" => BorderType::Thick,
        "double" => BorderType::Double,
        _ => BorderType::Plain,
    }
}

/// Render a path for display: collapse the home directory to `~`.
pub fn pretty_path(p: &std::path::Path) -> String {
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        let home = std::path::PathBuf::from(home);
        if let Ok(rest) = p.strip_prefix(&home) {
            if rest.as_os_str().is_empty() {
                return "~".to_string();
            }
            return format!("~/{}", rest.to_string_lossy());
        }
    }
    p.to_string_lossy().into_owned()
}


// =============================================================================
// UI Drawing Layouts & Formatting
// =============================================================================

pub fn ui(f: &mut Frame, app: &mut App) {
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
    let idle_tab_style   = Style::default().fg(Color::Rgb(203, 213, 225));
    let tab = |idx: usize, label: &str| -> Span<'static> {
        let s = if active_menu_idx == Some(idx) { active_tab_style } else { idle_tab_style };
        Span::styled(format!(" {} ", label), s)
    };

    let mut left_spans = vec![
        Span::raw(" "),
        tab(0, "Left"),
        tab(1, "File"),
        tab(2, "Command"),
        tab(3, "Options"),
        tab(4, "Right"),
    ];
    let left_len: usize = left_spans.iter().map(|s| s.content.chars().count()).sum();

    let brand = "rust-commander";
    let bindings = format!(" {} ", app.config.keybindings);
    let right_len = brand.chars().count() + bindings.chars().count() + 1;
    let pad = (header_rect.width as usize).saturating_sub(left_len + right_len);
    left_spans.push(Span::raw(" ".repeat(pad)));
    left_spans.push(Span::styled(brand, Style::default().fg(theme.accent).bold()));
    left_spans.push(Span::styled(bindings, Style::default().fg(Color::Rgb(100, 116, 139))));
    f.render_widget(Paragraph::new(Line::from(left_spans)).bg(theme.header_bg), header_rect);

    let border_type = get_border_type(&app.config.border_type);

    let workspace = chunks[1];
    if app.tree_mode {
        // Tree mode is a fixed two-pane takeover: tree pane + content pane.
        let tree_pane = app.root.first_leaf();
        let partner = app.partner;
        app.leaf_rects = vec![(tree_pane, panels_layout[0]), (partner, panels_layout[1])];
        let tree_active = app.focus == tree_pane;
        let content_active = app.focus == partner;
        draw_tree_panel(f, panels_layout[0], app, tree_active, &theme);
        draw_beautiful_contents_panel(f, panels_layout[1], app, content_active, &theme);
    } else if app.preview_mode {
        // Quick-view takeover: focused pane on the left, live preview on the right.
        let focus = app.focus;
        app.leaf_rects = vec![(focus, panels_layout[0])];
        let selected_item = app.get_active_panel().get_selected_item().cloned();
        if let Some(p) = app.panels[focus].as_mut() {
            draw_panel(f, panels_layout[0], p, PaneRole::Active, &theme, border_type, app.config.use_nerd_fonts);
        }
        draw_live_preview(f, panels_layout[1], selected_item, app);
    } else {
        // Normal mode: render the tiling split tree.
        let focus = app.focus;
        let partner = app.partner;
        let pinned = app.target_pinned;
        let rects = app.root.rects(workspace);
        app.leaf_rects = rects.clone();
        // Flag the partner (copy/move target) when 3+ panes are open, or whenever
        // a target is explicitly pinned. With two unpinned panes the "other" pane
        // is unambiguous and the hint is just noise.
        let show_partner = rects.len() > 2 || pinned;
        for (id, rect) in rects {
            if let Some(p) = app.panels[id].as_mut() {
                let role = if id == focus {
                    PaneRole::Active
                } else if show_partner && id == partner {
                    PaneRole::Target { pinned }
                } else {
                    PaneRole::Idle
                };
                draw_panel(f, rect, p, role, &theme, border_type, app.config.use_nerd_fonts);
            }
        }
    }
    let split_editor_area = if app.config.split_editor && app.leaf_rects.len() > 1 {
        app.leaf_rects.iter()
            .find(|(id, _)| *id == app.partner)
            .map(|(_, r)| *r)
    } else {
        None
    };

    // 3. Bottom Status Line
    let status_rect = chunks[2];
    match &app.dialog {
        Dialog::CommandLine { input } => {
            let line = Line::from(vec![
                Span::styled("Run Command: ", Style::default().fg(theme.accent).bold()),
                Span::raw(input.text.as_str()),
            ]);
            f.render_widget(Paragraph::new(line).bg(theme.inactive_selection_bg), status_rect);
            f.set_cursor_position(Position::new(
                status_rect.x + 13 + input.visual_cursor_col(),
                status_rect.y,
            ));
        }
        Dialog::Filter { input } => {
            let line = Line::from(vec![
                Span::styled("Filter: ", Style::default().fg(theme.accent).bold()),
                Span::raw(input.text.as_str()),
            ]);
            f.render_widget(Paragraph::new(line).bg(theme.inactive_selection_bg), status_rect);
            f.set_cursor_position(Position::new(
                status_rect.x + 8 + input.visual_cursor_col(),
                status_rect.y,
            ));
        }
        _ => {
            // Left: status message, or the focused selection (name + size).
            let panel = app.get_active_panel();
            let left = if !app.status_message.is_empty() {
                format!(" {}", app.status_message)
            } else if let Some(it) = panel.get_selected_item() {
                let meta = if it.is_dir || it.name == ".." {
                    "<dir>".to_string()
                } else {
                    format_size(it.size)
                };
                format!(" {}   {}", it.name, meta)
            } else {
                String::new()
            };
            // Right: position in list, plus an adaptive source→target role hint
            // when the destination is ambiguous (3+ panes) or explicitly pinned.
            let pos = panel.selected + 1;
            let total = panel.items.len();
            let panes = app.root.leaves().len();
            let basename = |p: &std::path::Path| {
                p.file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.to_string_lossy().to_string())
            };
            let right = if panes > 2 || app.target_pinned {
                let src = basename(&app.get_active_panel().path);
                let dst = basename(&app.panel(app.partner).path);
                let arrow = if app.target_pinned { "📌" } else { "→" };
                format!("● {}  {}  ○ {}  ·  {}/{} ", src, arrow, dst, pos, total)
            } else {
                format!("{}/{} ", pos, total)
            };
            let w = status_rect.width as usize;
            let pad = w.saturating_sub(left.chars().count() + right.chars().count());
            let line = Line::from(vec![
                Span::styled(left, Style::default().fg(theme.file_fg)),
                Span::raw(" ".repeat(pad)),
                Span::styled(right, Style::default().fg(theme.inactive_border)),
            ]);
            f.render_widget(
                Paragraph::new(line).style(Style::default().bg(theme.status_bg)),
                status_rect,
            );
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
        Span::styled(" Ctrl+Y ", ctrl_key),
        Span::styled("CopyPath ", label_style),
        Span::styled(" = ",       ctrl_key),
        Span::styled("Sync ",    label_style),
        Span::styled(" Ctrl+U ", ctrl_key),
        Span::styled("Swap ",    label_style),
        Span::styled(" | - ",    ctrl_key),
        Span::styled("Split ",   label_style),
        Span::styled(" Ctrl+W ", ctrl_key),
        Span::styled("ClosePane ", label_style),
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
        Dialog::Properties {
            name,
            path_str,
            size_str,
            permissions_str,
            modified_str,
            created_str,
            owner_str,
        } => {
            let area = centered_rect_min(65, 45, 55, 13, f.area());
            f.render_widget(Clear, area);
            let block = Block::default()
                .title(" File / Folder Properties ")
                .title_alignment(Alignment::Center)
                .borders(Borders::ALL)
                .border_type(border_type)
                .border_style(Style::default().fg(theme.active_border))
                .bg(theme.status_bg);
            
            let text = vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled(" Name:       ", Style::default().fg(Color::Cyan).bold()),
                    Span::styled(name, Style::default().fg(Color::White)),
                ]),
                Line::from(vec![
                    Span::styled(" Path:       ", Style::default().fg(Color::Cyan).bold()),
                    Span::styled(path_str, Style::default().fg(Color::Rgb(156, 163, 175))),
                ]),
                Line::from(vec![
                    Span::styled(" Size:       ", Style::default().fg(Color::Cyan).bold()),
                    Span::styled(size_str, Style::default().fg(Color::Green)),
                ]),
                Line::from(vec![
                    Span::styled(" Mode:       ", Style::default().fg(Color::Cyan).bold()),
                    Span::styled(permissions_str, Style::default().fg(Color::Yellow)),
                ]),
                Line::from(vec![
                    Span::styled(" Owner:      ", Style::default().fg(Color::Cyan).bold()),
                    Span::styled(owner_str, Style::default().fg(Color::White)),
                ]),
                Line::from(vec![
                    Span::styled(" Created:    ", Style::default().fg(Color::Cyan).bold()),
                    Span::styled(created_str, Style::default().fg(Color::White)),
                ]),
                Line::from(vec![
                    Span::styled(" Modified:   ", Style::default().fg(Color::Cyan).bold()),
                    Span::styled(modified_str, Style::default().fg(Color::White)),
                ]),
                Line::from(""),
                Line::from("Press [Esc], [Enter], or [Space] to close.").alignment(Alignment::Center),
            ];
            
            let para = Paragraph::new(text).block(block);
            f.render_widget(para, area);
        }
        Dialog::Menu { active_menu, active_item } => {
            if let Some(item_idx) = active_item {
                draw_menu_dropdown(f, *active_menu, *item_idx, &theme);
            }
        }
        Dialog::ConfirmDelete { item_name, .. } => {
            let trash = app.config.use_trash;
            let (title, verb, accent) = if trash {
                (" Move to Trash ", "Move to trash", Color::Rgb(234, 179, 8))
            } else {
                (" Confirm Delete ", "Permanently delete", Color::Rgb(239, 68, 68))
            };
            let area = centered_rect_min(50, 20, 40, 5, f.area());
            f.render_widget(Clear, area);
            let block = Block::default()
                .title(title)
                .title_alignment(Alignment::Center)
                .borders(Borders::ALL)
                .border_type(border_type)
                .border_style(Style::default().fg(accent))
                .bg(theme.status_bg);

            let text = vec![
                Line::from(""),
                Line::from(vec![
                    Span::raw(format!("{} '", verb)),
                    Span::styled(item_name, Style::default().fg(Color::Yellow).bold()),
                    Span::raw("'?"),
                ]).alignment(Alignment::Center),
                Line::from(""),
                Line::from("[Y]/[Enter] confirm   ·   [N]/[Esc] cancel").alignment(Alignment::Center),
            ];
            
            let para = Paragraph::new(text).block(block);
            f.render_widget(para, area);
        }
        Dialog::InputMkdir { input } => {
            let area = centered_rect_min(60, 20, 36, 5, f.area());
            f.render_widget(Clear, area);
            let block = Block::default()
                .title(" Create Directory ")
                .borders(Borders::ALL)
                .border_type(border_type)
                .border_style(Style::default().fg(theme.active_border))
                .bg(theme.status_bg);
            
            let label = Paragraph::new("Enter directory name:").block(Block::default());
            let input_text = Paragraph::new(input.text.as_str())
                .style(Style::default().bg(theme.inactive_selection_bg).fg(theme.file_fg))
                .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(theme.inactive_border)));

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
        Dialog::InputTouch { input } => {
            let area = centered_rect_min(60, 20, 36, 5, f.area());
            f.render_widget(Clear, area);
            let border_type = get_border_type(&app.config.border_type);
            let block = Block::default()
                .title(" Create File ")
                .borders(Borders::ALL)
                .border_type(border_type)
                .border_style(Style::default().fg(theme.active_border))
                .bg(theme.status_bg);
            
            let label = Paragraph::new("Enter file name:").block(Block::default());
            let input_text = Paragraph::new(input.text.as_str())
                .style(Style::default().bg(theme.inactive_selection_bg).fg(theme.file_fg))
                .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(theme.inactive_border)));

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
            let area = centered_rect_min(65, 20, 40, 6, f.area());
            f.render_widget(Clear, area);
            let name = if source_path.as_os_str().is_empty() {
                "Selected items (bulk)".to_string()
            } else {
                source_path.file_name().unwrap_or_default().to_string_lossy().to_string()
            };
            let block = Block::default()
                .title(format!(" Copy: {} ", name))
                .borders(Borders::ALL)
                .border_type(border_type)
                .border_style(Style::default().fg(theme.active_border))
                .bg(theme.status_bg);
            
            let label = Paragraph::new("Copy to location:").block(Block::default());
            let input_text = Paragraph::new(input.text.as_str())
                .style(Style::default().bg(theme.inactive_selection_bg).fg(theme.file_fg))
                .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(theme.inactive_border)));

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
            let area = centered_rect_min(65, 20, 40, 6, f.area());
            f.render_widget(Clear, area);
            let name = if source_path.as_os_str().is_empty() {
                "Selected items (bulk)".to_string()
            } else {
                source_path.file_name().unwrap_or_default().to_string_lossy().to_string()
            };
            let block = Block::default()
                .title(format!(" Move: {} ", name))
                .borders(Borders::ALL)
                .border_type(border_type)
                .border_style(Style::default().fg(theme.active_border))
                .bg(theme.status_bg);
            
            let label = Paragraph::new("Move/rename to location:").block(Block::default());
            let input_text = Paragraph::new(input.text.as_str())
                .style(Style::default().bg(theme.inactive_selection_bg).fg(theme.file_fg))
                .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(theme.inactive_border)));

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
        Dialog::ViewFile { path, content, scroll_offset, focused } => {
            let area = if let Some(a) = split_editor_area {
                a
            } else {
                centered_rect_min(90, 90, 40, 10, f.area())
            };
            f.render_widget(Clear, area);
            let filename = path.file_name().unwrap_or_default().to_string_lossy();
            
            let border_color = if *focused { theme.active_border } else { theme.inactive_border };
            let title = if *focused {
                format!(" Viewer: {} (Tab: Focus List, Esc: Close) ", filename)
            } else {
                format!(" Viewer: {} (Tab: Focus Viewer, Esc: Close) ", filename)
            };

            let block = Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_type(border_type)
                .border_style(Style::default().fg(border_color))
                .bg(theme.status_bg);

            let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
            let is_image = ["png", "jpg", "jpeg", "gif", "webp", "bmp"].contains(&ext.as_str());

            if is_image {
                if let Some(protocol) = &mut app.viewer_image_protocol {
                    let inner_area = block.inner(area);
                    f.render_widget(block, area);
                    let stateful_image = ratatui_image::StatefulImage::default().resize(ratatui_image::Resize::Fit(None));
                    f.render_stateful_widget(stateful_image, inner_area, protocol);
                    return;
                }
            }

            let parsed_lines = parse_ansi_text(content);
            let visible_lines = area.height.saturating_sub(2) as usize;
            let display_lines = parsed_lines
                .into_iter()
                .skip(*scroll_offset)
                .take(visible_lines)
                .collect::<Vec<Line>>();

            let para = Paragraph::new(display_lines)
                .block(block)
                .wrap(Wrap { trim: false });
            f.render_widget(para, area);
        }
        Dialog::Settings { active_row } => {
            let area = centered_rect_min(70, 70, 50, 24, f.area());
            f.render_widget(Clear, area);
            let block = Block::default()
                .title(" Settings Configuration ")
                .title_alignment(Alignment::Center)
                .borders(Borders::ALL)
                .border_type(border_type)
                .border_style(Style::default().fg(theme.active_border))
                .bg(theme.status_bg);

            let row_style = |row: usize| -> Style {
                if *active_row == row { Style::default().fg(Color::Yellow).bold() } else { Style::default() }
            };
            let save_style = if *active_row == 10 { Style::default().fg(Color::Green).bold() } else { Style::default() };

            let r0_check = if app.config.show_hidden { "[X] Show" } else { "[ ] Hide" };
            let r1_val = format!("< {} >", app.config.sort_by.to_uppercase());
            let r2_val = format!("< {} >", app.config.keybindings.to_uppercase());
            let r3_check = if app.config.confirm_quit { "[X] Enabled" } else { "[ ] Disabled" };
            let r4_val = format!("< {} >", app.config.default_editor.to_uppercase());
            let r5_val = format!("< {} >", app.config.editor_mode.to_uppercase());
            let r6_val = format!("< {} >", app.config.theme.to_uppercase());
            let r7_val = format!("< {} >", app.config.border_type.to_uppercase());
            let r8_check = if app.config.use_nerd_fonts { "[X] Enabled" } else { "[ ] Disabled" };
            let r9_check = if app.config.split_editor { "[X] Enabled" } else { "[ ] Disabled" };

            let text = vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled("  Show Hidden Files:   ", row_style(0)),
                    Span::styled(r0_check, Style::default().fg(Color::Cyan)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("  Sorting Criteria:    ", row_style(1)),
                    Span::styled(r1_val, Style::default().fg(Color::Cyan)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("  Keybindings Mode:    ", row_style(2)),
                    Span::styled(r2_val, Style::default().fg(Color::Cyan)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("  Quit Confirmation:   ", row_style(3)),
                    Span::styled(r3_check, Style::default().fg(Color::Cyan)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("  Default Editor:      ", row_style(4)),
                    Span::styled(r4_val, Style::default().fg(Color::Cyan)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("  Editor Mode:         ", row_style(5)),
                    Span::styled(r5_val, Style::default().fg(Color::Cyan)),
                    Span::styled("  (external=suspend to TTY, internal=TUI popup)", Style::default().fg(Color::DarkGray)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("  Color Theme:         ", row_style(6)),
                    Span::styled(&r6_val, Style::default().fg(theme.accent)),
                    Span::styled("  (live preview)", Style::default().fg(Color::DarkGray)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("  Border Style:        ", row_style(7)),
                    Span::styled(&r7_val, Style::default().fg(theme.accent)),
                    Span::styled("  (live preview)", Style::default().fg(Color::DarkGray)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("  Nerd Fonts Icons:    ", row_style(8)),
                    Span::styled(r8_check, Style::default().fg(Color::Cyan)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("  Split Editor/Viewer: ", row_style(9)),
                    Span::styled(r9_check, Style::default().fg(Color::Cyan)),
                    Span::styled("  (View/Edit in opposite pane)", Style::default().fg(Color::DarkGray)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("     [ SAVE & CLOSE CONFIGURATION ]     ", save_style),
                ]).alignment(Alignment::Center),
                Line::from(""),
                Line::from("↑↓ navigate  Space/Enter change  Esc exit").alignment(Alignment::Center),
            ];

            let para = Paragraph::new(text).block(block);
            f.render_widget(para, area);
        }
        Dialog::ConfirmQuit => {
            let area = centered_rect_min(44, 30, 36, 8, f.area());
            f.render_widget(Clear, area);
            let block = Block::default()
                .title(" ⏻ EXIT ")
                .title_alignment(Alignment::Center)
                .borders(Borders::ALL)
                .border_type(border_type)
                .border_style(Style::default().fg(Color::Rgb(220, 60, 60)))
                .bg(theme.status_bg);

            let inner_h = area.height.saturating_sub(2) as usize;
            let mut text = Vec::new();

            if inner_h >= 9 {
                // Full layout
                text.push(Line::from(""));
                text.push(Line::from(Span::styled(
                    "Terminate session?",
                    Style::default().fg(Color::Rgb(220, 60, 60)).bold(),
                )).alignment(Alignment::Center));
                text.push(Line::from(""));
                text.push(Line::from(Span::styled(
                    "All unsaved state will be lost.",
                    Style::default().fg(Color::Rgb(140, 190, 200)),
                )).alignment(Alignment::Center));
                text.push(Line::from(""));
                text.push(Line::from(vec![
                    Span::styled(" Y/Enter ", Style::default().bg(Color::Rgb(220, 60, 60)).fg(Color::White).bold()),
                    Span::styled(" Confirm  ", Style::default().fg(Color::Rgb(140, 190, 200))),
                ]).alignment(Alignment::Center));
                text.push(Line::from(""));
                text.push(Line::from(vec![
                    Span::styled(" Esc/N   ", Style::default().bg(Color::Rgb(0, 60, 80)).fg(Color::Rgb(0, 210, 220)).bold()),
                    Span::styled(" Cancel   ", Style::default().fg(Color::Rgb(140, 190, 200))),
                ]).alignment(Alignment::Center));
            } else {
                // Compact layout for small screens
                text.push(Line::from(Span::styled(
                    "Exit? (unsaved state lost)",
                    Style::default().fg(Color::Rgb(220, 60, 60)).bold(),
                )).alignment(Alignment::Center));
                if inner_h >= 3 { text.push(Line::from("")); }
                text.push(Line::from(vec![
                    Span::styled(" Y ", Style::default().bg(Color::Rgb(220, 60, 60)).fg(Color::White).bold()),
                    Span::styled(" Confirm  ", Style::default().fg(Color::Rgb(140, 190, 200))),
                    Span::styled(" Esc ", Style::default().bg(Color::Rgb(0, 60, 80)).fg(Color::Rgb(0, 210, 220)).bold()),
                    Span::styled(" Cancel", Style::default().fg(Color::Rgb(140, 190, 200))),
                ]).alignment(Alignment::Center));
            }

            let para = Paragraph::new(text).block(block);
            f.render_widget(para, area);
        }
        Dialog::Help { active_tab } => {
            let area = centered_rect_min(72, 78, 50, 14, f.area());
            f.render_widget(Clear, area);

            let tab_titles = ["[1] Navigation", "[2] File Ops", "[3] Shell & View", "[4] Tips"];
            let tab_bar_line = Line::from(
                tab_titles.iter().enumerate().map(|(i, &t)| {
                    if i == *active_tab {
                        Span::styled(format!(" {} ", t), Style::default().bg(theme.active_selection_bg).fg(Color::White).bold())
                    } else {
                        Span::styled(format!(" {} ", t), Style::default().fg(Color::DarkGray))
                    }
                }).collect::<Vec<_>>()
            );

            let outer_block = Block::default()
                .title(Line::from(vec![
                    Span::styled(" ❓ RC Help ", Style::default().fg(theme.accent).bold()),
                ]))
                .title_alignment(Alignment::Center)
                .title_bottom(Line::from("  Tab/←/→: switch  1-4: jump  Esc/q: close  ").alignment(Alignment::Center))
                .borders(Borders::ALL)
                .border_type(border_type)
                .border_style(Style::default().fg(theme.active_border))
                .bg(theme.status_bg);

            let inner = outer_block.inner(area);
            f.render_widget(outer_block, area);

            let sections = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Min(0)])
                .split(inner);

            f.render_widget(Paragraph::new(tab_bar_line).bg(theme.inactive_selection_bg), sections[0]);
            f.render_widget(
                Paragraph::new("─".repeat(sections[1].width as usize)).fg(theme.inactive_border),
                sections[1]
            );

            let content_area = sections[2];
            let key = |s: &'static str| Span::styled(format!(" {:<13}", s), Style::default().fg(theme.text_highlight).bold());
            let desc = |s: &'static str| Span::styled(format!("  {}", s), Style::default().fg(theme.file_fg));
            let head = |s: &'static str| Line::from(Span::styled(
                format!("  ── {} ", s),
                Style::default().fg(theme.accent).bold()
            ));
            let row = |k: &'static str, d: &'static str| Line::from(vec![key(k), desc(d)]);

            let content: Vec<Line> = match *active_tab {
                0 => vec![
                    Line::from(""),
                    head("Panel Navigation"),
                    row("Tab",           "Cycle focus to next pane (source ●)"),
                    row("Shift+Tab",     "Cycle focus to previous pane"),
                    row("↑ / k",         "Move cursor up"),
                    row("↓ / j",         "Move cursor down"),
                    row("Enter",         "Open directory or file viewer"),
                    row("Backspace",     "Go to parent directory"),
                    row("~",            "Jump to Home directory"),
                    row("g / G",         "Jump to top / bottom of list"),
                    Line::from(""),
                    head("Panes (Tiling)"),
                    row("|",             "Split focused pane left / right"),
                    row("-",             "Split focused pane top / bottom"),
                    row("Ctrl+W",        "Close focused pane (not the last)"),
                    row("Ctrl+←↑↓→",     "Resize the focused pane"),
                    row("t",             "Pin focused pane as copy/move target ○"),
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
                    row("F2 / Ctrl+I",   "Show item properties dialog"),
                    row("F5 / c",        "Copy selection to target pane ○"),
                    row("F6 / m",        "Move / Rename selection to target ○"),
                    row("F7 / n",        "Create new directory (mkdir)"),
                    row("Shift+F7",      "Create empty file (touch)"),
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
                    Line::from(""),
                    head("Clipboard & Panels"),
                    row("Ctrl+Y",        "Copy current panel path to clipboard"),
                    row("=",             "Sync other panel to this directory"),
                    row("Ctrl+U",        "Swap left and right panels"),
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
                    row("Shift+↑ / ↓",   "Scroll live preview text"),
                    row("Ctrl+T",        "Toggle directory tree view"),
                    Line::from(""),
                    head("Search & Jump (CLI)"),
                    row("Ctrl+F",        "Fuzzy find files (fd + fzf)"),
                    row("Ctrl+G",        "Live grep file text (rg + fzf)"),
                    row("Ctrl+J",        "Zoxide quick directory jump"),
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
                    head("Mouse"),
                    Line::from(Span::styled(
                        "  Click focuses a pane & selects; click again opens.",
                        Style::default().fg(Color::Rgb(203, 213, 225))
                    )),
                    Line::from(Span::styled(
                        "  Wheel scrolls; drag a seam to resize; header = menu.",
                        Style::default().fg(Color::Rgb(203, 213, 225))
                    )),
                    Line::from(""),
                    head("Config File"),
                    Line::from(Span::styled(
                        "  ~/.config/rust-commander/config.ini",
                        Style::default().fg(Color::Rgb(203, 213, 225))
                    )),
                ],
            };

            let para = Paragraph::new(content)
                .style(Style::default().bg(theme.status_bg))
                .wrap(Wrap { trim: false });
            f.render_widget(para, content_area);
        }
        Dialog::Error { message } => {

            let area = centered_rect_min(50, 25, 30, 5, f.area());
            f.render_widget(Clear, area);
            let block = Block::default()
                .title(" System Error ")
                .title_alignment(Alignment::Center)
                .borders(Borders::ALL)
                .border_type(border_type)
                .border_style(Style::default().fg(Color::Red))
                .bg(theme.status_bg);
            
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
            let area = centered_rect_min(75, 60, 40, 10, f.area());
            f.render_widget(Clear, area);
            let block = Block::default()
                .title(" Command Output ")
                .title_alignment(Alignment::Center)
                .borders(Borders::ALL)
                .border_type(border_type)
                .border_style(Style::default().fg(theme.active_border))
                .bg(theme.status_bg);
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
            let area = centered_rect_min(60, 40, 36, 8, f.area());
            f.render_widget(Clear, area);
            let block = Block::default()
                .title(" Bookmarks Manager ")
                .title_alignment(Alignment::Center)
                .borders(Borders::ALL)
                .border_type(border_type)
                .border_style(Style::default().fg(theme.active_border))
                .bg(theme.status_bg);

            let list_items: Vec<ListItem> = if app.config.bookmarks.is_empty() {
                vec![ListItem::new("  No bookmarks saved yet. Press [A] to add current directory.")]
            } else {
                app.config.bookmarks.iter().enumerate().map(|(idx, path)| {
                    let is_selected = idx == *selected_idx;
                    let prefix = if is_selected { "▶ " } else { "  " };
                    let style = if is_selected {
                        Style::default().bg(theme.active_selection_bg).fg(Color::White).bold()
                    } else {
                        Style::default().fg(theme.file_fg)
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
        Dialog::TerminalOverlay { input, output_lines, scroll_offset, .. } => {
            let area = centered_rect_min(90, 80, 50, 12, f.area());
            f.render_widget(Clear, area);

            let display_height = area.height.saturating_sub(6) as usize;

            // Scroll indicator
            let scroll_info = if output_lines.len() > display_height {
                let start_idx = *scroll_offset + 1;
                let end_idx = (*scroll_offset + display_height).min(output_lines.len());
                format!(" {}-{}/{} ", start_idx, end_idx, output_lines.len())
            } else {
                String::new()
            };

            let is_running = app.running_process.is_some();
            let title_spans = if is_running {
                vec![
                    Span::styled(" Terminal ", Style::default().fg(theme.accent).bold()),
                    Span::styled("● RUNNING ", Style::default().fg(theme.executable_fg).bold()),
                ]
            } else {
                vec![
                    Span::styled(" Terminal ", Style::default().fg(theme.accent).bold()),
                ]
            };

            let block = Block::default()
                .title(Line::from(title_spans))
                .title_alignment(Alignment::Center)
                .title_bottom(Line::from(vec![
                    Span::styled("  Esc", Style::default().fg(theme.accent).bold()),
                    Span::styled(":close ", Style::default().fg(Color::DarkGray)),
                    Span::styled("Ctrl+C", Style::default().fg(Color::Rgb(220, 60, 60)).bold()),
                    Span::styled(":kill ", Style::default().fg(Color::DarkGray)),
                    Span::styled("Ctrl+↑↓", Style::default().fg(theme.accent).bold()),
                    Span::styled(":history ", Style::default().fg(Color::DarkGray)),
                    Span::styled("↑↓/PgUp/Dn/Scroll", Style::default().fg(theme.accent).bold()),
                    Span::styled(":scroll ", Style::default().fg(Color::DarkGray)),
                    Span::styled("Ctrl+L", Style::default().fg(theme.accent).bold()),
                    Span::styled(":clear ", Style::default().fg(Color::DarkGray)),
                    Span::styled(&scroll_info, Style::default().fg(Color::Yellow)),
                ]).alignment(Alignment::Center))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.active_border))
                .bg(theme.status_bg);
            f.render_widget(block.clone(), area);

            let inner = block.inner(area);
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(3),    // Scrollable command output
                    Constraint::Length(1), // Separator line
                    Constraint::Length(1), // CWD line
                    Constraint::Length(1), // Shell prompt and input line
                ])
                .split(inner);

            let lines: Vec<Line> = output_lines.iter()
                .skip(*scroll_offset)
                .take(display_height)
                .map(|line| {
                    if line.starts_with("❯ ") {
                        Line::from(Span::styled(line, Style::default().fg(theme.executable_fg).bold()))
                    } else if line.starts_with("→ ") {
                        Line::from(Span::styled(line, Style::default().fg(theme.accent)))
                    } else if line.starts_with("stderr:") || line.starts_with("Failed to") || line.contains("Error") {
                        Line::from(Span::styled(line, Style::default().fg(Color::Rgb(220, 60, 60))))
                    } else if line.starts_with("[") && (line.contains("exited") || line.contains("Launching")) {
                        Line::from(Span::styled(line, Style::default().fg(Color::Yellow)))
                    } else if line.starts_with("⚠") {
                        Line::from(Span::styled(line, Style::default().fg(Color::Rgb(255, 180, 0))))
                    } else {
                        Line::from(Span::styled(line, Style::default().fg(theme.file_fg)))
                    }
                })
                .collect();

            f.render_widget(Paragraph::new(lines), chunks[0]);
            f.render_widget(Paragraph::new("─".repeat(chunks[1].width as usize))
                .fg(theme.inactive_border), chunks[1]);

            // CWD line
            let cwd_display = {
                let path = app.get_active_panel().path.display().to_string();
                if let Ok(home) = env::var("HOME") {
                    if path.starts_with(&home) {
                        format!("~{}", &path[home.len()..])
                    } else { path }
                } else { path }
            };
            f.render_widget(Paragraph::new(Line::from(vec![
                Span::styled(" 📂 ", Style::default()),
                Span::styled(&cwd_display, Style::default().fg(theme.accent)),
            ])), chunks[2]);

            let prompt = "❯ ";
            let prompt_len = prompt.chars().count() as u16;
            let input_para = Paragraph::new(Line::from(vec![
                Span::styled(prompt, Style::default().fg(theme.executable_fg).bold()),
                Span::styled(input.text.as_str(), Style::default().fg(Color::White)),
            ]));
            f.render_widget(input_para, chunks[3]);

            f.set_cursor_position(Position::new(
                chunks[3].x + prompt_len + input.visual_cursor_col(),
                chunks[3].y,
            ));
        }
        Dialog::InternalEditor { file_path, lines, cursor_row, cursor_col, scroll_row, scroll_col, modified } => {
            let area = if let Some(a) = split_editor_area {
                a
            } else {
                centered_rect_min(95, 92, 50, 12, f.area())
            };
            f.render_widget(Clear, area);

            let filename = file_path.file_name().unwrap_or_default().to_string_lossy();
            let mod_indicator = if *modified { " [+]" } else { "" };

            let block = Block::default()
                .title(Line::from(vec![
                    Span::styled(format!(" {} ", filename), Style::default().fg(Color::White).bold()),
                    Span::styled(mod_indicator, Style::default().fg(theme.accent).bold()),
                ]))
                .title_bottom(Line::from(vec![
                    Span::styled("  Ctrl+S", Style::default().fg(theme.accent).bold()),
                    Span::styled(":save ", Style::default().fg(Color::DarkGray)),
                    Span::styled("Ctrl+Q", Style::default().fg(theme.accent).bold()),
                    Span::styled(":discard ", Style::default().fg(Color::DarkGray)),
                    Span::styled("Tab", Style::default().fg(theme.accent).bold()),
                    Span::styled(":indent ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!(" Ln {}, Col {} ", cursor_row + 1, cursor_col + 1),
                        Style::default().fg(theme.accent)
                    ),
                ]).alignment(Alignment::Center))
                .borders(Borders::ALL)
                .border_type(border_type)
                .border_style(Style::default().fg(theme.active_border))
                .bg(theme.status_bg);
            f.render_widget(block.clone(), area);

            let inner = block.inner(area);
            let editor_height = inner.height as usize;
            let line_num_width = format!("{}", lines.len()).len().max(3);
            let text_width = inner.width.saturating_sub(line_num_width as u16 + 2) as usize;

            let mut display_lines: Vec<Line> = Vec::new();
            #[allow(clippy::needless_range_loop)]
            for row_idx in *scroll_row..(*scroll_row + editor_height).min(lines.len()) {
                let is_current = row_idx == *cursor_row;
                let num_style = if is_current {
                    Style::default().fg(theme.accent).bold()
                } else {
                    Style::default().fg(theme.inactive_border)
                };

                let line_num = format!("{:>width$} ", row_idx + 1, width = line_num_width);
                let line_content = &lines[row_idx];

                // Slice for horizontal scroll
                let visible: String = line_content.chars()
                    .skip(*scroll_col)
                    .take(text_width)
                    .collect();

                let text_style = if is_current {
                    Style::default().fg(Color::White).bg(theme.active_selection_bg)
                } else {
                    Style::default().fg(theme.file_fg)
                };

                // Pad to full width for highlight
                let padded = format!("{:<width$}", visible, width = text_width);

                display_lines.push(Line::from(vec![
                    Span::styled(line_num, num_style),
                    Span::styled("│", Style::default().fg(theme.inactive_border)),
                    Span::styled(padded, text_style),
                ]));
            }

            // Fill remaining empty lines
            for _ in display_lines.len()..editor_height {
                let line_num = " ".repeat(line_num_width + 1);
                display_lines.push(Line::from(vec![
                    Span::styled(line_num, Style::default()),
                    Span::styled("│", Style::default().fg(theme.inactive_border)),
                    Span::styled("~", Style::default().fg(theme.inactive_border)),
                ]));
            }

            f.render_widget(Paragraph::new(display_lines), inner);

            // Place cursor
            let cursor_screen_row = inner.y + (*cursor_row - *scroll_row) as u16;
            let cursor_screen_col = inner.x + line_num_width as u16 + 2 + (*cursor_col - *scroll_col) as u16;
            f.set_cursor_position(Position::new(cursor_screen_col, cursor_screen_row));
        }
    }

    // Background filesystem job progress overlay (independent of Dialog state)
    if app.fs_job.is_some() {
        draw_fs_progress(f, app, border_type, &theme);
    }
}

// Progress overlay for a running background copy/move/delete job.
fn draw_fs_progress(f: &mut Frame, app: &App, border_type: BorderType, theme: &Theme) {
    let job = match &app.fs_job {
        Some(j) => j,
        None => return,
    };

    let area = centered_rect_min(60, 30, 44, 9, f.area());
    f.render_widget(Clear, area);

    let title = format!(" {} ", job.kind.verb());
    let block = Block::default()
        .title(title)
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_type(border_type)
        .border_style(Style::default().fg(theme.active_border))
        .bg(theme.status_bg);
    f.render_widget(block.clone(), area);
    let inner = block.inner(area);

    let pct = (job.ratio() * 100.0).round() as u16;
    let bar_width = inner.width.saturating_sub(2) as usize;
    let filled = (bar_width as f64 * job.ratio()).round() as usize;
    let bar: String = "█".repeat(filled) + &"░".repeat(bar_width.saturating_sub(filled));

    let current = if job.current.is_empty() { "scanning…".to_string() } else { job.current.clone() };
    let current_disp = truncate_middle(&current, bar_width);

    let bytes_line = if job.total_bytes > 0 {
        format!("{} / {}", format_size(job.done_bytes), format_size(job.total_bytes))
    } else {
        format!("{} / {} items", job.done_files, job.total_files)
    };

    let text = vec![
        Line::from(Span::styled(current_disp, Style::default().fg(theme.file_fg))),
        Line::from(""),
        Line::from(Span::styled(bar, Style::default().fg(theme.accent))),
        Line::from(Span::styled(
            format!("{}%   {}   ({}/{} files)", pct, bytes_line, job.done_files, job.total_files),
            Style::default().fg(theme.inactive_border),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Esc / c — cancel",
            Style::default().fg(theme.inactive_border),
        ))
        .alignment(Alignment::Center),
    ];
    let para = Paragraph::new(text).wrap(Wrap { trim: true });
    f.render_widget(para, inner);
}

/// Shorten a string to `max` columns, keeping the head and tail with an ellipsis.
fn truncate_middle(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max || max < 4 {
        return s.chars().take(max).collect();
    }
    let head = (max - 1) / 2;
    let tail = max - 1 - head;
    let mut out: String = chars[..head].iter().collect();
    out.push('…');
    out.extend(&chars[chars.len() - tail..]);
    out
}

// Drops down overlay block under the active top tab
fn draw_menu_dropdown(f: &mut Frame, active_menu: usize, item_idx: usize, theme: &Theme) {
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
        .border_style(Style::default().fg(theme.active_border))
        .bg(theme.status_bg);

    let list_items: Vec<ListItem> = items.iter().enumerate().map(|(idx, item)| {
        let style = if idx == item_idx {
            Style::default().bg(theme.active_selection_bg).fg(Color::White).bold()
        } else {
            Style::default().fg(theme.file_fg)
        };
        ListItem::new(Line::from(format!(" {}", item))).style(style)
    }).collect();

    let list = List::new(list_items).block(block);
    f.render_widget(list, area);
}

fn draw_live_preview(f: &mut Frame, area: Rect, selected: Option<FileItem>, app: &mut App) {
    let theme = Theme::get_theme(&app.config.theme.clone());
    let border_type = get_border_type(&app.config.border_type);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(border_type)
        .border_style(Style::default().fg(theme.inactive_border))
        .bg(theme.status_bg);

    if let Some(item) = selected {
        let title_span = Span::styled(
            format!(" Live Preview: {} ", item.name),
            Style::default().fg(Color::Rgb(14, 116, 144)).bold(),
        );
        let active_block = block.title(title_span);

        // Check if we have a ready image preview
        if let PreviewState::ReadyImage { path, protocol, .. } = &mut app.preview_state {
            if path == &item.path {
                let inner_area = active_block.inner(area);
                f.render_widget(active_block, area);

                let stateful_image = ratatui_image::StatefulImage::default().resize(ratatui_image::Resize::Fit(None));
                f.render_stateful_widget(stateful_image, inner_area, protocol);
                return;
            }
        }

        let body = if item.name == ".." {
            "↩ Go up to parent folder".to_string()
        } else if item.is_dir {
            read_dir_preview(&item.path)
        } else {
            app.get_preview_content(item.path, area.width.saturating_sub(2), area.height.saturating_sub(2))
        };

        let content_lines = parse_ansi_text(&body);
        let display_height = area.height.saturating_sub(2) as usize;
        let total_lines = content_lines.len();
        let max_offset = total_lines.saturating_sub(display_height);
        if app.preview_scroll_offset > max_offset {
            app.preview_scroll_offset = max_offset;
        }

        let lines: Vec<Line> = content_lines
            .into_iter()
            .skip(app.preview_scroll_offset)
            .take(display_height)
            .collect();

        let para = Paragraph::new(lines).block(active_block).wrap(Wrap { trim: false });
        f.render_widget(para, area);
    } else {
        let active_block = block.title(" Live Preview ");
        let content_lines = parse_ansi_text("No item selected");
        let display_height = area.height.saturating_sub(2) as usize;
        let lines: Vec<Line> = content_lines
            .into_iter()
            .take(display_height)
            .collect();
        let para = Paragraph::new(lines).block(active_block).wrap(Wrap { trim: false });
        f.render_widget(para, area);
    }
}

/// How a pane is rendered relative to the current copy/move operation.
#[derive(Clone, Copy)]
enum PaneRole {
    /// The focused pane — the copy/move source.
    Active,
    /// The partner pane — the copy/move destination. `pinned` marks a target
    /// the user explicitly fixed in place.
    Target { pinned: bool },
    /// Any other pane.
    Idle,
}

fn draw_panel(f: &mut Frame, area: Rect, panel: &mut Panel, role: PaneRole, theme: &Theme, border_type: BorderType, use_nerd_fonts: bool) {
    let is_active = matches!(role, PaneRole::Active);
    let is_partner = matches!(role, PaneRole::Target { .. });
    let border_color = if is_active {
        theme.active_border
    } else if is_partner {
        theme.text_highlight
    } else {
        theme.inactive_border
    };
    let title_fg = if is_active { Color::White } else { Color::Rgb(120, 130, 145) };

    // Focus marker + path (home collapsed to ~). The partner pane (copy/move
    // destination) is flagged with ○ so the source→target link is visible when
    // more than two panes are open.
    let marker = if is_active { "● " } else if is_partner { "○ " } else { "  " };
    let mut title_spans = vec![
        Span::styled(marker, Style::default().fg(border_color).bold()),
        Span::styled(pretty_path(&panel.path), Style::default().fg(title_fg).bold()),
    ];
    if let PaneRole::Target { pinned } = role {
        let label = if pinned { "  → target 📌" } else { "  → target" };
        title_spans.push(Span::styled(
            label,
            Style::default().fg(theme.text_highlight).bold(),
        ));
    }
    if let Some(ref branch) = panel.git_branch {
        title_spans.push(Span::styled(
            format!("  {}", branch),
            Style::default().fg(theme.active_border),
        ));
    }
    if let Some(ref filter) = panel.filter {
        title_spans.push(Span::styled(
            format!("  /{}", filter),
            Style::default().fg(Color::Yellow),
        ));
    }
    if !panel.marked.is_empty() {
        title_spans.push(Span::styled(
            format!("  ✔{}", panel.marked.len()),
            Style::default().fg(theme.text_highlight).bold(),
        ));
    }
    title_spans.push(Span::raw(" "));

    // Bottom border shows sort mode + item count — quiet, right-aligned feel.
    let footer = format!(" {} · {} items ", panel.sort_by.to_lowercase(), panel.items.len().saturating_sub(1));

    let block = Block::default()
        .title(Line::from(title_spans))
        .title_bottom(Line::from(Span::styled(footer, Style::default().fg(Color::Rgb(110, 120, 135)))).alignment(Alignment::Right))
        .borders(Borders::ALL)
        .border_type(border_type)
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

    let total_items = panel.items.len();
    let display_height = area.height.saturating_sub(2) as usize;
    let current_offset = panel.scroll_state.offset();

    let mut offset = current_offset;
    if let Some(selected) = panel.scroll_state.selected() {
        if selected < offset {
            offset = selected;
        } else if selected >= offset + display_height {
            offset = selected.saturating_sub(display_height).saturating_add(1);
        }
    }
    let max_offset = total_items.saturating_sub(display_height);
    if offset > max_offset {
        offset = max_offset;
    }
    *panel.scroll_state.offset_mut() = offset;

    let list_items: Vec<ListItem> = panel.items.iter().enumerate().map(|(idx, item)| {
        if idx < offset || idx >= offset + display_height {
            return ListItem::new("");
        }

        let is_selected = Some(idx) == panel.scroll_state.selected();
        let is_marked = panel.marked.contains(&item.path);

        let icon = get_icon(item, use_nerd_fonts);

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
        // Reserve one column for the selection gutter on the left.
        let name_w = width.saturating_sub(time_w + size_w + 4) as usize;

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

        let gutter = if is_selected {
            if is_active { "▌" } else { "▎" }
        } else {
            " "
        };
        let gutter_span = Span::styled(gutter, Style::default().fg(if is_active { theme.active_border } else { theme.inactive_border }));

        let line = Line::from(vec![
            gutter_span,
            Span::raw(final_name_str),
            git_span,
            Span::raw(padding),
            Span::styled(" │ ", Style::default().fg(Color::Rgb(70, 80, 95))),
            Span::raw(format!("{:>width$}", size_str, width = size_w as usize)),
            Span::styled(" │ ", Style::default().fg(Color::Rgb(70, 80, 95))),
            Span::raw(time_str),
        ]);

        ListItem::new(line).style(item_style)
    }).collect();

    let list = List::new(list_items)
        .block(block)
        .highlight_style(Style::default());
    
    f.render_stateful_widget(list, area, &mut panel.scroll_state);
}

pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    centered_rect_min(percent_x, percent_y, 20, 6, r)
}

pub fn centered_rect_min(percent_x: u16, percent_y: u16, min_w: u16, min_h: u16, r: Rect) -> Rect {
    // Calculate desired size from percentage
    let mut w = r.width * percent_x / 100;
    let mut h = r.height * percent_y / 100;
    // Enforce minimum sizes
    w = w.max(min_w).min(r.width);
    h = h.max(min_h).min(r.height);
    // Center
    let x = r.x + r.width.saturating_sub(w) / 2;
    let y = r.y + r.height.saturating_sub(h) / 2;
    Rect::new(x, y, w, h)
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

    let total_items = app.tree_nodes.len();
    let display_height = area.height.saturating_sub(2) as usize;
    let current_offset = app.tree_state.offset();

    let mut offset = current_offset;
    let sel = app.tree_selected;
    if sel < offset {
        offset = sel;
    } else if sel >= offset + display_height {
        offset = sel.saturating_sub(display_height).saturating_add(1);
    }
    let max_offset = total_items.saturating_sub(display_height);
    if offset > max_offset {
        offset = max_offset;
    }
    *app.tree_state.offset_mut() = offset;

    let list_items: Vec<ListItem> = app.tree_nodes.iter().enumerate().map(|(idx, node)| {
        if idx < offset || idx >= offset + display_height {
            return ListItem::new("");
        }

        let is_selected = idx == app.tree_selected;
        
        let indent = "  ".repeat(node.depth);
        let use_nf = app.config.use_nerd_fonts;
        let folder_icon = if use_nf {
            if node.is_expanded { "\u{e5fe} " } else { "\u{e5ff} " }
        } else {
            if node.is_expanded { "📂 " } else { "📁 " }
        };
        let toggle_icon = if !node.has_subdirs {
            "  "
        } else if node.is_expanded {
            if use_nf { "\u{f107} " } else { "▼ " }
        } else {
            if use_nf { "\u{f105} " } else { "▶ " }
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

    app.tree_state.select(Some(sel));

    let list = List::new(list_items)
        .block(block)
        .highlight_style(Style::default());

    f.render_stateful_widget(list, area, &mut app.tree_state);
}

fn draw_beautiful_contents_panel(f: &mut Frame, area: Rect, app: &mut App, is_active: bool, theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(60),
            Constraint::Percentage(40),
        ])
        .split(area);

    let border_type = get_border_type(&app.config.border_type);
    let partner = app.partner;
    let selected_item = app.panel(partner).get_selected_item().cloned();
    if let Some(p) = app.panels[partner].as_mut() {
        let role = if is_active { PaneRole::Active } else { PaneRole::Idle };
        draw_panel(f, chunks[0], p, role, theme, border_type, app.config.use_nerd_fonts);
    }
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
                                            } else if parts[i + 1] == "2"
                                                && i + 4 < parts.len() {
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
                                    48
                                        if i + 1 < parts.len() => {
                                            if parts[i + 1] == "5" {
                                                if i + 2 < parts.len() {
                                                    if let Ok(idx) = parts[i + 2].parse::<u8>() {
                                                        current_style = current_style.bg(Color::Indexed(idx));
                                                    }
                                                    i += 2;
                                                }
                                            } else if parts[i + 1] == "2"
                                                && i + 4 < parts.len() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ansi_text() {
        let input = "\x1b[48;2;43;48;59m\x1b[38;2;180;142;173mimport\x1b[0m";
        let parsed = parse_ansi_text(input);
        assert_eq!(parsed.len(), 1);
        let line = &parsed[0];
        assert_eq!(line.spans.len(), 1, "expected 1 span, got {:?}", line.spans);
        let span = &line.spans[0];
        assert_eq!(span.content, "import");
        assert_eq!(span.style.fg, Some(Color::Rgb(180, 142, 173)));
        assert_eq!(span.style.bg, Some(Color::Rgb(43, 48, 59)));
    }
}
