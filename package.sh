#!/usr/bin/env bash
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# NodeRunner: Mainnet Protocol — Package Builder (deb / rpm)
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
#
# Usage:
#   ./package.sh              Build both .deb and .rpm
#   ./package.sh deb          Build .deb only
#   ./package.sh rpm          Build .rpm only
#   ./package.sh --minimal    Build without gamepad/sound
#   ./package.sh --help       Show help
#
# Output:
#   dist/noderunner_0.3.1-1_amd64.deb
#   dist/noderunner-0.3.1-1.x86_64.rpm
#
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

set -euo pipefail

# ── Package Metadata ──
PKG_NAME="noderunner"
PKG_VERSION="0.3.1"
PKG_RELEASE="1"
PKG_SUMMARY="NodeRunner: Mainnet Protocol — terminal-based action puzzle"
PKG_DESCRIPTION="A Lode Runner-inspired terminal game written in Rust.
Collect all tokens (gold) while avoiding sentinels (guards),
hack through firewalls (dig bricks), and escape to the top.
Features BFS-based AI, gamepad support, and procedural sound."
PKG_URL="https://github.com/example/noderunner"
PKG_LICENSE="MIT"
PKG_MAINTAINER="NodeRunner Team <noderunner@example.com>"
PKG_SECTION="games"
PKG_ARCH="amd64"
RPM_ARCH="x86_64"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DIST_DIR="${SCRIPT_DIR}/dist"
BUILD_ROOT="${SCRIPT_DIR}/build/pkg"

# ── Colors ──
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
CYAN='\033[0;36m'; BOLD='\033[1m'; NC='\033[0m'

info()  { printf "${CYAN}[INFO]${NC}  %s\n" "$1"; }
ok()    { printf "${GREEN}[OK]${NC}    %s\n" "$1"; }
warn()  { printf "${YELLOW}[WARN]${NC}  %s\n" "$1"; }
error() { printf "${RED}[ERROR]${NC} %s\n" "$1" >&2; }

# ── Help ──
show_help() {
    echo ""
    echo "Usage: $0 [deb|rpm] [OPTIONS]"
    echo ""
    echo "Targets:"
    echo "  (none)    Build both .deb and .rpm"
    echo "  deb       Build .deb package only"
    echo "  rpm       Build .rpm package only"
    echo ""
    echo "Options:"
    echo "  --minimal     Build without gamepad/sound dependencies"
    echo "  --no-build    Skip cargo build (use existing binary)"
    echo "  --help        Show this help"
    echo ""
    echo "Requirements:"
    echo "  - Rust toolchain (cargo, rustc)"
    echo "  - dpkg-deb (for .deb) — apt install dpkg"
    echo "  - rpmbuild (for .rpm) — apt install rpm  /  dnf install rpm-build"
    echo ""
    echo "Output:"
    echo "  dist/${PKG_NAME}_${PKG_VERSION}-${PKG_RELEASE}_${PKG_ARCH}.deb"
    echo "  dist/${PKG_NAME}-${PKG_VERSION}-${PKG_RELEASE}.${RPM_ARCH}.rpm"
    echo ""
    exit 0
}

# ── Parse Args ──
TARGET="all"
FEATURES="default"
NO_BUILD=false

for arg in "$@"; do
    case "$arg" in
        deb)        TARGET="deb" ;;
        rpm)        TARGET="rpm" ;;
        --minimal)  FEATURES="minimal" ;;
        --no-build) NO_BUILD=true ;;
        --help|-h)  show_help ;;
        *) error "Unknown option: $arg"; show_help ;;
    esac
done

# ── Detect Architecture ──
detect_arch() {
    local machine
    machine="$(uname -m)"
    case "$machine" in
        x86_64)  PKG_ARCH="amd64"; RPM_ARCH="x86_64" ;;
        aarch64) PKG_ARCH="arm64"; RPM_ARCH="aarch64" ;;
        armv7l)  PKG_ARCH="armhf"; RPM_ARCH="armv7hl" ;;
        *) warn "Unknown arch: $machine — using $machine"
           PKG_ARCH="$machine"; RPM_ARCH="$machine" ;;
    esac
}

# ── Check Tools ──
check_tools() {
    if ! $NO_BUILD; then
        if ! command -v cargo &>/dev/null; then
            error "cargo not found. Install Rust: https://rustup.rs"
            exit 1
        fi
        ok "cargo $(cargo --version | awk '{print $2}')"
    fi

    if [[ "$TARGET" == "deb" || "$TARGET" == "all" ]]; then
        if ! command -v dpkg-deb &>/dev/null; then
            error "dpkg-deb not found."
            echo "  Ubuntu/Debian:  sudo apt install dpkg"
            echo "  Fedora/RHEL:    sudo dnf install dpkg"
            exit 1
        fi
        ok "dpkg-deb found"
    fi

    if [[ "$TARGET" == "rpm" || "$TARGET" == "all" ]]; then
        if ! command -v rpmbuild &>/dev/null; then
            error "rpmbuild not found."
            echo "  Ubuntu/Debian:  sudo apt install rpm"
            echo "  Fedora/RHEL:    sudo dnf install rpm-build"
            exit 1
        fi
        ok "rpmbuild found"
    fi
}

# ── Build Binary ──
do_build() {
    if $NO_BUILD; then
        info "Skipping build (--no-build)"
        return
    fi

    info "Building ${PKG_NAME} (release)..."
    cd "${SCRIPT_DIR}"

    local cargo_args=("build" "--release")
    if [[ "$FEATURES" == "minimal" ]]; then
        cargo_args+=("--no-default-features")
        info "Features: minimal (no gamepad/sound)"
    else
        info "Features: gamepad + sound"
    fi

    cargo "${cargo_args[@]}"
    ok "Build complete"

    local binary="${SCRIPT_DIR}/target/release/${PKG_NAME}"
    if [[ ! -f "$binary" ]]; then
        error "Binary not found: $binary"
        exit 1
    fi
    local size
    size=$(du -h "$binary" | awk '{print $1}')
    ok "Binary size: ${size}"
}

# ── Prepare staging tree (FHS layout) ──
#
#   /usr/games/noderunner          — binary
#   /usr/share/noderunner/         — data root
#   /usr/share/noderunner/config.toml
#   /usr/share/noderunner/levels/  — level files
#   /usr/share/noderunner/packs/   — level pack files
#   /usr/share/doc/noderunner/     — documentation
#   /usr/share/applications/       — .desktop entry
#   /usr/share/man/man6/           — man page
#
prepare_staging() {
    local root="$1"

    info "Preparing staging tree..."
    rm -rf "$root"

    # Directories
    mkdir -p "${root}/usr/games"
    mkdir -p "${root}/usr/share/${PKG_NAME}/levels"
    mkdir -p "${root}/usr/share/${PKG_NAME}/packs"
    mkdir -p "${root}/usr/share/doc/${PKG_NAME}"
    mkdir -p "${root}/usr/share/applications"
    mkdir -p "${root}/usr/share/man/man6"

    # Binary
    install -m 0755 "${SCRIPT_DIR}/target/release/${PKG_NAME}" "${root}/usr/games/${PKG_NAME}"
    ok "Binary staged"

    # Strip binary
    if command -v strip &>/dev/null; then
        strip "${root}/usr/games/${PKG_NAME}"
        ok "Binary stripped"
    fi

    # Config (system-wide default)
    if [[ -f "${SCRIPT_DIR}/config.toml" ]]; then
        install -m 0644 "${SCRIPT_DIR}/config.toml" "${root}/usr/share/${PKG_NAME}/config.toml"
        ok "config.toml staged"
    fi

    # Levels
    local level_count=0
    if [[ -d "${SCRIPT_DIR}/levels" ]]; then
        for f in "${SCRIPT_DIR}/levels/"*.txt; do
            [[ -f "$f" ]] || continue
            install -m 0644 "$f" "${root}/usr/share/${PKG_NAME}/levels/"
            level_count=$((level_count + 1))
        done
        ok "Levels staged (${level_count} files)"
    fi

    # Packs
    local pack_count=0
    if [[ -d "${SCRIPT_DIR}/packs" ]]; then
        for f in "${SCRIPT_DIR}/packs/"*.nlp; do
            [[ -f "$f" ]] || continue
            install -m 0644 "$f" "${root}/usr/share/${PKG_NAME}/packs/"
            pack_count=$((pack_count + 1))
        done
        ok "Level packs staged (${pack_count} files)"
    fi

    # Documentation
    if [[ -f "${SCRIPT_DIR}/README.md" ]]; then
        install -m 0644 "${SCRIPT_DIR}/README.md" "${root}/usr/share/doc/${PKG_NAME}/"
    fi

    # Desktop entry
    cat > "${root}/usr/share/applications/${PKG_NAME}.desktop" << 'DESKTOP'
[Desktop Entry]
Type=Application
Name=NodeRunner: Mainnet Protocol
GenericName=Action Puzzle Game
Comment=Terminal-based Lode Runner game written in Rust
Exec=noderunner
Terminal=true
Categories=Game;ActionGame;
Keywords=loderunner;puzzle;terminal;retro;
DESKTOP
    ok "Desktop entry staged"

    # Man page
    create_manpage "${root}/usr/share/man/man6/${PKG_NAME}.6"
    if command -v gzip &>/dev/null; then
        gzip -9 "${root}/usr/share/man/man6/${PKG_NAME}.6"
    fi
    ok "Man page staged"
}

# ── Generate man page ──
create_manpage() {
    local dst="$1"
    cat > "$dst" << 'MANPAGE'
.TH NODERUNNER 6 "2026" "0.3.1" "Games"
.SH NAME
noderunner \- terminal-based action puzzle game
.SH SYNOPSIS
.B noderunner
.SH DESCRIPTION
NodeRunner: Mainnet Protocol is a Lode Runner\-inspired terminal game.
Collect all tokens while avoiding sentinels, hack through firewalls,
and escape to the top of each node.
.PP
Features BFS\-based guard AI, gamepad support (via gilrs), and
procedural sound effects (via rodio).
.SH CONTROLS
.TP
.B Arrow keys / WASD
Move player
.TP
.B Z / Q
Hack left (dig)
.TP
.B X / E
Hack right (dig)
.TP
.B F1
Pause
.TP
.B F2
Restart level
.TP
.B F3
Level packs
.TP
.B F5\-F8
Save to slot 1\-4
.TP
.B F9\-F12
Load from slot 1\-4
.TP
.B ESC
Back / quit
.SH FILES
.TP
.I /usr/share/noderunner/config.toml
System\-wide configuration
.TP
.I /usr/share/noderunner/levels/
Level data files
.TP
.I /usr/share/noderunner/packs/
Level pack files (.nlp)
.TP
.I ~/.local/share/noderunner/
User save data and custom levels/packs
.SH ENVIRONMENT
The game searches for data in: executable directory, current directory,
~/.local/share/noderunner/, and /usr/share/noderunner/ (in that order).
.SH AUTHOR
NodeRunner Team
MANPAGE
}

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# DEB Package
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

build_deb() {
    info "Building .deb package..."

    local deb_root="${BUILD_ROOT}/deb"
    prepare_staging "$deb_root"

    # DEBIAN control directory
    mkdir -p "${deb_root}/DEBIAN"

    # Calculate installed size (KB)
    local installed_size
    installed_size=$(du -sk "${deb_root}" | awk '{print $1}')

    # Dependencies
    local depends=""
    if [[ "$FEATURES" == "default" ]]; then
        depends="libc6 (>= 2.31), libudev1, libasound2"
    else
        depends="libc6 (>= 2.31)"
    fi

    # control file
    cat > "${deb_root}/DEBIAN/control" << EOF
Package: ${PKG_NAME}
Version: ${PKG_VERSION}-${PKG_RELEASE}
Section: ${PKG_SECTION}
Priority: optional
Architecture: ${PKG_ARCH}
Depends: ${depends}
Installed-Size: ${installed_size}
Maintainer: ${PKG_MAINTAINER}
Homepage: ${PKG_URL}
Description: ${PKG_SUMMARY}
$(echo "$PKG_DESCRIPTION" | sed 's/^/ /')
EOF

    # conffiles — mark config.toml as conffile so upgrades don't overwrite
    cat > "${deb_root}/DEBIAN/conffiles" << EOF
/usr/share/${PKG_NAME}/config.toml
EOF

    # postinst — create /usr/bin symlink
    cat > "${deb_root}/DEBIAN/postinst" << 'EOF'
#!/bin/sh
set -e
ln -sf /usr/games/noderunner /usr/bin/noderunner 2>/dev/null || true
EOF
    chmod 0755 "${deb_root}/DEBIAN/postinst"

    # postrm — remove symlink on purge
    cat > "${deb_root}/DEBIAN/postrm" << 'EOF'
#!/bin/sh
set -e
if [ "$1" = "remove" ] || [ "$1" = "purge" ]; then
    rm -f /usr/bin/noderunner
fi
EOF
    chmod 0755 "${deb_root}/DEBIAN/postrm"

    # Fix permissions
    find "${deb_root}" -type d -exec chmod 0755 {} +
    find "${deb_root}/usr" -type f -exec chmod 0644 {} +
    chmod 0755 "${deb_root}/usr/games/${PKG_NAME}"

    # Build .deb
    mkdir -p "${DIST_DIR}"
    local deb_file="${DIST_DIR}/${PKG_NAME}_${PKG_VERSION}-${PKG_RELEASE}_${PKG_ARCH}.deb"
    dpkg-deb --build --root-owner-group "${deb_root}" "${deb_file}"

    local deb_size
    deb_size=$(du -h "${deb_file}" | awk '{print $1}')
    ok "Created: ${deb_file} (${deb_size})"
}

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# RPM Package
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

build_rpm() {
    info "Building .rpm package..."

    local rpm_root="${BUILD_ROOT}/rpm"

    # RPM build tree
    mkdir -p "${rpm_root}"/{SPECS,RPMS,SRPMS,BUILD,SOURCES}

    # Stage files into SOURCES (rpmbuild won't touch this)
    local staging="${rpm_root}/SOURCES/staged"
    prepare_staging "${staging}"

    # Create a local RPM database (avoids /var/lib/rpm permission issues)
    local rpm_dbpath="${rpm_root}/rpmdb"
    mkdir -p "${rpm_dbpath}"

    # Dependencies
    local requires=""
    if [[ "$FEATURES" == "default" ]]; then
        requires=$'Requires: glibc, systemd-libs, alsa-lib'
    else
        requires=$'Requires: glibc'
    fi

    # spec file — %install copies from SOURCES/staged into buildroot
    cat > "${rpm_root}/SPECS/${PKG_NAME}.spec" << SPEC
Name:           ${PKG_NAME}
Version:        ${PKG_VERSION}
Release:        ${PKG_RELEASE}%{?dist}
Summary:        ${PKG_SUMMARY}
License:        ${PKG_LICENSE}
URL:            ${PKG_URL}
${requires}

# Disable debug package, stripping, and build-id (already stripped)
%global debug_package %{nil}
%define __strip /bin/true
%define _build_id_links none

# Disable dependency auto-detection (avoids rpmdb access)
AutoReqProv: no

%description
${PKG_DESCRIPTION}

%build

%install
# Copy staged files into buildroot (rpmbuild cleans buildroot before this)
cp -a %{_sourcedir}/staged/* %{buildroot}/

%post
ln -sf /usr/games/noderunner /usr/bin/noderunner 2>/dev/null || true

%postun
if [ "\$1" = "0" ]; then
    rm -f /usr/bin/noderunner
fi

%files
%attr(0755,root,root) /usr/games/${PKG_NAME}
%config(noreplace) /usr/share/${PKG_NAME}/config.toml
/usr/share/${PKG_NAME}/levels/
/usr/share/${PKG_NAME}/packs/
/usr/share/doc/${PKG_NAME}/
/usr/share/applications/${PKG_NAME}.desktop
/usr/share/man/man6/${PKG_NAME}.6.gz

%changelog
* $(date +'%a %b %d %Y') ${PKG_MAINTAINER} - ${PKG_VERSION}-${PKG_RELEASE}
- Initial package
SPEC

    # Build RPM
    mkdir -p "${DIST_DIR}"
    rpmbuild \
        --define "_topdir ${rpm_root}" \
        --define "_builddir ${rpm_root}/BUILD" \
        --define "_rpmdir ${rpm_root}/RPMS" \
        --define "_srcrpmdir ${rpm_root}/SRPMS" \
        --define "_specdir ${rpm_root}/SPECS" \
        --define "_sourcedir ${rpm_root}/SOURCES" \
        --define "_dbpath ${rpm_dbpath}" \
        -bb "${rpm_root}/SPECS/${PKG_NAME}.spec"

    # Find and copy the output RPM
    local rpm_file
    rpm_file=$(find "${rpm_root}/RPMS" -name "*.rpm" -type f | head -1)
    if [[ -n "$rpm_file" ]]; then
        local final_rpm="${DIST_DIR}/${PKG_NAME}-${PKG_VERSION}-${PKG_RELEASE}.${RPM_ARCH}.rpm"
        cp "$rpm_file" "$final_rpm"
        local rpm_size
        rpm_size=$(du -h "$final_rpm" | awk '{print $1}')
        ok "Created: ${final_rpm} (${rpm_size})"
    else
        error "RPM file not found in ${rpm_root}/RPMS/"
        return 1
    fi
}

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# Main
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

main() {
    echo ""
    printf "${BOLD}${CYAN}"
    echo "  ┌──────────────────────────────────────────┐"
    echo "  │  NodeRunner Package Builder  v${PKG_VERSION}       │"
    echo "  └──────────────────────────────────────────┘"
    printf "${NC}\n"

    detect_arch
    info "Architecture: ${PKG_ARCH} / ${RPM_ARCH}"
    info "Target: ${TARGET}"
    info "Features: ${FEATURES}"
    echo ""

    check_tools
    echo ""

    do_build
    echo ""

    local built=()

    if [[ "$TARGET" == "deb" || "$TARGET" == "all" ]]; then
        build_deb
        built+=("deb")
        echo ""
    fi

    if [[ "$TARGET" == "rpm" || "$TARGET" == "all" ]]; then
        build_rpm
        built+=("rpm")
        echo ""
    fi

    # Cleanup
    rm -rf "${BUILD_ROOT}"

    # Summary
    printf "\n${BOLD}${GREEN}"
    echo "  ╔═════════════════════════════════════════╗"
    echo "  ║       Packaging Complete!               ║"
    echo "  ╚═════════════════════════════════════════╝"
    printf "${NC}\n"

    echo "  Output directory: ${DIST_DIR}/"
    echo ""
    ls -lh "${DIST_DIR}/"*.{deb,rpm} 2>/dev/null | awk '{print "    " $NF " (" $5 ")"}'
    echo ""

    if [[ " ${built[*]} " =~ " deb " ]]; then
        echo "  Install .deb:"
        echo "    sudo dpkg -i ${DIST_DIR}/${PKG_NAME}_${PKG_VERSION}-${PKG_RELEASE}_${PKG_ARCH}.deb"
        echo "    sudo apt install -f   # resolve dependencies"
        echo ""
    fi

    if [[ " ${built[*]} " =~ " rpm " ]]; then
        echo "  Install .rpm:"
        echo "    sudo rpm -i ${DIST_DIR}/${PKG_NAME}-${PKG_VERSION}-${PKG_RELEASE}.${RPM_ARCH}.rpm"
        echo "    # or: sudo dnf install ${DIST_DIR}/${PKG_NAME}-*.rpm"
        echo ""
    fi

    echo "  Uninstall:"
    echo "    sudo dpkg -r ${PKG_NAME}       # Debian/Ubuntu"
    echo "    sudo rpm -e ${PKG_NAME}         # Fedora/RHEL"
    echo ""
}

main
