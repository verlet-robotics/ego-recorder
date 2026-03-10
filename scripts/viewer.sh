#!/usr/bin/env bash
# viewer.sh -- Launch the egorec viewer, auto-detecting dataset directories.
#
# Checks both the local repo datasets/ dir and /var/lib/ego-recorder.
# If both exist, prompts which to use. Then starts the Bun viewer.
#
# Usage:
#   ./scripts/viewer.sh                  # Auto-detect directory
#   ./scripts/viewer.sh /path/to/dir     # Explicit directory

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
VIEWER_DIR="${PROJECT_DIR}/viewer"

# Colors
BOLD='\033[1m'
CYAN='\033[0;36m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
DIM='\033[2m'
RED='\033[0;31m'
NC='\033[0m'

LOCAL_DIR="${PROJECT_DIR}/datasets"
SYSTEM_DIR="/var/lib/ego-recorder"

# Resolve ego-qc binary
QC_BIN=""
if [[ -f "${PROJECT_DIR}/rust/target/release/ego-qc" ]]; then
    QC_BIN="${PROJECT_DIR}/rust/target/release/ego-qc"
elif command -v ego-qc &>/dev/null; then
    QC_BIN="ego-qc"
fi

# ---------------------------------------------------------------------------
# Determine recordings directory
# ---------------------------------------------------------------------------
recordings_dir=""

if [[ $# -ge 1 ]]; then
    recordings_dir="$1"
else
    has_local=false
    has_system=false

    [[ -d "$LOCAL_DIR" ]] && has_local=true
    [[ -d "$SYSTEM_DIR" ]] && has_system=true

    if [[ "$has_local" == true && "$has_system" == true ]]; then
        echo ""
        echo -e "${BOLD}Multiple dataset directories found:${NC}"
        echo ""
        echo -e "  ${CYAN}1)${NC} ${LOCAL_DIR}"
        echo -e "  ${CYAN}2)${NC} ${SYSTEM_DIR}"
        echo ""
        read -rp "Which directory? [1/2]: " choice
        case "$choice" in
            2) recordings_dir="$SYSTEM_DIR" ;;
            *) recordings_dir="$LOCAL_DIR" ;;
        esac
    elif [[ "$has_local" == true ]]; then
        recordings_dir="$LOCAL_DIR"
    elif [[ "$has_system" == true ]]; then
        recordings_dir="$SYSTEM_DIR"
    else
        echo -e "${RED}No dataset directories found.${NC}"
        echo "  Expected: ${LOCAL_DIR}"
        echo "       or:  ${SYSTEM_DIR}"
        echo ""
        echo "  Usage: ./scripts/viewer.sh /path/to/recordings"
        exit 1
    fi
fi

if [[ ! -d "$recordings_dir" ]]; then
    echo -e "${RED}Directory does not exist: ${recordings_dir}${NC}"
    exit 1
fi

# ---------------------------------------------------------------------------
# Ensure bun deps are installed
# ---------------------------------------------------------------------------
if [[ ! -d "${VIEWER_DIR}/node_modules" ]]; then
    echo -e "${DIM}Installing viewer dependencies...${NC}"
    (cd "$VIEWER_DIR" && bun install)
fi

# ---------------------------------------------------------------------------
# Launch viewer
# ---------------------------------------------------------------------------
echo ""
echo -e "${BOLD}egorec-viewer${NC}"
echo "───────────────────────────────────"
echo -e "  Directory:  ${DIM}${recordings_dir}${NC}"
[[ -n "$QC_BIN" ]] && echo -e "  QC binary:  ${DIM}${QC_BIN}${NC}"
echo ""

cd "$VIEWER_DIR"

qc_args=()
[[ -n "$QC_BIN" ]] && qc_args=(--qc "$QC_BIN")

exec bun run dev -- --dir "$recordings_dir" "${qc_args[@]}"
