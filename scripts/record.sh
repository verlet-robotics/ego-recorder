#!/usr/bin/env bash
# record.sh -- Interactive recording launcher for ego-recorder
#
# Prompts for dataset, session name, and mode, then starts recording.
#
# Usage:
#   ./scripts/record.sh            # Interactive prompts
#   ./scripts/record.sh pick       # Pre-select dataset, prompt for rest

set -euo pipefail

BASE_DIR="/var/lib/ego-recorder"
EGO_RECORDER="ego-recorder"

# Colors
BOLD='\033[1m'
CYAN='\033[0;36m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
DIM='\033[2m'
NC='\033[0m'

# ---------------------------------------------------------------------------
# 1. Choose dataset
# ---------------------------------------------------------------------------
echo ""
echo -e "${BOLD}ego-recorder${NC}"
echo "───────────────────────────────────"

# List existing datasets
existing=()
if [[ -d "$BASE_DIR" ]]; then
    while IFS= read -r d; do
        existing+=("$(basename "$d")")
    done < <(find "$BASE_DIR" -maxdepth 1 -mindepth 1 -type d | sort)
fi

dataset="${1:-}"
if [[ -z "$dataset" ]]; then
    echo ""
    if [[ ${#existing[@]} -gt 0 ]]; then
        echo -e "${BOLD}Existing datasets:${NC}"
        for i in "${!existing[@]}"; do
            name="${existing[$i]}"
            # Count episodes from dataset.json if it exists
            manifest="$BASE_DIR/$name/dataset.json"
            if [[ -f "$manifest" ]]; then
                count=$(grep -c '"filename"' "$manifest" 2>/dev/null || echo 0)
                echo -e "  ${CYAN}$((i+1)))${NC} $name ${DIM}($count episodes)${NC}"
            else
                echo -e "  ${CYAN}$((i+1)))${NC} $name ${DIM}(no manifest)${NC}"
            fi
        done
        echo ""
    fi

    read -rp "Dataset name (number or new name): " choice

    # Check if they entered a number
    if [[ "$choice" =~ ^[0-9]+$ ]] && (( choice >= 1 && choice <= ${#existing[@]} )); then
        dataset="${existing[$((choice-1))]}"
    else
        dataset="$choice"
    fi
fi

if [[ -z "$dataset" ]]; then
    echo "No dataset selected." >&2
    exit 1
fi

output_dir="$BASE_DIR/$dataset"

# Initialize dataset if new
if [[ ! -f "$output_dir/dataset.json" ]]; then
    echo ""
    echo -e "${YELLOW}New dataset: ${dataset}${NC}"
    read -rp "Description (optional): " description
    mkdir -p "$output_dir"
    "$EGO_RECORDER" dataset init "$output_dir" --name "$dataset" \
        ${description:+--description "$description"} 2>/dev/null || true
    echo -e "${GREEN}Created ${output_dir}/dataset.json${NC}"
fi

# ---------------------------------------------------------------------------
# 2. Session name (optional)
# ---------------------------------------------------------------------------
echo ""
read -rp "Session name (Enter for auto '${dataset}_NNN'): " session_name

# ---------------------------------------------------------------------------
# 3. Mode
# ---------------------------------------------------------------------------
echo ""
echo -e "${BOLD}Mode:${NC}"
echo -e "  ${CYAN}1)${NC} headless ${DIM}(no GUI, starts immediately)${NC}"
echo -e "  ${CYAN}2)${NC} gui      ${DIM}(preview window)${NC}"
echo ""
read -rp "Mode [1]: " mode_choice

mode_flag="--headless"
mode_label="headless"
if [[ "$mode_choice" == "2" ]]; then
    mode_flag=""
    mode_label="gui"
fi

# ---------------------------------------------------------------------------
# 4. Confirm and launch
# ---------------------------------------------------------------------------
echo ""
echo "───────────────────────────────────"
echo -e "  Dataset:  ${BOLD}${dataset}${NC}"
echo -e "  Session:  ${BOLD}${session_name:-auto}${NC}"
echo -e "  Mode:     ${BOLD}${mode_label}${NC}"
echo -e "  Output:   ${DIM}${output_dir}${NC}"
echo "───────────────────────────────────"
echo ""

read -rp "Start recording? [Y/n] " confirm
if [[ "$confirm" =~ ^[Nn] ]]; then
    echo "Cancelled."
    exit 0
fi

# Build command
cmd=("$EGO_RECORDER")
if [[ -n "$mode_flag" ]]; then
    cmd+=("$mode_flag")
fi
cmd+=(-o "$output_dir")
if [[ -n "$session_name" ]]; then
    cmd+=(-s "$session_name")
fi

echo ""
echo -e "${DIM}${cmd[*]}${NC}"
echo ""
exec "${cmd[@]}"
