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
| **Ctrl + A** | **Ctrl + A** | Select / deselect all items in the active panel |
| **.** | **.** | Toggle show/hide hidden files (starting with `.`) |
| **Ctrl + P** | **p** | Toggle **Quick View Mode** (active folder live preview inside the inactive pane!) |
| **Ctrl + T** | **Ctrl + T** | Toggle **Tree View** in the left panel |
| **Ctrl + S** | **o** | Open **Settings/Configuration** overlay |
| **Ctrl + O** | **Ctrl + O** | Open **Terminal Overlay** (streaming command output) |
| **Ctrl + B** | **Ctrl + B** | Open **Bookmarks**; **Ctrl + D** bookmarks the current folder |
| **Ctrl + F** | **Ctrl + F** | Fuzzy Find files recursively in the active directory (uses `fd` + `fzf`) |
| **Ctrl + G** | **Ctrl + G** | Live Grep text recursively in the active directory (uses `rg` + `fzf`) |
| **Ctrl + J** | **Ctrl + J** | Jump to any directory using `zoxide` history database |
| **Ctrl + U** | **Ctrl + U** | Swap the left and right panel directories |
| **=** | **=** | Sync the inactive panel to the active panel's directory |
| **Ctrl + Y** | **Ctrl + Y** | Copy the active panel path to the clipboard |
| **:** | **:** | Open **Command Line prompt** to execute shell commands, `cd`, or quit (`q`, `exit`) |
| **/** | **/** | Open **Interactive Filter prompt** to filter active list in real-time |
| **~** | **~** | Jump directly to your Home (`~`) folder |
| **R** | **R** | Refresh both panels |
| **Esc / q / F10** | **Esc / q** | Close popups / Exit the application |
| **Ctrl + C** | **Ctrl + C** | Hard exit application immediately |

### Function Keys (Standard)

- **F1**: Help manual / documentation popup
- **F2 / Ctrl + I**: File/folder **Properties** (size, permissions, owner, timestamps)
- **F3 / v**: Full screen file text/hex viewer
- **F4 / e**: Edit file (suspends TUI, opens your default `$EDITOR` or `nano`)
- **F5 / c**: Copy selected/tagged file(s) or directory to the other panel
- **F6 / m**: Move/Rename selected/tagged item(s)
- **F7 / n**: Create a new directory
- **F8 / d**: Delete selected/tagged item(s) (recursive) with confirmation
- **F9**: Open Interactive Dropdown Menu Bar (Left, File, Command, Options, Right)

> **Background operations:** Copy, Move and Delete run on a background thread
> with a live progress bar (current item, byte/file counts, percentage). The UI
> never freezes on large files, deep trees, or slow/network mounts. Press
> **Esc** or **c** to cancel an operation in progress. Per-item errors are
> collected and reported at the end instead of aborting on the first failure.

> **Live updates:** Panels refresh automatically when files change on disk
> (via a filesystem watcher), so external changes appear without pressing **R**.

### Panes (Tiling)

Beyond the classic two-pane view you can split the workspace into more panes,
tiling-window-manager style. Keyboard-first, mouse-friendly.

| Shortcut | Action |
|---|---|
| **\|** | Split the focused pane into a left \| right pair |
| **-** | Split the focused pane into a top / bottom pair |
| **Ctrl + W** | Close the focused pane (refuses the last one; collapses to its sibling) |
| **Tab** | Cycle focus across all panes |
| **Ctrl + ← ↑ ↓ →** | Resize the focused pane |

### Mouse

Mouse works everywhere, while the keyboard stays first-class:

- **Click** a pane to focus it and select the item under the cursor; **click the
  already-selected item** to open it (enter directory / open file).
- **Wheel** scrolls the selection in the pane under the cursor.
- **Drag a seam** between panes to resize the split live.
- **Click the header row** to open the menu bar.

---

## 🐚 Shell Integration (Auto-cd on Exit)

To make `rc` automatically change your terminal's working directory (`cd`) to the last visited folder upon exit, add the following wrapper function to your shell configuration:

### Bash / Zsh
Add this to your `~/.bashrc` or `~/.zshrc`:
```bash
function rc() {
    local tmp="$(mktemp -t rc-cwd.XXXXXX)"
    command rc --write-last-dir="$tmp" "$@"
    if [ -f "$tmp" ]; then
        local dir="$(cat "$tmp")"
        rm -f "$tmp"
        if [ -d "$dir" ] && [ "$dir" != "$(pwd)" ]; then
            cd "$dir"
        fi
    fi
}
```

### Fish
Add this to `~/.config/fish/functions/rc.fish`:
```fish
function rc
    set tmp (mktemp -t rc-cwd.XXXXXX)
    command rc --write-last-dir=$tmp $argv
    if test -f "$tmp"
        set cl_dir (cat $tmp)
        rm -f $tmp
        if test -d "$cl_dir"; and test "$cl_dir" != (pwd)
            cd $cl_dir
        end
    end
end
```

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

## 🔍 External CLI Tools Integration

`rc` provides first-class integrations with popular CLI search and navigation tools. These commands run interactively in the foreground and immediately apply their selection back to the file manager:

*   **Fuzzy Find Files (`Ctrl+F`)**: Recursively finds files in the active directory using `fd` (or fallbacks) and `fzf`. Selecting a file highlights and positions the cursor on it in `rc`.
*   **Live Grep Text (`Ctrl+G`)**: Greps files recursively in the active directory using `rg` and `fzf`. Selecting a search result navigates to and highlights the target file.
*   **Zoxide Jump (`Ctrl+J`)**: Opens interactive database navigation using `zoxide query -i`. Selecting a directory jumps the active panel to that folder.

> [!NOTE]
> Make sure `fzf`, `fd`/`find`, `rg`, and/or `zoxide` are installed and available in your shell's `PATH`.

---

## 🎨 Native Previews & Rendering

*   **Syntax Highlighting**: Code and config files are natively highlighted inside the preview window using `syntect` with a pure-Rust background parser (compiled using `regex-fancy` for build compatibility). Highlighting states are lazy-loaded to ensure zero application startup latency.
*   **Pixel-Perfect Image Preview**: Renders PNG, JPG, GIF, WebP, and other formats directly inside the terminal preview panel using `ratatui-image`. Automatically uses the best available protocol for your terminal (Kitty Graphics Protocol, Sixel, or half-blocks fallback).
*   **Virtual Scroll Rendering**: Snappy performance even in directories with 10k+ files. `rc` uses viewport-based list virtualization so only visible files are formatted, styled, canonicalized, or checked for Git statuses.
*   **Debounced File Watcher**: A built-in filesystem event watcher debounces multiple updates in quick succession, preventing rendering floods during batch operations.

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
