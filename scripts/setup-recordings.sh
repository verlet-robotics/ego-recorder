#!/usr/bin/env bash
# setup-recordings.sh -- Interactive recordings directory setup
#
# Creates a recording directory with correct permissions, estimates disk usage,
# and optionally initializes a dataset manifest.
#
# Usage:
#   ./scripts/setup-recordings.sh                  # fully interactive
#   ./scripts/setup-recordings.sh -o ~/recordings  # pre-set directory

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

info()  { echo -e "${CYAN}[recordings]${NC} $*"; }
ok()    { echo -e "${GREEN}[recordings]${NC} $*"; }
warn()  { echo -e "${YELLOW}[recordings]${NC} $*"; }
err()   { echo -e "${RED}[recordings]${NC} $*" >&2; }

# ---------------------------------------------------------------------------
# Resolve paths
# ---------------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

# Find the ego-recorder binary
EGO_BIN=""
EGO_BIN_SHORT=""
for candidate in \
    "${PROJECT_DIR}/build/ego-recorder" \
    "$(command -v ego-recorder 2>/dev/null || true)"; do
    if [[ -x "$candidate" ]]; then
        EGO_BIN="$candidate"
        break
    fi
done

# Short display name for the binary
if command -v ego-recorder &>/dev/null; then
    EGO_BIN_SHORT="ego-recorder"
elif [[ -n "$EGO_BIN" ]]; then
    EGO_BIN_SHORT="./build/ego-recorder"
fi

# Shorten a path for display: use ~ for HOME, or relative to cwd
pretty_path() {
    local p="$1"
    # Try relative to cwd first
    local rel
    rel=$(python3 -c "import os; print(os.path.relpath('$p'))" 2>/dev/null || true)
    if [[ -n "$rel" && "${#rel}" -lt "${#p}" && "$rel" != ../* ]]; then
        echo "./$rel"
        return
    fi
    # Replace HOME with ~
    echo "${p/#$HOME/\~}"
}

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
OUTPUT_DIR=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        -o|--output) OUTPUT_DIR="$2"; shift 2 ;;
        --help|-h)
            echo "Usage: ./scripts/setup-recordings.sh [-o DIR]"
            echo ""
            echo "Interactive recording directory setup."
            echo "Optionally pass -o DIR to pre-set the directory path."
            exit 0 ;;
        *)
            err "Unknown option: $1"
            exit 1 ;;
    esac
done

# ---------------------------------------------------------------------------
# Interactive prompts
# ---------------------------------------------------------------------------
echo ""
echo -e "${BOLD}Set up recordings directory${NC}"
echo "───────────────────────────────────────"
echo ""

# Dataset name first (needed to build default directory)
while true; do
    read -rp "Dataset name (e.g. kitchen-pick): " DATASET_NAME
    if [[ -n "$DATASET_NAME" ]]; then
        break
    fi
    warn "Name is required."
done

# Directory -- default to ./datasets/<name>
if [[ -z "$OUTPUT_DIR" ]]; then
    default_dir="./datasets/${DATASET_NAME}"
    read -rp "Recording directory [${default_dir}]: " OUTPUT_DIR
    OUTPUT_DIR="${OUTPUT_DIR:-$default_dir}"
fi

# Expand ~ and resolve
OUTPUT_DIR="${OUTPUT_DIR/#\~/$HOME}"

# ---------------------------------------------------------------------------
# Check if this is for a systemd service (needs different ownership)
# ---------------------------------------------------------------------------
USE_SYSTEMD=false
echo ""
read -rp "Is this for the systemd headless service? [y/N] " ans
if [[ "$ans" =~ ^[Yy] ]]; then
    USE_SYSTEMD=true
    # Default to the standard production path
    if [[ "$OUTPUT_DIR" == "./datasets/${DATASET_NAME}" || "$OUTPUT_DIR" == "datasets/${DATASET_NAME}" ]]; then
        OUTPUT_DIR="/var/lib/ego-recorder/${DATASET_NAME}"
        info "Using default service path: ${OUTPUT_DIR}"
    fi
fi

# ---------------------------------------------------------------------------
# Create the directory
# ---------------------------------------------------------------------------
echo ""

if [[ "$USE_SYSTEMD" == true ]]; then
    info "Creating ${OUTPUT_DIR} (requires sudo)..."
    sudo mkdir -p "$OUTPUT_DIR"

    # Ensure ego-recorder user exists
    if id ego-recorder &>/dev/null; then
        sudo chown ego-recorder:ego-recorder "$OUTPUT_DIR"
        ok "Directory owned by ego-recorder service user"
    else
        warn "ego-recorder system user does not exist yet."
        warn "Run the systemd deployment first: sudo bash deploy/install.sh"
        warn "Then re-run this script to fix ownership."
    fi

    # Grant current user access so they can also run headless directly
    CURRENT_USER="${SUDO_USER:-$USER}"
    if [[ "$CURRENT_USER" != "root" && "$CURRENT_USER" != "ego-recorder" ]]; then
        info "Granting ${CURRENT_USER} write access..."
        sudo setfacl -R -m "u:${CURRENT_USER}:rwx" "$OUTPUT_DIR"
        sudo setfacl -R -d -m "u:${CURRENT_USER}:rwx" "$OUTPUT_DIR"
        ok "Both ${CURRENT_USER} and ego-recorder service can write here"
    fi
else
    mkdir -p "$OUTPUT_DIR"
fi

# Resolve to absolute path after creation
OUTPUT_DIR="$(cd "$OUTPUT_DIR" && pwd)"

ok "Directory ready: ${OUTPUT_DIR}"

# ---------------------------------------------------------------------------
# Show disk space info
# ---------------------------------------------------------------------------
echo ""
AVAIL_KB=$(df --output=avail "$OUTPUT_DIR" 2>/dev/null | tail -1 | tr -d ' ')
if [[ -n "$AVAIL_KB" && "$AVAIL_KB" =~ ^[0-9]+$ ]]; then
    AVAIL_GB=$(awk "BEGIN { printf \"%.1f\", ${AVAIL_KB} / 1048576 }")
    # ~150 MB/min at CRF 23
    AVAIL_HOURS=$(awk "BEGIN { printf \"%.1f\", (${AVAIL_KB} / 1048576) / 9 }")

    echo -e "${BOLD}Disk space:${NC}"
    echo "  Available:       ${AVAIL_GB} GB"
    echo "  Est. capacity:   ~${AVAIL_HOURS} hours of recording (at CRF 23)"
    echo ""
    echo "  Reference (CRF 23):"
    echo "    1 min  = ~150 MB"
    echo "    1 hour = ~9 GB"
    echo "    8 hours = ~72 GB"
fi

# ---------------------------------------------------------------------------
# Initialize the dataset
# ---------------------------------------------------------------------------
echo ""
read -rp "Initialize a dataset manifest in this directory? [Y/n] " create_dataset
if [[ ! "$create_dataset" =~ ^[Nn] ]]; then
    if [[ -z "$EGO_BIN" ]]; then
        warn "ego-recorder binary not found. Run ./setup.sh first to build."
        warn "Then re-run this script to create a dataset."
    elif [[ -f "${OUTPUT_DIR}/dataset.json" ]]; then
        warn "dataset.json already exists in ${OUTPUT_DIR}"
        read -rp "Overwrite? [y/N] " overwrite
        if [[ "$overwrite" =~ ^[Yy] ]]; then
            FORCE="--force"
        else
            info "Keeping existing dataset."
            FORCE=""
        fi
    fi

    # Gather remaining metadata if we have the binary
    if [[ -n "$EGO_BIN" && ( ! -f "${OUTPUT_DIR}/dataset.json" || "${FORCE:-}" == "--force" ) ]]; then
        read -rp "Description (optional): " DATASET_DESC
        read -rp "Tags (comma-separated, e.g. manipulation,kitchen): " DATASET_TAGS

        CMD=()
        [[ "$USE_SYSTEMD" == true ]] && CMD+=(sudo)
        CMD+=("$EGO_BIN" dataset init -o "$OUTPUT_DIR" --name "$DATASET_NAME")
        [[ -n "$DATASET_DESC" ]] && CMD+=(--description "$DATASET_DESC")
        [[ -n "$DATASET_TAGS" ]] && CMD+=(--tags "$DATASET_TAGS")
        [[ "${FORCE:-}" == "--force" ]] && CMD+=(--force)

        echo ""
        if "${CMD[@]}"; then
            ok "Dataset initialized!"
        else
            err "Failed to create dataset."
        fi
    fi
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo ""
echo -e "${BOLD}───────────────────────────────────────${NC}"
ok "Recording directory ready!"
echo ""
local_dir=$(pretty_path "$OUTPUT_DIR")
echo -e "  ${BOLD}Next steps:${NC}"
echo ""
if [[ -n "$EGO_BIN" ]]; then
    echo "    GUI:      ${EGO_BIN_SHORT} -o ${local_dir} -s my_session"
    echo "    Headless: ${EGO_BIN_SHORT} --headless -o ${local_dir} -s my_session"
    echo ""
    echo "    Inspect:  ${EGO_BIN_SHORT} dataset info ${local_dir}"
    echo "    Export:   ${EGO_BIN_SHORT} export rlds ${local_dir} -o ./output"
else
    echo "    Build first: ./setup.sh"
    echo ""
    echo "    GUI:      ./build/ego-recorder -o ${local_dir} -s my_session"
    echo "    Headless: ./build/ego-recorder --headless -o ${local_dir} -s my_session"
fi
echo ""
