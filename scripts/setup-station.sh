#!/usr/bin/env bash
# setup-station.sh -- Minimal setup for recording stations
#
# Installs only what's needed to build and run ego-recorder.
# Skips Rust export tools, Python deps, and tests.
#
# Usage:
#   ./scripts/setup-station.sh              # GUI build (default)
#   ./scripts/setup-station.sh --headless   # No GUI

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
BOLD='\033[1m'
YELLOW='\033[1;33m'
NC='\033[0m'

info()  { echo -e "${CYAN}[setup]${NC} $*"; }
ok()    { echo -e "${GREEN}[setup]${NC} $*"; }
warn()  { echo -e "${YELLOW}[setup]${NC} $*"; }
err()   { echo -e "${RED}[setup]${NC} $*" >&2; }

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
BUILD_DIR="${PROJECT_DIR}/build"
WITH_GUI=ON
NPROC=$(nproc 2>/dev/null || echo 4)

while [[ $# -gt 0 ]]; do
    case "$1" in
        --headless) WITH_GUI=OFF; shift ;;
        --help|-h)
            echo "Usage: ./scripts/setup-station.sh [--headless]"
            exit 0 ;;
        *) err "Unknown option: $1"; exit 1 ;;
    esac
done

# ---------------------------------------------------------------------------
# 1. System packages (recording only — no Rust, no Python, no FFmpeg extras)
# ---------------------------------------------------------------------------
install_deps() {
    info "Installing system dependencies..."

    local packages=(
        build-essential
        cmake
        pkg-config
        git
        curl
        ca-certificates

        # Compression
        libzstd-dev
        libturbojpeg0-dev

        # FFmpeg (H.264 encoder only needs these 3)
        libavcodec-dev
        libavutil-dev
        libswscale-dev

        # Audio alerts
        espeak-ng

        # systemd watchdog
        libsystemd-dev
    )

    if [[ "$WITH_GUI" == "ON" ]]; then
        packages+=(libglfw3-dev libopengl-dev)
    fi

    sudo apt-get update -qq
    sudo apt-get install -y "${packages[@]}"
    ok "System dependencies installed"
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
    sudo mkdir -p /etc/apt/keyrings
    if [[ ! -f /etc/apt/keyrings/librealsense.pgp ]]; then
        curl -sSf https://librealsense.intel.com/Debian/librealsense.pgp \
            | sudo tee /etc/apt/keyrings/librealsense.pgp > /dev/null
    fi

    source /etc/os-release
    local codename="${VERSION_CODENAME:-noble}"
    local repo_line="deb [signed-by=/etc/apt/keyrings/librealsense.pgp] https://librealsense.intel.com/Debian/apt-repo ${codename} main"

    if ! grep -qF "librealsense.intel.com" /etc/apt/sources.list.d/*.list /etc/apt/sources.list.d/*.sources 2>/dev/null; then
        echo "$repo_line" | sudo tee /etc/apt/sources.list.d/librealsense.list > /dev/null
        sudo apt-get update -qq
    fi

    sudo apt-get install -y librealsense2-dev librealsense2-utils
    ok "Intel RealSense SDK installed"
}

# ---------------------------------------------------------------------------
# 3. Build (recorder only — no Rust, no Python, no tests)
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
# Main
# ---------------------------------------------------------------------------
echo ""
echo -e "${BOLD}═══════════════════════════════════════${NC}"
echo -e "${BOLD}  ego-recorder station setup${NC}"
echo -e "${BOLD}═══════════════════════════════════════${NC}"
echo -e "  GUI: ${WITH_GUI}"
echo ""

cd "$PROJECT_DIR"
install_deps
install_realsense
build

echo ""
echo -e "${GREEN}${BOLD}  Setup complete!${NC}"
echo -e "  Binary: ${BUILD_DIR}/ego-recorder"
echo ""
if [[ "$WITH_GUI" == "ON" ]]; then
    echo "  Run: ${BUILD_DIR}/ego-recorder -s my_session -o ./recordings"
else
    echo "  Run: ${BUILD_DIR}/ego-recorder --headless -o ./recordings -d 300"
fi
echo ""
