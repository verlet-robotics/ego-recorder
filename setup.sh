#!/usr/bin/env bash
# setup.sh -- Full setup script for ego-recorder
#
# Installs all system dependencies, builds the project, and optionally
# sets up Python export tools and the systemd service.
#
# Usage:
#   ./setup.sh              # Interactive (prompts for options)
#   ./setup.sh --all        # Full install (GUI + Python + tests)
#   ./setup.sh --headless   # Headless build only (no GUI, no Python)
#   ./setup.sh --help       # Show usage
#
# Supports:
#   - Ubuntu 22.04 / 24.04
#   - Intel RealSense SDK via Intel apt repo or ROS 2 Jazzy packages

set -euo pipefail

# ---------------------------------------------------------------------------
# Colors
# ---------------------------------------------------------------------------
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m' # No Color

info()  { echo -e "${CYAN}[setup]${NC} $*"; }
ok()    { echo -e "${GREEN}[setup]${NC} $*"; }
warn()  { echo -e "${YELLOW}[setup]${NC} $*"; }
err()   { echo -e "${RED}[setup]${NC} $*" >&2; }

# ---------------------------------------------------------------------------
# Defaults
# ---------------------------------------------------------------------------
PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD_DIR="${PROJECT_DIR}/build"
WITH_GUI=ON
WITH_PYTHON=ON
BUILD_TESTS=ON
INSTALL_PYTHON_EXPORT=false
INSTALL_SYSTEMD=false
INTERACTIVE=true
NPROC=$(nproc 2>/dev/null || echo 4)

# ---------------------------------------------------------------------------
# Usage
# ---------------------------------------------------------------------------
usage() {
    cat <<'EOF'
Usage: ./setup.sh [OPTIONS]

Options:
  --all             Full build: GUI + Python + tests + Python export deps
  --headless        Headless build only (no GUI, no Python extension)
  --no-gui          Disable GUI (Dear ImGui / GLFW / OpenGL)
  --no-python       Disable Python extension module (pybind11)
  --no-tests        Disable unit tests
  --with-export     Install Python export dependencies (RLDS + LeRobot)
  --with-systemd    Also run the systemd deployment (requires sudo)
  --build-dir DIR   Set build directory (default: ./build)
  --help            Show this help

Examples:
  ./setup.sh                    # Interactive prompts
  ./setup.sh --all              # Everything, non-interactive
  ./setup.sh --headless         # Minimal headless build
  ./setup.sh --all --with-systemd  # Full build + deploy as service
EOF
    exit 0
}

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
    case "$1" in
        --all)
            WITH_GUI=ON; WITH_PYTHON=ON; BUILD_TESTS=ON
            INSTALL_PYTHON_EXPORT=true; INTERACTIVE=false
            shift ;;
        --headless)
            WITH_GUI=OFF; WITH_PYTHON=OFF; BUILD_TESTS=ON
            INTERACTIVE=false
            shift ;;
        --no-gui)       WITH_GUI=OFF; shift ;;
        --no-python)    WITH_PYTHON=OFF; shift ;;
        --no-tests)     BUILD_TESTS=OFF; shift ;;
        --with-export)  INSTALL_PYTHON_EXPORT=true; shift ;;
        --with-systemd) INSTALL_SYSTEMD=true; shift ;;
        --build-dir)    BUILD_DIR="$2"; shift 2 ;;
        --help|-h)      usage ;;
        *)
            err "Unknown option: $1"
            usage ;;
    esac
done

# ---------------------------------------------------------------------------
# Interactive prompts
# ---------------------------------------------------------------------------
if [[ "$INTERACTIVE" == true ]]; then
    echo ""
    echo -e "${BOLD}ego-recorder setup${NC}"
    echo "─────────────────────────────────────"
    echo ""

    read -rp "Build with GUI (Dear ImGui)? [Y/n] " ans
    [[ "$ans" =~ ^[Nn] ]] && WITH_GUI=OFF

    read -rp "Build Python extension module? [Y/n] " ans
    [[ "$ans" =~ ^[Nn] ]] && WITH_PYTHON=OFF

    read -rp "Build unit tests? [Y/n] " ans
    [[ "$ans" =~ ^[Nn] ]] && BUILD_TESTS=OFF

    read -rp "Install Python export deps (RLDS + LeRobot)? [y/N] " ans
    [[ "$ans" =~ ^[Yy] ]] && INSTALL_PYTHON_EXPORT=true

    read -rp "Install systemd service (requires sudo)? [y/N] " ans
    [[ "$ans" =~ ^[Yy] ]] && INSTALL_SYSTEMD=true

    echo ""
fi

# ---------------------------------------------------------------------------
# Print configuration
# ---------------------------------------------------------------------------
echo ""
echo -e "${BOLD}Build configuration:${NC}"
echo "  GUI:              ${WITH_GUI}"
echo "  Python extension: ${WITH_PYTHON}"
echo "  Unit tests:       ${BUILD_TESTS}"
echo "  Python export:    ${INSTALL_PYTHON_EXPORT}"
echo "  Systemd deploy:   ${INSTALL_SYSTEMD}"
echo "  Build directory:  ${BUILD_DIR}"
echo ""

# ---------------------------------------------------------------------------
# Detect OS
# ---------------------------------------------------------------------------
if [[ ! -f /etc/os-release ]]; then
    err "Cannot detect OS. This script supports Ubuntu 22.04 / 24.04."
    exit 1
fi
source /etc/os-release
if [[ "$ID" != "ubuntu" ]]; then
    warn "Detected $ID $VERSION_ID -- this script is tested on Ubuntu. Proceeding anyway."
fi
info "Detected: $PRETTY_NAME"

# ---------------------------------------------------------------------------
# 1. Install Intel RealSense SDK (if not already available)
# ---------------------------------------------------------------------------
install_realsense_sdk() {
    # Check if realsense2 is already findable (e.g. via ROS 2 packages)
    if pkg-config --exists realsense2 2>/dev/null; then
        ok "librealsense2 already available via pkg-config ($(pkg-config --modversion realsense2))"
        return 0
    fi

    # Check if ROS 2 Jazzy provides it
    if [[ -d /opt/ros/jazzy/lib/x86_64-linux-gnu/cmake/realsense2 ]]; then
        ok "librealsense2 available via ROS 2 Jazzy"
        return 0
    fi

    # Check if the apt package is already installed
    if dpkg -s librealsense2-dev &>/dev/null; then
        ok "librealsense2-dev already installed"
        return 0
    fi

    info "Installing Intel RealSense SDK..."

    # Add Intel RealSense apt repository
    # https://github.com/IntelRealSense/librealsense/blob/master/doc/distribution_linux.md
    sudo mkdir -p /etc/apt/keyrings
    if [[ ! -f /etc/apt/keyrings/librealsense.pgp ]]; then
        info "Adding Intel RealSense apt key..."
        curl -sSf https://librealsense.intel.com/Debian/librealsense.pgp \
            | sudo tee /etc/apt/keyrings/librealsense.pgp > /dev/null
    fi

    # Determine distribution codename
    local codename="${VERSION_CODENAME:-noble}"
    local repo_line="deb [signed-by=/etc/apt/keyrings/librealsense.pgp] https://librealsense.intel.com/Debian/apt-repo ${codename} main"

    if ! grep -qF "librealsense.intel.com" /etc/apt/sources.list.d/*.list /etc/apt/sources.list.d/*.sources 2>/dev/null; then
        info "Adding Intel RealSense apt repository (${codename})..."
        echo "$repo_line" | sudo tee /etc/apt/sources.list.d/librealsense.list > /dev/null
        sudo apt-get update -qq
    fi

    sudo apt-get install -y librealsense2-dev librealsense2-utils
    ok "Intel RealSense SDK installed"
}

# ---------------------------------------------------------------------------
# 2. Install system dependencies
# ---------------------------------------------------------------------------
install_system_deps() {
    info "Installing system dependencies..."

    local packages=(
        # Build tools
        cmake
        g++
        pkg-config
        git
        curl
        ca-certificates

        # Compression libraries
        libzstd-dev
        libturbojpeg0-dev

        # FFmpeg (H.264 encoding)
        libavcodec-dev
        libavutil-dev
        libswscale-dev
    )

    # GUI dependencies
    if [[ "$WITH_GUI" == "ON" ]]; then
        packages+=(
            libglfw3-dev
            libopengl-dev
        )
    fi

    # Python extension dependencies
    if [[ "$WITH_PYTHON" == "ON" ]]; then
        packages+=(python3-dev)
    fi

    # systemd integration
    packages+=(libsystemd-dev)

    sudo apt-get update -qq
    sudo apt-get install -y "${packages[@]}"
    ok "System dependencies installed"
}

# ---------------------------------------------------------------------------
# 3. Build the project
# ---------------------------------------------------------------------------
build_project() {
    info "Configuring CMake..."

    local cmake_args=(
        -B "$BUILD_DIR"
        -DWITH_GUI="$WITH_GUI"
        -DWITH_PYTHON="$WITH_PYTHON"
        -DBUILD_TESTS="$BUILD_TESTS"
        -DCMAKE_BUILD_TYPE=Release
    )

    # Auto-detect ROS 2 Jazzy realsense2 if system package not available
    if ! pkg-config --exists realsense2 2>/dev/null; then
        if [[ -d /opt/ros/jazzy/lib/x86_64-linux-gnu/cmake/realsense2 ]]; then
            info "Using ROS 2 Jazzy librealsense2"
            cmake_args+=(-DCMAKE_PREFIX_PATH="/opt/ros/jazzy")
        fi
    fi

    cmake "${cmake_args[@]}" "$PROJECT_DIR"

    info "Building with ${NPROC} parallel jobs..."
    cmake --build "$BUILD_DIR" --parallel "$NPROC"

    ok "Build complete: ${BUILD_DIR}/ego-recorder"
}

# ---------------------------------------------------------------------------
# 4. Run tests
# ---------------------------------------------------------------------------
run_tests() {
    if [[ "$BUILD_TESTS" == "ON" ]]; then
        info "Running unit tests..."
        cd "$BUILD_DIR"
        if ctest --output-on-failure --timeout 30; then
            ok "All tests passed"
        else
            warn "Some tests failed (this may be expected without a camera connected)"
        fi
        cd "$PROJECT_DIR"
    fi
}

# ---------------------------------------------------------------------------
# 5. Install Python export dependencies
# ---------------------------------------------------------------------------
install_python_export() {
    if [[ "$INSTALL_PYTHON_EXPORT" != true ]]; then
        return 0
    fi

    info "Installing Python export dependencies..."

    # Ensure pip is available
    if ! command -v pip3 &>/dev/null && ! command -v pip &>/dev/null; then
        sudo apt-get install -y python3-pip python3-venv
    fi

    local pip_cmd="pip3"
    command -v pip3 &>/dev/null || pip_cmd="pip"

    # Use --user only if we're NOT inside a virtualenv.
    # Check VIRTUAL_ENV/CONDA_PREFIX env vars AND ask Python directly
    # (handles venvs activated without sourcing activate).
    local in_venv=false
    if [[ -n "${VIRTUAL_ENV:-}" || -n "${CONDA_PREFIX:-}" ]]; then
        in_venv=true
    elif python3 -c "import sys; exit(0 if sys.prefix != sys.base_prefix else 1)" 2>/dev/null; then
        in_venv=true
    fi

    local pip_flags=()
    if [[ "$in_venv" == false ]]; then
        pip_flags+=(--user)
    fi

    # RLDS export deps
    if [[ -f "${PROJECT_DIR}/python/requirements-rlds.txt" ]]; then
        info "Installing RLDS export dependencies..."
        $pip_cmd install "${pip_flags[@]}" -r "${PROJECT_DIR}/python/requirements-rlds.txt"
        ok "RLDS export dependencies installed"
    fi

    # LeRobot export deps
    if [[ -f "${PROJECT_DIR}/python/requirements-lerobot.txt" ]]; then
        info "Installing LeRobot export dependencies..."
        $pip_cmd install "${pip_flags[@]}" -r "${PROJECT_DIR}/python/requirements-lerobot.txt"
        ok "LeRobot export dependencies installed"
    fi
}

# ---------------------------------------------------------------------------
# 6. Systemd deployment
# ---------------------------------------------------------------------------
deploy_systemd() {
    if [[ "$INSTALL_SYSTEMD" != true ]]; then
        return 0
    fi

    info "Running systemd deployment..."
    if [[ "$(id -u)" -ne 0 ]]; then
        info "Systemd deployment requires root -- invoking with sudo..."
        sudo bash "${PROJECT_DIR}/deploy/install.sh"
    else
        bash "${PROJECT_DIR}/deploy/install.sh"
    fi
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
main() {
    echo ""
    echo -e "${BOLD}═══════════════════════════════════════${NC}"
    echo -e "${BOLD}  ego-recorder setup${NC}"
    echo -e "${BOLD}═══════════════════════════════════════${NC}"
    echo ""

    cd "$PROJECT_DIR"

    install_system_deps
    install_realsense_sdk
    build_project
    run_tests
    install_python_export
    deploy_systemd

    echo ""
    echo -e "${BOLD}═══════════════════════════════════════${NC}"
    echo -e "${GREEN}${BOLD}  Setup complete!${NC}"
    echo -e "${BOLD}═══════════════════════════════════════${NC}"
    echo ""
    echo -e "  Binary:  ${BUILD_DIR}/ego-recorder"
    if [[ "$WITH_PYTHON" == "ON" ]]; then
        echo -e "  Python:  PYTHONPATH=${BUILD_DIR} python3 -c 'import egorec_reader'"
    fi
    echo ""
    echo -e "  ${BOLD}Quick start:${NC}"
    if [[ "$WITH_GUI" == "ON" ]]; then
        echo "    ${BUILD_DIR}/ego-recorder -s my_session -o ./recordings"
    else
        echo "    ${BUILD_DIR}/ego-recorder --headless -o ./recordings -d 300"
    fi
    echo ""
    echo "  See README.md for full usage."
    echo ""
}

main
