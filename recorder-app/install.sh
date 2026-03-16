#!/usr/bin/env bash
# install.sh — Build the Ego Recorder app and pin it to the GNOME dock.
#
# Usage:
#   ./install.sh          # Build + install + pin to dock
#   ./install.sh --skip-build  # Install existing binary + pin (skip build)
#
# Run this AFTER setup.sh has installed all dependencies.

set -euo pipefail
cd "$(dirname "$0")"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

info()  { echo -e "${CYAN}[install]${NC} $*"; }
ok()    { echo -e "${GREEN}[install]${NC} $*"; }
warn()  { echo -e "${YELLOW}[install]${NC} $*"; }
error() { echo -e "${RED}[install]${NC} $*" >&2; }

SKIP_BUILD=false
[[ "${1:-}" == "--skip-build" ]] && SKIP_BUILD=true

APP_NAME="ego-recorder-app"
DESKTOP_ID="${APP_NAME}.desktop"
BINARY_SRC="src-tauri/target/release/${APP_NAME}"
INSTALL_DIR="$HOME/.local/bin"
ICON_SRC="src-tauri/icons/128x128@2x.png"
ICON_DIR="$HOME/.local/share/icons/hicolor/256x256/apps"
DESKTOP_DIR="$HOME/.local/share/applications"

echo ""
echo -e "${BOLD}═══════════════════════════════════════${NC}"
echo -e "${BOLD}  Ego Recorder — Install & Pin${NC}"
echo -e "${BOLD}═══════════════════════════════════════${NC}"
echo ""

# ── Step 1: Build (unless --skip-build) ──────────────────────────────────────

if [[ "$SKIP_BUILD" == false ]]; then
    # Build C++ recorder if not already built
    CPP_BINARY="../build/ego-recorder"
    if [[ ! -x "$CPP_BINARY" ]]; then
        info "Building C++ ego-recorder..."
        NPROC=$(nproc 2>/dev/null || echo 4)

        cmake_args=(-B ../build -S .. -DCMAKE_BUILD_TYPE=Release -DWITH_GUI=OFF -DWITH_PYTHON=OFF -DWITH_SYSTEMD=OFF)

        if ! pkg-config --exists realsense2 2>/dev/null; then
            arch_triple="$(dpkg-architecture -qDEB_HOST_MULTIARCH 2>/dev/null || echo "x86_64-linux-gnu")"
            if [[ -d "/opt/ros/jazzy/lib/${arch_triple}/cmake/realsense2" ]]; then
                cmake_args+=(-DCMAKE_PREFIX_PATH="/opt/ros/jazzy")
            fi
        fi

        cmake "${cmake_args[@]}"
        cmake --build ../build --parallel "$NPROC"
        ok "C++ recorder built"
    else
        ok "C++ recorder already built"
    fi

    # Install frontend deps + build Tauri app
    info "Installing frontend dependencies..."
    bun install

    info "Building Tauri app (this takes a few minutes on first build)..."
    bun run tauri build

    if [[ ! -x "$BINARY_SRC" ]]; then
        error "Build failed — no binary at $BINARY_SRC"
        exit 1
    fi
    ok "Tauri build complete"
fi

# ── Step 2: Install binary ───────────────────────────────────────────────────

if [[ ! -x "$BINARY_SRC" ]]; then
    error "No binary found at $BINARY_SRC — run without --skip-build first"
    exit 1
fi

mkdir -p "$INSTALL_DIR"
cp "$BINARY_SRC" "$INSTALL_DIR/$APP_NAME"
chmod +x "$INSTALL_DIR/$APP_NAME"
ok "Binary installed to $INSTALL_DIR/$APP_NAME"

# Ensure ~/.local/bin is on PATH
if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
    warn "$INSTALL_DIR is not on your PATH"
    warn "Add to ~/.bashrc:  export PATH=\"\$HOME/.local/bin:\$PATH\""
fi

# ── Step 3: Install icon ────────────────────────────────────────────────────

mkdir -p "$ICON_DIR"
cp "$ICON_SRC" "$ICON_DIR/${APP_NAME}.png"
ok "Icon installed"

# ── Step 4: Create .desktop entry ────────────────────────────────────────────

mkdir -p "$DESKTOP_DIR"
cat > "$DESKTOP_DIR/$DESKTOP_ID" <<EOF
[Desktop Entry]
Name=Ego Recorder
Comment=Record egocentric demonstration episodes
Exec=$INSTALL_DIR/$APP_NAME
Icon=$APP_NAME
Terminal=false
Type=Application
Categories=Utility;
StartupWMClass=ego-recorder-app
EOF

# Validate the desktop file if desktop-file-validate is available
if command -v desktop-file-validate &>/dev/null; then
    if desktop-file-validate "$DESKTOP_DIR/$DESKTOP_ID" 2>/dev/null; then
        ok "Desktop entry created and validated"
    else
        warn "Desktop entry created (validation warnings — safe to ignore)"
    fi
else
    ok "Desktop entry created"
fi

# Update desktop database so GNOME picks it up
if command -v update-desktop-database &>/dev/null; then
    update-desktop-database "$DESKTOP_DIR" 2>/dev/null || true
fi

# ── Step 5: Pin to GNOME dock at the top ─────────────────────────────────────

if command -v gsettings &>/dev/null; then
    CURRENT=$(gsettings get org.gnome.shell favorite-apps 2>/dev/null || echo "[]")

    if echo "$CURRENT" | grep -q "'${DESKTOP_ID}'"; then
        # Already in favorites — move to top
        FILTERED=$(echo "$CURRENT" | sed "s/, *'${DESKTOP_ID}'//; s/'${DESKTOP_ID}', *//; s/'${DESKTOP_ID}'//")
        NEW=$(echo "$FILTERED" | sed "s/\[/['${DESKTOP_ID}', /")
        # Clean up any double commas or trailing comma before ]
        NEW=$(echo "$NEW" | sed 's/, *,/,/g; s/, *\]/]/g; s/\[ *,/[/g')
        gsettings set org.gnome.shell favorite-apps "$NEW"
        ok "Moved to top of dock favorites"
    else
        # Add to the front
        if [[ "$CURRENT" == "[]" || "$CURRENT" == "@as []" ]]; then
            NEW="['${DESKTOP_ID}']"
        else
            NEW=$(echo "$CURRENT" | sed "s/\[/['${DESKTOP_ID}', /")
        fi
        gsettings set org.gnome.shell favorite-apps "$NEW"
        ok "Pinned to top of dock favorites"
    fi
else
    warn "gsettings not found — could not pin to dock. Add manually from app grid."
fi

echo ""
echo -e "${BOLD}═══════════════════════════════════════${NC}"
echo -e "${GREEN}${BOLD}  Install complete!${NC}"
echo -e "${BOLD}═══════════════════════════════════════${NC}"
echo ""
echo "  Binary:   $INSTALL_DIR/$APP_NAME"
echo "  Desktop:  $DESKTOP_DIR/$DESKTOP_ID"
echo "  Dock:     Pinned at top of sidebar"
echo ""
