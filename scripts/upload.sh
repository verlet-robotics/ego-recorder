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

source "${SCRIPT_DIR}/lib-env.sh"

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
