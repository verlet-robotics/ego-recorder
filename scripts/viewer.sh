#!/usr/bin/env bash
# viewer.sh -- Launch the egorec viewer app (Tauri desktop), auto-detecting dataset directories.
#
# Checks both the local repo datasets/ dir and /var/lib/ego-recorder.
# If both exist, prompts which to use. Passes --dir to the viewer for instant loading.
#
# Usage:
#   ./scripts/viewer.sh                  # Auto-detect directory
#   ./scripts/viewer.sh /path/to/dir      # Explicit directory
#   ./scripts/viewer.sh --workspace /path/to/curation-workspace

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
VIEWER_DIR="${PROJECT_DIR}/viewer-app"

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

# Ensure Bun is in PATH (from setup-station install)
if [[ -d "${HOME}/.bun/bin" ]]; then
    export PATH="${HOME}/.bun/bin:${PATH}"
fi

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
recordings_dir=""
workspace_dir=""
qc_bin=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --workspace)
            workspace_dir="$2"
            shift 2
            ;;
        --qc)
            qc_bin="$2"
            shift 2
            ;;
        -*)
            echo -e "${RED}Unknown option: $1${NC}"
            echo "Usage: ./scripts/viewer.sh [--workspace PATH] [--qc PATH] [DIR]"
            exit 1
            ;;
        *)
            recordings_dir="$1"
            shift
            ;;
    esac
done

# Resolve ego-qc binary
if [[ -z "$qc_bin" ]]; then
    if [[ -f "${PROJECT_DIR}/rust/target/release/ego-qc" ]]; then
        qc_bin="${PROJECT_DIR}/rust/target/release/ego-qc"
    elif command -v ego-qc &>/dev/null; then
        qc_bin="ego-qc"
    fi
fi

# ---------------------------------------------------------------------------
# Determine recordings directory (only when neither dir nor workspace was passed)
# ---------------------------------------------------------------------------
if [[ -z "$recordings_dir" && -z "$workspace_dir" ]]; then
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

# Validate paths
if [[ -n "$recordings_dir" && ! -d "$recordings_dir" ]]; then
    echo -e "${RED}Directory does not exist: ${recordings_dir}${NC}"
    exit 1
fi
if [[ -n "$workspace_dir" && ! -d "$workspace_dir" ]]; then
    echo -e "${RED}Workspace does not exist: ${workspace_dir}${NC}"
    exit 1
fi

# ---------------------------------------------------------------------------
# Check viewer-app exists
# ---------------------------------------------------------------------------
if [[ ! -d "$VIEWER_DIR" ]]; then
    echo -e "${RED}viewer-app not found at ${VIEWER_DIR}${NC}"
    echo "  Run ./scripts/setup-station.sh first (without --no-viewer)"
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
# Launch viewer (Tauri desktop app)
# ---------------------------------------------------------------------------
echo ""
echo -e "${BOLD}egorec-viewer${NC}"
echo "───────────────────────────────────"
[[ -n "$recordings_dir" ]] && echo -e "  Directory:  ${DIM}${recordings_dir}${NC}"
[[ -n "$workspace_dir" ]] && echo -e "  Workspace:  ${DIM}${workspace_dir}${NC}"
[[ -n "$qc_bin" ]] && echo -e "  QC binary:  ${DIM}${qc_bin}${NC}"
echo ""

cd "$VIEWER_DIR"

app_args=()
[[ -n "$recordings_dir" ]] && app_args+=(--dir "$recordings_dir")
[[ -n "$workspace_dir" ]] && app_args+=(--workspace "$workspace_dir")
[[ -n "$qc_bin" ]] && app_args+=(--qc "$qc_bin")

exec bun run tauri dev -- "${app_args[@]}"
