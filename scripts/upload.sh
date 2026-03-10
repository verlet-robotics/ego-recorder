#!/usr/bin/env bash
# upload.sh -- Interactive upload launcher for ego-uploader
#
# Checks for R2 credentials and prompts if missing, then launches
# the Python uploader.
#
# Usage:
#   ./scripts/upload.sh              # Interactive upload (default)
#   ./scripts/upload.sh --once       # Single pass then exit
#   ./scripts/upload.sh [ARGS...]    # Pass any args to ego_uploader.py

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
ENV_FILE="${PROJECT_DIR}/python/.env"
CONFIG_FILE="${PROJECT_DIR}/deploy/upload_config.toml"

# Colors & helpers (required by lib-env.sh)
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
DIM='\033[2m'
NC='\033[0m'

info()  { echo -e "${CYAN}[upload]${NC} $*"; }
ok()    { echo -e "${GREEN}[upload]${NC} $*"; }
warn()  { echo -e "${YELLOW}[upload]${NC} $*"; }
error() { echo -e "${RED}[upload]${NC} $*" >&2; }

source "${SCRIPT_DIR}/lib-env.sh"

# ---------------------------------------------------------------------------
# 0. Ensure Python venv with uploader deps
# ---------------------------------------------------------------------------
if [[ -z "${VIRTUAL_ENV:-}" && -z "${CONDA_PREFIX:-}" ]]; then
    venv_dir="${PROJECT_DIR}/.venv"
    if [[ ! -d "$venv_dir" ]]; then
        info "Creating Python venv at ${venv_dir}..."
        python3 -m venv "$venv_dir"
    fi
    source "${venv_dir}/bin/activate"

    # Install uploader deps if boto3 is missing
    if ! python3 -c "import boto3" 2>/dev/null; then
        info "Installing uploader dependencies..."
        pip install -q -r "${PROJECT_DIR}/python/requirements-uploader.txt"
        ok "Dependencies installed."
    fi
fi

# ---------------------------------------------------------------------------
# 1. Ensure R2 credentials exist
# ---------------------------------------------------------------------------
r2_configured=true
if [[ ! -f "$ENV_FILE" ]]; then
    r2_configured=false
else
    # Check that the required keys are actually set (non-empty)
    has_endpoint=$(grep -q '^R2_ENDPOINT=.\+' "$ENV_FILE" 2>/dev/null && echo yes || echo no)
    has_key=$(grep -q '^R2_ACCESS_KEY_ID=.\+' "$ENV_FILE" 2>/dev/null && echo yes || echo no)
    has_secret=$(grep -q '^R2_SECRET_ACCESS_KEY=.\+' "$ENV_FILE" 2>/dev/null && echo yes || echo no)
    if [[ "$has_endpoint" != "yes" || "$has_key" != "yes" || "$has_secret" != "yes" ]]; then
        r2_configured=false
    fi
fi

if [[ "$r2_configured" == "false" ]]; then
    echo ""
    warn "R2 credentials not configured."
    echo ""
    prompt_r2_credentials "$ENV_FILE" || {
        echo ""
        error "Cannot upload without R2 credentials."
        echo "  Configure manually:  nano ${ENV_FILE}"
        echo "  Or re-run:           ./scripts/upload.sh"
        exit 1
    }
    echo ""
fi

# ---------------------------------------------------------------------------
# 2. Launch uploader
# ---------------------------------------------------------------------------
# Default to interactive mode if no args given
if [[ $# -eq 0 ]]; then
    set -- --interactive
fi

exec python3 "${PROJECT_DIR}/python/ego_uploader.py" \
    --config "$CONFIG_FILE" \
    "$@"
