#!/usr/bin/env bash
# deploy/install.sh -- System installation script for ego-recorder
#
# Usage: sudo ./install.sh
#
# What this script does:
#   1. Creates the ego-recorder system user (no home, no login shell)
#   2. Adds user to plugdev and video groups for USB device access
#   3. Installs the ego-recorder binary to /usr/local/bin/
#   4. Installs the config file to /etc/ego-recorder/config.toml (preserves existing)
#   5. Installs the systemd unit file and enables the service (does not start)
#   6. Installs udev rules to disable USB autosuspend for RealSense cameras
#   7. Installs logind.conf drop-in as fallback lid-close prevention
#
# Requirements: Run as root. Binary must be compiled and present in the same
# directory as this script (or the parent directory if run from deploy/).

set -euo pipefail

# ---------------------------------------------------------------------------
# Check root
# ---------------------------------------------------------------------------
if [ "$(id -u)" -ne 0 ]; then
    echo "Error: install.sh must be run as root." >&2
    echo "Usage: sudo ./install.sh" >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Resolve paths
# ---------------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Binary: look in parent directory first (project root after build), then cwd.
if [ -f "${SCRIPT_DIR}/../build/ego-recorder" ]; then
    BINARY_SRC="${SCRIPT_DIR}/../build/ego-recorder"
elif [ -f "${SCRIPT_DIR}/ego-recorder" ]; then
    BINARY_SRC="${SCRIPT_DIR}/ego-recorder"
else
    echo "Error: ego-recorder binary not found." >&2
    echo "Build the project first: cmake -B build . && cmake --build build -j\$(nproc)" >&2
    exit 1
fi

BINARY_DEST=/usr/local/bin/ego-recorder
CONF_DIR=/etc/ego-recorder
SYSTEMD_DIR=/etc/systemd/system
UDEV_DIR=/etc/udev/rules.d
LOGIND_DIR=/etc/systemd/logind.conf.d

# ---------------------------------------------------------------------------
# 1. Create system user
# ---------------------------------------------------------------------------
echo "[install] Creating system user 'ego-recorder'..."
if id ego-recorder &>/dev/null; then
    echo "[install]   User already exists -- skipping useradd."
else
    useradd \
        --system \
        --no-create-home \
        --home-dir /dev/null \
        --shell /usr/sbin/nologin \
        --comment "ego-recorder service account" \
        ego-recorder
    echo "[install]   User created."
fi

# ---------------------------------------------------------------------------
# 2. Add to groups for USB and video device access
# ---------------------------------------------------------------------------
echo "[install] Adding ego-recorder to plugdev and video groups..."
usermod -aG plugdev ego-recorder
usermod -aG video ego-recorder

# ---------------------------------------------------------------------------
# 3. Install binary
# ---------------------------------------------------------------------------
echo "[install] Installing binary to ${BINARY_DEST}..."
install -m 755 "${BINARY_SRC}" "${BINARY_DEST}"

# ---------------------------------------------------------------------------
# 4. Install config (only if not already present -- preserve user edits)
# ---------------------------------------------------------------------------
echo "[install] Installing config to ${CONF_DIR}/config.toml..."
mkdir -p "${CONF_DIR}"
if [ -f "${CONF_DIR}/config.toml" ]; then
    echo "[install]   config.toml already exists -- skipping to preserve user edits."
    echo "[install]   Updated example config saved as ${CONF_DIR}/config.toml.example"
    install -m 644 "${SCRIPT_DIR}/config.toml.example" "${CONF_DIR}/config.toml.example"
else
    install -m 644 "${SCRIPT_DIR}/config.toml.example" "${CONF_DIR}/config.toml"
    echo "[install]   Config installed. Edit ${CONF_DIR}/config.toml before starting."
fi

# ---------------------------------------------------------------------------
# 5. Install systemd unit file and reload
# ---------------------------------------------------------------------------
echo "[install] Installing systemd unit file..."
install -m 644 "${SCRIPT_DIR}/ego-recorder.service" "${SYSTEMD_DIR}/ego-recorder.service"
systemctl daemon-reload
echo "[install]   Service installed. Enable with: systemctl enable --now ego-recorder.service"

# ---------------------------------------------------------------------------
# 6. Install udev rules (disable USB autosuspend for RealSense cameras)
# ---------------------------------------------------------------------------
echo "[install] Installing udev rules..."
install -m 644 "${SCRIPT_DIR}/99-ego-recorder.rules" "${UDEV_DIR}/99-ego-recorder.rules"
udevadm control --reload-rules
udevadm trigger
echo "[install]   udev rules installed and reloaded."

# ---------------------------------------------------------------------------
# 7. Install logind.conf drop-in (fallback lid-close prevention)
# ---------------------------------------------------------------------------
echo "[install] Installing logind.conf drop-in (fallback lid-close prevention)..."
mkdir -p "${LOGIND_DIR}"
install -m 644 "${SCRIPT_DIR}/50-ego-recorder-lid.conf" "${LOGIND_DIR}/50-ego-recorder-lid.conf"
echo "[install]   logind drop-in installed (takes effect on next reboot or: systemctl restart systemd-logind)"

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo ""
echo "================================================================"
echo " ego-recorder installation complete"
echo "================================================================"
echo ""
echo " Binary:      ${BINARY_DEST}"
echo " Config:      ${CONF_DIR}/config.toml"
echo " Service:     ${SYSTEMD_DIR}/ego-recorder.service"
echo " udev rules:  ${UDEV_DIR}/99-ego-recorder.rules"
echo " logind:      ${LOGIND_DIR}/50-ego-recorder-lid.conf"
echo ""
echo " Next steps:"
echo "   1. Edit ${CONF_DIR}/config.toml (set output.dir to your recording path)"
echo "   2. Ensure the output directory exists and is writable"
echo "   3. Plug in the RealSense camera"
echo "   4. Enable and start:  systemctl enable --now ego-recorder.service"
echo "   5. Check status:      systemctl status ego-recorder.service"
echo "   6. Follow logs:       journalctl -fu ego-recorder.service"
echo ""
