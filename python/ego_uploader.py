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
                  "file_settle_s", "connectivity_timeout_s"):
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
) -> list[PendingFile]:
    """Scan episodes_dir for .egorec files not yet uploaded."""
    uploaded = manifest.uploaded_files
    now = time.time()
    pending: list[PendingFile] = []

    if not episodes_dir.exists():
        return pending

    for path in sorted(episodes_dir.rglob("*.egorec")):
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

def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while True:
            chunk = f.read(65536)
            if not chunk:
                break
            h.update(chunk)
    return h.hexdigest()


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
) -> bool:
    """Upload a single file to R2. Returns True on success."""
    try:
        s3_client.upload_file(
            str(pending.abs_path),
            bucket,
            object_key,
            ExtraArgs={"ContentType": "application/octet-stream"},
        )
        return True
    except (BotoCoreError, ClientError, ConnectionClosedError,
            EndpointConnectionError, OSError) as e:
        log.error("Upload failed for %s: %s", pending.rel_path, e)
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


def upload_loop(cfg: AppConfig, *, once: bool = False) -> None:
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
        )
        if pending:
            log.info("Found %d pending episode(s) to upload.", len(pending))

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
            log.info(
                "Uploading %s (%.1f MB, attempt %d) -> %s/%s",
                pf.rel_path,
                pf.size_bytes / (1024 * 1024),
                prev_attempts + 1,
                cfg.cloud.bucket,
                object_key,
            )

            success = upload_file(s3_client, cfg.cloud.bucket, pf, object_key)

            if success:
                checksum = sha256_file(pf.abs_path)
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

                # Notify facility of upload completion
                if facility and episode_id:
                    facility.complete_episode(episode_id, checksum, pf.size_bytes)

                log.info(
                    "Uploaded %s (sha256=%s...)",
                    pf.rel_path, checksum[:12],
                )
            else:
                new_attempts = prev_attempts + 1
                retry_state[pf.rel_path] = (new_attempts, time.time())
                next_backoff = min((2 ** new_attempts) * 10, MAX_BACKOFF_S)
                log.warning(
                    "Failed upload attempt %d for %s (next retry in %ds)",
                    new_attempts, pf.rel_path, next_backoff,
                )
                # Re-create client on failure (connection may be stale)
                s3_client = None

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

    # Validate required config
    if not cfg.cloud.endpoint:
        log.error("R2_ENDPOINT not set in .env file. Exiting.")
        sys.exit(1)
    if not cfg.cloud.access_key_id or not cfg.cloud.secret_access_key:
        log.error("R2_ACCESS_KEY_ID / R2_SECRET_ACCESS_KEY not set in .env file. Exiting.")
        sys.exit(1)

    signal.signal(signal.SIGINT, _handle_signal)
    signal.signal(signal.SIGTERM, _handle_signal)

    upload_loop(cfg, once=args.once)


if __name__ == "__main__":
    main()
