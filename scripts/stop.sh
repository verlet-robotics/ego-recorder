#!/usr/bin/env bash
# stop.sh -- Stop and disable the ego-recorder pipeline services.
#
# Reverses what setup-pipeline.sh enables: stops both services and disables them
# from starting on boot. Does NOT uninstall binaries, configs, or data.
#
# Usage:
#   sudo ./scripts/stop.sh              # Stop and disable both services
#   sudo ./scripts/stop.sh --purge      # Also remove installed files and user

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

info()  { echo -e "${CYAN}[stop]${NC} $*"; }
ok()    { echo -e "${GREEN}[stop]${NC} $*"; }
warn()  { echo -e "${YELLOW}[stop]${NC} $*"; }
err()   { echo -e "${RED}[stop]${NC} $*" >&2; }

DO_PURGE=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --purge)  DO_PURGE=true; shift ;;
        --help|-h)
            echo "Usage: sudo ./scripts/stop.sh [--purge]"
            echo ""
            echo "  (default)   Stop and disable services (keeps files + data)"
            echo "  --purge     Also remove installed binaries, configs, venv, and user"
            echo "              (recordings in /var/lib/ego-recorder are NEVER deleted)"
            exit 0 ;;
        *)  err "Unknown option: $1"; exit 1 ;;
    esac
done

if [[ "$(id -u)" -ne 0 ]]; then
    err "This script must be run as root."
    echo "Usage: sudo ./scripts/stop.sh" >&2
    exit 1
fi

echo ""
echo -e "${BOLD}═══════════════════════════════════════════════════${NC}"
echo -e "${BOLD}  ego-recorder pipeline stop${NC}"
echo -e "${BOLD}═══════════════════════════════════════════════════${NC}"
echo ""

# ---------------------------------------------------------------------------
# 1. Stop services
# ---------------------------------------------------------------------------
for svc in ego-uploader ego-recorder; do
    if systemctl is-active --quiet "${svc}.service" 2>/dev/null; then
        info "Stopping ${svc}.service..."
        systemctl stop "${svc}.service"
        ok "${svc}.service stopped."
    else
        info "${svc}.service is not running."
    fi
done

# ---------------------------------------------------------------------------
# 2. Disable services (remove from boot)
# ---------------------------------------------------------------------------
for svc in ego-uploader ego-recorder; do
    if systemctl is-enabled --quiet "${svc}.service" 2>/dev/null; then
        info "Disabling ${svc}.service..."
        systemctl disable "${svc}.service"
        ok "${svc}.service disabled."
    else
        info "${svc}.service is not enabled."
    fi
done

# ---------------------------------------------------------------------------
# 3. Purge (optional)
# ---------------------------------------------------------------------------
if [[ "$DO_PURGE" == true ]]; then
    echo ""
    echo -e "${BOLD}── Purging installed files ──${NC}"
    echo ""

    # Service files
    for f in /etc/systemd/system/ego-recorder.service /etc/systemd/system/ego-uploader.service; do
        if [[ -f "$f" ]]; then
            info "Removing $f"
            rm -f "$f"
        fi
    done
    systemctl daemon-reload

    # Binaries
    if [[ -f /usr/local/bin/ego-recorder ]]; then
        info "Removing /usr/local/bin/ego-recorder"
        rm -f /usr/local/bin/ego-recorder
    fi
    if [[ -f /usr/local/bin/ego-qc ]]; then
        info "Removing /usr/local/bin/ego-qc"
        rm -f /usr/local/bin/ego-qc
    fi

    # Uploader venv + scripts
    if [[ -d /opt/ego-uploader ]]; then
        info "Removing /opt/ego-uploader/"
        rm -rf /opt/ego-uploader
    fi

    # Config dir (but NOT recordings)
    if [[ -d /etc/ego-recorder ]]; then
        info "Removing /etc/ego-recorder/"
        rm -rf /etc/ego-recorder
    fi

    # udev rules
    if [[ -f /etc/udev/rules.d/99-ego-recorder.rules ]]; then
        info "Removing udev rules"
        rm -f /etc/udev/rules.d/99-ego-recorder.rules
        udevadm control --reload-rules 2>/dev/null || true
    fi

    # logind drop-in
    if [[ -f /etc/systemd/logind.conf.d/50-ego-recorder-lid.conf ]]; then
        info "Removing logind drop-in"
        rm -f /etc/systemd/logind.conf.d/50-ego-recorder-lid.conf
    fi

    # System user
    if id ego-recorder &>/dev/null; then
        info "Removing system user 'ego-recorder'"
        userdel ego-recorder 2>/dev/null || true
    fi

    ok "Purge complete."
    echo ""
    warn "Recordings at /var/lib/ego-recorder/ were NOT deleted."
    warn "Remove them manually if needed: sudo rm -rf /var/lib/ego-recorder"
fi

echo ""
echo -e "${GREEN}${BOLD}  Services stopped and disabled.${NC}"
echo ""
echo "  To re-enable:  sudo ./scripts/setup-pipeline.sh"
echo "  To start once:  sudo systemctl start ego-recorder ego-uploader"
echo ""
