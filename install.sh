#!/usr/bin/env bash
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# NodeRunner: Mainnet Protocol — Installer (Linux / macOS)
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
#
# Usage:
#   ./install.sh              Build + install (default features)
#   ./install.sh --minimal    Build without gamepad/sound
#   ./install.sh --uninstall  Remove installed files
#   ./install.sh --help       Show this help
#
# Install locations:
#   Data:   ~/.local/share/noderunner/
#   Binary: ~/.local/bin/noderunner (symlink)
#
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

set -euo pipefail

# ── Config ──
APP_NAME="noderunner"
APP_DISPLAY="NodeRunner: Mainnet Protocol"
VERSION="0.3.2"

DATA_DIR="${HOME}/.local/share/${APP_NAME}"
BIN_DIR="${HOME}/.local/bin"
BIN_LINK="${BIN_DIR}/${APP_NAME}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ── Colors ──
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

info()  { printf "${CYAN}[INFO]${NC}  %s\n" "$1"; }
ok()    { printf "${GREEN}[OK]${NC}    %s\n" "$1"; }
warn()  { printf "${YELLOW}[WARN]${NC}  %s\n" "$1"; }
error() { printf "${RED}[ERROR]${NC} %s\n" "$1"; }

# ── Banner ──
banner() {
    printf "\n${BOLD}${CYAN}"
    echo "  ┌─────────────────────────────────────────┐"
    echo "  │  NodeRunner: Mainnet Protocol  v${VERSION}   │"
    echo "  │  Terminal Action Puzzle                  │"
    echo "  └─────────────────────────────────────────┘"
    printf "${NC}\n"
}

# ── Help ──
show_help() {
    banner
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  (none)       Build and install with all features (gamepad + sound)"
    echo "  --minimal    Build without gamepad and sound support"
    echo "  --uninstall  Remove all installed files"
    echo "  --help       Show this help message"
    echo ""
    echo "Install locations:"
    echo "  Data:    ${DATA_DIR}/"
    echo "  Binary:  ${BIN_LINK}"
    echo ""
    echo "Requirements:"
    echo "  - Rust toolchain (rustc + cargo)  https://rustup.rs"
    echo "  - Linux: libudev-dev, libasound2-dev (for gamepad/sound features)"
    echo "  - macOS: Xcode command line tools"
    echo ""
}

# ── Uninstall ──
do_uninstall() {
    banner
    info "Uninstalling ${APP_DISPLAY}..."

    if [ -L "${BIN_LINK}" ]; then
        rm -f "${BIN_LINK}"
        ok "Removed symlink ${BIN_LINK}"
    fi

    if [ -d "${DATA_DIR}" ]; then
        # Preserve save files
        local saves=()
        for f in "${DATA_DIR}"/save*.dat; do
            [ -f "$f" ] && saves+=("$f")
        done

        if [ ${#saves[@]} -gt 0 ]; then
            warn "Save files found:"
            for f in "${saves[@]}"; do
                echo "        $(basename "$f")"
            done
            printf "  Delete save files too? [y/N] "
            read -r ans
            if [[ "$ans" =~ ^[Yy] ]]; then
                rm -rf "${DATA_DIR}"
                ok "Removed ${DATA_DIR} (including saves)"
            else
                # Remove everything except saves
                find "${DATA_DIR}" -maxdepth 1 -not -name 'save*.dat' -not -name "$(basename "${DATA_DIR}")" -exec rm -rf {} +
                # Remove levels/ and packs/ dirs
                rm -rf "${DATA_DIR}/levels" "${DATA_DIR}/packs"
                ok "Removed ${DATA_DIR} (save files preserved)"
            fi
        else
            rm -rf "${DATA_DIR}"
            ok "Removed ${DATA_DIR}"
        fi
    else
        info "Nothing to remove — not installed."
    fi

    echo ""
    ok "Uninstall complete."
    exit 0
}

# ── Dependency check ──
check_deps() {
    # Rust
    if ! command -v cargo &>/dev/null; then
        error "cargo not found. Install Rust: https://rustup.rs"
        echo ""
        echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
        echo ""
        exit 1
    fi
    ok "cargo $(cargo --version | awk '{print $2}')"

    if ! command -v rustc &>/dev/null; then
        error "rustc not found."
        exit 1
    fi
    ok "rustc $(rustc --version | awk '{print $2}')"

    # Platform-specific deps (Linux only)
    if [[ "$(uname -s)" == "Linux" ]]; then
        local missing=()

        # libudev (for gilrs/gamepad)
        if [ "$FEATURES" != "minimal" ]; then
            if ! pkg-config --exists libudev 2>/dev/null; then
                if ! dpkg -s libudev-dev &>/dev/null 2>&1; then
                    missing+=("libudev-dev")
                fi
            fi
            # libasound (for rodio/sound)
            if ! pkg-config --exists alsa 2>/dev/null; then
                if ! dpkg -s libasound2-dev &>/dev/null 2>&1; then
                    missing+=("libasound2-dev")
                fi
            fi
        fi

        if [ ${#missing[@]} -gt 0 ]; then
            warn "Missing system libraries: ${missing[*]}"
            echo ""
            echo "  Install with:"
            echo "    sudo apt install ${missing[*]}"
            echo ""
            echo "  Or build without these features:"
            echo "    $0 --minimal"
            echo ""
            exit 1
        fi
    fi
}

# ── Build ──
do_build() {
    info "Building ${APP_DISPLAY} (${BUILD_MODE})..."
    echo ""

    cd "${SCRIPT_DIR}"

    local cargo_args=("build" "--release")

    if [ "$FEATURES" = "minimal" ]; then
        cargo_args+=("--no-default-features")
        info "Features: none (minimal build)"
    else
        info "Features: gamepad + sound"
    fi

    echo ""
    if cargo "${cargo_args[@]}"; then
        echo ""
        ok "Build successful!"
    else
        echo ""
        error "Build failed. Check errors above."
        exit 1
    fi
}

# ── Install ──
do_install() {
    info "Installing to ${DATA_DIR}/..."

    # Create directories
    mkdir -p "${DATA_DIR}"
    mkdir -p "${BIN_DIR}"

    # Copy binary
    local binary="${SCRIPT_DIR}/target/release/${APP_NAME}"
    if [ ! -f "${binary}" ]; then
        error "Binary not found: ${binary}"
        error "Build may have failed."
        exit 1
    fi
    cp "${binary}" "${DATA_DIR}/${APP_NAME}"
    chmod +x "${DATA_DIR}/${APP_NAME}"
    ok "Binary installed"

    # Copy config (don't overwrite existing)
    if [ -f "${SCRIPT_DIR}/config.toml" ]; then
        if [ -f "${DATA_DIR}/config.toml" ]; then
            warn "config.toml already exists — keeping existing"
            cp "${SCRIPT_DIR}/config.toml" "${DATA_DIR}/config.toml.new"
            info "New default saved as config.toml.new"
        else
            cp "${SCRIPT_DIR}/config.toml" "${DATA_DIR}/config.toml"
            ok "config.toml installed"
        fi
    fi

    # Copy levels
    if [ -d "${SCRIPT_DIR}/levels" ]; then
        mkdir -p "${DATA_DIR}/levels"
        cp -r "${SCRIPT_DIR}/levels/"*.txt "${DATA_DIR}/levels/" 2>/dev/null || true
        local count
        count=$(ls "${DATA_DIR}/levels/"*.txt 2>/dev/null | wc -l)
        ok "Levels installed (${count} files)"
    fi

    # Copy packs
    if [ -d "${SCRIPT_DIR}/packs" ]; then
        mkdir -p "${DATA_DIR}/packs"
        cp -r "${SCRIPT_DIR}/packs/"*.nlp "${DATA_DIR}/packs/" 2>/dev/null || true
        local pcount
        pcount=$(ls "${DATA_DIR}/packs/"*.nlp 2>/dev/null | wc -l)
        ok "Level packs installed (${pcount} files)"
    fi

    # Create symlink
    ln -sf "${DATA_DIR}/${APP_NAME}" "${BIN_LINK}"
    ok "Symlink created: ${BIN_LINK}"

    # Verify PATH
    if ! echo "$PATH" | tr ':' '\n' | grep -q "^${BIN_DIR}$"; then
        warn "${BIN_DIR} is not in your PATH"
        echo ""
        echo "  Add to your shell config (~/.bashrc or ~/.zshrc):"
        echo "    export PATH=\"\$HOME/.local/bin:\$PATH\""
        echo ""
    fi

    # Linux: create desktop entry
    if [[ "$(uname -s)" == "Linux" ]]; then
        create_desktop_entry
    fi
}

# ── Desktop Entry (Linux) ──
create_desktop_entry() {
    local desktop_dir="${HOME}/.local/share/applications"
    local desktop_file="${desktop_dir}/${APP_NAME}.desktop"
    mkdir -p "${desktop_dir}"

    cat > "${desktop_file}" << EOF
[Desktop Entry]
Type=Application
Name=${APP_DISPLAY}
Comment=Terminal-based action puzzle game
Exec=${BIN_LINK}
Terminal=true
Categories=Game;
Keywords=loderunner;puzzle;terminal;
EOF
    ok "Desktop entry created"
}

# ── Summary ──
show_summary() {
    echo ""
    printf "${BOLD}${GREEN}"
    echo "  ╔═════════════════════════════════════════╗"
    echo "  ║       Installation Complete!            ║"
    echo "  ╚═════════════════════════════════════════╝"
    printf "${NC}\n"

    echo "  Run the game:"
    echo ""
    printf "    ${BOLD}${APP_NAME}${NC}\n"
    echo ""
    echo "  Install location:  ${DATA_DIR}/"
    echo "  Binary:            ${BIN_LINK}"
    echo "  Config:            ${DATA_DIR}/config.toml"
    echo "  Levels:            ${DATA_DIR}/levels/"
    echo "  Level packs:       ${DATA_DIR}/packs/"
    echo "  Save data:         ${DATA_DIR}/save_*.dat"
    echo ""
    echo "  Uninstall:"
    echo "    $0 --uninstall"
    echo ""
}

# ── Main ──
main() {
    FEATURES="default"
    BUILD_MODE="release (all features)"

    case "${1:-}" in
        --help|-h)
            show_help
            exit 0
            ;;
        --uninstall)
            do_uninstall
            ;;
        --minimal)
            FEATURES="minimal"
            BUILD_MODE="release (minimal — no gamepad/sound)"
            ;;
        "")
            ;;  # default
        *)
            error "Unknown option: $1"
            show_help
            exit 1
            ;;
    esac

    banner
    check_deps
    do_build
    do_install
    show_summary
}

main "$@"
