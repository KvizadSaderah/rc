#!/usr/bin/env bash
set -euo pipefail

# =============================================================================
# Rust Commander — Interactive Release Script
# Usage: ./release.sh
# Requires: gh (GitHub CLI), cargo, git
# =============================================================================

INFO='\033[0;36m'
SUCCESS='\033[0;32m'
ERROR='\033[0;31m'
WARNING='\033[0;33m'
BOLD='\033[1m'
DIM='\033[2m'
RESET='\033[0m'

step()    { echo -e "\n${INFO}${BOLD}▶ $*${RESET}"; }
ok()      { echo -e "${SUCCESS}✓ $*${RESET}"; }
warn()    { echo -e "${WARNING}⚠ $*${RESET}"; }
die()     { echo -e "${ERROR}✗ $*${RESET}" >&2; exit 1; }
ask()     { echo -e "${BOLD}$*${RESET}"; }

# =============================================================================
# 0. Dependency Checks
# =============================================================================
step "Checking dependencies"

for cmd in git cargo gh; do
    if ! command -v "$cmd" &>/dev/null; then
        die "'$cmd' not found. Install it first."
    fi
    ok "$cmd: $(command -v "$cmd")"
done

# Must be in project root
CARGO_TOML="Cargo.toml"
[[ -f "$CARGO_TOML" ]] || die "Cargo.toml not found. Run from project root."

# gh auth check
if ! gh auth status &>/dev/null; then
    die "Not logged in to GitHub CLI. Run: gh auth login"
fi

# =============================================================================
# 1. Current State
# =============================================================================
step "Current state"

# Robust package version extraction (restricted to [package] section)
CURRENT_VERSION=$(awk -F '"' '/^\[package\]/{p=1;next}/^\[/{p=0}/^version *=/{if(p){print $2;exit}}' "$CARGO_TOML")
CURRENT_BRANCH=$(git rev-parse --abbrev-ref HEAD)
UNCOMMITTED=$(git status --porcelain)

echo -e "  Branch  : ${BOLD}${CURRENT_BRANCH}${RESET}"
echo -e "  Version : ${BOLD}v${CURRENT_VERSION}${RESET}"
echo -e "  Remote  : $(git remote get-url origin 2>/dev/null || echo 'none')"

if [[ -n "$UNCOMMITTED" ]]; then
    warn "Uncommitted changes detected:"
    git status --short
    echo ""
    ask "Commit them now? [y/N]"
    read -r ANSWER
    if [[ "$ANSWER" =~ ^[Yy]$ ]]; then
        ask "Commit message (leave blank to abort):"
        read -r COMMIT_MSG
        [[ -z "$COMMIT_MSG" ]] && die "Aborted."
        git add -A
        git commit -m "$COMMIT_MSG"
        ok "Committed."
    else
        warn "Proceeding with uncommitted changes!"
        warn "WARNING: Your compiled release binary WILL include these uncommitted local changes,"
        warn "but those changes WILL NOT be included in the git source tag!"
        warn "This can result in your release binary and source code tag falling out of sync."
        echo ""
    fi
fi

# =============================================================================
# 2. Version Bump
# =============================================================================
step "Version bump"

IFS='.' read -r MAJOR MINOR PATCH <<< "$CURRENT_VERSION"

echo -e "  Current version: ${BOLD}v${CURRENT_VERSION}${RESET}"
echo ""
echo -e "  ${DIM}1)${RESET} patch  →  v${MAJOR}.${MINOR}.$((PATCH + 1))  ${DIM}(bug fixes)${RESET}"
echo -e "  ${DIM}2)${RESET} minor  →  v${MAJOR}.$((MINOR + 1)).0  ${DIM}(new features)${RESET}"
echo -e "  ${DIM}3)${RESET} major  →  v$((MAJOR + 1)).0.0  ${DIM}(breaking changes)${RESET}"
echo -e "  ${DIM}4)${RESET} custom  ${DIM}(enter manually)${RESET}"
echo -e "  ${DIM}5)${RESET} keep   →  v${CURRENT_VERSION}  ${DIM}(don't bump)${RESET}"
echo ""
ask "Choose bump type [1-5, default: 1]:"
read -r BUMP_CHOICE
BUMP_CHOICE="${BUMP_CHOICE:-1}"

case "$BUMP_CHOICE" in
    1) NEW_VERSION="${MAJOR}.${MINOR}.$((PATCH + 1))" ;;
    2) NEW_VERSION="${MAJOR}.$((MINOR + 1)).0" ;;
    3) NEW_VERSION="$((MAJOR + 1)).0.0" ;;
    4)
        ask "Enter new version (without 'v'):"
        read -r NEW_VERSION
        [[ -z "$NEW_VERSION" ]] && die "Aborted."
        # Validate format
        [[ "$NEW_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "Invalid version format. Use X.Y.Z"
        ;;
    5) NEW_VERSION="$CURRENT_VERSION" ;;
    *) die "Invalid choice." ;;
esac

echo ""
echo -e "  ${BOLD}v${CURRENT_VERSION}${RESET} → ${SUCCESS}${BOLD}v${NEW_VERSION}${RESET}"
ask "Confirm? [Y/n]"
read -r CONFIRM
[[ "${CONFIRM:-Y}" =~ ^[Nn]$ ]] && die "Aborted."

# =============================================================================
# 3. Release Notes
# =============================================================================
step "Release notes"

# Show commits since last tag
LAST_TAG=$(git describe --tags --abbrev=0 2>/dev/null || echo "")
if [[ -n "$LAST_TAG" ]]; then
    echo -e "  ${DIM}Commits since ${LAST_TAG}:${RESET}"
    git log "${LAST_TAG}..HEAD" --oneline | sed 's/^/    /'
    echo ""
fi

ask "Release title (leave blank to use default):"
read -r RELEASE_TITLE
if [[ -z "$RELEASE_TITLE" ]]; then
    RELEASE_TITLE="v${NEW_VERSION}"
fi

echo ""
echo -e "  ${DIM}Release notes (Enter to auto-generate from git log,${RESET}"
echo -e "  ${DIM}or type notes line by line and finish with '.'):${RESET}"
echo ""

NOTES_LINES=()
while IFS= read -r line; do
    [[ "$line" == "." ]] && break
    [[ -z "$line" ]] && break
    NOTES_LINES+=("$line")
done

if [[ ${#NOTES_LINES[@]} -eq 0 ]]; then
    # Auto-generate from git log
    if [[ -n "$LAST_TAG" ]]; then
        RELEASE_NOTES=$(git log "${LAST_TAG}..HEAD" --pretty=format:"- %s" | head -30)
    else
        RELEASE_NOTES=$(git log --pretty=format:"- %s" | head -30)
    fi
    RELEASE_NOTES="## What's Changed

${RELEASE_NOTES}"
    ok "Release notes auto-generated from git log."
else
    RELEASE_NOTES=$(printf '%s\n' "${NOTES_LINES[@]}")
fi

# =============================================================================
# 4. Summary & Final Confirmation
# =============================================================================
step "Release plan"

echo ""
echo -e "  Version   : ${BOLD}v${NEW_VERSION}${RESET}"
echo -e "  Title     : ${BOLD}${RELEASE_TITLE}${RESET}"
echo -e "  Tag       : ${BOLD}v${NEW_VERSION}${RESET}"
echo -e "  Branch    : ${BOLD}${CURRENT_BRANCH}${RESET}"
echo -e "  Target    : ${BOLD}$(git remote get-url origin)${RESET}"
echo ""
echo -e "  ${DIM}Steps that will run:${RESET}"
echo -e "  ${DIM}  1. Bump Cargo.toml version${RESET}"
echo -e "  ${DIM}  2. cargo build --release${RESET}"
echo -e "  ${DIM}  3. git commit + push${RESET}"
echo -e "  ${DIM}  4. git tag + push tag${RESET}"
echo -e "  ${DIM}  5. Package binary → rc-macos.tar.gz${RESET}"
echo -e "  ${DIM}  6. gh release create with asset${RESET}"
echo ""
ask "🚀 Proceed? [Y/n]"
read -r GO
[[ "${GO:-Y}" =~ ^[Nn]$ ]] && die "Aborted."

# =============================================================================
# 5. Bump version in Cargo.toml
# =============================================================================
step "Bumping version in Cargo.toml"

if [[ "$NEW_VERSION" != "$CURRENT_VERSION" ]]; then
    # macOS-compatible sed
    if [[ "$(uname)" == "Darwin" ]]; then
        sed -i '' "s/^version = \"${CURRENT_VERSION}\"/version = \"${NEW_VERSION}\"/" "$CARGO_TOML"
    else
        sed -i "s/^version = \"${CURRENT_VERSION}\"/version = \"${NEW_VERSION}\"/" "$CARGO_TOML"
    fi
    ok "Cargo.toml → v${NEW_VERSION}"
else
    ok "Version unchanged (v${CURRENT_VERSION})"
fi

# =============================================================================
# 5.5 Run Unit Tests
# =============================================================================
step "Running unit tests (cargo test)"
if ! cargo test; then
    die "Tests failed. Refusing to release broken build."
fi
ok "All tests passed successfully."

# =============================================================================
# 6. Build release binary
# =============================================================================
step "Building release binary (cargo build --release)"

cargo build --release
ok "Build complete: target/release/rc"

# =============================================================================
# 7. Git commit, push
# =============================================================================
step "Committing & pushing"

if [[ "$NEW_VERSION" != "$CURRENT_VERSION" ]]; then
    git add Cargo.toml Cargo.lock
    git commit -m "v${NEW_VERSION} — release"
    ok "Committed version bump."
fi

# Pull rebase to avoid rejection if behind
git pull --rebase --autostash origin "$CURRENT_BRANCH" 2>/dev/null || true

git push origin "$CURRENT_BRANCH"
ok "Pushed to origin/${CURRENT_BRANCH}"

# =============================================================================
# 8. Tag
# =============================================================================
step "Tagging v${NEW_VERSION}"

TAG="v${NEW_VERSION}"

TAG_EXISTS_LOCAL=$(git rev-parse "$TAG" &>/dev/null && echo "true" || echo "false")
TAG_EXISTS_REMOTE=$(git ls-remote --tags origin "$TAG" 2>/dev/null | grep -q "refs/tags/${TAG}" && echo "true" || echo "false")

if [[ "$TAG_EXISTS_LOCAL" == "true" || "$TAG_EXISTS_REMOTE" == "true" ]]; then
    warn "Tag ${TAG} already exists (local: ${TAG_EXISTS_LOCAL}, remote: ${TAG_EXISTS_REMOTE})."
    ask "Delete and re-create it locally and on remote? [y/N]"
    read -r DEL_TAG
    if [[ "$DEL_TAG" =~ ^[Yy]$ ]]; then
        if [[ "$TAG_EXISTS_LOCAL" == "true" ]]; then
            git tag -d "$TAG"
        fi
        git push origin ":refs/tags/${TAG}" 2>/dev/null || true
        ok "Deleted existing tag ${TAG} locally and on origin remote."
    else
        die "Aborted. Tag already exists."
    fi
fi

git tag -a "$TAG" -m "${RELEASE_TITLE}"
git push origin "$TAG"
ok "Tag ${TAG} pushed."

# =============================================================================
# 9. Package binary
# =============================================================================
step "Packaging binary"

DIST_DIR="$(mktemp -d)"
BIN_SRC="target/release/rc"

[[ -f "$BIN_SRC" ]] || die "Binary not found at ${BIN_SRC}"

cp "$BIN_SRC" "${DIST_DIR}/rc"
chmod +x "${DIST_DIR}/rc"

OS_TYPE=$(uname -s | tr '[:upper:]' '[:lower:]')
case "$OS_TYPE" in
    darwin) ARCHIVE="rc-macos.tar.gz" ;;
    linux) ARCHIVE="rc-linux.tar.gz" ;;
    *) ARCHIVE="rc-${OS_TYPE}.tar.gz" ;;
esac
tar -czf "${ARCHIVE}" -C "${DIST_DIR}" rc
rm -rf "${DIST_DIR}"
ok "Packaged: ${ARCHIVE} ($(du -sh "$ARCHIVE" | cut -f1))"

# =============================================================================
# 10. GitHub Release
# =============================================================================
step "Creating GitHub Release"

gh release create "$TAG" \
    --title "${RELEASE_TITLE}" \
    --notes "${RELEASE_NOTES}" \
    "${ARCHIVE}"

ok "Release created: $(gh release view "$TAG" --json url -q .url)"

# Cleanup
rm -f "${ARCHIVE}"

# =============================================================================
# Done
# =============================================================================
echo ""
echo -e "${SUCCESS}${BOLD}════════════════════════════════════════${RESET}"
echo -e "${SUCCESS}${BOLD}  🎉 Released v${NEW_VERSION} successfully!${RESET}"
echo -e "${SUCCESS}${BOLD}════════════════════════════════════════${RESET}"
echo ""
echo -e "  View release : ${BOLD}gh release view ${TAG} --web${RESET}"
echo -e "  Install cmd  : ${BOLD}curl -fsSL https://raw.githubusercontent.com/KvizadSaderah/rc/master/install.sh | bash${RESET}"
echo ""
