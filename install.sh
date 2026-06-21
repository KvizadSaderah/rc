#!/bin/bash

# Color definitions
INFO='\033[0;36m'
SUCCESS='\033[0;32m'
ERROR='\033[0;31m'
WARNING='\033[0;33m'
CLEAR='\033[0m'
BOLD='\033[1m'

echo -e "${INFO}${BOLD}====================================================${CLEAR}"
echo -e "${INFO}${BOLD}   Rust Commander (rc) - Installer Script           ${CLEAR}"
echo -e "${INFO}${BOLD}====================================================${CLEAR}"

# 1. Prepare installation folder (~/.local/bin)
INSTALL_DIR="${HOME}/.local/bin"
mkdir -p "$INSTALL_DIR"

# 2. Platform Detection
OS="$(uname -s)"

if [ "$OS" = "Darwin" ]; then
    echo -e "${INFO}Detected OS: macOS (Darwin). Installing precompiled binary...${CLEAR}"
    
    # Fetch Latest Release Information from GitHub API
    echo -e "${INFO}Fetching latest release info from GitHub...${CLEAR}"
    RELEASE_JSON=$(curl -s https://api.github.com/repos/KvizadSaderah/rc/releases/latest)

    # Extract download URL for macOS asset
    DOWNLOAD_URL=$(echo "$RELEASE_JSON" | grep -o -E 'https://github.com/KvizadSaderah/rc/releases/download/[^"]*rc-macos.tar.gz' | head -n 1)
    TAG_NAME=$(echo "$RELEASE_JSON" | grep -o -E '"tag_name": "[^"]*' | head -n 1 | cut -d'"' -f4)

    if [ -z "$DOWNLOAD_URL" ]; then
        echo -e "${ERROR}Error: Could not find a macOS release asset on GitHub.${CLEAR}"
        exit 1
    fi

    echo -e "${INFO}Downloading Rust Commander ${TAG_NAME}...${CLEAR}"
    TEMP_DIR=$(mktemp -d)
    TAR_PATH="${TEMP_DIR}/rc-macos.tar.gz"

    curl -L -s -o "$TAR_PATH" "$DOWNLOAD_URL"
    if [ $? -ne 0 ]; then
        echo -e "${ERROR}Error: Failed to download release asset.${CLEAR}"
        exit 1
    fi

    echo -e "${INFO}Extracting binary...${CLEAR}"
    tar -xzf "$TAR_PATH" -C "$TEMP_DIR"
    if [ ! -f "${TEMP_DIR}/rc" ]; then
        echo -e "${ERROR}Error: Failed to find the extracted 'rc' binary.${CLEAR}"
        exit 1
    fi

    cp "${TEMP_DIR}/rc" "${INSTALL_DIR}/rc"
    chmod +x "${INSTALL_DIR}/rc"
    rm -rf "$TEMP_DIR"

elif [ "$OS" = "Linux" ]; then
    echo -e "${INFO}Detected OS: Linux. Checking for Rust (cargo)...${CLEAR}"
    if command -v cargo >/dev/null 2>&1; then
        echo -e "${INFO}Cargo found! Compiling Rust Commander from source...${CLEAR}"
        cargo install --git https://github.com/KvizadSaderah/rc.git --root "${HOME}/.local"
        if [ $? -ne 0 ]; then
            echo -e "${ERROR}Error: Cargo compilation failed.${CLEAR}"
            exit 1
        fi
        TAG_NAME="(compiled from source)"
    else
        echo -e "${ERROR}Error: Precompiled binaries for Linux are coming soon.${CLEAR}"
        echo -e "To install now, please install Rust and Cargo first:"
        echo -e "  ${BOLD}curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh${CLEAR}"
        echo -e "Then re-run this script, or run:"
        echo -e "  ${BOLD}cargo install --git https://github.com/KvizadSaderah/rc.git --root ~/.local${CLEAR}"
        exit 1
    fi
else
    echo -e "${ERROR}Error: Unsupported platform '$OS'.${CLEAR}"
    exit 1
fi

# 3. Path Verification
echo -e "\n${SUCCESS}${BOLD}✓ Rust Commander (rc) ${TAG_NAME} installed to ${INSTALL_DIR}/rc!${CLEAR}"

if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
    echo -e "\n${WARNING}${BOLD}⚠️  Warning: ${INSTALL_DIR} is not in your PATH!${CLEAR}"
    echo -e "To be able to run ${BOLD}rc${CLEAR} from anywhere, add it to your profile:"
    
    # Detect shell profile
    SHELL_NAME=$(basename "$SHELL")
    if [ "$SHELL_NAME" = "zsh" ]; then
        echo -e "Run: ${BOLD}echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.zshrc && source ~/.zshrc${CLEAR}"
    elif [ "$SHELL_NAME" = "bash" ]; then
        echo -e "Run: ${BOLD}echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.bashrc && source ~/.bashrc${CLEAR}"
    else
        echo -e "Run: ${BOLD}export PATH=\"\$HOME/.local/bin:\$PATH\"${CLEAR} in your shell startup file."
    fi
else
    echo -e "Type ${BOLD}rc${CLEAR} to launch the file manager from anywhere."
fi
