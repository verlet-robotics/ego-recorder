#!/usr/bin/env bash
# Fast CLI upload for .egorec files — companion to the Tauri app.
# Reads credentials from ~/.config/ego-recorder-app/config.toml,
# writes the same .upload_manifest.json the app uses.
#
# Usage:
#   ./scripts/upload.sh                    # upload all pending in default output dir
#   ./scripts/upload.sh /path/to/recordings
#   ./scripts/upload.sh /path/to/recordings/my-dataset   # single dataset
#
# Requires: aws-cli (v2), jq, sha256sum, toml-parsing via grep/sed (no extra deps)

set -euo pipefail

CONFIG="$HOME/.config/ego-recorder-app/config.toml"
MAX_CONCURRENT=2
CHUNK_MB=32

# ── Parse TOML config ──────────────────────────────────────────────
read_toml() {
  local key="$1"
  grep -E "^\s*${key}\s*=" "$CONFIG" 2>/dev/null \
    | head -1 \
    | sed -E 's/^[^=]+=\s*"?//; s/"?\s*$//'
}

if [[ ! -f "$CONFIG" ]]; then
  echo "ERROR: Config not found at $CONFIG"
  echo "Run the Ego Recorder app first to create it, or create it manually."
  exit 1
fi

ENDPOINT=$(read_toml endpoint)
BUCKET=$(read_toml bucket)
ACCESS_KEY=$(read_toml access_key)
SECRET_KEY=$(read_toml secret_key)
REGION=$(read_toml region)
PREFIX=$(read_toml prefix)
DEFAULT_DIR=$(read_toml output_dir)
CHUNK_MB_CFG=$(read_toml multipart_chunk_mb)

REGION="${REGION:-auto}"
CHUNK_MB="${CHUNK_MB_CFG:-$CHUNK_MB}"
RECORDINGS_DIR="${1:-$DEFAULT_DIR}"

if [[ -z "$ENDPOINT" || -z "$BUCKET" || -z "$ACCESS_KEY" || -z "$SECRET_KEY" ]]; then
  echo "ERROR: Upload credentials not configured in $CONFIG"
  echo "Need: endpoint, bucket, access_key, secret_key under [upload]"
  exit 1
fi

if [[ -z "$RECORDINGS_DIR" || ! -d "$RECORDINGS_DIR" ]]; then
  echo "ERROR: Recordings directory not found: ${RECORDINGS_DIR:-'(not set)'}"
  echo "Usage: $0 [/path/to/recordings]"
  exit 1
fi

# ── Determine base dir (for manifest + relative paths) ─────────────
# If the user pointed at a dataset subdir, the manifest lives in the parent
MANIFEST_DIR="$RECORDINGS_DIR"
if [[ -f "$RECORDINGS_DIR/../.upload_manifest.json" ]] && [[ ! -f "$RECORDINGS_DIR/.upload_manifest.json" ]]; then
  # Looks like a dataset subdir — use parent for manifest, scan subdir for files
  MANIFEST_DIR="$(cd "$RECORDINGS_DIR/.." && pwd)"
fi
# If the base output_dir is configured and our dir is inside it, use that as manifest root
if [[ -n "$DEFAULT_DIR" && "$RECORDINGS_DIR" == "$DEFAULT_DIR"* ]]; then
  MANIFEST_DIR="$DEFAULT_DIR"
fi

MANIFEST="$MANIFEST_DIR/.upload_manifest.json"

# ── Set up AWS CLI env ─────────────────────────────────────────────
export AWS_ACCESS_KEY_ID="$ACCESS_KEY"
export AWS_SECRET_ACCESS_KEY="$SECRET_KEY"
export AWS_DEFAULT_REGION="$REGION"

S3_ENDPOINT_URL="$ENDPOINT"

# ── Load manifest ─────────────────────────────────────────────────
if [[ -f "$MANIFEST" ]]; then
  UPLOADED=$(jq -r '.uploads[] | select(.success==true) | .filename' "$MANIFEST" 2>/dev/null || true)
else
  UPLOADED=""
  echo '{"version":1,"uploads":[]}' > "$MANIFEST"
fi

# ── Find pending files ────────────────────────────────────────────
PENDING=()
while IFS= read -r -d '' file; do
  rel_path="${file#"$MANIFEST_DIR/"}"
  if echo "$UPLOADED" | grep -qxF "$rel_path"; then
    continue
  fi
  PENDING+=("$file")
done < <(find "$RECORDINGS_DIR" -name '*.egorec' ! -path '*/.pruned/*' -print0 | sort -z)

if [[ ${#PENDING[@]} -eq 0 ]]; then
  echo "Nothing to upload — all .egorec files already in manifest."
  exit 0
fi

echo "Found ${#PENDING[@]} file(s) to upload (concurrency: $MAX_CONCURRENT)"
echo ""

# ── Upload function (called per file) ─────────────────────────────
upload_one() {
  local file="$1"
  local rel_path="${file#"$MANIFEST_DIR/"}"
  local size
  size=$(stat -c%s "$file" 2>/dev/null || stat -f%z "$file" 2>/dev/null)
  local basename
  basename=$(basename "$file")

  # S3 key
  local s3_key="$rel_path"
  if [[ -n "${PREFIX:-}" ]]; then
    s3_key="${PREFIX%/}/$rel_path"
  fi

  # Hash
  printf "  %-50s  hashing..." "$basename"
  local sha256
  sha256=$(sha256sum "$file" | awk '{print $1}')
  printf "\r  %-50s  uploading (%s)..." "$basename" "$(numfmt --to=iec-i --suffix=B "$size" 2>/dev/null || echo "${size}B")"

  # Upload
  if aws s3 cp "$file" "s3://$BUCKET/$s3_key" \
      --endpoint-url "$S3_ENDPOINT_URL" \
      --no-progress \
      --expected-size "$size" \
      > /dev/null 2>&1; then

    printf "\r  %-50s  done  (%s)\n" "$basename" "$(numfmt --to=iec-i --suffix=B "$size" 2>/dev/null || echo "${size}B")"

    # Record in manifest (atomic: write tmp, rename)
    # Use flock to serialize concurrent manifest writes
    (
      flock -x 200
      local tmp="$MANIFEST.tmp"
      local now
      now=$(date -u +"%Y-%m-%dT%H:%M:%S+00:00")
      jq --arg fn "$rel_path" \
         --arg key "$s3_key" \
         --arg at "$now" \
         --argjson sz "$size" \
         --arg sha "$sha256" \
         '.uploads += [{"filename":$fn,"r2_key":$key,"uploaded_at":$at,"size_bytes":$sz,"sha256":$sha,"attempt_count":1,"success":true}]' \
         "$MANIFEST" > "$tmp"
      mv "$tmp" "$MANIFEST"
    ) 200>"$MANIFEST.lock"
  else
    printf "\r  %-50s  FAILED\n" "$basename"
    return 1
  fi
}

export -f upload_one
export MANIFEST_DIR MANIFEST BUCKET PREFIX S3_ENDPOINT_URL
export AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY AWS_DEFAULT_REGION

# ── Run uploads with bounded concurrency ──────────────────────────
FAILED=0
printf '%s\0' "${PENDING[@]}" | xargs -0 -P "$MAX_CONCURRENT" -I{} bash -c 'upload_one "$@"' _ {} || FAILED=1

# ── Summary ───────────────────────────────────────────────────────
echo ""
DONE=$(jq '[.uploads[] | select(.success==true)] | length' "$MANIFEST")
echo "Manifest: $DONE file(s) uploaded total"

if [[ $FAILED -ne 0 ]]; then
  echo "Some uploads failed — re-run to retry."
  exit 1
fi

echo "All uploads complete."
