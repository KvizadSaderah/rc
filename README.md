# Rust Commander — Fast Dual-Pane File Manager in Rust

A lightweight, modern, and extremely fast dual-pane terminal file manager written in Rust, utilizing `ratatui` for UI rendering and `crossterm` for terminal event processing.

Designed with rich aesthetics, inspired by Midnight Commander but styled with modern, curated dark color palettes.

## 🚀 Installation & Quick Start

### Option 1: One-line install (Recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/KvizadSaderah/rc/master/install.sh | bash
```

This installer detects your OS (macOS/Linux), downloads the precompiled binary (on macOS) or automatically compiles it from source using `cargo` (on Linux), and installs it to `~/.local/bin/rc`.

### Option 2: Build from source

If you want to build and run it manually:

```bash
git clone https://github.com/KvizadSaderah/rc.git
cd rc
cargo build --release
mkdir -p ~/.local/bin
ln -sf $(pwd)/target/release/rc ~/.local/bin/rc
```

> [!NOTE]
> Make sure `~/.local/bin` is in your `PATH` environment variable. If not, add `export PATH="$HOME/.local/bin:$PATH"` to your `.zshrc` or `.bashrc`.

Once installed, simply type **`rc`** to launch the file manager from anywhere!

---

## 🎮 Navigation & Keyboard Controls

The navigation controls dynamically adapt depending on whether you are using **Standard** (default) or **Vim** keybindings:

### Core File Manager Keys

| Shortcut (Standard) | Shortcut (Vim Mode) | Action |
|---|---|---|
| **Tab** | **Tab** | Switch focus between the Left and Right panels |
| **Up / Down** | **j / k** | Move selection cursor in the active panel list |
| **Enter / Right** | **l** | Enter directories / Open file preview viewer |
| **Backspace / Left** | **h** | Go to the parent directory |
| **Space** | **Space** | Tag/Mark file for bulk Copy/Move/Delete (moves cursor down automatically) |
| **.** | **.** | Toggle show/hide hidden files (starting with `.`) |
| **Ctrl + P** | **p** | Toggle **Quick View Mode** (active folder live preview inside the inactive pane!) |
| **Ctrl + S** | **o** | Open **Settings/Configuration** overlay |
| **:** | **:** | Open **Command Line prompt** to execute shell commands, `cd`, or quit (`q`, `exit`) |
| **/** | **/** | Open **Interactive Filter prompt** to filter active list in real-time |
| **~** | **~** | Jump directly to your Home (`~`) folder |
| **R** | **R** | Refresh both panels |
| **Esc / q / F10** | **Esc / q** | Close popups / Exit the application |
| **Ctrl + C** | **Ctrl + C** | Hard exit application immediately |

### Function Keys (Standard)

- **F1**: Help manual / documentation popup
- **F3 / v**: Full screen file text/hex viewer
- **F4 / e**: Edit file (suspends TUI, opens your default `$EDITOR` or `nano`)
- **F5 / c**: Copy selected file/directory to target panel location
- **F6 / m**: Move/Rename selected item
- **F7 / n**: Create a new directory
- **F8 / d**: Delete selected item (recursive) with confirmation
- **F9**: Open Interactive Dropdown Menu Bar (Left, File, Command, Options, Right)

---

## ⚙️ Configuration File (`config.ini`)

Settings are loaded automatically at startup and saved to:
`~/.config/rust-commander/config.ini`

### Config Parameters:
- `show_hidden`: `true` or `false`
- `sort_by`: `name`, `size`, or `time`
- `keybindings`: `standard` or `vim`
- `default_editor`: system command to open files (e.g. `nano`, `vim`, `code`)

Press **Ctrl+S** (or **o** in Vim mode) to open the interactive settings dialog. Use **Up/Down** to navigate rows, **Space** to toggle settings, and **Enter** to save changes to disk.

---

## 🎨 Visual Aesthetics & Layout

- **Header Bar**: Shows the app title, keybinding configuration status.
- **Twin File Lists**: Active frame is outlined in Cyan (`Color::Rgb(6, 182, 212)`), inactive is slate gray (`Color::Rgb(71, 85, 105)`).
- **Color Coding**:
  - 📁 **Directories**: Cyan/SkyBlue text (`Color::Rgb(56, 189, 248)`)
  - ⚙️ **Executables**: Green text (`Color::Rgb(74, 222, 128)`)
  - 🔗 **Symlinks**: Rose/Pink text (`Color::Rgb(244, 63, 94)`)
  - 📄 **Standard Files**: Off-white/Gray text (`Color::Rgb(226, 232, 240)`)
- **Live Preview Pane**: When active, the opposite panel renders file tree summaries for directories, or text summaries/hex representations for files. Cached dynamically for buttery-smooth scrolls.
