#!/usr/bin/env python3
"""
ego_uploader.py -- Background service that uploads completed .egorec episodes
to Cloudflare R2 when network connectivity is detected.

Features:
  - WiFi/network detection via nmcli, /sys/class/net, and ip route (3-tier)
  - HTTP connectivity probe to R2 endpoint before uploading
  - Uploads episodes one-by-one via boto3 (S3-compatible)
  - Persistent upload manifest (.upload_manifest.json) with SHA-256 checksums
  - Skips files currently being written (mtime settle check + flock)
  - Retry with exponential backoff on failure
  - Graceful shutdown on SIGINT/SIGTERM

Usage:
  python3 ego_uploader.py [--config upload_config.toml] [--once]

Run as a systemd service alongside ego-recorder for continuous operation.
"""

import argparse
import fcntl
import hashlib
import json
import logging
import os
import signal
import socket
import subprocess
import sys
import threading
import time
from dataclasses import asdict, dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Optional

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib  # Python < 3.11

import boto3
from boto3.s3.transfer import TransferConfig
from botocore.config import Config as BotoConfig
from botocore.exceptions import (
    BotoCoreError,
    ClientError,
    ConnectionClosedError,
    EndpointConnectionError,
    NoCredentialsError,
)

# ---------------------------------------------------------------------------
# Logging
# ---------------------------------------------------------------------------

log = logging.getLogger("ego_uploader")

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

DEFAULT_CONFIG_PATH = Path(__file__).parent.parent / "deploy" / "upload_config.toml"


@dataclass
class CloudCfg:
    endpoint: str = ""
    bucket: str = "ego-data-verlet"
    access_key_id: str = ""
    secret_access_key: str = ""
    region: str = "auto"
    prefix: str = ""


@dataclass
class UploadCfg:
    episodes_dir: str = "/var/lib/ego-recorder"
    poll_interval_s: int = 30
    file_settle_s: int = 10
    connectivity_timeout_s: int = 5
    progress_interval_s: int = 10          # How often to log upload progress
    multipart_chunksize_mb: int = 32       # Multipart upload chunk size
    max_concurrency: int = 4               # Parallel upload threads
    delete_after_upload: bool = False       # Delete local file after verified upload


@dataclass
class FacilityCfg:
    """Facility API config for managed upload flow."""
    enabled: bool = False
    url: str = ""  # e.g. "http://192.168.1.100:8100"
    dataset_name: str = ""  # Defaults to dataset.json name or hostname


@dataclass
class AppConfig:
    cloud: CloudCfg = field(default_factory=CloudCfg)
    upload: UploadCfg = field(default_factory=UploadCfg)
    facility: FacilityCfg = field(default_factory=FacilityCfg)


def load_config(path: Path) -> AppConfig:
    cfg = AppConfig()

    # Load .env file (looks next to this script, then cwd)
    env_path = Path(__file__).parent / ".env"
    if env_path.exists():
        from dotenv import load_dotenv
        load_dotenv(env_path)
    elif Path(".env").exists():
        from dotenv import load_dotenv
        load_dotenv()

    if not path.exists():
        log.warning("Config file %s not found -- using defaults.", path)
    else:
        with open(path, "rb") as f:
            raw = tomllib.load(f)

        cloud = raw.get("cloud", {})
        for k in ("endpoint", "bucket", "region", "prefix"):
            if k in cloud:
                setattr(cfg.cloud, k, cloud[k])

        upload = raw.get("upload", {})
        for k in ("episodes_dir", "poll_interval_s",
                  "file_settle_s", "connectivity_timeout_s",
                  "progress_interval_s", "multipart_chunksize_mb",
                  "max_concurrency", "delete_after_upload"):
            if k in upload:
                setattr(cfg.upload, k, upload[k])

        facility = raw.get("facility", {})
        for k in ("enabled", "url", "dataset_name"):
            if k in facility:
                setattr(cfg.facility, k, facility[k])

    # R2 credentials from environment variables (.env file)
    cfg.cloud.endpoint          = os.environ.get("R2_ENDPOINT", cfg.cloud.endpoint)
    cfg.cloud.access_key_id     = os.environ.get("R2_ACCESS_KEY_ID", cfg.cloud.access_key_id)
    cfg.cloud.secret_access_key = os.environ.get("R2_SECRET_ACCESS_KEY", cfg.cloud.secret_access_key)
    cfg.cloud.bucket            = os.environ.get("R2_BUCKET", cfg.cloud.bucket)

    # Facility URL from env
    if os.environ.get("FACILITY_URL"):
        cfg.facility.url = os.environ["FACILITY_URL"]
        cfg.facility.enabled = True

    return cfg


# ---------------------------------------------------------------------------
# Upload manifest (persistent JSON)
# ---------------------------------------------------------------------------

@dataclass
class UploadRecord:
    filename: str            # Relative path from episodes_dir
    r2_key: str = ""         # Object key in bucket
    uploaded_at: str = ""    # ISO 8601
    size_bytes: int = 0
    sha256: str = ""
    attempt_count: int = 0
    success: bool = False


@dataclass
class UploadManifest:
    version: int = 1
    uploads: list = field(default_factory=list)   # list[UploadRecord]


class ManifestStore:
    """Thread-safe persistent upload manifest."""

    def __init__(self, episodes_dir: str):
        self._path = Path(episodes_dir) / ".upload_manifest.json"
        self._manifest = UploadManifest()
        self._load()

    # --- public queries ---

    @property
    def uploaded_files(self) -> set[str]:
        return {r["filename"] if isinstance(r, dict) else r.filename
                for r in self._manifest.uploads}

    @property
    def uploaded_count(self) -> int:
        return len(self._manifest.uploads)

    # --- mutations ---

    def record_success(self, rec: UploadRecord) -> None:
        self._manifest.uploads.append(asdict(rec))
        self._save()

    # --- persistence ---

    def _load(self) -> None:
        if not self._path.exists():
            return
        try:
            with open(self._path) as f:
                raw = json.load(f)
            self._manifest.version = raw.get("version", 1)
            self._manifest.uploads = raw.get("uploads", [])
        except (json.JSONDecodeError, OSError) as e:
            log.warning("Failed to load upload manifest: %s", e)

    def _save(self) -> None:
        data = {
            "version": self._manifest.version,
            "uploads": self._manifest.uploads,
        }
        tmp = self._path.with_suffix(".json.tmp")
        try:
            with open(tmp, "w") as f:
                json.dump(data, f, indent=2)
                f.write("\n")
                f.flush()
                os.fsync(f.fileno())
            tmp.rename(self._path)
        except OSError as e:
            log.error("Failed to save upload manifest: %s", e)
            try:
                tmp.unlink(missing_ok=True)
            except OSError:
                pass


# ---------------------------------------------------------------------------
# Facility API client (managed upload flow)
# ---------------------------------------------------------------------------

class FacilityClient:
    """Thin HTTP client for the facility API ego endpoints.

    Used to register datasets and episodes before upload, and mark them
    complete after upload. Falls back gracefully if the facility is unreachable.
    """

    def __init__(self, base_url: str):
        self._base_url = base_url.rstrip("/")
        self._dataset_id: Optional[str] = None
        self._s3_prefix: Optional[str] = None

    @property
    def dataset_id(self) -> Optional[str]:
        return self._dataset_id

    @property
    def s3_prefix(self) -> Optional[str]:
        return self._s3_prefix

    def register_dataset(
        self, name: str, description: str = "", tags: list = None, device_id: str = ""
    ) -> bool:
        """Register or look up a dataset. Returns True on success."""
        import urllib.request
        import urllib.error

        payload = json.dumps({
            "name": name,
            "description": description,
            "tags": tags or [],
            "device_id": device_id or socket.gethostname(),
        }).encode()

        try:
            req = urllib.request.Request(
                f"{self._base_url}/facility/ego/datasets",
                data=payload,
                headers={"Content-Type": "application/json"},
                method="POST",
            )
            with urllib.request.urlopen(req, timeout=10) as resp:
                data = json.loads(resp.read())
                self._dataset_id = data.get("dataset_id")
                self._s3_prefix = data.get("s3_prefix")
                log.info(
                    "Registered ego dataset: id=%s prefix=%s (created=%s)",
                    self._dataset_id, self._s3_prefix, data.get("created", True),
                )
                return True
        except (urllib.error.URLError, OSError, json.JSONDecodeError) as e:
            log.warning("Failed to register dataset with facility: %s", e)
            return False

    def register_episode(self, filename: str, metadata: dict) -> Optional[dict]:
        """Register an episode before upload. Returns {episode_id, s3_key} or None."""
        import urllib.request
        import urllib.error

        if not self._dataset_id:
            return None

        payload = json.dumps({"filename": filename, **metadata}).encode()

        try:
            req = urllib.request.Request(
                f"{self._base_url}/facility/ego/datasets/{self._dataset_id}/episodes",
                data=payload,
                headers={"Content-Type": "application/json"},
                method="POST",
            )
            with urllib.request.urlopen(req, timeout=10) as resp:
                return json.loads(resp.read())
        except (urllib.error.URLError, OSError, json.JSONDecodeError) as e:
            log.warning("Failed to register episode with facility: %s", e)
            return None

    def complete_episode(self, episode_id: str, sha256: str, file_size_bytes: int) -> bool:
        """Mark an episode as uploaded. Returns True on success."""
        import urllib.request
        import urllib.error

        payload = json.dumps({
            "status": "uploaded",
            "sha256": sha256,
            "file_size_bytes": file_size_bytes,
        }).encode()

        try:
            req = urllib.request.Request(
                f"{self._base_url}/facility/ego/episodes/{episode_id}",
                data=payload,
                headers={"Content-Type": "application/json"},
                method="PATCH",
            )
            with urllib.request.urlopen(req, timeout=10) as resp:
                return resp.status in (200, 201)
        except (urllib.error.URLError, OSError) as e:
            log.warning("Failed to complete episode on facility: %s", e)
            return False


# ---------------------------------------------------------------------------
# Network detection (3-tier fallback)
# ---------------------------------------------------------------------------

def check_wifi_available() -> bool:
    """Check if any network interface (WiFi or Ethernet) is connected."""
    # Method 1: nmcli (NetworkManager -- most reliable on desktop/laptop Linux)
    try:
        out = subprocess.run(
            ["nmcli", "-t", "-f", "TYPE,STATE", "device"],
            capture_output=True, text=True, timeout=5,
        )
        if out.returncode == 0:
            for line in out.stdout.splitlines():
                if "wifi:connected" in line or "ethernet:connected" in line:
                    return True
            # nmcli ran OK but nothing connected -- trust it
            return False
    except (FileNotFoundError, subprocess.TimeoutExpired, OSError):
        pass

    # Method 2: /sys/class/net operstate
    try:
        net_dir = Path("/sys/class/net")
        if net_dir.exists():
            for iface in net_dir.iterdir():
                if iface.name == "lo":
                    continue
                operstate = iface / "operstate"
                if operstate.exists():
                    state = operstate.read_text().strip()
                    if state == "up":
                        return True
    except OSError:
        pass

    # Method 3: ip route default
    try:
        out = subprocess.run(
            ["ip", "route", "show", "default"],
            capture_output=True, text=True, timeout=5,
        )
        if out.returncode == 0 and "default" in out.stdout:
            return True
    except (FileNotFoundError, subprocess.TimeoutExpired, OSError):
        pass

    return False


def check_connectivity(endpoint: str, timeout: int = 5) -> bool:
    """Check actual internet connectivity by probing the R2 endpoint."""
    if not check_wifi_available():
        return False

    # Parse host from endpoint URL
    host = endpoint
    for prefix in ("https://", "http://"):
        if host.startswith(prefix):
            host = host[len(prefix):]
            break
    host = host.rstrip("/")

    # Quick TCP connect to port 443
    try:
        sock = socket.create_connection((host, 443), timeout=timeout)
        sock.close()
        return True
    except (socket.timeout, socket.error, OSError):
        pass

    return False


# ---------------------------------------------------------------------------
# File scanning
# ---------------------------------------------------------------------------

@dataclass
class PendingFile:
    abs_path: Path
    rel_path: str
    size_bytes: int
    mtime: float = 0.0


def is_file_locked(path: Path) -> bool:
    """Check if a file is locked by another process (flock)."""
    try:
        fd = os.open(str(path), os.O_RDONLY)
    except OSError:
        return True
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        fcntl.flock(fd, fcntl.LOCK_UN)
        return False
    except (OSError, BlockingIOError):
        return True
    finally:
        os.close(fd)


def scan_pending_episodes(
    episodes_dir: Path,
    manifest: ManifestStore,
    settle_s: int,
    subdirectory: Optional[str] = None,
) -> list[PendingFile]:
    """Scan episodes_dir for .egorec files not yet uploaded.

    If subdirectory is given, only scan that subdirectory (dataset name).
    """
    uploaded = manifest.uploaded_files
    now = time.time()
    pending: list[PendingFile] = []

    if not episodes_dir.exists():
        return pending

    scan_root = episodes_dir / subdirectory if subdirectory else episodes_dir
    if not scan_root.exists():
        return pending

    for path in sorted(scan_root.rglob("*.egorec")):
        if not path.is_file():
            continue

        # Skip files inside .pruned/ directories (removed by ego-qc prune/splice)
        if ".pruned" in path.parts:
            continue

        rel = str(path.relative_to(episodes_dir))

        # Skip already uploaded
        if rel in uploaded:
            continue

        # Skip very small files (incomplete, header-only)
        try:
            stat = path.stat()
        except OSError:
            continue
        if stat.st_size < 1024:
            continue

        # Skip recently modified files (still being written)
        age = now - stat.st_mtime
        if age < settle_s:
            continue

        # Skip locked files
        if is_file_locked(path):
            continue

        pending.append(PendingFile(
            abs_path=path,
            rel_path=rel,
            size_bytes=stat.st_size,
            mtime=stat.st_mtime,
        ))

    # Sort by mtime (oldest first), using cached stat to avoid TOCTOU race
    pending.sort(key=lambda f: f.mtime)
    return pending


# ---------------------------------------------------------------------------
# SHA-256 helper
# ---------------------------------------------------------------------------

def sha256_file(path: Path, label: str = "") -> str:
    """Compute SHA-256 of a file with progress logging for large files."""
    file_size = path.stat().st_size
    h = hashlib.sha256()
    bytes_read = 0
    start = time.monotonic()
    last_log = start

    with open(path, "rb") as f:
        while True:
            chunk = f.read(1 << 20)  # 1 MB chunks
            if not chunk:
                break
            h.update(chunk)
            bytes_read += len(chunk)

            now = time.monotonic()
            if now - last_log >= 5.0 and file_size > 100 * 1024 * 1024:  # log every 5s for >100MB
                pct = (bytes_read / file_size) * 100 if file_size else 0
                speed = bytes_read / (now - start) / (1024 * 1024)
                log.info(
                    "  ↳ checksumming %s: %.1f%% (%.0f MB/s)",
                    label or path.name, pct, speed,
                )
                last_log = now

    elapsed = time.monotonic() - start
    speed = bytes_read / elapsed / (1024 * 1024) if elapsed > 0 else 0
    hexdigest = h.hexdigest()
    log.info(
        "Checksum complete: %s sha256=%s…  (%.1f MB in %.1fs, %.0f MB/s disk read)",
        label or path.name, hexdigest[:16],
        bytes_read / (1024 * 1024), elapsed, speed,
    )
    return hexdigest


# ---------------------------------------------------------------------------
# Upload progress tracker
# ---------------------------------------------------------------------------

def _fmt_size(b: float) -> str:
    """Human-readable file size."""
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if abs(b) < 1024:
            return f"{b:.1f} {unit}"
        b /= 1024
    return f"{b:.1f} PB"


def _fmt_duration(seconds: float) -> str:
    """Human-readable duration."""
    if seconds < 60:
        return f"{seconds:.0f}s"
    m, s = divmod(int(seconds), 60)
    if m < 60:
        return f"{m}m {s:02d}s"
    h, m = divmod(m, 60)
    return f"{h}h {m:02d}m {s:02d}s"


class UploadProgressTracker:
    """boto3 upload Callback that logs bandwidth, speed, ETA, and progress."""

    def __init__(self, filename: str, total_bytes: int, log_interval_s: int = 10):
        self._filename = filename
        self._total = total_bytes
        self._log_interval = log_interval_s
        self._lock = threading.Lock()
        self._bytes_transferred = 0
        self._start = time.monotonic()
        self._last_log_time = self._start
        self._last_log_bytes = 0

    def __call__(self, bytes_amount: int) -> None:
        with self._lock:
            self._bytes_transferred += bytes_amount
            now = time.monotonic()
            since_last = now - self._last_log_time

            if since_last >= self._log_interval:
                self._log_progress(now)

    def _log_progress(self, now: float) -> None:
        elapsed = now - self._start
        transferred = self._bytes_transferred
        total = self._total

        pct = (transferred / total * 100) if total > 0 else 0
        avg_speed = transferred / elapsed if elapsed > 0 else 0

        # Current interval speed
        interval_bytes = transferred - self._last_log_bytes
        interval_time = now - self._last_log_time
        cur_speed = interval_bytes / interval_time if interval_time > 0 else avg_speed

        # ETA based on average speed
        remaining_bytes = total - transferred
        eta_s = remaining_bytes / avg_speed if avg_speed > 0 else 0

        log.info(
            "  ↳ %5.1f%% | %s / %s | %s/s cur, %s/s avg | ETA %s",
            pct,
            _fmt_size(transferred), _fmt_size(total),
            _fmt_size(cur_speed), _fmt_size(avg_speed),
            _fmt_duration(eta_s),
        )

        self._last_log_time = now
        self._last_log_bytes = transferred

    def finish_summary(self) -> tuple[float, float]:
        """Log final summary. Returns (elapsed_s, avg_speed_bytes_per_s)."""
        elapsed = time.monotonic() - self._start
        avg_speed = self._bytes_transferred / elapsed if elapsed > 0 else 0
        log.info(
            "✓ Uploaded %s — %s in %s (%s/s avg)",
            self._filename,
            _fmt_size(self._bytes_transferred),
            _fmt_duration(elapsed),
            _fmt_size(avg_speed),
        )
        return elapsed, avg_speed


class SessionStats:
    """Tracks cumulative upload statistics for a poll cycle."""

    def __init__(self):
        self.files_uploaded: int = 0
        self.total_bytes: int = 0
        self.total_elapsed: float = 0.0
        self.start_time: float = time.monotonic()

    def record(self, size_bytes: int, elapsed_s: float) -> None:
        self.files_uploaded += 1
        self.total_bytes += size_bytes
        self.total_elapsed += elapsed_s

    def log_summary(self) -> None:
        if self.files_uploaded == 0:
            return
        wall_time = time.monotonic() - self.start_time
        avg_speed = self.total_bytes / self.total_elapsed if self.total_elapsed > 0 else 0
        log.info(
            "── Upload cycle complete: %d file(s), %s total, %s wall time, %s/s avg ──",
            self.files_uploaded,
            _fmt_size(self.total_bytes),
            _fmt_duration(wall_time),
            _fmt_size(avg_speed),
        )


# ---------------------------------------------------------------------------
# S3/R2 uploader
# ---------------------------------------------------------------------------

def make_s3_client(cloud: CloudCfg):
    return boto3.client(
        "s3",
        endpoint_url=cloud.endpoint,
        aws_access_key_id=cloud.access_key_id,
        aws_secret_access_key=cloud.secret_access_key,
        region_name=cloud.region,
        config=BotoConfig(
            retries={"max_attempts": 0},  # We handle retries ourselves
            connect_timeout=30,
            read_timeout=120,
            tcp_keepalive=True,
        ),
    )


def make_transfer_config(upload_cfg: UploadCfg) -> TransferConfig:
    """Build boto3 TransferConfig tuned for large .egorec files."""
    return TransferConfig(
        multipart_chunksize=upload_cfg.multipart_chunksize_mb * 1024 * 1024,
        max_concurrency=upload_cfg.max_concurrency,
        multipart_threshold=upload_cfg.multipart_chunksize_mb * 1024 * 1024,
        use_threads=True,
    )


def make_object_key(prefix: str, rel_path: str) -> str:
    key = prefix
    if key and not key.endswith("/"):
        key += "/"
    key += rel_path
    return key


def upload_file(
    s3_client,
    bucket: str,
    pending: PendingFile,
    object_key: str,
    transfer_config: TransferConfig,
    progress_interval_s: int = 10,
) -> tuple[bool, float]:
    """Upload a single file to R2 with progress tracking.

    Returns (success, elapsed_seconds).
    """
    tracker = UploadProgressTracker(
        filename=pending.rel_path,
        total_bytes=pending.size_bytes,
        log_interval_s=progress_interval_s,
    )
    try:
        s3_client.upload_file(
            str(pending.abs_path),
            bucket,
            object_key,
            ExtraArgs={"ContentType": "application/octet-stream"},
            Config=transfer_config,
            Callback=tracker,
        )
        elapsed, _ = tracker.finish_summary()
        return True, elapsed
    except (BotoCoreError, ClientError, ConnectionClosedError,
            EndpointConnectionError, OSError) as e:
        log.error("Upload failed for %s: %s", pending.rel_path, e)
        return False, 0.0


def verify_upload_on_r2(
    s3_client,
    bucket: str,
    object_key: str,
    expected_size: int,
) -> bool:
    """Verify an object exists in R2 with the expected size via head_object."""
    try:
        resp = s3_client.head_object(Bucket=bucket, Key=object_key)
        remote_size = resp.get("ContentLength", 0)
        if remote_size == expected_size:
            log.info(
                "R2 verification passed: %s (%s)",
                object_key, _fmt_size(remote_size),
            )
            return True
        else:
            log.error(
                "R2 verification FAILED: %s — expected %s, got %s",
                object_key, _fmt_size(expected_size), _fmt_size(remote_size),
            )
            return False
    except (BotoCoreError, ClientError) as e:
        log.error("R2 verification error for %s: %s", object_key, e)
        return False


# ---------------------------------------------------------------------------
# Main upload loop
# ---------------------------------------------------------------------------

_shutdown = False


def _handle_signal(signum, _frame):
    global _shutdown
    sig_name = signal.Signals(signum).name
    log.info("Received %s -- shutting down.", sig_name)
    _shutdown = True


def upload_loop(cfg: AppConfig, *, once: bool = False, dataset_filter: Optional[str] = None) -> None:
    global _shutdown

    episodes_dir = Path(cfg.upload.episodes_dir)
    if not episodes_dir.exists():
        log.error("Episodes directory does not exist: %s", episodes_dir)
        sys.exit(1)

    manifest = ManifestStore(str(episodes_dir))
    log.info(
        "Starting uploader (dir=%s, bucket=%s, prefix=%s, %d already uploaded)",
        episodes_dir, cfg.cloud.bucket, cfg.cloud.prefix or "(none)",
        manifest.uploaded_count,
    )
    log.info(
        "Upload config: multipart=%dMB, concurrency=%d, progress_interval=%ds, delete_after_upload=%s",
        cfg.upload.multipart_chunksize_mb, cfg.upload.max_concurrency,
        cfg.upload.progress_interval_s, cfg.upload.delete_after_upload,
    )

    # Facility API client (managed upload flow)
    facility: Optional[FacilityClient] = None
    if cfg.facility.enabled and cfg.facility.url:
        facility = FacilityClient(cfg.facility.url)
        # Try to read dataset.json for metadata
        ds_name = cfg.facility.dataset_name or socket.gethostname()
        ds_desc = ""
        ds_tags: list = []
        dataset_json = episodes_dir / "dataset.json"
        if dataset_json.exists():
            try:
                with open(dataset_json) as f:
                    ds_meta = json.load(f)
                ds_name = ds_meta.get("name", ds_name)
                ds_desc = ds_meta.get("description", "")
                ds_tags = ds_meta.get("tags", [])
            except (json.JSONDecodeError, OSError):
                pass
        facility.register_dataset(ds_name, ds_desc, ds_tags)
        log.info("Facility mode enabled: %s", cfg.facility.url)

    s3_client: Optional[object] = None
    transfer_config: Optional[TransferConfig] = None

    # Per-file retry tracking: rel_path -> (attempts, last_attempt_time)
    retry_state: dict[str, tuple[int, float]] = {}
    was_disconnected = False

    # Max backoff cap: 5 minutes (backoff grows as min(2^attempts * 10, 300))
    MAX_BACKOFF_S = 300

    while not _shutdown:
        # Check connectivity
        has_conn = check_connectivity(
            cfg.cloud.endpoint,
            timeout=cfg.upload.connectivity_timeout_s,
        )
        if not has_conn:
            was_disconnected = True
            if once:
                log.warning("No connectivity -- exiting (--once mode).")
                return
            log.debug("No connectivity -- sleeping %ds.", cfg.upload.poll_interval_s)
            _interruptible_sleep(cfg.upload.poll_interval_s)
            continue

        # Connection restored after a drop -- reset all backoff timers so
        # every previously-failed file gets a fresh attempt immediately.
        if was_disconnected:
            if retry_state:
                log.info(
                    "Connectivity restored -- resetting backoff for %d file(s).",
                    len(retry_state),
                )
                retry_state.clear()
            was_disconnected = False

        # Lazy-init S3 client (deferred until first connectivity)
        if s3_client is None:
            try:
                s3_client = make_s3_client(cfg.cloud)
                transfer_config = make_transfer_config(cfg.upload)
                # Validate credentials with a lightweight call
                s3_client.head_bucket(Bucket=cfg.cloud.bucket)
                log.info("Connected to R2 bucket: %s", cfg.cloud.bucket)
            except NoCredentialsError:
                log.error("R2 credentials missing or invalid.")
                s3_client = None
                _interruptible_sleep(cfg.upload.poll_interval_s)
                continue
            except (BotoCoreError, ClientError) as e:
                log.error("Cannot reach R2 bucket %s: %s", cfg.cloud.bucket, e)
                s3_client = None
                _interruptible_sleep(cfg.upload.poll_interval_s)
                continue

        # Scan for pending episodes
        pending = scan_pending_episodes(
            episodes_dir, manifest, cfg.upload.file_settle_s,
            subdirectory=dataset_filter,
        )
        if pending:
            total_size = sum(pf.size_bytes for pf in pending)
            log.info(
                "Found %d pending episode(s) to upload (%s total)",
                len(pending), _fmt_size(total_size),
            )

        session = SessionStats()

        for pf in pending:
            if _shutdown:
                break

            # Check retry backoff (capped, never gives up)
            if pf.rel_path in retry_state:
                attempts, last_time = retry_state[pf.rel_path]
                backoff = min((2 ** attempts) * 10, MAX_BACKOFF_S)
                if time.time() - last_time < backoff:
                    continue

            # Re-check connectivity before each upload
            if not check_connectivity(
                cfg.cloud.endpoint,
                timeout=cfg.upload.connectivity_timeout_s,
            ):
                log.warning("Lost connectivity -- pausing uploads.")
                was_disconnected = True
                break

            # Register with facility and determine object key
            episode_id: Optional[str] = None
            if facility and facility.dataset_id:
                # Read .egorec header for metadata
                ep_metadata: dict = {}
                try:
                    from egorec_header import read_metadata
                    meta = read_metadata(pf.abs_path)
                    ep_metadata = meta.to_episode_dict()
                except Exception as e:
                    log.debug("Could not read .egorec header for %s: %s", pf.rel_path, e)

                reg = facility.register_episode(pf.rel_path, ep_metadata)
                if reg:
                    episode_id = reg.get("episode_id")
                    object_key = reg.get("s3_key", make_object_key(cfg.cloud.prefix, pf.rel_path))
                else:
                    object_key = make_object_key(cfg.cloud.prefix, pf.rel_path)
            else:
                object_key = make_object_key(cfg.cloud.prefix, pf.rel_path)

            prev_attempts = retry_state.get(pf.rel_path, (0, 0))[0]

            # Pre-compute SHA-256 before upload
            log.info(
                "Preparing %s (%s, attempt %d) -> %s/%s",
                pf.rel_path,
                _fmt_size(pf.size_bytes),
                prev_attempts + 1,
                cfg.cloud.bucket,
                object_key,
            )
            checksum = sha256_file(pf.abs_path, label=pf.rel_path)

            log.info(
                "Uploading %s (%s)...",
                pf.rel_path, _fmt_size(pf.size_bytes),
            )

            success, elapsed = upload_file(
                s3_client, cfg.cloud.bucket, pf, object_key,
                transfer_config=transfer_config,
                progress_interval_s=cfg.upload.progress_interval_s,
            )

            if success:
                retry_state.pop(pf.rel_path, None)
                rec = UploadRecord(
                    filename=pf.rel_path,
                    r2_key=object_key,
                    uploaded_at=datetime.now(timezone.utc).isoformat(),
                    size_bytes=pf.size_bytes,
                    sha256=checksum,
                    attempt_count=prev_attempts + 1,
                    success=True,
                )
                manifest.record_success(rec)
                session.record(pf.size_bytes, elapsed)

                # Notify facility of upload completion
                if facility and episode_id:
                    facility.complete_episode(episode_id, checksum, pf.size_bytes)

                # Verified delete: confirm the object exists on R2 before deleting locally
                if cfg.upload.delete_after_upload:
                    if verify_upload_on_r2(s3_client, cfg.cloud.bucket, object_key, pf.size_bytes):
                        try:
                            pf.abs_path.unlink()
                            log.info(
                                "Deleted local file after R2-verified upload: %s",
                                pf.rel_path,
                            )
                        except OSError as e:
                            log.error(
                                "Failed to delete local file %s: %s",
                                pf.rel_path, e,
                            )
                    else:
                        log.warning(
                            "Keeping local file %s -- R2 verification failed, will NOT delete.",
                            pf.rel_path,
                        )
            else:
                new_attempts = prev_attempts + 1
                retry_state[pf.rel_path] = (new_attempts, time.time())
                next_backoff = min((2 ** new_attempts) * 10, MAX_BACKOFF_S)
                log.warning(
                    "Failed upload attempt %d for %s (next retry in %s)",
                    new_attempts, pf.rel_path, _fmt_duration(next_backoff),
                )
                # Re-create client on failure (connection may be stale)
                s3_client = None

        session.log_summary()

        if once:
            log.info("--once mode: scan complete, exiting.")
            return

        _interruptible_sleep(cfg.upload.poll_interval_s)

    log.info(
        "Uploader stopped (%d uploaded total).",
        manifest.uploaded_count,
    )


def _interruptible_sleep(seconds: int) -> None:
    """Sleep in 1-second increments so shutdown is responsive."""
    for _ in range(seconds):
        if _shutdown:
            break
        time.sleep(1)


# ---------------------------------------------------------------------------
# Dataset discovery (for interactive mode)
# ---------------------------------------------------------------------------

@dataclass
class DatasetInfo:
    """Summary of a discovered dataset in the episodes directory."""
    name: str
    path: Path
    episode_count: int = 0
    pending_count: int = 0
    total_size: int = 0
    pending_size: int = 0
    description: str = ""


def discover_datasets(
    episodes_dir: Path,
    manifest: ManifestStore,
    settle_s: int,
) -> list[DatasetInfo]:
    """Find dataset subdirectories (those with dataset.json) and summarize each."""
    datasets: list[DatasetInfo] = []
    if not episodes_dir.exists():
        return datasets

    for child in sorted(episodes_dir.iterdir()):
        if not child.is_dir():
            continue
        ds_json = child / "dataset.json"
        if not ds_json.exists():
            continue

        info = DatasetInfo(name=child.name, path=child)

        # Read dataset.json for metadata
        try:
            with open(ds_json) as f:
                meta = json.load(f)
            info.description = meta.get("description", "")
            info.episode_count = len(meta.get("episodes", []))
        except (json.JSONDecodeError, OSError):
            pass

        # Count all .egorec files and pending ones
        all_egorec = list(child.rglob("*.egorec"))
        for ef in all_egorec:
            if not ef.is_file() or ".pruned" in ef.parts:
                continue
            try:
                st = ef.stat()
            except OSError:
                continue
            info.total_size += st.st_size

        # Get pending files for this dataset
        pending = scan_pending_episodes(
            episodes_dir, manifest, settle_s, subdirectory=child.name,
        )
        info.pending_count = len(pending)
        info.pending_size = sum(pf.size_bytes for pf in pending)

        datasets.append(info)

    return datasets


# ---------------------------------------------------------------------------
# Interactive upload mode
# ---------------------------------------------------------------------------

BOLD = "\033[1m"
CYAN = "\033[0;36m"
GREEN = "\033[0;32m"
YELLOW = "\033[1;33m"
DIM = "\033[2m"
RED = "\033[0;31m"
NC = "\033[0m"


def _collect_scan_dirs(cfg_episodes_dir: Path) -> list[Path]:
    """Return deduplicated list of directories to scan for datasets.

    Always includes the configured episodes_dir.  Also includes the local
    ``datasets/`` directory next to the source tree (if it exists and is
    different from episodes_dir).
    """
    dirs: list[Path] = []
    if cfg_episodes_dir.exists():
        dirs.append(cfg_episodes_dir.resolve())

    local_datasets = (Path(__file__).parent.parent / "datasets").resolve()
    if local_datasets.exists() and local_datasets not in dirs:
        dirs.append(local_datasets)

    return dirs


def interactive_upload(cfg: AppConfig) -> None:
    """Interactive upload session: pick dataset, choose delete, upload."""
    episodes_dir = Path(cfg.upload.episodes_dir)
    scan_dirs = _collect_scan_dirs(episodes_dir)

    if not scan_dirs:
        print(f"{RED}No episode directories found.{NC}")
        sys.exit(1)

    # Use first available dir for the manifest
    manifest = ManifestStore(str(scan_dirs[0]))

    print()
    print(f"{BOLD}ego-uploader{NC} — interactive mode")
    print("───────────────────────────────────")
    for sd in scan_dirs:
        print(f"  Directory:       {DIM}{sd}{NC}")
    print(f"  Bucket:          {BOLD}{cfg.cloud.bucket}{NC}")
    print(f"  Prefix:          {DIM}{cfg.cloud.prefix or '(none)'}{NC}")
    print(f"  Already uploaded: {manifest.uploaded_count} file(s)")
    print()

    # Discover datasets from all scan directories
    print(f"{DIM}Scanning for datasets...{NC}")
    datasets: list[DatasetInfo] = []
    seen_names: set[str] = set()
    for sd in scan_dirs:
        for ds in discover_datasets(sd, manifest, cfg.upload.file_settle_s):
            if ds.name not in seen_names:
                datasets.append(ds)
                seen_names.add(ds.name)

    if not datasets:
        print(f"{YELLOW}No datasets found in {', '.join(str(d) for d in scan_dirs)}{NC}")
        print(f"{DIM}(Datasets must contain a dataset.json manifest){NC}")
        sys.exit(0)

    # Also check for loose .egorec files not in any dataset
    all_pending: list[PendingFile] = []
    for sd in scan_dirs:
        all_pending.extend(scan_pending_episodes(
            sd, manifest, cfg.upload.file_settle_s,
        ))
    dataset_pending = sum(ds.pending_count for ds in datasets)
    loose_pending = len(all_pending) - dataset_pending

    # Display dataset menu
    print(f"{BOLD}Datasets:{NC}")
    print()
    total_pending = 0
    total_pending_size = 0
    for i, ds in enumerate(datasets, 1):
        pending_label = (
            f"{GREEN}{ds.pending_count} pending ({_fmt_size(ds.pending_size)}){NC}"
            if ds.pending_count > 0
            else f"{DIM}0 pending{NC}"
        )
        desc_label = f"  {DIM}{ds.description}{NC}" if ds.description else ""
        print(
            f"  {CYAN}{i}){NC} {BOLD}{ds.name}{NC}"
            f"  — {ds.episode_count} episode(s), {pending_label}{desc_label}"
        )
        total_pending += ds.pending_count
        total_pending_size += ds.pending_size

    print()
    if loose_pending > 0:
        print(f"  {DIM}+ {loose_pending} loose .egorec file(s) not in any dataset{NC}")
    print(f"  {CYAN}A){NC} {BOLD}All datasets{NC}  — {total_pending} pending ({_fmt_size(total_pending_size)}")
    if loose_pending > 0:
        print(f"  {CYAN}L){NC} {BOLD}All (including loose files){NC}  — {len(all_pending)} pending")
    print()

    # Get selection
    choice = input("Select dataset (number, A=all, L=loose+all, q=quit): ").strip()

    if choice.lower() in ("q", "quit", "exit"):
        print("Cancelled.")
        return

    selected_ds: Optional[DatasetInfo] = None  # None = all datasets
    include_loose = False

    if choice.lower() == "a":
        # Upload all datasets (but not loose files)
        selected_ds = None
    elif choice.lower() == "l":
        # Upload everything (all datasets + loose files)
        selected_ds = None
        include_loose = True
    elif choice.isdigit():
        idx = int(choice) - 1
        if 0 <= idx < len(datasets):
            selected_ds = datasets[idx]
        else:
            print(f"{RED}Invalid selection.{NC}")
            return
    else:
        # Try matching by name
        matches = [ds for ds in datasets if ds.name.lower() == choice.lower()]
        if matches:
            selected_ds = matches[0]
        else:
            print(f"{RED}No dataset matching '{choice}'.{NC}")
            return

    # Get pending count for selection
    if selected_ds:
        sel_pending = selected_ds.pending_count
        sel_size = selected_ds.pending_size
    elif include_loose:
        sel_pending = len(all_pending)
        sel_size = sum(pf.size_bytes for pf in all_pending)
    else:
        sel_pending = total_pending
        sel_size = total_pending_size

    if sel_pending == 0:
        print(f"\n{GREEN}Nothing to upload — all episodes are already synced.{NC}")
        return

    # Ask about delete-after-upload
    print()
    print(f"{BOLD}Delete local files after upload?{NC}")
    print(f"  {DIM}Files are only deleted after R2 verification (head_object size check).{NC}")
    print(f"  {CYAN}1){NC} {BOLD}No{NC}  — keep local files {DIM}(default){NC}")
    print(f"  {CYAN}2){NC} {BOLD}Yes{NC} — delete after verified upload")
    print()
    delete_choice = input("Choice [1]: ").strip()
    delete_after = delete_choice == "2"

    # Confirmation
    dataset_label = selected_ds.name if selected_ds else ("all (+ loose)" if include_loose else "all datasets")
    print()
    print("───────────────────────────────────")
    print(f"  Dataset:    {BOLD}{dataset_label}{NC}")
    print(f"  Pending:    {BOLD}{sel_pending} file(s) ({_fmt_size(sel_size)}){NC}")
    print(f"  Bucket:     {BOLD}{cfg.cloud.bucket}{NC}")
    print(f"  Delete:     {BOLD}{'Yes (R2-verified)' if delete_after else 'No'}{NC}")
    print("───────────────────────────────────")
    print()

    confirm = input(f"Start upload? [Y/n] ").strip()
    if confirm.lower().startswith("n"):
        print("Cancelled.")
        return

    # Apply settings and run
    cfg.upload.delete_after_upload = delete_after

    # Point episodes_dir at the selected dataset's parent so upload_loop
    # finds files in the right place (may be local datasets/ dir).
    if selected_ds:
        cfg.upload.episodes_dir = str(selected_ds.path.parent)
        dataset_filter = selected_ds.name
    else:
        # "All" — run upload_loop once per scan dir
        dataset_filter = None

    if include_loose or not selected_ds:
        # Upload from each scan directory
        print()
        for sd in scan_dirs:
            cfg.upload.episodes_dir = str(sd)
            upload_loop(cfg, once=True, dataset_filter=dataset_filter)
    else:
        print()
        upload_loop(cfg, once=True, dataset_filter=dataset_filter)


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------

def main() -> None:
    parser = argparse.ArgumentParser(
        description="Upload .egorec episodes to Cloudflare R2",
    )
    parser.add_argument(
        "--config", "-c",
        type=Path,
        default=DEFAULT_CONFIG_PATH,
        help="Path to TOML config file (default: upload_config.toml)",
    )
    parser.add_argument(
        "--once",
        action="store_true",
        help="Run a single scan+upload pass then exit",
    )
    parser.add_argument(
        "--interactive", "-i",
        action="store_true",
        help="Interactive mode: choose dataset, delete behavior, then upload",
    )
    parser.add_argument(
        "--dataset", "-d",
        type=str,
        default=None,
        help="Upload only this dataset subdirectory (non-interactive)",
    )
    parser.add_argument(
        "--delete",
        action="store_true",
        help="Delete local files after R2-verified upload (overrides config)",
    )
    parser.add_argument(
        "--verbose", "-v",
        action="store_true",
        help="Enable debug logging",
    )
    args = parser.parse_args()

    logging.basicConfig(
        level=logging.DEBUG if args.verbose else logging.INFO,
        format="%(asctime)s [%(levelname)s] %(message)s",
        datefmt="%Y-%m-%d %H:%M:%S",
    )

    cfg = load_config(args.config)

    # CLI overrides
    if args.delete:
        cfg.upload.delete_after_upload = True

    # Validate required config
    if not cfg.cloud.endpoint:
        log.error("R2_ENDPOINT not set in .env file. Exiting.")
        sys.exit(1)
    if not cfg.cloud.access_key_id or not cfg.cloud.secret_access_key:
        log.error("R2_ACCESS_KEY_ID / R2_SECRET_ACCESS_KEY not set in .env file. Exiting.")
        sys.exit(1)

    signal.signal(signal.SIGINT, _handle_signal)
    signal.signal(signal.SIGTERM, _handle_signal)

    if args.interactive:
        interactive_upload(cfg)
    else:
        upload_loop(cfg, once=args.once, dataset_filter=args.dataset)


if __name__ == "__main__":
    main()
