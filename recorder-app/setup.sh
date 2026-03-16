#!/usr/bin/env bash
# setup.sh — One-script build chain for the Ego Recorder desktop app.
#
# Usage:
#   ./setup.sh          # Full build (C++ binary + frontend + Tauri app)
#   ./setup.sh --dev    # Dev mode: install deps only, skip full build
#   ./setup.sh --check  # Check system dependencies only
#
# This script is idempotent — safe to run multiple times.

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

info()  { echo -e "${CYAN}[setup]${NC} $*"; }
ok()    { echo -e "${GREEN}[setup]${NC} $*"; }
warn()  { echo -e "${YELLOW}[setup]${NC} $*"; }
error() { echo -e "${RED}[setup]${NC} $*" >&2; }

MODE="${1:-build}"
NPROC=$(nproc 2>/dev/null || echo 4)

echo ""
echo -e "${BOLD}═══════════════════════════════════════${NC}"
echo -e "${BOLD}  Ego Recorder App — Setup${NC}"
echo -e "${BOLD}═══════════════════════════════════════${NC}"
echo ""

# ── Step 1: Install system dependencies ───────────────────────────────────────

install_system_deps() {
    info "Installing system build dependencies..."

    local packages=(
        build-essential
        cmake
        pkg-config
        git
        curl
        ca-certificates
        file
        unzip
        wget

        # Compression
        libzstd-dev
        libturbojpeg0-dev

        # OpenSSL (needed if building librealsense from source)
        libssl-dev

        # FFmpeg — C++ encoder needs avcodec/avutil/swscale
        libavcodec-dev
        libavformat-dev
        libavutil-dev
        libswscale-dev
        libswresample-dev

        # Tauri / WebKit deps
        libwebkit2gtk-4.1-dev
        libxdo-dev
        libayatana-appindicator3-dev
        librsvg2-dev
        patchelf

        # X11/XCB — required by Tauri's windowing on Linux
        libxcb-render0-dev
        libxcb-shape0-dev
        libxcb-xfixes0-dev
        libxkbcommon-x11-dev

        # D-Bus — compile-time (zbus crate) + runtime (lid-close inhibitor)
        libdbus-1-dev
        dbus

        # systemd (watchdog, lid-close safe)
        libsystemd-dev

        # Rust build deps (bindgen needs libclang)
        libclang-dev

        # Audio — aplay used by ego-recorder for countdown/recording beeps
        alsa-utils
    )

    sudo apt-get update -qq
    sudo apt-get install -y "${packages[@]}"
    ok "System dependencies installed"
}

# ── Step 1b: Rust toolchain ───────────────────────────────────────────────────

install_rust() {
    if command -v cargo &>/dev/null; then
        ok "Rust toolchain already installed ($(cargo --version))"
        return 0
    fi

    info "Installing Rust toolchain via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal
    source "$HOME/.cargo/env"
    ok "Rust toolchain installed ($(cargo --version))"
}

# ── Step 1c: Bun runtime ─────────────────────────────────────────────────────

install_bun() {
    if command -v bun &>/dev/null; then
        ok "Bun already installed ($(bun --version))"
        return 0
    fi

    info "Installing Bun runtime..."
    curl -fsSL https://bun.sh/install | bash
    if [[ -f "${HOME}/.bun/bin/bun" ]]; then
        export BUN_INSTALL="${HOME}/.bun"
        export PATH="${BUN_INSTALL}/bin:${PATH}"
        ok "Bun installed ($(bun --version))"
    else
        error "Bun install completed but binary not found at ~/.bun/bin/bun"
        exit 1
    fi
}

# ── Step 1d: Intel RealSense SDK ──────────────────────────────────────────────

install_realsense() {
    # Already available via pkg-config
    if pkg-config --exists realsense2 2>/dev/null; then
        ok "librealsense2 already available ($(pkg-config --modversion realsense2))"
        return 0
    fi

    # Available via ROS 2 Jazzy
    local arch_triple
    arch_triple="$(dpkg-architecture -qDEB_HOST_MULTIARCH 2>/dev/null || echo "x86_64-linux-gnu")"
    if [[ -d "/opt/ros/jazzy/lib/${arch_triple}/cmake/realsense2" ]]; then
        ok "librealsense2 available via ROS 2 Jazzy"
        local ros_lib="/opt/ros/jazzy/lib/${arch_triple}"
        if ! ldconfig -p 2>/dev/null | grep -q librealsense2; then
            info "Adding ${ros_lib} to ldconfig..."
            echo "$ros_lib" | sudo tee /etc/ld.so.conf.d/ros-jazzy.conf > /dev/null
            sudo ldconfig
        fi
        return 0
    fi

    # Already installed via apt
    if dpkg -s librealsense2-dev &>/dev/null; then
        ok "librealsense2-dev already installed"
        return 0
    fi

    info "Installing Intel RealSense SDK..."

    # Try Intel apt repo first
    if install_realsense_apt; then
        ok "Intel RealSense SDK installed via apt"
        return 0
    fi

    # Fallback: build from source
    warn "Intel apt repo unavailable — building librealsense from source (this takes a few minutes)..."
    install_realsense_from_source
    ok "Intel RealSense SDK built and installed from source"
}

install_realsense_apt() {
    sudo mkdir -p /etc/apt/keyrings

    # Remove any stale repo entry
    if [[ -f /etc/apt/sources.list.d/librealsense.list ]]; then
        sudo rm -f /etc/apt/sources.list.d/librealsense.list
    fi

    if [[ ! -f /etc/apt/keyrings/librealsense.pgp ]]; then
        curl -sSf https://librealsense.intel.com/Debian/librealsense.pgp \
            | sudo tee /etc/apt/keyrings/librealsense.pgp > /dev/null
    fi

    source /etc/os-release
    local codename="${VERSION_CODENAME:-noble}"
    local repo_line="deb [signed-by=/etc/apt/keyrings/librealsense.pgp] https://librealsense.intel.com/Debian/apt-repo ${codename} main"

    echo "$repo_line" | sudo tee /etc/apt/sources.list.d/librealsense.list > /dev/null

    if ! sudo apt-get update -qq 2>/dev/null; then
        warn "Intel apt repo not available for ${codename} — removing repo entry"
        sudo rm -f /etc/apt/sources.list.d/librealsense.list
        sudo apt-get update -qq 2>/dev/null || true
        return 1
    fi

    if ! sudo apt-get install -y librealsense2-dev librealsense2-utils 2>/dev/null; then
        warn "librealsense2 packages not available — removing repo entry"
        sudo rm -f /etc/apt/sources.list.d/librealsense.list
        return 1
    fi
}

install_realsense_from_source() {
    local rs_dir="/tmp/librealsense-build"
    local rs_build="${rs_dir}/build"

    sudo apt-get install -y libusb-1.0-0-dev

    if [[ ! -d "$rs_dir" ]]; then
        info "Cloning librealsense..."
        git clone --depth 1 https://github.com/IntelRealSense/librealsense.git "$rs_dir"
    fi

    mkdir -p "$rs_build"
    info "Configuring librealsense..."
    cmake -S "$rs_dir" -B "$rs_build" \
        -DCMAKE_BUILD_TYPE=Release \
        -DBUILD_EXAMPLES=OFF \
        -DBUILD_GRAPHICAL_EXAMPLES=OFF \
        -DBUILD_GLSL_EXTENSIONS=OFF

    info "Building librealsense (this may take a while)..."
    cmake --build "$rs_build" --parallel "$NPROC"

    info "Installing librealsense to /usr/local..."
    sudo cmake --install "$rs_build"
    sudo ldconfig

    # Install udev rules for camera access without root
    local rs_rules="${rs_dir}/config/99-realsense-libusb.rules"
    if [[ -f "$rs_rules" ]]; then
        sudo install -m 644 "$rs_rules" /etc/udev/rules.d/99-realsense-libusb.rules
        sudo udevadm control --reload-rules
        sudo udevadm trigger
        ok "Installed RealSense udev rules"
    fi
}

# ── Step 1e: User groups (camera access without root) ─────────────────────────

setup_user_groups() {
    local target_user="${SUDO_USER:-$USER}"
    [[ "$target_user" == "root" ]] && return 0

    local groups_added=()
    for grp in plugdev video; do
        if ! id -nG "$target_user" | grep -qw "$grp"; then
            sudo usermod -aG "$grp" "$target_user"
            groups_added+=("$grp")
        fi
    done

    if [[ ${#groups_added[@]} -gt 0 ]]; then
        ok "Added ${target_user} to groups: ${groups_added[*]}"
        warn "Log out and back in for group changes to take effect"
    fi
}

# ── Step 2: Check all dependencies ────────────────────────────────────────────

check_deps() {
    info "Verifying dependencies..."
    local ok_count=0
    local fail_count=0

    for cmd in rustc cargo bun cmake pkg-config; do
        if command -v "$cmd" &>/dev/null; then
            ok "  $cmd ... found"
            ok_count=$((ok_count + 1))
        else
            error "  $cmd ... NOT FOUND"
            fail_count=$((fail_count + 1))
        fi
    done

    if pkg-config --exists realsense2 2>/dev/null; then
        ok "  librealsense2 ... found ($(pkg-config --modversion realsense2))"
        ok_count=$((ok_count + 1))
    else
        error "  librealsense2 ... NOT FOUND"
        fail_count=$((fail_count + 1))
    fi

    for lib in libavcodec libavutil libswscale libswresample; do
        if pkg-config --exists "$lib" 2>/dev/null; then
            ok "  $lib ... found"
            ok_count=$((ok_count + 1))
        else
            error "  $lib ... NOT FOUND"
            fail_count=$((fail_count + 1))
        fi
    done

    if [[ "$fail_count" -gt 0 ]]; then
        error "${fail_count} dependencies missing. Run ./setup.sh to install them."
        return 1
    fi

    ok "All ${ok_count} dependencies present."
    return 0
}

# ── Step 3: Build C++ ego-recorder binary ─────────────────────────────────────

build_cpp() {
    local build_dir="../build"
    local binary="${build_dir}/ego-recorder"

    if [[ -x "$binary" ]]; then
        ok "C++ ego-recorder binary already built at $binary"
        return 0
    fi

    info "Building C++ ego-recorder..."

    local cmake_args=(
        -B "$build_dir" -S ..
        -DCMAKE_BUILD_TYPE=Release
        -DWITH_GUI=OFF
        -DWITH_PYTHON=OFF
        -DWITH_SYSTEMD=OFF
    )

    # Auto-detect ROS 2 Jazzy realsense2
    if ! pkg-config --exists realsense2 2>/dev/null; then
        local arch_triple
        arch_triple="$(dpkg-architecture -qDEB_HOST_MULTIARCH 2>/dev/null || echo "x86_64-linux-gnu")"
        if [[ -d "/opt/ros/jazzy/lib/${arch_triple}/cmake/realsense2" ]]; then
            cmake_args+=(-DCMAKE_PREFIX_PATH="/opt/ros/jazzy")
        fi
    fi

    cmake "${cmake_args[@]}"
    cmake --build "$build_dir" --parallel "$NPROC"
    ok "C++ build complete: $binary"
}

# ── Step 4: Install frontend dependencies ─────────────────────────────────────

install_frontend() {
    cd "$SCRIPT_DIR"
    info "Installing frontend dependencies..."
    bun install
    ok "Frontend dependencies installed"
}

# ── Step 5: Build Tauri app ───────────────────────────────────────────────────

build_tauri() {
    cd "$SCRIPT_DIR"
    info "Building Tauri app..."
    bun run tauri build

    local binary="src-tauri/target/release/ego-recorder-app"
    if [[ -x "$binary" ]]; then
        ok "Build complete: $binary"
    else
        error "Build produced no binary at $binary"
        exit 1
    fi
}

install_app() {
    local binary="src-tauri/target/release/ego-recorder-app"
    if [[ ! -x "$binary" ]]; then
        error "No binary to install. Run ./setup.sh first."
        exit 1
    fi
    mkdir -p "$HOME/.local/bin"
    cp "$binary" "$HOME/.local/bin/ego-recorder-app"
    ok "Installed to ~/.local/bin/ego-recorder-app"
}

# ── Main ──────────────────────────────────────────────────────────────────────

if [[ "$MODE" == "--check" ]]; then
    check_deps
    exit $?
fi

install_system_deps
install_rust

# Ensure cargo is in PATH for the rest of the script (rustup installer
# only sources env inside the install_rust function scope)
if [[ -f "$HOME/.cargo/env" ]]; then
    source "$HOME/.cargo/env"
fi

install_bun

# Ensure bun is in PATH for the rest of the script
if [[ -d "$HOME/.bun/bin" ]]; then
    export BUN_INSTALL="$HOME/.bun"
    export PATH="$BUN_INSTALL/bin:$PATH"
fi

install_realsense
setup_user_groups

# Verify everything landed
check_deps || exit 1

build_cpp
install_frontend

if [[ "$MODE" == "--dev" ]]; then
    echo ""
    ok "Dev mode: dependencies installed."
    echo ""
    echo "  Next steps:"
    echo "    source ~/.bashrc    # pick up bun/cargo in PATH"
    echo "    bun run tauri dev   # start dev server"
    echo ""
    exit 0
fi

build_tauri

if [[ "$MODE" == "--install" ]]; then
    install_app
fi

echo ""
echo -e "${BOLD}═══════════════════════════════════════${NC}"
echo -e "${GREEN}${BOLD}  Setup complete!${NC}"
echo -e "${BOLD}═══════════════════════════════════════${NC}"
echo ""
echo "  C++ recorder: ../build/ego-recorder"
echo "  Tauri app:    src-tauri/target/release/ego-recorder-app"
echo ""
echo "  Dev mode:  bun run tauri dev"
echo ""
