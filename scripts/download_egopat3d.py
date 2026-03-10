#!/usr/bin/env python3
"""Download selected EgoPAT3D episodes from HuggingFace and convert to .egorec format.

Each episode is stored in a ~15GB ZIP on HuggingFace (gated dataset — requires
an access token). We use HTTP range requests to extract only what we need:
rgb_video.mp4 + first N depth PNGs per episode.

Usage:
    python scripts/download_egopat3d.py [--frames 900] [--output datasets/egopat3d]
    python scripts/download_egopat3d.py --skip-convert   # download only
    python scripts/download_egopat3d.py --token hf_...   # pass HF token directly

Prerequisites:
    1. Request access to EgoPAT3Dv2 on HuggingFace:
       https://huggingface.co/datasets/qianlima/EgoPAT3Dv2
    2. Set your token via --token, HF_TOKEN env var, or ~/.huggingface/token
    3. pip install requests  (system python usually has it)
    4. Build ego-convert:  cd rust && cargo build -p ego-convert --release
"""

import argparse
import io
import json
import os
import pwd
import re
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path

try:
    import requests
except ImportError:
    print("Error: requests library required. Install with: pip install requests")
    sys.exit(1)

# HuggingFace dataset base URL
HF_BASE = "https://huggingface.co/datasets/qianlima/EgoPAT3Dv2/resolve/main"

# Selected episodes: (scene_name, scene_index, zip_name, friendly_name)
EPISODES = [
    ("kitchenCounter", 6, "6.3.zip", "kitchenCounter_3"),
    ("drawer", 5, "5.1.zip", "drawer_1"),
    ("desk", 4, "4.2.zip", "desk_2"),
]


# ---------------------------------------------------------------------------
# Permissions
# ---------------------------------------------------------------------------

def ensure_directory_writable(dir_path):
    """Ensure dir_path exists and is writable by the current user.

    On stations running the ego-recorder systemd service, the datasets/
    directory is owned by the ego-recorder system user. The current user
    needs write access via POSIX ACLs (same pattern as setup-recordings.sh).

    Returns True if the directory is ready, False if we couldn't fix it.
    """
    dir_path = Path(dir_path)

    # Parent must exist (or be creatable) first
    parent = dir_path.parent
    if not parent.exists():
        try:
            parent.mkdir(parents=True, exist_ok=True)
        except PermissionError:
            return _try_sudo_mkdir(parent) and _try_sudo_mkdir(dir_path)

    # Try creating the directory directly
    if not dir_path.exists():
        try:
            dir_path.mkdir(parents=True, exist_ok=True)
            return True
        except PermissionError:
            return _try_sudo_mkdir(dir_path)

    # Directory exists — check if we can write
    if os.access(dir_path, os.W_OK):
        return True

    # Not writable — try to fix with setfacl
    return _try_fix_permissions(dir_path)


def _try_sudo_mkdir(dir_path):
    """Create directory with sudo and grant current user access via ACL."""
    current_user = os.environ.get("USER", "")
    print(f"\n  Directory {dir_path} requires elevated permissions.")
    print(f"  This station uses the ego-recorder systemd service — the datasets/")
    print(f"  directory is owned by the ego-recorder system user.")
    print(f"")
    print(f"  Running: sudo mkdir -p {dir_path}")
    print(f"           sudo chown ego-recorder:ego-recorder {dir_path}")
    print(f"           sudo setfacl -R -m u:{current_user}:rwx {dir_path}")
    print(f"           sudo setfacl -R -d -m u:{current_user}:rwx {dir_path}")
    print()

    # Check if ego-recorder user exists
    try:
        pwd.getpwnam("ego-recorder")
        owner = "ego-recorder:ego-recorder"
    except KeyError:
        owner = None

    # mkdir
    r = subprocess.run(["sudo", "mkdir", "-p", str(dir_path)])
    if r.returncode != 0:
        print(f"  ERROR: sudo mkdir failed. Run manually:")
        print(f"    sudo mkdir -p {dir_path}")
        if owner:
            print(f"    sudo chown {owner} {dir_path}")
        print(f"    sudo setfacl -R -m u:{current_user}:rwx {dir_path}")
        print(f"    sudo setfacl -R -d -m u:{current_user}:rwx {dir_path}")
        return False

    # chown if ego-recorder exists
    if owner:
        subprocess.run(["sudo", "chown", owner, str(dir_path)])

    # setfacl to grant current user access
    if current_user and current_user != "root":
        subprocess.run([
            "sudo", "setfacl", "-R", "-m", f"u:{current_user}:rwx", str(dir_path)
        ])
        subprocess.run([
            "sudo", "setfacl", "-R", "-d", "-m", f"u:{current_user}:rwx", str(dir_path)
        ])

    return os.access(dir_path, os.W_OK)


def _try_fix_permissions(dir_path):
    """Try to fix permissions on an existing directory via setfacl."""
    current_user = os.environ.get("USER", "")
    if not current_user or current_user == "root":
        return False

    print(f"\n  Directory {dir_path} exists but is not writable.")
    print(f"  Granting {current_user} access via POSIX ACL (requires sudo)...")
    print()

    r = subprocess.run([
        "sudo", "setfacl", "-R", "-m", f"u:{current_user}:rwx", str(dir_path)
    ])
    if r.returncode != 0:
        print(f"  ERROR: Could not fix permissions. Run manually:")
        print(f"    sudo setfacl -R -m u:{current_user}:rwx {dir_path}")
        print(f"    sudo setfacl -R -d -m u:{current_user}:rwx {dir_path}")
        return False

    subprocess.run([
        "sudo", "setfacl", "-R", "-d", "-m", f"u:{current_user}:rwx", str(dir_path)
    ])

    return os.access(dir_path, os.W_OK)


# ---------------------------------------------------------------------------
# HuggingFace auth
# ---------------------------------------------------------------------------

def resolve_hf_token(token_arg):
    """Resolve HuggingFace token from arg > env > file."""
    if token_arg:
        return token_arg
    token = os.environ.get("HF_TOKEN") or os.environ.get("HUGGING_FACE_HUB_TOKEN")
    if token:
        return token
    token_file = Path.home() / ".huggingface" / "token"
    if token_file.exists():
        return token_file.read_text().strip()
    # Also check the newer cache location
    token_file2 = Path.home() / ".cache" / "huggingface" / "token"
    if token_file2.exists():
        return token_file2.read_text().strip()
    return None


# ---------------------------------------------------------------------------
# HTTP range file for partial ZIP extraction
# ---------------------------------------------------------------------------

class HTTPRangeFile(io.RawIOBase):
    """Seekable file-like object backed by HTTP Range requests.

    Implements enough of the file interface for zipfile.ZipFile to work.
    Uses a read-ahead buffer (default 4MB) to avoid issuing an HTTP request
    for every small read — critical when extracting files from remote ZIPs.
    """

    CHUNK_SIZE = 4 * 1024 * 1024  # 4MB read-ahead

    def __init__(self, url, session=None):
        super().__init__()
        self.url = url
        self.session = session or requests.Session()
        self.pos = 0
        self._size = None
        self._buf = b""
        self._buf_start = 0  # file offset where _buf begins

    @property
    def size(self):
        if self._size is None:
            r = self.session.head(self.url, allow_redirects=True, timeout=30)
            r.raise_for_status()
            self._size = int(r.headers["Content-Length"])
        return self._size

    def readable(self):
        return True

    def seekable(self):
        return True

    def tell(self):
        return self.pos

    def seek(self, offset, whence=io.SEEK_SET):
        if whence == io.SEEK_SET:
            self.pos = offset
        elif whence == io.SEEK_CUR:
            self.pos += offset
        elif whence == io.SEEK_END:
            self.pos = self.size + offset
        # Invalidate buffer if seek lands outside it
        if not (self._buf_start <= self.pos <= self._buf_start + len(self._buf)):
            self._buf = b""
            self._buf_start = self.pos
        return self.pos

    def _fetch(self, start, length):
        """Fetch bytes from the remote file, verifying partial content response."""
        end = min(start + length - 1, self.size - 1)
        if start > end:
            return b""
        headers = {"Range": f"bytes={start}-{end}"}
        r = self.session.get(self.url, headers=headers, timeout=120)
        r.raise_for_status()
        if r.status_code != 206:
            raise IOError(
                f"Server returned {r.status_code} instead of 206 Partial Content. "
                "Range requests may not be supported."
            )
        return r.content

    def read(self, size=-1):
        if size == 0:
            return b""
        if size < 0:
            size = self.size - self.pos

        end = min(self.pos + size, self.size)
        if self.pos >= end:
            return b""

        # Check if the request is fully within the buffer
        buf_end = self._buf_start + len(self._buf)
        if self._buf and self._buf_start <= self.pos and end <= buf_end:
            offset = self.pos - self._buf_start
            data = self._buf[offset:offset + size]
            self.pos += len(data)
            return data

        # Fetch a chunk at least as large as requested, with read-ahead
        fetch_size = max(size, self.CHUNK_SIZE)
        self._buf = self._fetch(self.pos, fetch_size)
        self._buf_start = self.pos

        data = self._buf[:size]
        self.pos += len(data)
        return data

    def readinto(self, b):
        data = self.read(len(b))
        n = len(data)
        b[:n] = data
        return n


# ---------------------------------------------------------------------------
# Download
# ---------------------------------------------------------------------------

def download_episode(
    scene_index, zip_name, friendly_name, output_dir, max_frames, session
):
    """Download one episode from HuggingFace ZIP using range requests."""
    episode_dir = output_dir / friendly_name
    d2rgb_dir = episode_dir / "d2rgb"

    # Check if already downloaded
    video_path = episode_dir / "rgb_video.mp4"
    if video_path.exists() and d2rgb_dir.exists():
        existing_pngs = len(list(d2rgb_dir.glob("*.png")))
        if existing_pngs >= max_frames:
            print(f"  {friendly_name}: already downloaded ({existing_pngs} frames)")
            return True

    zip_url = f"{HF_BASE}/{scene_index}/{zip_name}"
    print(f"  {friendly_name}: opening {zip_url}")

    try:
        range_file = HTTPRangeFile(zip_url, session)
        # Verify file exists and get size
        file_size = range_file.size
        print(f"  {friendly_name}: ZIP size = {file_size / (1024**3):.1f} GB")

        # Open as ZIP — zipfile reads central directory from end
        with zipfile.ZipFile(range_file) as zf:
            names = zf.namelist()
            name_set = set(names)

            # Find rgb_video.mp4
            video_entries = [n for n in names if n.endswith("rgb_video.mp4")]
            if not video_entries:
                print(f"  {friendly_name}: ERROR - no rgb_video.mp4 found in ZIP")
                return False
            video_entry = video_entries[0]

            # Find depth PNGs — format: {scene}/{episode}/d2rgb/{N}.png
            depth_prefix = None
            for n in names:
                if "/d2rgb/" in n and n.endswith(".png"):
                    depth_prefix = n.rsplit("/d2rgb/", 1)[0] + "/d2rgb/"
                    break

            if not depth_prefix:
                print(f"  {friendly_name}: ERROR - no d2rgb/ directory found in ZIP")
                return False

            # Extract rgb_video.mp4
            episode_dir.mkdir(parents=True, exist_ok=True)
            print(f"  {friendly_name}: extracting rgb_video.mp4...")
            with zf.open(video_entry) as src, open(video_path, "wb") as dst:
                shutil.copyfileobj(src, dst)

            # Extract depth PNGs
            d2rgb_dir.mkdir(parents=True, exist_ok=True)
            extracted = 0
            for i in range(1, max_frames + 1):
                png_name = f"{depth_prefix}{i}.png"
                if png_name not in name_set:
                    break
                target_path = d2rgb_dir / f"{i}.png"
                if target_path.exists():
                    extracted += 1
                    continue
                try:
                    with zf.open(png_name) as src, open(target_path, "wb") as dst:
                        shutil.copyfileobj(src, dst)
                    extracted += 1
                    if extracted % 100 == 0:
                        print(f"  {friendly_name}: extracted {extracted}/{max_frames} depth PNGs...")
                except KeyError:
                    break

            print(f"  {friendly_name}: extracted {extracted} depth PNGs + rgb_video.mp4")

            # Try to extract annotations
            extract_annotations(zf, names, friendly_name, episode_dir)

        return True

    except requests.exceptions.HTTPError as e:
        if e.response is not None and e.response.status_code == 401:
            print(f"  {friendly_name}: ERROR - 401 Unauthorized")
            print(f"  The EgoPAT3Dv2 dataset is gated. You need to:")
            print(f"    1. Request access at https://huggingface.co/datasets/qianlima/EgoPAT3Dv2")
            print(f"    2. Pass your token: --token hf_... or set HF_TOKEN env var")
            return False
        raise
    except Exception as e:
        print(f"  {friendly_name}: ERROR - {e}")
        return False


def extract_annotations(zf, names, friendly_name, episode_dir):
    """Try to extract hand_frames/clip_ranges.txt and parse into annotations.json."""
    clip_entries = [n for n in names if "clip_ranges" in n.lower() or "hand_frames" in n.lower()]
    action_entries = [n for n in names if "action" in n.lower() and n.endswith(".txt")]

    annotations = []

    for entry in clip_entries + action_entries:
        try:
            with zf.open(entry) as f:
                content = f.read().decode("utf-8", errors="replace")

            for line in content.strip().split("\n"):
                parts = line.strip().split()
                if len(parts) >= 2:
                    try:
                        start = int(parts[0])
                        end = int(parts[1])
                        label = parts[2] if len(parts) > 2 else "manipulation"
                        annotations.append({
                            "start_frame": start,
                            "end_frame": end,
                            "label": label,
                            "source": os.path.basename(entry),
                        })
                    except ValueError:
                        continue
        except Exception:
            continue

    if annotations:
        ann_path = episode_dir / "annotations.json"
        with open(ann_path, "w") as f:
            json.dump(annotations, f, indent=2)
        print(f"  {friendly_name}: saved {len(annotations)} annotations")


# ---------------------------------------------------------------------------
# Conversion
# ---------------------------------------------------------------------------

def find_ego_convert():
    """Find or build the ego-convert binary."""
    script_dir = Path(__file__).resolve().parent
    rust_dir = script_dir.parent / "rust"
    ego_convert = rust_dir / "target" / "release" / "ego-convert"

    if ego_convert.exists():
        return ego_convert

    print("  Building ego-convert (first time only)...")
    result = subprocess.run(
        ["cargo", "build", "-p", "ego-convert", "--release"],
        cwd=str(rust_dir),
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        print(f"  ERROR: cargo build failed:\n{result.stderr}")
        return None

    return ego_convert if ego_convert.exists() else None


def convert_to_egorec(episode_dir, egorec_dir, friendly_name, width, height, fps, ego_convert):
    """Convert downloaded episode to .egorec using ego-convert import."""
    video_path = episode_dir / "rgb_video.mp4"
    depth_dir = episode_dir / "d2rgb"
    output_path = egorec_dir / f"{friendly_name}.egorec"

    if output_path.exists():
        print(f"  {friendly_name}: .egorec already exists, skipping conversion")
        return True

    egorec_dir.mkdir(parents=True, exist_ok=True)

    cmd = [
        str(ego_convert),
        "import",
        "--video", str(video_path),
        "--depth-dir", str(depth_dir),
        "--output", str(output_path),
        "--width", str(width),
        "--height", str(height),
        "--fps", str(fps),
        "--session-name", friendly_name,
    ]

    print(f"  {friendly_name}: converting to .egorec...")
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        print(f"  ERROR: ego-convert failed:\n{result.stderr}")
        return False

    # Print the "Wrote N frames" line from stderr
    for line in result.stderr.strip().split("\n"):
        if line.startswith("Wrote "):
            print(f"  {line}")
            break
    return True


def write_dataset_json(egorec_dir, converted_episodes, ego_convert):
    """Write dataset.json manifest for the converted episodes."""
    dataset_path = egorec_dir / "dataset.json"
    if dataset_path.exists():
        print("  dataset.json already exists, skipping")
        return

    episodes = []
    for name in converted_episodes:
        egorec_path = egorec_dir / f"{name}.egorec"
        if not egorec_path.exists():
            continue

        # Get frame count and duration from validate output (no --quiet)
        result = subprocess.run(
            [str(ego_convert), "validate", str(egorec_path)],
            capture_output=True, text=True,
        )
        frames = 0
        duration = 0.0
        for line in result.stdout.split("\n"):
            if "frames" in line:
                m = re.search(r"(\d+) frames, ([\d.]+)s", line)
                if m:
                    frames = int(m.group(1))
                    duration = float(m.group(2))

        episodes.append({
            "filename": f"{name}.egorec",
            "session_name": name,
            "frames": frames,
            "duration_s": duration,
        })

    dataset = {
        "name": "egopat3d",
        "description": "EgoPAT3D egocentric RGB-D manipulation episodes converted to .egorec format.",
        "version": 1,
        "tags": ["egopat3d", "egocentric", "manipulation", "azure-kinect", "benchmark"],
        "source": {
            "dataset": "EgoPAT3D",
            "url": "https://ai4ce.github.io/EgoPAT3D/",
            "paper": "https://arxiv.org/abs/2209.13929",
            "license": "CC BY-NC-SA 4.0",
        },
        "episodes": episodes,
    }

    with open(dataset_path, "w") as f:
        json.dump(dataset, f, indent=2)
    print(f"  Wrote dataset.json ({len(episodes)} episodes)")


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    parser = argparse.ArgumentParser(
        description="Download EgoPAT3D episodes and convert to .egorec",
        epilog=(
            "The EgoPAT3Dv2 dataset on HuggingFace is gated. Request access at:\n"
            "  https://huggingface.co/datasets/qianlima/EgoPAT3Dv2\n\n"
            "Then provide your token via --token, HF_TOKEN env var, or\n"
            "~/.huggingface/token (created by `huggingface-cli login`)."
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "--frames", type=int, default=900,
        help="Max depth frames per episode (default: 900, ~30s at 30fps)",
    )
    parser.add_argument(
        "--output", type=str, default=None,
        help="Output directory (default: datasets/egopat3d)",
    )
    parser.add_argument(
        "--width", type=int, default=1280,
        help="Target width for .egorec (default: 1280)",
    )
    parser.add_argument(
        "--height", type=int, default=720,
        help="Target height for .egorec (default: 720)",
    )
    parser.add_argument(
        "--fps", type=int, default=30,
        help="Target FPS for .egorec (default: 30)",
    )
    parser.add_argument(
        "--token", type=str, default=None,
        help="HuggingFace access token (or set HF_TOKEN env var)",
    )
    parser.add_argument(
        "--skip-convert", action="store_true",
        help="Download only, skip .egorec conversion",
    )
    args = parser.parse_args()

    # Resolve output directory
    script_dir = Path(__file__).resolve().parent
    ego_recorder_dir = script_dir.parent
    if args.output:
        output_dir = Path(args.output).resolve()
    else:
        output_dir = ego_recorder_dir / "datasets" / "egopat3d"

    egorec_dir = output_dir / "egorec"

    # Ensure we can write to the output directory
    # On stations with the ego-recorder systemd service, datasets/ is owned
    # by the ego-recorder system user. We need ACL access.
    if not ensure_directory_writable(output_dir):
        print(f"\nERROR: Cannot write to {output_dir}")
        print(f"On stations with the ego-recorder service, run:")
        user = os.environ.get("USER", "your_user")
        print(f"  sudo mkdir -p {output_dir}")
        print(f"  sudo setfacl -R -m u:{user}:rwx {output_dir}")
        print(f"  sudo setfacl -R -d -m u:{user}:rwx {output_dir}")
        sys.exit(1)

    print("EgoPAT3D Download Pipeline")
    print(f"  Output:     {output_dir}")
    print(f"  Frames:     {args.frames} per episode (~{args.frames / 30:.0f}s at 30fps)")
    print(f"  Resolution: {args.width}x{args.height}")
    print()

    # Resolve HuggingFace token
    hf_token = resolve_hf_token(args.token)
    if not hf_token:
        print("WARNING: No HuggingFace token found.")
        print("  EgoPAT3Dv2 is a gated dataset — downloads will fail without a token.")
        print("  Set via: --token hf_..., HF_TOKEN env var, or huggingface-cli login")
        print()

    session = requests.Session()
    session.headers.update({
        "User-Agent": "ego-recorder/1.0 (research dataset download)",
    })
    if hf_token:
        session.headers["Authorization"] = f"Bearer {hf_token}"

    # Download episodes
    print("Downloading episodes...")
    downloaded = []
    for _scene_name, scene_index, zip_name, friendly_name in EPISODES:
        success = download_episode(
            scene_index, zip_name, friendly_name,
            output_dir, args.frames, session,
        )
        if success:
            downloaded.append(friendly_name)
        print()

    print(f"Downloaded {len(downloaded)}/{len(EPISODES)} episodes")

    if args.skip_convert:
        print("Skipping .egorec conversion (--skip-convert)")
        return

    if not downloaded:
        print("No episodes downloaded, nothing to convert")
        return

    # Find ego-convert
    ego_convert = find_ego_convert()
    if not ego_convert:
        print("ERROR: ego-convert binary not available. Build with:")
        print("  cd rust && cargo build -p ego-convert --release")
        sys.exit(1)

    # Convert to .egorec
    print("\nConverting to .egorec format...")
    converted = []
    for friendly_name in downloaded:
        episode_dir = output_dir / friendly_name
        success = convert_to_egorec(
            episode_dir, egorec_dir, friendly_name,
            args.width, args.height, args.fps, ego_convert,
        )
        if success:
            converted.append(friendly_name)
        print()

    print(f"Converted {len(converted)}/{len(downloaded)} episodes")

    if not converted:
        return

    # Write dataset manifest
    write_dataset_json(egorec_dir, converted, ego_convert)

    # Validate and analyze
    print("\nValidating .egorec files...")
    result = subprocess.run(
        [str(ego_convert), "validate", str(egorec_dir)],
        capture_output=True, text=True,
    )
    print(result.stdout.strip())

    print("\nAnalyzing .egorec files...")
    result = subprocess.run(
        [str(ego_convert), "analyze", "--verbose", str(egorec_dir)],
        capture_output=True, text=True,
    )
    print(result.stdout.strip())

    # Summary
    print("\n" + "=" * 60)
    print(f"Dataset ready: {egorec_dir}")
    print(f"  {len(converted)} episodes converted to .egorec")
    print(f"  Validate: ego-convert validate {egorec_dir}")
    print(f"  Analyze:  ego-convert analyze --verbose {egorec_dir}")
    print(f"  Export:   ego-convert lerobot {egorec_dir}/*.egorec")


if __name__ == "__main__":
    main()
