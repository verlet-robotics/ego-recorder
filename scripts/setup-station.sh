#!/usr/bin/env bash
# setup-station.sh -- Minimal setup for recording stations
#
# Installs only what's needed to build and run ego-recorder and the dataset
# viewer app. Skips Rust export tools, Python deps, and tests.
#
# Usage:
#   ./scripts/setup-station.sh                       # GUI + viewer (default)
#   ./scripts/setup-station.sh --headless             # No GUI, no viewer
#   ./scripts/setup-station.sh --headless --with-systemd  # Headless + systemd service
#   ./scripts/setup-station.sh --no-viewer           # Recorder only, no viewer-app

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
BOLD='\033[1m'
YELLOW='\033[1;33m'
DIM='\033[2m'
NC='\033[0m'

info()  { echo -e "${CYAN}[setup]${NC} $*"; }
ok()    { echo -e "${GREEN}[setup]${NC} $*"; }
warn()  { echo -e "${YELLOW}[setup]${NC} $*"; }
err()   { echo -e "${RED}[setup]${NC} $*" >&2; }

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

if [[ ! -f "${SCRIPT_DIR}/lib-env.sh" ]]; then
    echo "Error: lib-env.sh not found. Run this script from the ego-recorder repository." >&2
    exit 1
fi
source "${SCRIPT_DIR}/lib-env.sh"
BUILD_DIR="${PROJECT_DIR}/build"
WITH_GUI=ON
WITH_VIEWER=true
INSTALL_SYSTEMD=false
NPROC=$(nproc 2>/dev/null || echo 4)

# Track whether librealsense was built from source (for udev rules)
REALSENSE_BUILT_FROM_SOURCE=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --headless)      WITH_GUI=OFF; WITH_VIEWER=false; shift ;;
        --with-viewer)  WITH_VIEWER=true; shift ;;
        --no-viewer)    WITH_VIEWER=false; shift ;;
        --with-systemd)  INSTALL_SYSTEMD=true; shift ;;
        --help|-h)
            echo "Usage: ./scripts/setup-station.sh [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --headless       Build without GUI and viewer (no GLFW/OpenGL)"
            echo "  --with-viewer    Include dataset viewer app (default for GUI builds)"
            echo "  --no-viewer      Skip viewer app build"
            echo "  --with-systemd   Deploy as a systemd service after building"
            exit 0 ;;
        *) err "Unknown option: $1"; exit 1 ;;
    esac
done

# ---------------------------------------------------------------------------
# 0. Preflight (minimal systems: ensure apt, network, and essential tools)
# ---------------------------------------------------------------------------
preflight() {
    if [[ ! -f /etc/os-release ]]; then
        err "Cannot detect OS. This script requires Ubuntu 22.04/24.04 or Debian."
        exit 1
    fi

    if ! command -v apt-get &>/dev/null; then
        err "apt-get not found. This script requires a Debian-based system."
        exit 1
    fi

    info "Updating package index (requires network)..."
    if ! sudo apt-get update -qq 2>/dev/null; then
        err "apt-get update failed. Check network and sudo access."
        exit 1
    fi
    ok "Preflight OK"
}

# ---------------------------------------------------------------------------
# 1. System packages (recording only — no Rust, no Python, no FFmpeg extras)
# ---------------------------------------------------------------------------
install_deps() {
    info "Installing system dependencies..."

    # Remove any stale Intel RealSense repo that may poison apt-get update
    # (e.g. if a previous run added it for an unsupported Ubuntu codename).
    # The repo will be re-added properly by install_realsense_apt if needed.
    if [[ -f /etc/apt/sources.list.d/librealsense.list ]]; then
        info "Removing stale Intel RealSense apt repo (will re-add if needed)..."
        sudo rm -f /etc/apt/sources.list.d/librealsense.list
    fi

    local packages=(
        build-essential
        cmake
        pkg-config
        git
        curl
        wget
        file
        unzip
        ca-certificates

        # Rust build deps (bindgen needs libclang)
        libclang-dev

        # Compression
        libzstd-dev
        libturbojpeg0-dev

        # OpenSSL (needed if building librealsense from source)
        libssl-dev

        # FFmpeg — C++ encoder needs avcodec/avutil/swscale,
        # Rust ffmpeg-next crate also needs avformat/swresample/avfilter
        libavcodec-dev
        libavdevice-dev
        libavfilter-dev
        libavformat-dev
        libavutil-dev
        libswscale-dev
        libswresample-dev

        # Audio alerts
        espeak-ng

        # systemd watchdog
        libsystemd-dev
    )

    if [[ "$WITH_GUI" == "ON" ]]; then
        packages+=(libglfw3-dev libopengl-dev)
    fi

    # Tauri / WebKit deps for dataset viewer app
    if [[ "$WITH_VIEWER" == true ]]; then
        packages+=(
            libwebkit2gtk-4.1-dev
            libxdo-dev
            libayatana-appindicator3-dev
            librsvg2-dev
        )
    fi

    sudo apt-get update -qq
    sudo apt-get install -y "${packages[@]}"
    ok "System dependencies installed"
}

# ---------------------------------------------------------------------------
# 1b. Bun runtime (for viewer-app frontend)
# ---------------------------------------------------------------------------
install_bun() {
    if command -v bun &>/dev/null; then
        ok "Bun already installed ($(bun --version))"
        return 0
    fi

    if ! command -v curl &>/dev/null; then
        err "curl required for Bun install. Run install_deps first."
        exit 1
    fi

    info "Installing Bun runtime (requires network)..."
    if ! curl -fsSL https://bun.sh/install | bash; then
        err "Bun install script failed. Check network connectivity."
        exit 1
    fi

    if [[ -f "${HOME}/.bun/bin/bun" ]]; then
        export BUN_INSTALL="${HOME}/.bun"
        export PATH="${BUN_INSTALL}/bin:${PATH}"
        ok "Bun installed (~/.bun/bin/bun)"
    else
        err "Bun install completed but binary not found at ~/.bun/bin/bun"
        exit 1
    fi
}

# ---------------------------------------------------------------------------
# 2. Intel RealSense SDK
# ---------------------------------------------------------------------------
install_realsense() {
    if pkg-config --exists realsense2 2>/dev/null; then
        ok "librealsense2 already available ($(pkg-config --modversion realsense2))"
        return 0
    fi

    if [[ -d /opt/ros/jazzy/lib/x86_64-linux-gnu/cmake/realsense2 ]]; then
        ok "librealsense2 available via ROS 2 Jazzy"
        local ros_lib="/opt/ros/jazzy/lib/x86_64-linux-gnu"
        if ! ldconfig -p 2>/dev/null | grep -q librealsense2; then
            info "Adding ${ros_lib} to ldconfig..."
            echo "$ros_lib" | sudo tee /etc/ld.so.conf.d/ros-jazzy.conf > /dev/null
            sudo ldconfig
        fi
        return 0
    fi

    if dpkg -s librealsense2-dev &>/dev/null; then
        ok "librealsense2-dev already installed"
        return 0
    fi

    info "Installing Intel RealSense SDK..."

    # Try the Intel apt repo first
    if install_realsense_apt; then
        ok "Intel RealSense SDK installed via apt"
        return 0
    fi

    # Fallback: build from source
    warn "Intel apt repo packages unavailable — building librealsense from source..."
    install_realsense_from_source
    REALSENSE_BUILT_FROM_SOURCE=true
    ok "Intel RealSense SDK built and installed from source"
}

install_realsense_apt() {
    sudo mkdir -p /etc/apt/keyrings
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
    local rs_dir="${SCRIPT_DIR}/librealsense"
    local rs_build="${rs_dir}/build"

    local rs_deps=(libusb-1.0-0-dev)
    if [[ "$WITH_GUI" == "ON" ]]; then
        rs_deps+=(libglfw3-dev libgtk-3-dev)
    fi
    sudo apt-get install -y "${rs_deps[@]}"

    if [[ ! -d "$rs_dir" ]]; then
        info "Cloning librealsense..."
        git clone --depth 1 https://github.com/IntelRealSense/librealsense.git "$rs_dir"
    else
        info "Using existing librealsense source at ${rs_dir}"
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

    info "Installing librealsense..."
    sudo cmake --install "$rs_build"
    sudo ldconfig
}

# ---------------------------------------------------------------------------
# 3. udev rules (camera access without root + USB autosuspend)
# ---------------------------------------------------------------------------
install_udev_rules() {
    info "Installing udev rules..."

    local udev_dir="/etc/udev/rules.d"

    # If librealsense was built from source, install its udev rules so the
    # camera is accessible without root.
    if [[ "$REALSENSE_BUILT_FROM_SOURCE" == true ]]; then
        local rs_rules="${SCRIPT_DIR}/librealsense/config/99-realsense-libusb.rules"
        if [[ -f "$rs_rules" ]]; then
            sudo install -m 644 "$rs_rules" "${udev_dir}/99-realsense-libusb.rules"
            ok "Installed librealsense udev rules"
        else
            warn "librealsense udev rules not found at ${rs_rules} — camera may require sudo"
        fi
    fi

    # Always install ego-recorder rules (USB autosuspend prevention)
    local ego_rules="${PROJECT_DIR}/deploy/99-ego-recorder.rules"
    if [[ -f "$ego_rules" ]]; then
        sudo install -m 644 "$ego_rules" "${udev_dir}/99-ego-recorder.rules"
        ok "Installed ego-recorder udev rules (USB autosuspend)"
    fi

    sudo udevadm control --reload-rules
    sudo udevadm trigger
}

# ---------------------------------------------------------------------------
# 4. User group access (plugdev + video for camera without root)
# ---------------------------------------------------------------------------
setup_user_groups() {
    local target_user="${SUDO_USER:-$USER}"
    if [[ "$target_user" == "root" ]]; then
        return 0
    fi

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

# ---------------------------------------------------------------------------
# 5. Build (recorder only — no Python, no tests)
# ---------------------------------------------------------------------------
build() {
    info "Configuring CMake..."

    local cmake_args=(
        -B "$BUILD_DIR"
        -DWITH_GUI="$WITH_GUI"
        -DWITH_PYTHON=OFF
        -DBUILD_TESTS=OFF
        -DWITH_RUST_EXPORT=OFF
        -DCMAKE_BUILD_TYPE=Release
    )

    if ! pkg-config --exists realsense2 2>/dev/null; then
        if [[ -d /opt/ros/jazzy/lib/x86_64-linux-gnu/cmake/realsense2 ]]; then
            cmake_args+=(-DCMAKE_PREFIX_PATH="/opt/ros/jazzy")
        fi
    fi

    cmake "${cmake_args[@]}" "$PROJECT_DIR"

    info "Building with ${NPROC} parallel jobs..."
    cmake --build "$BUILD_DIR" --parallel "$NPROC"

    ok "Build complete: ${BUILD_DIR}/ego-recorder"
}

# ---------------------------------------------------------------------------
# 5b. Build ego-qc (Rust QC tools for on-device data pruning)
# ---------------------------------------------------------------------------
build_ego_qc() {
    info "Building ego-qc (Rust QC tools)..."

    # Ensure Rust is available (install or load from previous install)
    if ! command -v cargo &>/dev/null; then
        if [[ -f "${HOME}/.cargo/env" ]]; then
            source "${HOME}/.cargo/env"
        fi
    fi
    if ! command -v cargo &>/dev/null; then
        info "Installing Rust toolchain via rustup (requires network)..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
            | sh -s -- -y --default-toolchain stable --profile minimal
        source "${HOME}/.cargo/env"
        ok "Rust toolchain installed"
    else
        ok "Rust already available ($(rustc --version 2>/dev/null || true))"
    fi

    local rust_dir="${PROJECT_DIR}/rust"
    cargo build --release --manifest-path "${rust_dir}/Cargo.toml" -p ego-qc

    ok "Build complete: ${rust_dir}/target/release/ego-qc"
}

# ---------------------------------------------------------------------------
# 5c. Build viewer-app (Tauri desktop app for dataset review)
# ---------------------------------------------------------------------------
build_viewer_app() {
    if [[ "$WITH_VIEWER" != true ]]; then
        return 0
    fi

    info "Building dataset viewer app (viewer-app)..."

    # Ensure Bun is in PATH
    if [[ -d "${HOME}/.bun/bin" ]]; then
        export PATH="${HOME}/.bun/bin:${PATH}"
    fi
    if ! command -v bun &>/dev/null; then
        err "Bun not found. Run install_bun first."
        exit 1
    fi

    # Ensure Rust/cargo is in PATH
    if ! command -v cargo &>/dev/null && [[ -f "${HOME}/.cargo/env" ]]; then
        source "${HOME}/.cargo/env"
    fi
    if ! command -v cargo &>/dev/null; then
        err "cargo not found. Rust should have been installed by build_ego_qc."
        exit 1
    fi

    local viewer_dir="${PROJECT_DIR}/viewer-app"
    if [[ ! -d "$viewer_dir" ]]; then
        warn "viewer-app directory not found at ${viewer_dir}, skipping"
        return 0
    fi

    (cd "$viewer_dir" && bun install)
    (cd "$viewer_dir" && cargo build --release --manifest-path src-tauri/Cargo.toml)

    ok "Viewer app built: ${viewer_dir}"
}

# ---------------------------------------------------------------------------
# 6. Systemd deployment (optional)
# ---------------------------------------------------------------------------
deploy_systemd() {
    if [[ "$INSTALL_SYSTEMD" != true ]]; then
        return 0
    fi

    info "Running systemd deployment..."
    if [[ "$(id -u)" -ne 0 ]]; then
        sudo bash "${PROJECT_DIR}/deploy/install.sh"
    else
        bash "${PROJECT_DIR}/deploy/install.sh"
    fi
}

# ---------------------------------------------------------------------------
# 7. Cloud upload configuration (optional)
# ---------------------------------------------------------------------------
setup_upload_config() {
    echo ""
    read -rp "Configure R2 cloud upload for recordings? [y/N] " ans
    if [[ ! "$ans" =~ ^[Yy] ]]; then
        return 0
    fi

    local env_file="${PROJECT_DIR}/python/.env"
    prompt_r2_credentials "$env_file" || true
    setup_facility "$env_file" || true
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
echo ""
echo -e "${BOLD}═══════════════════════════════════════${NC}"
echo -e "${BOLD}  ego-recorder station setup${NC}"
echo -e "${BOLD}═══════════════════════════════════════${NC}"
echo -e "  GUI:     ${WITH_GUI}"
echo -e "  Viewer:  ${WITH_VIEWER}"
echo -e "  systemd: ${INSTALL_SYSTEMD}"
echo ""

cd "$PROJECT_DIR"
preflight
install_deps
[[ "$WITH_VIEWER" == true ]] && install_bun
install_realsense
install_udev_rules
setup_user_groups
build
build_ego_qc
build_viewer_app
deploy_systemd
setup_upload_config

RUST_DIR="${PROJECT_DIR}/rust"
VIEWER_DIR="${PROJECT_DIR}/viewer-app"
echo ""
echo -e "${GREEN}${BOLD}  Setup complete!${NC}"
echo -e "  Recorder: ${BUILD_DIR}/ego-recorder"
echo -e "  QC tools: ${RUST_DIR}/target/release/ego-qc"
[[ "$WITH_VIEWER" == true ]] && echo -e "  Viewer:   ${VIEWER_DIR}"
echo ""
if [[ "$INSTALL_SYSTEMD" == true ]]; then
    echo "  Service installed. Next steps:"
    echo "    1. Edit /etc/ego-recorder/config.toml"
    echo "    2. systemctl enable --now ego-recorder.service"
else
    echo "  Record:  ./scripts/record.sh"
    echo "  Upload:  ./scripts/upload.sh"
    echo "  Viewer:  ./scripts/viewer.sh"
fi
if [[ "$WITH_VIEWER" == true ]] && [[ -d "${HOME}/.bun/bin" ]]; then
    echo ""
    echo -e "  ${DIM}To use Bun in new terminals, add to ~/.bashrc:${NC}"
    echo -e "  ${DIM}  export PATH=\"\$HOME/.bun/bin:\$PATH\"${NC}"
    echo -e "  ${DIM}(viewer.sh adds it automatically when launched)${NC}"
fi
echo ""
echo "  QC usage:"
echo "    ${RUST_DIR}/target/release/ego-qc analyze datasets/"
echo "    ${RUST_DIR}/target/release/ego-qc prune --apply datasets/"
echo ""
