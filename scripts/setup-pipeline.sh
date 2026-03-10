#!/usr/bin/env bash
# setup-pipeline.sh -- One-shot setup for the full ego-recorder pipeline.
#
# Installs and enables two systemd services that start on every boot:
#   1. ego-recorder.service  -- headless RGBD capture from RealSense camera
#   2. ego-uploader.service  -- background R2 cloud sync of recorded episodes
#
# Usage:
#   sudo ./scripts/setup-pipeline.sh                   # Full install (build + deploy both services)
#   sudo ./scripts/setup-pipeline.sh --no-build        # Skip build (binary already compiled)
#   sudo ./scripts/setup-pipeline.sh --no-upload       # Skip uploader service setup
#   sudo ./scripts/setup-pipeline.sh --recorder-only   # Alias for --no-upload
#
# Prerequisites (installed automatically):
#   - Ubuntu 22.04 / 24.04
#   - Intel RealSense camera
#   - Internet connection (for apt packages + pip)

set -euo pipefail

# ---------------------------------------------------------------------------
# Colors
# ---------------------------------------------------------------------------
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

info()  { echo -e "${CYAN}[setup]${NC} $*"; }
ok()    { echo -e "${GREEN}[setup]${NC} $*"; }
warn()  { echo -e "${YELLOW}[setup]${NC} $*"; }
err()   { echo -e "${RED}[setup]${NC} $*" >&2; }

# ---------------------------------------------------------------------------
# Paths
# ---------------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RECORDER_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
source "${SCRIPT_DIR}/lib-env.sh"
BUILD_DIR="${RECORDER_DIR}/build"
NPROC=$(nproc 2>/dev/null || echo 4)

# Install locations
BINARY_DEST=/usr/local/bin/ego-recorder
CONF_DIR=/etc/ego-recorder
UPLOADER_INSTALL_DIR=/opt/ego-uploader
SYSTEMD_DIR=/etc/systemd/system
UDEV_DIR=/etc/udev/rules.d
LOGIND_DIR=/etc/systemd/logind.conf.d
DATA_DIR=/var/lib/ego-recorder/recordings

# Options
DO_BUILD=true
DO_UPLOAD=true

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
usage() {
    cat <<'EOF'
Usage: sudo ./scripts/setup-pipeline.sh [OPTIONS]

Sets up the full ego-recorder pipeline (recorder + cloud uploader) as
systemd services that start automatically on every boot.

Options:
  --no-build        Skip building ego-recorder (use existing binary)
  --no-upload       Skip uploader service (recorder only)
  --recorder-only   Alias for --no-upload
  --help            Show this help
EOF
    exit 0
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --no-build)       DO_BUILD=false; shift ;;
        --no-upload|--recorder-only) DO_UPLOAD=false; shift ;;
        --help|-h)        usage ;;
        *)                err "Unknown option: $1"; usage ;;
    esac
done

# ---------------------------------------------------------------------------
# Root check
# ---------------------------------------------------------------------------
if [[ "$(id -u)" -ne 0 ]]; then
    err "This script must be run as root."
    echo "Usage: sudo ./scripts/setup-pipeline.sh" >&2
    exit 1
fi

# Who invoked sudo? We need the real user for file ownership hints.
REAL_USER="${SUDO_USER:-$(whoami)}"

echo ""
echo -e "${BOLD}═══════════════════════════════════════════════════${NC}"
echo -e "${BOLD}  ego-recorder pipeline setup${NC}"
echo -e "${BOLD}═══════════════════════════════════════════════════${NC}"
echo ""

# ---------------------------------------------------------------------------
# Detect OS
# ---------------------------------------------------------------------------
if [[ ! -f /etc/os-release ]]; then
    err "Cannot detect OS. This script supports Ubuntu 22.04 / 24.04."
    exit 1
fi
source /etc/os-release
info "Detected: $PRETTY_NAME"

# ===================================================================
# PART 1: ego-recorder (build + systemd service)
# ===================================================================

echo ""
echo -e "${BOLD}── Part 1: ego-recorder ──${NC}"
echo ""

# ---------------------------------------------------------------------------
# 1a. System dependencies
# ---------------------------------------------------------------------------
info "Installing system dependencies..."
apt-get update -qq

PACKAGES=(
    cmake g++ pkg-config git curl ca-certificates
    libzstd-dev libturbojpeg0-dev
    libavcodec-dev libavutil-dev libswscale-dev
    libsystemd-dev
    espeak-ng
    python3 python3-pip python3-venv
)
apt-get install -y "${PACKAGES[@]}"
ok "System dependencies installed."

# ---------------------------------------------------------------------------
# 1b. Intel RealSense SDK
# ---------------------------------------------------------------------------
install_realsense_sdk() {
    if pkg-config --exists realsense2 2>/dev/null; then
        ok "librealsense2 already available ($(pkg-config --modversion realsense2))"
        return 0
    fi
    if [[ -d /opt/ros/jazzy/lib/x86_64-linux-gnu/cmake/realsense2 ]]; then
        ok "librealsense2 available via ROS 2 Jazzy"
        local ros_lib="/opt/ros/jazzy/lib/x86_64-linux-gnu"
        if ! ldconfig -p 2>/dev/null | grep -q librealsense2; then
            echo "$ros_lib" | tee /etc/ld.so.conf.d/ros-jazzy.conf > /dev/null
            ldconfig
        fi
        return 0
    fi
    if dpkg -s librealsense2-dev &>/dev/null; then
        ok "librealsense2-dev already installed"
        return 0
    fi

    info "Installing Intel RealSense SDK..."
    mkdir -p /etc/apt/keyrings
    if [[ ! -f /etc/apt/keyrings/librealsense.pgp ]]; then
        curl -sSf https://librealsense.intel.com/Debian/librealsense.pgp \
            | tee /etc/apt/keyrings/librealsense.pgp > /dev/null
    fi
    local codename="${VERSION_CODENAME:-noble}"
    local repo_line="deb [signed-by=/etc/apt/keyrings/librealsense.pgp] https://librealsense.intel.com/Debian/apt-repo ${codename} main"
    if ! grep -qF "librealsense.intel.com" /etc/apt/sources.list.d/*.list /etc/apt/sources.list.d/*.sources 2>/dev/null; then
        echo "$repo_line" | tee /etc/apt/sources.list.d/librealsense.list > /dev/null
        apt-get update -qq
    fi
    apt-get install -y librealsense2-dev librealsense2-utils
    ok "Intel RealSense SDK installed."
}
install_realsense_sdk

# ---------------------------------------------------------------------------
# 1c. Build ego-recorder
# ---------------------------------------------------------------------------
if [[ "$DO_BUILD" == true ]]; then
    info "Building ego-recorder (headless, release)..."

    CMAKE_ARGS=(
        -B "$BUILD_DIR"
        -DWITH_GUI=OFF
        -DWITH_PYTHON=OFF
        -DBUILD_TESTS=OFF
        -DWITH_RUST_EXPORT=OFF
        -DCMAKE_BUILD_TYPE=Release
    )

    # Auto-detect ROS 2 Jazzy
    if ! pkg-config --exists realsense2 2>/dev/null; then
        if [[ -d /opt/ros/jazzy/lib/x86_64-linux-gnu/cmake/realsense2 ]]; then
            CMAKE_ARGS+=(-DCMAKE_PREFIX_PATH="/opt/ros/jazzy")
        fi
    fi

    cmake "${CMAKE_ARGS[@]}" "$RECORDER_DIR"
    cmake --build "$BUILD_DIR" --parallel "$NPROC"
    ok "Build complete: ${BUILD_DIR}/ego-recorder"
fi

# Verify binary exists
if [[ -f "${BUILD_DIR}/ego-recorder" ]]; then
    BINARY_SRC="${BUILD_DIR}/ego-recorder"
elif [[ -f "${BINARY_DEST}" ]]; then
    info "Using existing installed binary at ${BINARY_DEST}"
    BINARY_SRC=""
else
    err "ego-recorder binary not found. Run without --no-build first."
    exit 1
fi

# ---------------------------------------------------------------------------
# 1c-2. Build ego-qc (Rust QC tools for on-device data pruning)
# ---------------------------------------------------------------------------
QC_BINARY_DEST=/usr/local/bin/ego-qc
RUST_DIR="${RECORDER_DIR}/rust"

build_ego_qc() {
    info "Building ego-qc (Rust QC tools)..."

    # Install Rust toolchain for the real user if not present
    local cargo_bin=""
    if [[ -n "${REAL_USER}" && "${REAL_USER}" != "root" ]]; then
        local user_home
        user_home=$(eval echo "~${REAL_USER}")
        cargo_bin="${user_home}/.cargo/bin/cargo"
    fi

    if [[ -n "${cargo_bin}" && -x "${cargo_bin}" ]]; then
        info "Using Rust toolchain from ${REAL_USER}"
        sudo -u "${REAL_USER}" "${cargo_bin}" build --release \
            --manifest-path "${RUST_DIR}/Cargo.toml" -p ego-qc
    elif command -v cargo &>/dev/null; then
        cargo build --release --manifest-path "${RUST_DIR}/Cargo.toml" -p ego-qc
    else
        info "Installing Rust toolchain via rustup for ${REAL_USER}..."
        sudo -u "${REAL_USER}" bash -c \
            'curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal'
        local user_home
        user_home=$(eval echo "~${REAL_USER}")
        cargo_bin="${user_home}/.cargo/bin/cargo"
        sudo -u "${REAL_USER}" "${cargo_bin}" build --release \
            --manifest-path "${RUST_DIR}/Cargo.toml" -p ego-qc
    fi

    ok "ego-qc build complete"
}

if [[ "$DO_BUILD" == true ]]; then
    build_ego_qc
fi

# Install ego-qc binary
if [[ -f "${RUST_DIR}/target/release/ego-qc" ]]; then
    info "Installing ego-qc to ${QC_BINARY_DEST}..."
    install -m 755 "${RUST_DIR}/target/release/ego-qc" "${QC_BINARY_DEST}"
elif [[ -f "${QC_BINARY_DEST}" ]]; then
    info "Using existing ego-qc at ${QC_BINARY_DEST}"
else
    warn "ego-qc binary not found -- QC tools will not be available."
fi

# ---------------------------------------------------------------------------
# 1d. Create system user
# ---------------------------------------------------------------------------
info "Creating system user 'ego-recorder'..."
if id ego-recorder &>/dev/null; then
    info "  User already exists."
else
    useradd --system --no-create-home --home-dir /dev/null \
            --shell /usr/sbin/nologin \
            --comment "ego-recorder service account" ego-recorder
    ok "  User created."
fi
usermod -aG plugdev ego-recorder 2>/dev/null || true
usermod -aG video ego-recorder 2>/dev/null || true

# ---------------------------------------------------------------------------
# 1e. Create data directory
# ---------------------------------------------------------------------------
info "Creating data directory: ${DATA_DIR}"
mkdir -p "${DATA_DIR}"
chown ego-recorder:ego-recorder "${DATA_DIR}"
chmod 750 "${DATA_DIR}"

# ---------------------------------------------------------------------------
# 1f. Install binary
# ---------------------------------------------------------------------------
if [[ -n "${BINARY_SRC:-}" ]]; then
    info "Installing binary to ${BINARY_DEST}..."
    install -m 755 "${BINARY_SRC}" "${BINARY_DEST}"
fi

# ---------------------------------------------------------------------------
# 1g. Install config
# ---------------------------------------------------------------------------
info "Installing recorder config..."
mkdir -p "${CONF_DIR}"
if [[ -f "${CONF_DIR}/config.toml" ]]; then
    info "  config.toml already exists -- preserving."
else
    install -m 644 "${RECORDER_DIR}/deploy/config.toml.example" "${CONF_DIR}/config.toml"
    ok "  Config installed at ${CONF_DIR}/config.toml"
fi

# ---------------------------------------------------------------------------
# 1h. Install udev rules
# ---------------------------------------------------------------------------
info "Installing udev rules..."
install -m 644 "${RECORDER_DIR}/deploy/99-ego-recorder.rules" "${UDEV_DIR}/99-ego-recorder.rules"
udevadm control --reload-rules
udevadm trigger
ok "  udev rules installed."

# ---------------------------------------------------------------------------
# 1i. Install logind drop-in
# ---------------------------------------------------------------------------
info "Installing logind lid-close prevention..."
mkdir -p "${LOGIND_DIR}"
install -m 644 "${RECORDER_DIR}/deploy/50-ego-recorder-lid.conf" "${LOGIND_DIR}/50-ego-recorder-lid.conf"

# ---------------------------------------------------------------------------
# 1j. Install ego-recorder systemd service
# ---------------------------------------------------------------------------
info "Installing ego-recorder.service..."
install -m 644 "${RECORDER_DIR}/deploy/ego-recorder.service" "${SYSTEMD_DIR}/ego-recorder.service"

# ===================================================================
# PART 2: ego-uploader (Python R2 sync service)
# ===================================================================

if [[ "$DO_UPLOAD" == true ]]; then
    echo ""
    echo -e "${BOLD}── Part 2: ego-uploader (R2 cloud sync) ──${NC}"
    echo ""

    # ------------------------------------------------------------------
    # 2a. Install uploader files
    # ------------------------------------------------------------------
    info "Installing uploader to ${UPLOADER_INSTALL_DIR}..."
    mkdir -p "${UPLOADER_INSTALL_DIR}"
    install -m 644 "${RECORDER_DIR}/python/ego_uploader.py" "${UPLOADER_INSTALL_DIR}/ego_uploader.py"
    install -m 644 "${RECORDER_DIR}/python/egorec_header.py" "${UPLOADER_INSTALL_DIR}/egorec_header.py"
    install -m 644 "${RECORDER_DIR}/python/requirements-uploader.txt" "${UPLOADER_INSTALL_DIR}/requirements.txt"

    # Install upload config (preserve existing)
    if [[ -f "${CONF_DIR}/upload_config.toml" ]]; then
        info "  upload_config.toml already exists -- preserving."
    else
        install -m 644 "${RECORDER_DIR}/deploy/upload_config.toml" "${CONF_DIR}/upload_config.toml"
        # Patch episodes_dir to point to actual data dir
        sed -i "s|episodes_dir = .*|episodes_dir = \"${DATA_DIR}\"|" "${CONF_DIR}/upload_config.toml"
        ok "  Upload config installed at ${CONF_DIR}/upload_config.toml"
    fi

    # ------------------------------------------------------------------
    # 2b. Python virtualenv + dependencies
    # ------------------------------------------------------------------
    info "Setting up Python virtualenv..."
    python3 -m venv "${UPLOADER_INSTALL_DIR}/venv"
    "${UPLOADER_INSTALL_DIR}/venv/bin/pip" install --upgrade pip -q
    "${UPLOADER_INSTALL_DIR}/venv/bin/pip" install -r "${UPLOADER_INSTALL_DIR}/requirements.txt" -q
    ok "Python dependencies installed."

    # ------------------------------------------------------------------
    # 2c. R2 credentials (.env)
    # ------------------------------------------------------------------
    ENV_FILE="${CONF_DIR}/.env"
    prompt_r2_credentials "${ENV_FILE}" || true

    # ------------------------------------------------------------------
    # 2c-2. Facility API connection (with auto-detection)
    # ------------------------------------------------------------------
    setup_facility "${ENV_FILE}" "${CONF_DIR}/upload_config.toml" || true

    # ------------------------------------------------------------------
    # 2d. Install ego-uploader systemd service
    # ------------------------------------------------------------------
    info "Installing ego-uploader.service..."
    install -m 644 "${RECORDER_DIR}/deploy/ego-uploader.service" "${SYSTEMD_DIR}/ego-uploader.service"
    ok "ego-uploader.service installed."
fi

# ===================================================================
# PART 3: Enable and start services
# ===================================================================

echo ""
echo -e "${BOLD}── Part 3: Enabling services ──${NC}"
echo ""

systemctl daemon-reload

# Enable ego-recorder (starts on boot, waits for camera hotplug)
info "Enabling ego-recorder.service..."
systemctl enable ego-recorder.service
ok "ego-recorder.service enabled."

if [[ "$DO_UPLOAD" == true ]]; then
    info "Enabling ego-uploader.service..."
    systemctl enable ego-uploader.service
    ok "ego-uploader.service enabled."
fi

# Start services now
info "Starting ego-recorder.service..."
systemctl start ego-recorder.service || warn "ego-recorder.service failed to start (camera may not be plugged in -- it will auto-start when detected)."

if [[ "$DO_UPLOAD" == true ]]; then
    info "Starting ego-uploader.service..."
    systemctl start ego-uploader.service || warn "ego-uploader.service failed to start (check R2 credentials)."
fi

# ===================================================================
# Summary
# ===================================================================

echo ""
echo -e "${BOLD}═══════════════════════════════════════════════════${NC}"
echo -e "${GREEN}${BOLD}  Setup complete!${NC}"
echo -e "${BOLD}═══════════════════════════════════════════════════${NC}"
echo ""
echo -e "  ${BOLD}Services:${NC}"
echo "    ego-recorder.service   -- headless RGBD capture (starts on boot)"
if [[ "$DO_UPLOAD" == true ]]; then
echo "    ego-uploader.service   -- R2 cloud sync (starts on boot)"
fi
echo ""
echo -e "  ${BOLD}Files:${NC}"
echo "    Recorder:       ${BINARY_DEST}"
echo "    QC tools:       ${QC_BINARY_DEST}"
echo "    Recorder config:${CONF_DIR}/config.toml"
echo "    Recordings:     ${DATA_DIR}"
if [[ "$DO_UPLOAD" == true ]]; then
echo "    Upload config:  ${CONF_DIR}/upload_config.toml"
echo "    R2 credentials: ${CONF_DIR}/.env"
echo "    Uploader:       ${UPLOADER_INSTALL_DIR}/"
fi
echo ""
echo -e "  ${BOLD}Commands:${NC}"
echo "    Status:       systemctl status ego-recorder ego-uploader"
echo "    Logs:         journalctl -fu ego-recorder"
echo "                  journalctl -fu ego-uploader"
echo "    Stop:         sudo systemctl stop ego-recorder ego-uploader"
echo "    Disable:      sudo systemctl disable ego-recorder ego-uploader"
if [[ "$DO_UPLOAD" == true ]]; then
echo "    Edit R2 creds: sudo nano ${CONF_DIR}/.env && sudo systemctl restart ego-uploader"
fi
echo ""
echo "  Both services are now enabled and will start automatically on every boot."
echo ""
