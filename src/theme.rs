use ratatui::style::Color;

#[derive(Clone, Debug, Copy)]
pub struct Theme {
    pub active_border: Color,
    pub inactive_border: Color,
    pub active_selection_bg: Color,
    pub inactive_selection_bg: Color,
    pub header_bg: Color,
    pub status_bg: Color,
    pub folder_fg: Color,
    pub symlink_fg: Color,
    pub executable_fg: Color,
    pub file_fg: Color,
    pub text_highlight: Color,
    pub accent: Color,
}

impl Theme {
    pub fn all_names() -> &'static [&'static str] {
        &["eve", "monokai", "catppuccin", "dracula", "nord", "solarized", "gruvbox", "tokyo-night", "lazygit"]
    }

    pub fn get_theme(name: &str) -> Self {
        match name {
            "monokai" => Self {
                active_border:         Color::Rgb(249, 38, 114),
                inactive_border:       Color::Rgb(80, 80, 60),
                active_selection_bg:   Color::Rgb(60, 56, 40),
                inactive_selection_bg: Color::Rgb(39, 40, 34),
                header_bg:             Color::Rgb(39, 40, 34),
                status_bg:             Color::Rgb(30, 30, 26),
                folder_fg:             Color::Rgb(102, 217, 239),
                symlink_fg:            Color::Rgb(249, 38, 114),
                executable_fg:         Color::Rgb(166, 226, 46),
                file_fg:               Color::Rgb(248, 248, 242),
                text_highlight:        Color::Rgb(230, 219, 116),
                accent:                Color::Rgb(249, 38, 114),
            },
            "catppuccin" => Self {
                active_border:         Color::Rgb(137, 180, 250),
                inactive_border:       Color::Rgb(69, 71, 90),
                active_selection_bg:   Color::Rgb(49, 50, 68),
                inactive_selection_bg: Color::Rgb(30, 30, 46),
                header_bg:             Color::Rgb(24, 24, 37),
                status_bg:             Color::Rgb(17, 17, 27),
                folder_fg:             Color::Rgb(137, 180, 250),
                symlink_fg:            Color::Rgb(250, 179, 135),
                executable_fg:         Color::Rgb(166, 227, 161),
                file_fg:               Color::Rgb(205, 214, 244),
                text_highlight:        Color::Rgb(249, 226, 175),
                accent:                Color::Rgb(203, 166, 247),
            },
            "dracula" => Self {
                active_border:         Color::Rgb(189, 147, 249),
                inactive_border:       Color::Rgb(68, 71, 90),
                active_selection_bg:   Color::Rgb(68, 71, 90),
                inactive_selection_bg: Color::Rgb(40, 42, 54),
                header_bg:             Color::Rgb(40, 42, 54),
                status_bg:             Color::Rgb(33, 34, 44),
                folder_fg:             Color::Rgb(139, 233, 253),
                symlink_fg:            Color::Rgb(255, 121, 198),
                executable_fg:         Color::Rgb(80, 250, 123),
                file_fg:               Color::Rgb(248, 248, 242),
                text_highlight:        Color::Rgb(241, 250, 140),
                accent:                Color::Rgb(189, 147, 249),
            },
            "nord" => Self {
                active_border:         Color::Rgb(136, 192, 208),
                inactive_border:       Color::Rgb(59, 66, 82),
                active_selection_bg:   Color::Rgb(59, 66, 82),
                inactive_selection_bg: Color::Rgb(46, 52, 64),
                header_bg:             Color::Rgb(46, 52, 64),
                status_bg:             Color::Rgb(36, 40, 50),
                folder_fg:             Color::Rgb(136, 192, 208),
                symlink_fg:            Color::Rgb(208, 135, 112),
                executable_fg:         Color::Rgb(163, 190, 140),
                file_fg:               Color::Rgb(216, 222, 233),
                text_highlight:        Color::Rgb(235, 203, 139),
                accent:                Color::Rgb(129, 161, 193),
            },
            "solarized" => Self {
                active_border:         Color::Rgb(38, 139, 210),
                inactive_border:       Color::Rgb(7, 54, 66),
                active_selection_bg:   Color::Rgb(7, 54, 66),
                inactive_selection_bg: Color::Rgb(0, 43, 54),
                header_bg:             Color::Rgb(0, 43, 54),
                status_bg:             Color::Rgb(0, 34, 43),
                folder_fg:             Color::Rgb(38, 139, 210),
                symlink_fg:            Color::Rgb(203, 75, 22),
                executable_fg:         Color::Rgb(133, 153, 0),
                file_fg:               Color::Rgb(147, 161, 161),
                text_highlight:        Color::Rgb(181, 137, 0),
                accent:                Color::Rgb(42, 161, 152),
            },
            "gruvbox" => Self {
                active_border:         Color::Rgb(215, 153, 33),
                inactive_border:       Color::Rgb(60, 56, 54),
                active_selection_bg:   Color::Rgb(60, 56, 54),
                inactive_selection_bg: Color::Rgb(40, 40, 40),
                header_bg:             Color::Rgb(40, 40, 40),
                status_bg:             Color::Rgb(30, 30, 28),
                folder_fg:             Color::Rgb(131, 165, 152),
                symlink_fg:            Color::Rgb(214, 93, 14),
                executable_fg:         Color::Rgb(184, 187, 38),
                file_fg:               Color::Rgb(235, 219, 178),
                text_highlight:        Color::Rgb(250, 189, 47),
                accent:                Color::Rgb(215, 153, 33),
            },
            "tokyo-night" => Self {
                active_border:         Color::Rgb(122, 162, 247),
                inactive_border:       Color::Rgb(41, 46, 66),
                active_selection_bg:   Color::Rgb(41, 46, 66),
                inactive_selection_bg: Color::Rgb(26, 27, 38),
                header_bg:             Color::Rgb(26, 27, 38),
                status_bg:             Color::Rgb(22, 22, 30),
                folder_fg:             Color::Rgb(122, 162, 247),
                symlink_fg:            Color::Rgb(255, 158, 100),
                executable_fg:         Color::Rgb(158, 206, 106),
                file_fg:               Color::Rgb(169, 177, 214),
                text_highlight:        Color::Rgb(224, 175, 104),
                accent:                Color::Rgb(187, 154, 247),
            },
            "lazygit" => Self {
                active_border:         Color::Rgb(74, 222, 128),   // green (vibrant green, like lazygit active border)
                inactive_border:       Color::Rgb(71, 85, 105),   // muted slate gray (like lazygit inactive borders)
                active_selection_bg:   Color::Rgb(124, 58, 237),  // violet/purple highlight (like the selection color in user screenshot)
                inactive_selection_bg: Color::Rgb(30, 41, 59),    // dark blue-gray
                header_bg:             Color::Rgb(15, 23, 42),     // slate-900 dark background
                status_bg:             Color::Rgb(15, 23, 42),
                folder_fg:             Color::Rgb(56, 189, 248),   // sky-blue folders
                symlink_fg:            Color::Rgb(244, 63, 94),    // rose symlinks
                executable_fg:         Color::Rgb(74, 222, 128),   // green executables
                file_fg:               Color::Rgb(226, 232, 240),  // off-white files
                text_highlight:        Color::Rgb(250, 204, 21),   // yellow text highlights
                accent:                Color::Rgb(74, 222, 128),   // green accent
            },
            _ => Self { // "eve" — Eve Online deep space
                active_border:         Color::Rgb(0, 210, 220),
                inactive_border:       Color::Rgb(20, 55, 65),
                active_selection_bg:   Color::Rgb(0, 60, 80),
                inactive_selection_bg: Color::Rgb(5, 20, 28),
                header_bg:             Color::Rgb(3, 10, 18),
                status_bg:             Color::Rgb(2, 7, 13),
                folder_fg:             Color::Rgb(0, 210, 220),
                symlink_fg:            Color::Rgb(255, 140, 0),
                executable_fg:         Color::Rgb(80, 255, 160),
                file_fg:               Color::Rgb(140, 190, 200),
                text_highlight:        Color::Rgb(255, 180, 0),
                accent:                Color::Rgb(0, 210, 220),
            },
        }
    }
}
