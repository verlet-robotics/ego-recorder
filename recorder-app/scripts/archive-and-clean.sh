#!/usr/bin/env bash
# archive-and-clean.sh
#
# Interactively mounts an external drive, moves NOT-uploaded .egorec files
# to it (without overwriting existing data), then removes ALL .egorec data
# from the local disk. Offers to unmount the drive at the end.

set -euo pipefail

# ── Colours ──────────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
CYAN='\033[0;36m'; BOLD='\033[1m'; NC='\033[0m'

info()  { echo -e "${CYAN}[INFO]${NC}  $*"; }
ok()    { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC}  $*"; }
err()   { echo -e "${RED}[ERROR]${NC} $*"; }

# ── Resolve recording directory ──────────────────────────────────────────
CONFIG_FILE="$HOME/.config/ego-recorder-app/config.toml"
RECORDING_DIR="$HOME/Documents/ego-recordings"

if [[ -f "$CONFIG_FILE" ]]; then
    configured_dir=$(grep -Po '^\s*output_dir\s*=\s*"\K[^"]+' "$CONFIG_FILE" 2>/dev/null || true)
    if [[ -n "$configured_dir" ]]; then
        # Expand ~ if present
        configured_dir="${configured_dir/#\~/$HOME}"
        RECORDING_DIR="$configured_dir"
    fi
fi

if [[ ! -d "$RECORDING_DIR" ]]; then
    err "Recording directory not found: $RECORDING_DIR"
    exit 1
fi

info "Recording directory: ${BOLD}$RECORDING_DIR${NC}"

MANIFEST="$RECORDING_DIR/.upload_manifest.json"

# ── Build set of uploaded filenames from manifest ────────────────────────
declare -A UPLOADED=()
if [[ -f "$MANIFEST" ]]; then
    while IFS= read -r fname; do
        UPLOADED["$fname"]=1
    done < <(python3 -c "
import json, sys
with open('$MANIFEST') as f:
    m = json.load(f)
for r in m.get('uploads', []):
    if r.get('success'):
        print(r['filename'])
" 2>/dev/null || true)
    info "Manifest loaded: ${#UPLOADED[@]} uploaded file(s) tracked"
else
    warn "No upload manifest found — all files will be treated as not-uploaded"
fi

# ── Discover all .egorec files ───────────────────────────────────────────
mapfile -t ALL_FILES < <(find "$RECORDING_DIR" -name '*.egorec' -type f \
    ! -path '*/.pruned/*' -size +1023c | sort)

if [[ ${#ALL_FILES[@]} -eq 0 ]]; then
    info "No .egorec files found in $RECORDING_DIR. Nothing to do."
    exit 0
fi

# Partition into uploaded / not-uploaded
NOT_UPLOADED=()
ALREADY_UPLOADED=()
for f in "${ALL_FILES[@]}"; do
    rel="${f#"$RECORDING_DIR"/}"
    if [[ -n "${UPLOADED[$rel]+_}" ]]; then
        ALREADY_UPLOADED+=("$f")
    else
        NOT_UPLOADED+=("$f")
    fi
done

total_size=$(du -shc "${ALL_FILES[@]}" 2>/dev/null | tail -1 | awk '{print $1}')
not_up_size="0B"
if [[ ${#NOT_UPLOADED[@]} -gt 0 ]]; then
    not_up_size=$(du -shc "${NOT_UPLOADED[@]}" 2>/dev/null | tail -1 | awk '{print $1}')
fi

echo ""
echo -e "${BOLD}Summary${NC}"
echo    "  Total .egorec files:        ${#ALL_FILES[@]}  ($total_size)"
echo    "  Not uploaded (to archive):   ${#NOT_UPLOADED[@]}  ($not_up_size)"
echo    "  Already uploaded (to delete): ${#ALREADY_UPLOADED[@]}"
echo ""

if [[ ${#NOT_UPLOADED[@]} -eq 0 ]]; then
    warn "All files are already uploaded. Skipping archive step."
fi

# ── Interactive drive mounting ───────────────────────────────────────────
if [[ ${#NOT_UPLOADED[@]} -gt 0 ]]; then
    info "Detecting available block devices..."
    echo ""
    # Show unmounted partitions that look like external drives
    lsblk -o NAME,SIZE,TYPE,FSTYPE,MOUNTPOINT,MODEL -p | head -1
    lsblk -o NAME,SIZE,TYPE,FSTYPE,MOUNTPOINT,MODEL -p | grep -E 'part|disk' | grep -v 'loop'
    echo ""

    read -rp "Enter the device to mount (e.g. /dev/sdb1): " DEVICE

    if [[ ! -b "$DEVICE" ]]; then
        err "$DEVICE is not a valid block device"
        exit 1
    fi

    # Check if already mounted
    EXISTING_MOUNT=$(findmnt -n -o TARGET "$DEVICE" 2>/dev/null || true)
    if [[ -n "$EXISTING_MOUNT" ]]; then
        info "$DEVICE is already mounted at $EXISTING_MOUNT"
        MOUNT_POINT="$EXISTING_MOUNT"
        SELF_MOUNTED=false
    else
        MOUNT_POINT="/mnt/ego-archive"
        info "Mounting $DEVICE at $MOUNT_POINT ..."
        sudo mkdir -p "$MOUNT_POINT"
        sudo mount "$DEVICE" "$MOUNT_POINT"
        # Ensure current user can write
        if [[ ! -w "$MOUNT_POINT" ]]; then
            sudo chmod a+rwx "$MOUNT_POINT"
        fi
        SELF_MOUNTED=true
        ok "Mounted $DEVICE at $MOUNT_POINT"
    fi

    # Check free space on target
    avail_kb=$(df --output=avail "$MOUNT_POINT" | tail -1 | tr -d ' ')
    needed_kb=$(du -sk "${NOT_UPLOADED[@]}" | awk '{s+=$1} END{print s}')
    if (( needed_kb > avail_kb )); then
        avail_h=$(numfmt --to=iec --from-unit=1024 "$avail_kb")
        needed_h=$(numfmt --to=iec --from-unit=1024 "$needed_kb")
        err "Not enough space on $MOUNT_POINT: need $needed_h, have $avail_h"
        if [[ "${SELF_MOUNTED:-false}" == true ]]; then
            sudo umount "$MOUNT_POINT"
        fi
        exit 1
    fi

    # ── Archive not-uploaded files ───────────────────────────────────────
    ARCHIVE_DIR="$MOUNT_POINT/ego-recordings"
    mkdir -p "$ARCHIVE_DIR"

    info "Archiving ${#NOT_UPLOADED[@]} not-uploaded file(s) to $ARCHIVE_DIR ..."
    archived=0
    skipped=0
    for f in "${NOT_UPLOADED[@]}"; do
        rel="${f#"$RECORDING_DIR"/}"
        dest="$ARCHIVE_DIR/$rel"
        dest_dir="$(dirname "$dest")"
        mkdir -p "$dest_dir"

        if [[ -e "$dest" ]]; then
            # Avoid overwriting — rename with timestamp suffix
            base="${dest%.egorec}"
            ts=$(date +%Y%m%d_%H%M%S)
            dest="${base}_${ts}.egorec"
            if [[ -e "$dest" ]]; then
                dest="${base}_${ts}_$$.egorec"
            fi
            warn "Destination exists, archiving as: $(basename "$dest")"
            ((skipped++)) || true
        fi

        cp --preserve=timestamps "$f" "$dest"
        ((archived++)) || true
        printf "\r  Archived %d / %d" "$archived" "${#NOT_UPLOADED[@]}"
    done
    echo ""
    ok "Archived $archived file(s) to $ARCHIVE_DIR"
    if [[ $skipped -gt 0 ]]; then
        warn "$skipped file(s) were renamed to avoid overwriting"
    fi

    # Also copy the manifest and any dataset.json for reference
    if [[ -f "$MANIFEST" ]]; then
        cp --preserve=timestamps "$MANIFEST" "$ARCHIVE_DIR/.upload_manifest.json"
    fi
    # Copy dataset.json files
    find "$RECORDING_DIR" -name 'dataset.json' -type f | while read -r dj; do
        rel="${dj#"$RECORDING_DIR"/}"
        dest_dj="$ARCHIVE_DIR/$rel"
        mkdir -p "$(dirname "$dest_dj")"
        cp --no-clobber --preserve=timestamps "$dj" "$dest_dj" 2>/dev/null || true
    done
    ok "Copied manifest and dataset metadata to archive"

    # Sync to ensure data is flushed to disk
    info "Syncing filesystem..."
    sync
    ok "Archive complete and synced to disk"
fi

# ── Clean local recording directory ──────────────────────────────────────
echo ""
echo -e "${BOLD}${RED}WARNING:${NC} This will delete ALL ${#ALL_FILES[@]} .egorec file(s) from:"
echo -e "  ${BOLD}$RECORDING_DIR${NC}"
echo ""
read -rp "Type 'yes' to confirm deletion: " CONFIRM

if [[ "$CONFIRM" != "yes" ]]; then
    warn "Deletion cancelled. Archived files remain on the external drive."
    if [[ "${SELF_MOUNTED:-false}" == true ]]; then
        read -rp "Unmount $DEVICE? [y/N]: " DO_UMOUNT
        if [[ "$DO_UMOUNT" =~ ^[Yy]$ ]]; then
            sudo umount "$MOUNT_POINT"
            ok "Unmounted $DEVICE — safe to unplug"
        fi
    fi
    exit 0
fi

info "Deleting ${#ALL_FILES[@]} .egorec file(s)..."
deleted=0
for f in "${ALL_FILES[@]}"; do
    rm -f "$f"
    ((deleted++)) || true
    printf "\r  Deleted %d / %d" "$deleted" "${#ALL_FILES[@]}"
done
echo ""

# Remove empty dataset directories (but keep ones with non-egorec content)
find "$RECORDING_DIR" -mindepth 1 -type d -empty -delete 2>/dev/null || true

# Clear the upload manifest since all files are gone
if [[ -f "$MANIFEST" ]]; then
    info "Clearing upload manifest..."
    python3 -c "
import json
with open('$MANIFEST', 'w') as f:
    json.dump({'version': 1, 'uploads': []}, f, indent=2)
"
    ok "Upload manifest cleared"
fi

ok "Deleted $deleted file(s) from local disk"

# ── Show reclaimed space ─────────────────────────────────────────────────
echo ""
echo -e "${GREEN}${BOLD}Disk space reclaimed: ~$total_size${NC}"

# ── Unmount option ───────────────────────────────────────────────────────
if [[ "${SELF_MOUNTED:-false}" == true ]]; then
    echo ""
    read -rp "Unmount $DEVICE for safe removal? [Y/n]: " DO_UMOUNT
    if [[ ! "$DO_UMOUNT" =~ ^[Nn]$ ]]; then
        sync
        sudo umount "$MOUNT_POINT"
        ok "Unmounted $DEVICE — safe to unplug"
    else
        warn "$DEVICE is still mounted at $MOUNT_POINT"
    fi
fi

echo ""
ok "Done!"
