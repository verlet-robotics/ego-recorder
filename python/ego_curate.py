#!/usr/bin/env python3
"""Curate raw ego-recorder uploads into reviewable task artifacts.

This pipeline keeps raw R2 objects immutable. It inventories recordings,
downloads selected files to a local workspace, runs `ego-qc` for episode-level
quality control, proposes conservative manipulation intervals, generates
storyboards for VLM classification, clusters open-ended labels into task
buckets, and publishes only derived metadata/proxies back to R2.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import importlib
import json
import math
import mimetypes
import os
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from collections import Counter, defaultdict
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path, PurePosixPath
from typing import Any, Iterable

import boto3
from botocore.config import Config as BotoConfig
from botocore.exceptions import BotoCoreError, ClientError
from dotenv import load_dotenv

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib  # type: ignore

from egorec_header import (
    FOOTER_SIZE,
    HEADER_SIZE,
    read_footer,
    read_header,
    read_metadata,
)


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONFIG_PATH = ROOT / "deploy" / "upload_config.toml"
DEFAULT_ENV_PATH = Path(__file__).with_name(".env")
PIPELINE_VERSION = "v1"
DEFAULT_WORKSPACE = ROOT / "curation-workspace"

SHORT_EPISODE_REJECT_S = 5.0
FULL_EPISODE_CLASSIFY_S = 120.0
INTERVAL_MIN_GAP_S = 1.0
INTERVAL_MIN_DURATION_S = 1.5
INTERVAL_PAD_S = 0.5
HAND_SAMPLE_FPS = 5.0
HAND_MIN_AREA_FRACTION = 0.01
HAND_MAX_SEGMENT_S = 20.0
HAND_MIN_DETECTION_CONFIDENCE = 0.5
VLM_MAX_WINDOW_S = 180.0
VLM_WINDOW_OVERLAP_S = 15.0
CLUSTER_SIMILARITY_THRESHOLD = 0.82
LOW_CONFIDENCE_THRESHOLD = 0.55


def load_runtime_env() -> None:
    if DEFAULT_ENV_PATH.exists():
        load_dotenv(DEFAULT_ENV_PATH)
    elif Path(".env").exists():
        load_dotenv(Path(".env"))


@dataclass
class R2Config:
    endpoint: str
    bucket: str
    access_key_id: str
    secret_access_key: str
    region: str = "auto"
    prefix: str = ""


@dataclass
class WorkspacePaths:
    root: Path
    inventory_raw: Path
    staging_raw: Path
    staging_manifest: Path
    profiles_dir: Path
    mp4_cache_dir: Path
    episodes: Path
    intervals: Path
    labels: Path
    bucket_map: Path
    review_queue: Path
    review_overrides: Path
    proxies_dir: Path
    storyboards_dir: Path
    segments_dir: Path
    segments_manifest: Path
    hand_samples_dir: Path
    effective_dir: Path
    effective_episodes: Path
    effective_intervals: Path
    effective_labels: Path
    effective_bucket_map: Path

    @classmethod
    def from_root(cls, root: Path) -> "WorkspacePaths":
        return cls(
            root=root,
            inventory_raw=root / "inventory" / PIPELINE_VERSION / "raw_objects.jsonl",
            staging_raw=root / "staging" / PIPELINE_VERSION / "raw",
            staging_manifest=root / "staging" / PIPELINE_VERSION / "stage_manifest.jsonl",
            profiles_dir=root / "staging" / PIPELINE_VERSION / "profiles",
            mp4_cache_dir=root / "staging" / PIPELINE_VERSION / "mp4_cache",
            episodes=root / "curation" / PIPELINE_VERSION / "episodes.jsonl",
            intervals=root / "curation" / PIPELINE_VERSION / "intervals.jsonl",
            labels=root / "curation" / PIPELINE_VERSION / "labels.jsonl",
            bucket_map=root / "curation" / PIPELINE_VERSION / "bucket_map.json",
            review_queue=root / "curation" / PIPELINE_VERSION / "review_queue.jsonl",
            review_overrides=root / "curation" / PIPELINE_VERSION / "review_overrides.json",
            proxies_dir=root / "curation" / PIPELINE_VERSION / "proxies",
            storyboards_dir=root / "curation" / PIPELINE_VERSION / "storyboards",
            segments_dir=root / "curation" / PIPELINE_VERSION / "segments",
            segments_manifest=root / "curation" / PIPELINE_VERSION / "segments.jsonl",
            hand_samples_dir=root / "curation" / PIPELINE_VERSION / "hand_samples",
            effective_dir=root / "curation" / PIPELINE_VERSION / "effective",
            effective_episodes=root / "curation" / PIPELINE_VERSION / "effective" / "episodes.jsonl",
            effective_intervals=root / "curation" / PIPELINE_VERSION / "effective" / "intervals.jsonl",
            effective_labels=root / "curation" / PIPELINE_VERSION / "effective" / "labels.jsonl",
            effective_bucket_map=root / "curation" / PIPELINE_VERSION / "effective" / "bucket_map.json",
        )


class UnionFind:
    def __init__(self, n: int) -> None:
        self.parent = list(range(n))
        self.rank = [0] * n

    def find(self, x: int) -> int:
        if self.parent[x] != x:
            self.parent[x] = self.find(self.parent[x])
        return self.parent[x]

    def union(self, a: int, b: int) -> None:
        ra = self.find(a)
        rb = self.find(b)
        if ra == rb:
            return
        if self.rank[ra] < self.rank[rb]:
            self.parent[ra] = rb
        elif self.rank[ra] > self.rank[rb]:
            self.parent[rb] = ra
        else:
            self.parent[rb] = ra
            self.rank[ra] += 1


@dataclass(frozen=True)
class HandPresenceSample:
    timestamp_s: float
    hand_count: int
    max_area_fraction: float

    @property
    def present(self) -> bool:
        return self.hand_count > 0 and self.max_area_fraction >= HAND_MIN_AREA_FRACTION


def log_progress(stage: str, current: int, total: int, item: str = "") -> None:
    """Emit a structured progress line parseable by the viewer server."""
    payload = {"stage": stage, "current": current, "total": total}
    if item:
        payload["item"] = item
    print(f"PROGRESS: {json.dumps(payload)}", flush=True)


def ensure_parent(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)


def ensure_dir(path: Path) -> None:
    path.mkdir(parents=True, exist_ok=True)


def now_iso() -> str:
    return datetime.now(timezone.utc).isoformat()


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    if not path.exists():
        return []
    rows = []
    with open(path) as f:
        for line in f:
            line = line.strip()
            if line:
                rows.append(json.loads(line))
    return rows


def write_jsonl(path: Path, rows: Iterable[dict[str, Any]]) -> None:
    ensure_parent(path)
    with open(path, "w") as f:
        for row in rows:
            f.write(json.dumps(row, sort_keys=True))
            f.write("\n")


def append_jsonl(path: Path, rows: list[dict[str, Any]]) -> None:
    """Append rows to a JSONL file so partial results are visible during a run."""
    ensure_parent(path)
    with open(path, "a") as f:
        for row in rows:
            f.write(json.dumps(row, sort_keys=True))
            f.write("\n")
        f.flush()
        os.fsync(f.fileno())


def read_json(path: Path, default: Any) -> Any:
    if not path.exists():
        return default
    with open(path) as f:
        return json.load(f)


def write_json(path: Path, payload: Any) -> None:
    ensure_parent(path)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")


def merge_jsonl(path: Path, rows: Iterable[dict[str, Any]], key_fields: list[str]) -> None:
    merged: dict[tuple[Any, ...], dict[str, Any]] = {}
    for row in read_jsonl(path):
        key = tuple(row.get(field) for field in key_fields)
        merged[key] = row
    for row in rows:
        key = tuple(row.get(field) for field in key_fields)
        merged[key] = row
    ordered = sorted(merged.values(), key=lambda row: tuple(str(row.get(f, "")) for f in key_fields))
    write_jsonl(path, ordered)


def default_review_overrides() -> dict[str, Any]:
    return {
        "version": PIPELINE_VERSION,
        "updated_at": None,
        "episodes": {},
        "intervals": {},
        "labels": {},
        "buckets": {
            "renames": {},
            "interval_assignments": {},
        },
    }


def load_review_overrides(paths: WorkspacePaths) -> dict[str, Any]:
    payload = read_json(paths.review_overrides, default_review_overrides())
    defaults = default_review_overrides()
    for key, value in defaults.items():
        if key not in payload:
            payload[key] = value
    payload["buckets"].setdefault("renames", {})
    payload["buckets"].setdefault("interval_assignments", {})
    return payload


def slugify(value: str, *, limit: int = 80) -> str:
    text = re.sub(r"[^a-zA-Z0-9]+", "-", value.strip().lower()).strip("-")
    return text[:limit] or "item"


def stable_hash(*parts: Any, length: int = 12) -> str:
    digest = hashlib.sha1("::".join(str(p) for p in parts).encode("utf-8")).hexdigest()
    return digest[:length]


def interval_id(source_key: str, start_s: float, end_s: float) -> str:
    start_ms = int(round(start_s * 1000.0))
    end_ms = int(round(end_s * 1000.0))
    return f"int_{stable_hash(source_key, start_ms, end_ms)}"


def review_entry(stage: str, identifier: str, reason: str, **extra: Any) -> dict[str, Any]:
    entry = {
        "review_id": f"rev_{stable_hash(stage, identifier, reason, length=16)}",
        "stage": stage,
        "identifier": identifier,
        "reason": reason,
        "created_at": now_iso(),
    }
    entry.update(extra)
    return entry


def load_r2_config(config_path: Path) -> R2Config:
    load_runtime_env()

    raw: dict[str, Any] = {}
    if config_path.exists():
        with open(config_path, "rb") as f:
            raw = tomllib.load(f)

    cloud = raw.get("cloud", {})
    endpoint = os.environ.get("R2_ENDPOINT") or os.environ.get("R2_ENDPOINT_URL") or cloud.get("endpoint", "")
    bucket = os.environ.get("R2_BUCKET") or os.environ.get("R2_BUCKET_NAME") or cloud.get("bucket", "ego-data-verlet")
    access_key_id = os.environ.get("R2_ACCESS_KEY_ID", "")
    secret_access_key = os.environ.get("R2_SECRET_ACCESS_KEY", "")
    region = cloud.get("region", "auto")
    prefix = cloud.get("prefix", "")

    missing = [
        name for name, value in (
            ("R2_ENDPOINT", endpoint),
            ("R2_ACCESS_KEY_ID", access_key_id),
            ("R2_SECRET_ACCESS_KEY", secret_access_key),
            ("R2_BUCKET", bucket),
        )
        if not value
    ]
    if missing:
        raise SystemExit(f"Missing R2 configuration: {', '.join(missing)}")

    return R2Config(
        endpoint=endpoint,
        bucket=bucket,
        access_key_id=access_key_id,
        secret_access_key=secret_access_key,
        region=region,
        prefix=prefix,
    )


def make_s3_client(cfg: R2Config):
    return boto3.client(
        "s3",
        endpoint_url=cfg.endpoint,
        aws_access_key_id=cfg.access_key_id,
        aws_secret_access_key=cfg.secret_access_key,
        region_name=cfg.region,
        config=BotoConfig(
            retries={"max_attempts": 5, "mode": "adaptive"},
            connect_timeout=30,
            read_timeout=300,
            tcp_keepalive=True,
            signature_version="s3v4",
        ),
    )


def read_object_range(s3_client, bucket: str, key: str, start: int, end: int) -> bytes:
    resp = s3_client.get_object(Bucket=bucket, Key=key, Range=f"bytes={start}-{end}")
    return resp["Body"].read()


def iter_bucket_objects(s3_client, bucket: str, prefix: str | None) -> Iterable[dict[str, Any]]:
    kwargs: dict[str, Any] = {"Bucket": bucket}
    if prefix:
        kwargs["Prefix"] = prefix
    paginator = s3_client.get_paginator("list_objects_v2")
    for page in paginator.paginate(**kwargs):
        for obj in page.get("Contents", []):
            yield obj


def object_relative_key(base_prefix: str, key: str) -> str:
    if base_prefix and key.startswith(base_prefix):
        return key[len(base_prefix):].lstrip("/")
    return key


def inventory_row_from_object(s3_client, bucket: str, obj: dict[str, Any], base_prefix: str) -> dict[str, Any]:
    key = obj["Key"]
    size_bytes = obj["Size"]
    parts = PurePosixPath(key).parts
    row: dict[str, Any] = {
        "bucket": bucket,
        "key": key,
        "relative_key": object_relative_key(base_prefix, key),
        "source_prefix": parts[0] if parts else "",
        "size_bytes": size_bytes,
        "etag": obj.get("ETag", "").strip('"'),
        "last_modified": obj["LastModified"].astimezone(timezone.utc).isoformat() if obj.get("LastModified") else None,
        "schema_version": PIPELINE_VERSION,
        "inventory_created_at": now_iso(),
        "header_ok": False,
        "footer_ok": False,
        "validation_status": "invalid",
    }

    try:
        header_bytes = read_object_range(s3_client, bucket, key, 0, HEADER_SIZE - 1)
        header = read_header(header_bytes)
        row.update(
            {
                "header_ok": True,
                "session_name": header.session_name,
                "recorded_at": header.recorded_at.isoformat() if header.recorded_at else None,
                "camera_serial": header.serial_number,
                "usb_type": header.usb_type,
                "has_imu": header.has_imu,
                "has_index": header.has_index,
                "color_width": header.color_width,
                "color_height": header.color_height,
                "depth_width": header.depth_width,
                "depth_height": header.depth_height,
                "rgb_codec": header.rgb_codec,
                "depth_codec": header.depth_codec,
                "rgb_quality": header.rgb_quality,
                "zstd_level": header.zstd_level,
            }
        )
    except Exception as exc:  # noqa: BLE001
        row["validation_error"] = f"header: {exc}"
        return row

    if size_bytes < HEADER_SIZE + FOOTER_SIZE:
        row["validation_error"] = "footer: file too small for footer"
        return row

    try:
        footer_bytes = read_object_range(s3_client, bucket, key, size_bytes - FOOTER_SIZE, size_bytes - 1)
        footer = read_footer(footer_bytes)
        row.update(
            {
                "footer_ok": True,
                "validation_status": "valid",
                "frame_count": footer.total_frames,
                "duration_s": round(footer.duration_s, 6),
                "fps": round(footer.fps, 4),
                "index_entry_count": footer.index_entry_count,
            }
        )
    except Exception as exc:  # noqa: BLE001
        row["validation_error"] = f"footer: {exc}"

    return row


def resolve_workspace(args: argparse.Namespace) -> WorkspacePaths:
    root = Path(args.workspace).resolve()
    return WorkspacePaths.from_root(root)


def resolve_ego_qc_binary(explicit: str | None) -> str:
    candidates = []
    if explicit:
        candidates.append(Path(explicit))
    candidates.extend(
        [
            ROOT / "rust" / "target" / "debug" / "ego-qc",
            ROOT / "target" / "debug" / "ego-qc",
            ROOT / "rust" / "target" / "release" / "ego-qc",
            ROOT / "target" / "release" / "ego-qc",
        ]
    )
    for candidate in candidates:
        if candidate.exists():
            return str(candidate)
    which = shutil.which("ego-qc")
    if which:
        return which
    raise SystemExit("Could not find ego-qc binary. Build it first or pass --ego-qc.")


def run_command(cmd: list[str], *, cwd: Path | None = None, quiet: bool = False) -> subprocess.CompletedProcess[str]:
    if not quiet:
        print("$", " ".join(cmd))
    return subprocess.run(
        cmd,
        cwd=str(cwd or ROOT),
        check=False,
        text=True,
        capture_output=True,
    )


def assert_success(proc: subprocess.CompletedProcess[str], context: str) -> None:
    if proc.returncode != 0:
        message = proc.stderr.strip() or proc.stdout.strip() or f"{context} failed"
        raise RuntimeError(f"{context}: {message}")


def make_profile_key(row: dict[str, Any]) -> str:
    serial = row.get("camera_serial") or ""
    if serial:
        return serial
    return f"source-{slugify(str(row.get('source_prefix', 'unknown')))}"


def load_rows_by_key(path: Path, key_field: str) -> dict[str, dict[str, Any]]:
    return {str(row[key_field]): row for row in read_jsonl(path)}


def decode_chat_content(content: Any) -> str:
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        parts = []
        for item in content:
            if isinstance(item, dict):
                text = item.get("text")
                if text:
                    parts.append(text)
        return "\n".join(parts)
    return str(content)


def strip_json_fence(text: str) -> str:
    text = text.strip()
    if text.startswith("```"):
        lines = text.splitlines()
        if len(lines) >= 3:
            text = "\n".join(lines[1:-1]).strip()
    return text


def extract_json_object(text: str) -> dict[str, Any]:
    text = strip_json_fence(text)
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        start = text.find("{")
        end = text.rfind("}")
        if start != -1 and end != -1 and start < end:
            return json.loads(text[start : end + 1])
        raise


def normalize_open_label(label: str) -> str:
    label = label.strip().lower()
    label = re.sub(r"[^a-z0-9\s]+", " ", label)
    label = re.sub(r"\s+", " ", label).strip()
    return label or "unknown"


def cluster_text(label_row: dict[str, Any]) -> str:
    task_name = normalize_open_label(str(label_row.get("proposed_task_name", "")))
    caption = normalize_open_label(str(label_row.get("short_caption", "")))
    objects = " ".join(sorted(normalize_open_label(obj) for obj in label_row.get("primary_objects", [])))
    return " | ".join(part for part in (task_name, objects, caption) if part)


def cosine_similarity(a: list[float], b: list[float]) -> float:
    dot = sum(x * y for x, y in zip(a, b))
    norm_a = math.sqrt(sum(x * x for x in a))
    norm_b = math.sqrt(sum(y * y for y in b))
    if norm_a == 0.0 or norm_b == 0.0:
        return 0.0
    return dot / (norm_a * norm_b)


def make_analysis_windows(start_s: float, end_s: float, max_window_s: float, overlap_s: float) -> list[dict[str, float]]:
    duration = max(0.0, end_s - start_s)
    if duration <= max_window_s:
        return [{"start_s": round(start_s, 6), "end_s": round(end_s, 6)}]

    windows = []
    cursor = start_s
    step = max(max_window_s - overlap_s, 1.0)
    while cursor < end_s:
        window_end = min(cursor + max_window_s, end_s)
        windows.append({"start_s": round(cursor, 6), "end_s": round(window_end, 6)})
        if window_end >= end_s:
            break
        cursor += step
    return windows


def clamp(value: float, lower: float, upper: float) -> float:
    return max(lower, min(value, upper))


def maybe_round(value: float, digits: int = 6) -> float:
    return round(float(value), digits)


def overlap_s(start_a: float, end_a: float, start_b: float, end_b: float) -> float:
    return max(0.0, min(end_a, end_b) - max(start_a, start_b))


def load_egorec_reader() -> Any:
    try:
        return importlib.import_module("egorec_reader")
    except ImportError as first_exc:
        build_dir = ROOT / "build"
        if build_dir.exists():
            build_path = str(build_dir)
            if build_path not in sys.path:
                sys.path.insert(0, build_path)
            try:
                return importlib.import_module("egorec_reader")
            except ImportError as second_exc:
                raise RuntimeError(
                    "egorec_reader is required for hand-aware interval suggestion. "
                    "Build with: cmake -B build -DWITH_PYTHON=ON && cmake --build build"
                ) from second_exc
        raise RuntimeError(
            "egorec_reader is required for hand-aware interval suggestion. "
            "Build with: cmake -B build -DWITH_PYTHON=ON && cmake --build build"
        ) from first_exc


def landmarks_area_fraction(landmarks: list[Any]) -> float:
    xs = [clamp(float(point.x), 0.0, 1.0) for point in landmarks]
    ys = [clamp(float(point.y), 0.0, 1.0) for point in landmarks]
    if not xs or not ys:
        return 0.0
    return max(0.0, (max(xs) - min(xs)) * (max(ys) - min(ys)))


def detect_hand_presence_samples(recording_path: Path, sample_fps: float) -> list[HandPresenceSample]:
    try:
        import mediapipe as mp  # type: ignore
    except ImportError as exc:
        raise RuntimeError("mediapipe is required for hand-aware interval suggestion") from exc

    egorec_reader = load_egorec_reader()
    try:
        reader = egorec_reader.EgorecFile(str(recording_path))
    except Exception as exc:
        raise RuntimeError(f"Could not open .egorec for hand detection: {recording_path}") from exc

    frame_count = int(reader.frame_count())
    header = reader.header()
    duration_s = float(header.get("duration_s", 0.0) or 0.0)
    source_fps = frame_count / duration_s if frame_count > 0 and duration_s > 0.0 else 30.0
    frame_step = max(int(round(source_fps / max(sample_fps, 0.1))), 1)
    samples: list[HandPresenceSample] = []

    with mp.solutions.hands.Hands(
        static_image_mode=False,
        model_complexity=0,
        max_num_hands=2,
        min_detection_confidence=HAND_MIN_DETECTION_CONFIDENCE,
        min_tracking_confidence=HAND_MIN_DETECTION_CONFIDENCE,
    ) as hands:
        for frame_idx, frame in enumerate(reader.frames()):
            if frame_idx % frame_step != 0:
                continue

            timestamp_s = frame_idx / source_fps
            result = hands.process(frame["rgb"])
            hand_count = 0
            max_area_fraction = 0.0
            handedness = result.multi_handedness or []

            for idx, hand_landmarks in enumerate(result.multi_hand_landmarks or []):
                score = 1.0
                if idx < len(handedness):
                    score = max((item.score for item in handedness[idx].classification), default=1.0)
                area_fraction = landmarks_area_fraction(hand_landmarks.landmark)
                if score >= HAND_MIN_DETECTION_CONFIDENCE and area_fraction >= HAND_MIN_AREA_FRACTION:
                    hand_count += 1
                    max_area_fraction = max(max_area_fraction, area_fraction)

            samples.append(
                HandPresenceSample(
                    timestamp_s=maybe_round(timestamp_s),
                    hand_count=hand_count,
                    max_area_fraction=maybe_round(max_area_fraction),
                )
            )

    return samples


def build_presence_spans(samples: list[HandPresenceSample], sample_period_s: float, merge_gap_s: float) -> list[tuple[float, float]]:
    present_samples = [sample for sample in samples if sample.present]
    if not present_samples:
        return []

    spans: list[tuple[float, float]] = []
    start_s = present_samples[0].timestamp_s
    last_end_s = start_s + sample_period_s
    for sample in present_samples[1:]:
        sample_start = sample.timestamp_s
        sample_end = sample.timestamp_s + sample_period_s
        if sample_start - last_end_s <= merge_gap_s:
            last_end_s = sample_end
            continue
        spans.append((start_s, last_end_s))
        start_s = sample_start
        last_end_s = sample_end
    spans.append((start_s, last_end_s))
    return spans


def merge_spans(spans: list[tuple[float, float]], gap_s: float) -> list[tuple[float, float]]:
    if not spans:
        return []
    ordered = sorted(spans)
    merged = [ordered[0]]
    for start_s, end_s in ordered[1:]:
        last_start_s, last_end_s = merged[-1]
        if start_s - last_end_s <= gap_s:
            merged[-1] = (last_start_s, max(last_end_s, end_s))
        else:
            merged.append((start_s, end_s))
    return merged


def choose_split_point(
    start_s: float,
    end_s: float,
    samples: list[HandPresenceSample],
    sample_period_s: float,
) -> float:
    present_blocks = [
        (sample.timestamp_s, sample.timestamp_s + sample_period_s)
        for sample in samples
        if sample.present and overlap_s(sample.timestamp_s, sample.timestamp_s + sample_period_s, start_s, end_s) > 0.0
    ]

    largest_gap = 0.0
    split_point = start_s + (end_s - start_s) / 2.0
    cursor = start_s
    for block_start_s, block_end_s in present_blocks:
        gap = block_start_s - cursor
        if gap > largest_gap:
            largest_gap = gap
            split_point = cursor + gap / 2.0
        cursor = max(cursor, block_end_s)
    final_gap = end_s - cursor
    if final_gap > largest_gap:
        split_point = cursor + final_gap / 2.0

    if split_point <= start_s + 0.25 or split_point >= end_s - 0.25:
        split_point = start_s + (end_s - start_s) / 2.0
    return split_point


def split_long_spans(
    spans: list[tuple[float, float]],
    samples: list[HandPresenceSample],
    sample_period_s: float,
    max_segment_s: float,
) -> list[tuple[float, float]]:
    split: list[tuple[float, float]] = []
    for start_s, end_s in spans:
        duration_s = end_s - start_s
        if duration_s <= max_segment_s:
            split.append((start_s, end_s))
            continue
        split_point = choose_split_point(start_s, end_s, samples, sample_period_s)
        split.extend(
            split_long_spans(
                [(start_s, split_point), (split_point, end_s)],
                samples,
                sample_period_s,
                max_segment_s,
            )
        )
    return split


def summarize_interval_candidate(
    start_s: float,
    end_s: float,
    samples: list[HandPresenceSample],
    activity_proposals: list[dict[str, Any]],
    sample_period_s: float,
) -> dict[str, Any]:
    window_samples = [
        sample
        for sample in samples
        if overlap_s(sample.timestamp_s, sample.timestamp_s + sample_period_s, start_s, end_s) > 0.0
    ]
    present_samples = [sample for sample in window_samples if sample.present]
    hand_presence_fraction = len(present_samples) / len(window_samples) if window_samples else 0.0
    max_hands_seen = max((sample.hand_count for sample in present_samples), default=0)

    weighted_active = 0.0
    weighted_duration = 0.0
    for proposal in activity_proposals:
        duration = overlap_s(start_s, end_s, float(proposal["start_s"]), float(proposal["end_s"]))
        if duration <= 0.0:
            continue
        weighted_duration += duration
        weighted_active += duration * float(proposal.get("active_fraction", 0.0) or 0.0)
    active_fraction = weighted_active / weighted_duration if weighted_duration > 0.0 else 0.0
    proposal_score = 0.7 * hand_presence_fraction + 0.3 * active_fraction

    return {
        "start_s": maybe_round(start_s),
        "end_s": maybe_round(end_s),
        "duration_s": maybe_round(max(0.0, end_s - start_s)),
        "hand_presence_fraction": maybe_round(hand_presence_fraction),
        "active_fraction": maybe_round(active_fraction),
        "max_hands_seen": max_hands_seen,
        "proposal_score": maybe_round(proposal_score),
    }


def suggest_hand_activity_intervals(
    duration_s: float,
    activity_proposals: list[dict[str, Any]],
    hand_samples: list[HandPresenceSample],
    *,
    sample_fps: float,
    min_duration_s: float,
    merge_gap_s: float,
    pad_s: float,
    max_segment_s: float,
) -> list[dict[str, Any]]:
    if not activity_proposals or not hand_samples:
        return []

    sample_period_s = 1.0 / max(sample_fps, 0.1)
    presence_spans = build_presence_spans(hand_samples, sample_period_s, merge_gap_s)
    if not presence_spans:
        return []

    raw_spans: list[tuple[float, float]] = []
    for proposal in activity_proposals:
        proposal_start_s = float(proposal["start_s"])
        proposal_end_s = float(proposal["end_s"])
        for presence_start_s, presence_end_s in presence_spans:
            overlap_start_s = max(proposal_start_s, presence_start_s)
            overlap_end_s = min(proposal_end_s, presence_end_s)
            if overlap_end_s <= overlap_start_s:
                continue
            raw_spans.append(
                (
                    clamp(overlap_start_s - pad_s, 0.0, duration_s),
                    clamp(overlap_end_s + pad_s, 0.0, duration_s),
                )
            )

    merged_spans = merge_spans(raw_spans, merge_gap_s)
    split_spans = split_long_spans(merged_spans, hand_samples, sample_period_s, max_segment_s)
    final_spans = [
        (start_s, end_s)
        for start_s, end_s in merge_spans(split_spans, 0.0)
        if end_s - start_s >= min_duration_s
    ]

    summaries = [
        summarize_interval_candidate(start_s, end_s, hand_samples, activity_proposals, sample_period_s)
        for start_s, end_s in final_spans
    ]
    return [summary for summary in summaries if summary["hand_presence_fraction"] > 0.0]


def save_hand_samples(hand_samples_dir: Path, source_key: str, samples: list[HandPresenceSample]) -> None:
    filename = slugify(source_key, limit=120) + ".jsonl"
    out_path = hand_samples_dir / filename
    ensure_parent(out_path)
    rows = [
        {
            "t": sample.timestamp_s,
            "n": sample.hand_count,
            "a": sample.max_area_fraction,
        }
        for sample in samples
    ]
    write_jsonl(out_path, rows)


def hand_samples_path_for_key(hand_samples_dir: Path, source_key: str) -> Path:
    return hand_samples_dir / (slugify(source_key, limit=120) + ".jsonl")


def load_activity_proposals(
    ego_qc: str,
    local_path: str,
    profile_path: str | None,
    *,
    min_gap_seconds: float,
    min_duration_seconds: float,
    pad_seconds: float,
) -> list[dict[str, Any]]:
    with tempfile.NamedTemporaryFile(suffix=".json") as tmp:
        cmd = [
            ego_qc,
            "intervals",
            local_path,
            "--output",
            tmp.name,
            "--min-gap",
            str(min_gap_seconds),
            "--min-duration",
            str(min_duration_seconds),
            "--pad",
            str(pad_seconds),
        ]
        if profile_path:
            cmd.extend(["--profile", str(profile_path)])
        proc = run_command(cmd, quiet=True)
        assert_success(proc, f"interval export {local_path}")
        payload = json.loads(Path(tmp.name).read_text())
        report = payload[0] if payload else None
        return report.get("proposals", []) if report else []


def prompt_vlm(interval: dict[str, Any]) -> tuple[str, str]:
    system_prompt = (
        "You are classifying egocentric RGB-D manipulation footage. "
        "Return strict JSON with keys: "
        "is_manipulation, proposed_task_name, short_caption, primary_objects, confidence, reason. "
        "Use a concise open-ended task name when manipulation is present. "
        "If the clip is not a meaningful manipulation, set is_manipulation=false and proposed_task_name='idle_or_other'."
    )
    user_prompt = (
        "Classify this storyboard from an egocentric recording interval.\n"
        f"Source prefix: {interval.get('source_prefix')}\n"
        f"Session name: {interval.get('session_name')}\n"
        f"Interval seconds: {interval.get('start_s')} to {interval.get('end_s')}\n"
        f"Interval duration: {interval.get('duration_s')} seconds\n"
        "Focus on the dominant human manipulation, not camera motion."
    )
    return system_prompt, user_prompt


class OpenAICompatibleClient:
    def __init__(self, *, base_url: str, api_key: str, model: str | None, embedding_model: str | None) -> None:
        self.base_url = base_url.rstrip("/")
        self.api_key = api_key
        self.model = model
        self.embedding_model = embedding_model

    @classmethod
    def from_env(cls, args: argparse.Namespace, *, require_model: bool = True) -> "OpenAICompatibleClient":
        load_runtime_env()
        base_url = (
            args.base_url
            or os.environ.get("VLM_BASE_URL")
            or os.environ.get("OPENAI_BASE_URL")
            or "https://api.openai.com/v1"
        )
        api_key = (
            args.api_key
            or os.environ.get("VLM_API_KEY")
            or os.environ.get("OPENAI_API_KEY")
        )
        model = (
            args.model
            or os.environ.get("VLM_MODEL")
            or os.environ.get("OPENAI_MODEL")
        )
        embedding_model = (
            getattr(args, "embedding_model", None)
            or os.environ.get("VLM_EMBEDDING_MODEL")
            or os.environ.get("OPENAI_EMBEDDING_MODEL")
        )
        required = [("api key", api_key)]
        if require_model:
            required.append(("model", model))
        missing = [name for name, value in required if not value]
        if missing:
            raise SystemExit(f"Missing VLM configuration: {', '.join(missing)}")
        return cls(base_url=base_url, api_key=api_key, model=model, embedding_model=embedding_model)

    def _post_json(self, path: str, payload: dict[str, Any]) -> dict[str, Any]:
        data = json.dumps(payload).encode("utf-8")
        req = urllib.request.Request(
            f"{self.base_url.rstrip('/')}/{path.lstrip('/')}",
            data=data,
            headers={
                "Authorization": f"Bearer {self.api_key}",
                "Content-Type": "application/json",
            },
            method="POST",
        )
        try:
            with urllib.request.urlopen(req, timeout=120) as resp:
                return json.loads(resp.read().decode("utf-8"))
        except urllib.error.HTTPError as exc:
            body = exc.read().decode("utf-8", errors="replace")
            raise RuntimeError(f"HTTP {exc.code}: {body}") from exc

    def classify_storyboard(self, storyboard_path: Path, interval: dict[str, Any]) -> tuple[dict[str, Any], dict[str, Any]]:
        if not self.model:
            raise RuntimeError("Chat model is not configured")
        mime_type = mimetypes.guess_type(storyboard_path.name)[0] or "image/jpeg"
        image_data = base64.b64encode(storyboard_path.read_bytes()).decode("ascii")
        image_url = f"data:{mime_type};base64,{image_data}"
        system_prompt, user_prompt = prompt_vlm(interval)
        payload = {
            "model": self.model,
            "temperature": 0.2,
            "response_format": {"type": "json_object"},
            "messages": [
                {"role": "system", "content": system_prompt},
                {
                    "role": "user",
                    "content": [
                        {"type": "text", "text": user_prompt},
                        {"type": "image_url", "image_url": {"url": image_url}},
                    ],
                },
            ],
        }
        response = self._post_json("/chat/completions", payload)
        content = decode_chat_content(response["choices"][0]["message"]["content"])
        parsed = extract_json_object(content)
        return parsed, response

    def embed_texts(self, texts: list[str]) -> list[list[float]]:
        if not self.embedding_model:
            raise RuntimeError("Embedding model is not configured")
        payload = {
            "model": self.embedding_model,
            "input": texts,
        }
        response = self._post_json("/embeddings", payload)
        data = sorted(response["data"], key=lambda item: item["index"])
        return [item["embedding"] for item in data]


def content_type_for_file(path: Path) -> str:
    return mimetypes.guess_type(path.name)[0] or "application/octet-stream"


def cmd_inventory(args: argparse.Namespace) -> None:
    paths = resolve_workspace(args)
    cfg = load_r2_config(Path(args.config))
    s3_client = make_s3_client(cfg)

    rows = []
    counts = Counter()
    objects = list(iter_bucket_objects(s3_client, cfg.bucket, args.prefix))
    for i, obj in enumerate(objects):
        row = inventory_row_from_object(s3_client, cfg.bucket, obj, args.prefix or "")
        rows.append(row)
        counts[row["validation_status"]] += 1
        log_progress("inventory", i + 1, len(objects), obj["Key"])

    rows.sort(key=lambda row: row["key"])
    write_jsonl(paths.inventory_raw, rows)

    total_bytes = sum(int(row["size_bytes"]) for row in rows)
    print(f"Wrote {len(rows)} inventory rows to {paths.inventory_raw}")
    print(f"Valid: {counts['valid']}  Invalid: {counts['invalid']}  Total bytes: {total_bytes}")


def cmd_stage(args: argparse.Namespace) -> None:
    paths = resolve_workspace(args)
    cfg = load_r2_config(Path(args.config))
    s3_client = make_s3_client(cfg)
    inventory_rows = read_jsonl(paths.inventory_raw)
    if not inventory_rows:
        raise SystemExit(f"Missing inventory file: {paths.inventory_raw}")

    selected = []
    for row in inventory_rows:
        if not args.include_invalid and row.get("validation_status") != "valid":
            continue
        if args.source_prefix and row.get("source_prefix") != args.source_prefix:
            continue
        selected.append(row)

    if args.max_files:
        selected = selected[: args.max_files]

    ensure_dir(paths.staging_raw)
    write_jsonl(paths.staging_manifest, [])
    stage_rows = []
    for i, row in enumerate(selected):
        local_path = paths.staging_raw / row["key"]
        ensure_parent(local_path)
        should_download = True
        if local_path.exists() and local_path.stat().st_size == int(row["size_bytes"]):
            should_download = False

        if should_download:
            print(f"Downloading {row['key']} -> {local_path}")
            s3_client.download_file(cfg.bucket, row["key"], str(local_path))
        else:
            print(f"Skipping existing {local_path}")
        log_progress("stage", i + 1, len(selected), row["key"])

        stage_row = dict(row)
        stage_row.update(
            {
                "local_path": str(local_path),
                "staged_at": now_iso(),
            }
        )
        stage_rows.append(stage_row)
        append_jsonl(paths.staging_manifest, [stage_row])

    stage_rows.sort(key=lambda row: row["key"])
    write_jsonl(paths.staging_manifest, stage_rows)
    print(f"Wrote {len(stage_rows)} staged rows to {paths.staging_manifest}")


def cmd_qc(args: argparse.Namespace) -> None:
    paths = resolve_workspace(args)
    ego_qc = resolve_ego_qc_binary(args.ego_qc)
    stage_rows = read_jsonl(paths.staging_manifest)
    if not stage_rows:
        raise SystemExit(f"Missing staging manifest: {paths.staging_manifest}")

    groups: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in stage_rows:
        groups[make_profile_key(row)].append(row)

    ensure_dir(paths.profiles_dir)
    profile_by_group: dict[str, str] = {}
    for group_key, rows in groups.items():
        profile_path = paths.profiles_dir / f"{slugify(group_key)}.json"
        cmd = [ego_qc, "calibrate", *[row["local_path"] for row in rows], "--save-profile", str(profile_path)]
        proc = run_command(cmd)
        assert_success(proc, f"calibrate {group_key}")
        profile_by_group[group_key] = str(profile_path)

    existing_episodes = read_jsonl(paths.episodes) if args.skip_existing else []
    existing_keys = {str(row.get("source_key")) for row in existing_episodes}

    eligible = [row for row in stage_rows if not (args.skip_existing and row["key"] in existing_keys)]
    if args.limit:
        eligible = eligible[: args.limit]

    episode_rows = list(existing_episodes)
    review_rows = []
    write_jsonl(paths.episodes, existing_episodes)
    for i, row in enumerate(eligible):
        local_path = Path(row["local_path"])
        log_progress("qc", i + 1, len(eligible), row["key"])
        meta = read_metadata(local_path)
        group_key = make_profile_key(row)
        profile_path = profile_by_group[group_key]

        validate_proc = run_command([ego_qc, "validate", str(local_path), "--quiet"], quiet=True)
        validate_ok = validate_proc.returncode == 0

        with tempfile.NamedTemporaryFile(suffix=".json") as tmp:
            analyze_cmd = [
                ego_qc,
                "analyze",
                str(local_path),
                "--report",
                tmp.name,
                "--profile",
                profile_path,
            ]
            analyze_proc = run_command(analyze_cmd, quiet=True)

            analyze_result: dict[str, Any] | None = None
            analysis_error = None
            if analyze_proc.returncode == 0:
                payload = json.loads(Path(tmp.name).read_text())
                analyze_result = payload[0] if payload else None
            else:
                analysis_error = analyze_proc.stderr.strip() or analyze_proc.stdout.strip()

        duration_s = meta.duration_s
        frame_count = meta.frame_count
        validation_status = "valid" if (validate_ok and meta.footer) else "invalid"

        if validation_status != "valid":
            episode_status = "invalid"
        elif duration_s < args.reject_shorter_than_s:
            episode_status = "reject"
        elif analyze_result is None:
            episode_status = "invalid"
        else:
            verdict = analyze_result.get("verdict")
            if verdict == "Keep":
                episode_status = "keep"
            elif verdict == "PruneConfident":
                episode_status = "reject"
            else:
                episode_status = "review"

        episode_row = {
            "episode_id": f"ep_{stable_hash(row['key'], length=16)}",
            "source_key": row["key"],
            "source_prefix": row.get("source_prefix"),
            "local_path": str(local_path),
            "camera_serial": meta.header.serial_number,
            "session_name": meta.header.session_name,
            "recorded_at": meta.header.recorded_at.isoformat() if meta.header.recorded_at else None,
            "duration_s": round(duration_s, 6),
            "frame_count": frame_count,
            "fps": round(meta.fps, 4),
            "size_bytes": meta.file_size,
            "validate_ok": validate_ok,
            "validation_status": validation_status,
            "analysis_error": analysis_error,
            "analyze_verdict": analyze_result.get("verdict") if analyze_result else None,
            "activity_score": analyze_result.get("activity_score") if analyze_result else None,
            "reasons_keep": analyze_result.get("reasons_keep", []) if analyze_result else [],
            "reasons_prune": analyze_result.get("reasons_prune", []) if analyze_result else [],
            "used_profile": analyze_result.get("used_profile") if analyze_result else False,
            "profile_path": profile_path,
            "episode_status": episode_status,
        }
        episode_rows.append(episode_row)
        append_jsonl(paths.episodes, [episode_row])

        if validation_status != "valid":
            review_rows.append(
                review_entry(
                    "qc",
                    row["key"],
                    "invalid_episode",
                    source_key=row["key"],
                    details=analysis_error or validate_proc.stderr.strip() or "validation failed",
                )
            )
        elif episode_status == "review":
            review_rows.append(
                review_entry(
                    "qc",
                    row["key"],
                    "needs_human_review",
                    source_key=row["key"],
                    verdict=episode_row["analyze_verdict"],
                    activity_score=episode_row["activity_score"],
                )
            )

    episode_rows.sort(key=lambda row: row["source_key"])
    write_jsonl(paths.episodes, episode_rows)
    if review_rows:
        merge_jsonl(paths.review_queue, review_rows, ["review_id"])

    counts = Counter(row["episode_status"] for row in episode_rows)
    print(f"Wrote {len(episode_rows)} episode rows to {paths.episodes}")
    print(f"keep={counts['keep']} review={counts['review']} reject={counts['reject']} invalid={counts['invalid']}")


def cmd_intervals(args: argparse.Namespace) -> None:
    paths = resolve_workspace(args)
    ego_qc = resolve_ego_qc_binary(args.ego_qc)
    episodes = read_jsonl(paths.episodes)
    if not episodes:
        raise SystemExit(f"Missing episode file: {paths.episodes}")

    existing_intervals = read_jsonl(paths.intervals) if args.skip_existing else []
    existing_sources = {str(row.get("source_key")) for row in existing_intervals}

    interval_rows = list(existing_intervals)
    review_rows = []
    eligible = [ep for ep in episodes if ep.get("episode_status") in {"keep", "review"}]
    to_process = [
        ep for ep in eligible
        if not (args.skip_existing and str(ep.get("source_key")) in existing_sources)
    ]
    if args.limit:
        to_process = to_process[: args.limit]

    write_jsonl(paths.intervals, existing_intervals)
    for i, episode in enumerate(to_process):
        log_progress("intervals", i + 1, len(to_process), episode.get("source_key", ""))
        status = episode.get("episode_status")

        duration_s = float(episode.get("duration_s") or 0.0)
        local_path = str(episode["local_path"])
        session_name = episode.get("session_name")
        profile_path = episode.get("profile_path")
        try:
            proposals = load_activity_proposals(
                ego_qc,
                local_path,
                str(profile_path) if profile_path else None,
                min_gap_seconds=args.min_gap_seconds,
                min_duration_seconds=args.min_duration_seconds,
                pad_seconds=args.pad_seconds,
            )
        except RuntimeError as exc:
            review_rows.append(
                review_entry(
                    "intervals",
                    episode["source_key"],
                    "interval_export_failed",
                    source_key=episode["source_key"],
                    details=str(exc),
                )
            )
            continue

        if not proposals:
            review_rows.append(
                review_entry(
                    "intervals",
                    episode["source_key"],
                    "no_interval_proposals",
                    source_key=episode["source_key"],
                    duration_s=duration_s,
                )
            )
            continue

        try:
            hand_samples = detect_hand_presence_samples(Path(local_path), args.hand_sample_fps)
        except RuntimeError as exc:
            review_rows.append(
                review_entry(
                    "intervals",
                    episode["source_key"],
                    "hand_detection_failed",
                    source_key=episode["source_key"],
                    details=str(exc),
                )
            )
            continue

        ensure_dir(paths.hand_samples_dir)
        save_hand_samples(paths.hand_samples_dir, episode["source_key"], hand_samples)

        if not any(sample.present for sample in hand_samples):
            review_rows.append(
                review_entry(
                    "intervals",
                    episode["source_key"],
                    "no_hands_detected",
                    source_key=episode["source_key"],
                    duration_s=duration_s,
                )
            )
            continue

        candidates = suggest_hand_activity_intervals(
            duration_s,
            proposals,
            hand_samples,
            sample_fps=args.hand_sample_fps,
            min_duration_s=args.min_duration_seconds,
            merge_gap_s=args.min_gap_seconds,
            pad_s=args.pad_seconds,
            max_segment_s=args.max_segment_seconds,
        )

        if not candidates:
            review_rows.append(
                review_entry(
                    "intervals",
                    episode["source_key"],
                    "no_hand_activity_overlap",
                    source_key=episode["source_key"],
                    duration_s=duration_s,
                )
            )
            continue

        episode_intervals = []
        for proposal in candidates:
            start_s = float(proposal["start_s"])
            end_s = float(proposal["end_s"])
            iid = interval_id(episode["source_key"], start_s, end_s)
            row_data = {
                "interval_id": iid,
                "source_key": episode["source_key"],
                "source_prefix": episode.get("source_prefix"),
                "session_name": session_name,
                "camera_serial": episode.get("camera_serial"),
                "local_path": local_path,
                "episode_status": status,
                "proposal_source": "hand_activity_v1",
                "start_s": round(start_s, 6),
                "end_s": round(end_s, 6),
                "duration_s": round(float(proposal["duration_s"]), 6),
                "active_fraction": round(float(proposal["active_fraction"]), 6),
                "hand_presence_fraction": round(float(proposal["hand_presence_fraction"]), 6),
                "max_hands_seen": int(proposal["max_hands_seen"]),
                "proposal_score": round(float(proposal["proposal_score"]), 6),
                "effective_start_s": round(start_s, 6),
                "effective_end_s": round(end_s, 6),
                "effective_duration_s": round(float(proposal["duration_s"]), 6),
                "analysis_windows": make_analysis_windows(start_s, end_s, args.vlm_max_window_seconds, args.vlm_overlap_seconds),
            }
            interval_rows.append(row_data)
            episode_intervals.append(row_data)

        append_jsonl(paths.intervals, episode_intervals)

    interval_rows.sort(key=lambda row: (row["source_key"], row["start_s"], row["end_s"]))
    write_jsonl(paths.intervals, interval_rows)
    if review_rows:
        merge_jsonl(paths.review_queue, review_rows, ["review_id"])
    print(f"Wrote {len(interval_rows)} interval rows to {paths.intervals}")


def run_ffmpeg(cmd: list[str]) -> None:
    proc = run_command(cmd, quiet=True)
    assert_success(proc, "ffmpeg")


def media_file_is_valid(path: Path) -> bool:
    ffprobe = shutil.which("ffprobe")
    if not ffprobe or not path.exists():
        return False
    proc = run_command(
        [
            ffprobe,
            "-v",
            "error",
            "-show_entries",
            "format=duration,size",
            "-of",
            "json",
            str(path),
        ],
        quiet=True,
    )
    return proc.returncode == 0


def full_mp4_path(paths: WorkspacePaths, source_key: str) -> Path:
    return (paths.mp4_cache_dir / source_key).with_suffix(".mp4")


def ensure_full_mp4(paths: WorkspacePaths, ego_qc: str, interval: dict[str, Any]) -> Path:
    local_mp4 = full_mp4_path(paths, interval["source_key"])
    depth_mp4 = local_mp4.with_name(f"{local_mp4.stem}.depth.mp4")
    meta_json = local_mp4.with_name(f"{local_mp4.stem}.meta.json")

    if local_mp4.exists() and media_file_is_valid(local_mp4):
        return local_mp4
    if local_mp4.exists():
        local_mp4.unlink(missing_ok=True)
        depth_mp4.unlink(missing_ok=True)
        meta_json.unlink(missing_ok=True)

    out_dir = local_mp4.parent
    ensure_dir(out_dir)
    cmd = [ego_qc, "mp4", interval["local_path"], "--output", str(out_dir), "--quiet"]
    proc = run_command(cmd, quiet=True)
    if proc.returncode != 0:
        local_mp4.unlink(missing_ok=True)
        depth_mp4.unlink(missing_ok=True)
        meta_json.unlink(missing_ok=True)
        message = proc.stderr.strip() or proc.stdout.strip() or "mp4 conversion failed"
        raise RuntimeError(f"mp4 {interval['source_key']}: {message}")
    if not media_file_is_valid(local_mp4):
        local_mp4.unlink(missing_ok=True)
        depth_mp4.unlink(missing_ok=True)
        meta_json.unlink(missing_ok=True)
        raise RuntimeError(f"mp4 {interval['source_key']}: generated MP4 is invalid")
    return local_mp4


def cmd_proxies(args: argparse.Namespace) -> None:
    paths = resolve_workspace(args)
    ego_qc = resolve_ego_qc_binary(args.ego_qc)
    intervals = read_jsonl(paths.intervals)
    if not intervals:
        raise SystemExit(f"Missing interval file: {paths.intervals}")

    ensure_dir(paths.proxies_dir)
    ensure_dir(paths.storyboards_dir)
    ffmpeg = shutil.which("ffmpeg")
    if not ffmpeg:
        raise SystemExit("ffmpeg is required for proxy/storyboard generation")

    review_rows = []
    for i, interval in enumerate(intervals):
        log_progress("proxies", i + 1, len(intervals), interval.get("interval_id", ""))
        try:
            mp4_path = ensure_full_mp4(paths, ego_qc, interval)
            clip_path = paths.proxies_dir / f"{interval['interval_id']}.mp4"
            storyboard_path = paths.storyboards_dir / f"{interval['interval_id']}.jpg"
            start_s = float(interval["start_s"])
            duration_s = max(float(interval["duration_s"]), 0.1)

            if not clip_path.exists():
                run_ffmpeg(
                    [
                        ffmpeg,
                        "-y",
                        "-ss",
                        f"{start_s:.3f}",
                        "-t",
                        f"{duration_s:.3f}",
                        "-i",
                        str(mp4_path),
                        "-an",
                        "-c:v",
                        "libx264",
                        "-preset",
                        "fast",
                        "-crf",
                        "23",
                        str(clip_path),
                    ]
                )

            if not storyboard_path.exists():
                fps = max(args.frames_per_storyboard / max(duration_s, 1.0), 0.1)
                filtergraph = f"fps={fps:.6f},scale=320:-1,tile=4x2"
                run_ffmpeg(
                    [
                        ffmpeg,
                        "-y",
                        "-i",
                        str(clip_path),
                        "-vf",
                        filtergraph,
                        "-frames:v",
                        "1",
                        str(storyboard_path),
                    ]
                )
        except Exception as exc:  # noqa: BLE001
            review_rows.append(
                review_entry(
                    "proxies",
                    interval["interval_id"],
                    "proxy_generation_failed",
                    interval_id=interval["interval_id"],
                    source_key=interval["source_key"],
                    details=str(exc),
                )
            )

    if review_rows:
        merge_jsonl(paths.review_queue, review_rows, ["review_id"])

    print(f"Generated proxies in {paths.proxies_dir} and storyboards in {paths.storyboards_dir}")


def cmd_label(args: argparse.Namespace) -> None:
    paths = resolve_workspace(args)
    client = OpenAICompatibleClient.from_env(args)
    maybe_materialize_effective_artifacts(args, paths)
    episodes_path = paths.effective_episodes if paths.effective_episodes.exists() else paths.episodes
    intervals_path = paths.effective_intervals if paths.effective_intervals.exists() else paths.intervals
    episodes = read_jsonl(episodes_path)
    intervals = filter_active_intervals(read_jsonl(intervals_path), episodes)
    if not intervals:
        raise SystemExit(f"No active intervals available for labeling in {intervals_path}")

    existing = {row["interval_id"]: row for row in read_jsonl(paths.labels)}
    output_rows = dict(existing)
    review_rows = []

    selected = intervals[: args.limit] if args.limit else intervals
    for i, interval in enumerate(selected):
        iid = interval["interval_id"]
        log_progress("label", i + 1, len(selected), iid)
        if args.skip_existing and iid in existing:
            continue

        storyboard_path = paths.storyboards_dir / f"{iid}.jpg"
        if not storyboard_path.exists():
            review_rows.append(
                review_entry(
                    "label",
                    iid,
                    "missing_storyboard",
                    interval_id=iid,
                    source_key=interval["source_key"],
                )
            )
            continue

        try:
            parsed, raw_response = client.classify_storyboard(storyboard_path, interval)
            confidence = float(parsed.get("confidence", 0.0) or 0.0)
            raw_is_manipulation = parsed.get("is_manipulation", False)
            is_manipulation = raw_is_manipulation is True or str(raw_is_manipulation).strip().lower() == "true"
            proposed_task_name = str(parsed.get("proposed_task_name", "") or "").strip() or "unknown"
            primary_objects = parsed.get("primary_objects", [])
            if not isinstance(primary_objects, list):
                primary_objects = []

            label_row = {
                "interval_id": iid,
                "source_key": interval["source_key"],
                "proposal_source": interval.get("proposal_source"),
                "is_manipulation": is_manipulation,
                "proposed_task_name": proposed_task_name,
                "normalized_task_name": normalize_open_label(proposed_task_name),
                "short_caption": str(parsed.get("short_caption", "") or ""),
                "primary_objects": [str(obj) for obj in primary_objects],
                "confidence": confidence,
                "reason": str(parsed.get("reason", "") or ""),
                "storyboard_path": str(storyboard_path),
                "labeled_at": now_iso(),
                "raw_model_name": client.model,
                "raw_response_id": raw_response.get("id"),
            }
            output_rows[iid] = label_row

            if confidence < args.low_confidence_threshold or not is_manipulation:
                review_rows.append(
                    review_entry(
                        "label",
                        iid,
                        "low_confidence_or_non_manipulation",
                        interval_id=iid,
                        source_key=interval["source_key"],
                        confidence=confidence,
                        proposed_task_name=proposed_task_name,
                    )
                )
        except Exception as exc:  # noqa: BLE001
            review_rows.append(
                review_entry(
                    "label",
                    iid,
                    "vlm_failure",
                    interval_id=iid,
                    source_key=interval["source_key"],
                    details=str(exc),
                )
            )

    label_rows = sorted(output_rows.values(), key=lambda row: row["interval_id"])
    write_jsonl(paths.labels, label_rows)
    if review_rows:
        merge_jsonl(paths.review_queue, review_rows, ["review_id"])
    print(f"Wrote {len(label_rows)} label rows to {paths.labels}")


def cluster_by_similarity(vectors: list[list[float]], threshold: float) -> list[list[int]]:
    n = len(vectors)
    uf = UnionFind(n)
    for i in range(n):
        for j in range(i + 1, n):
            if cosine_similarity(vectors[i], vectors[j]) >= threshold:
                uf.union(i, j)

    clusters: dict[int, list[int]] = defaultdict(list)
    for idx in range(n):
        clusters[uf.find(idx)].append(idx)
    return list(clusters.values())


def choose_canonical_name(rows: list[dict[str, Any]]) -> str:
    counter = Counter(row["normalized_task_name"] for row in rows)
    return counter.most_common(1)[0][0]


def build_effective_bucket_map(
    label_rows: list[dict[str, Any]],
    bucket_map: dict[str, Any],
    overrides: dict[str, Any],
) -> dict[str, Any]:
    rename_overrides = overrides.get("buckets", {}).get("renames", {})
    assignment_overrides = overrides.get("buckets", {}).get("interval_assignments", {})

    buckets_by_id: dict[str, dict[str, Any]] = {
        str(bucket["bucket_id"]): dict(bucket)
        for bucket in bucket_map.get("buckets", [])
    }
    mapping: dict[str, str] = {
        str(interval_id_): str(bucket_id)
        for interval_id_, bucket_id in bucket_map.get("mapping", {}).items()
    }

    for interval_id_, override in assignment_overrides.items():
        bucket_id = str(override.get("bucket_id") or "").strip()
        if not bucket_id:
            continue
        mapping[str(interval_id_)] = bucket_id
        if bucket_id not in buckets_by_id:
            canonical_name = str(override.get("canonical_task_name") or bucket_id).strip() or bucket_id
            buckets_by_id[bucket_id] = {
                "bucket_id": bucket_id,
                "canonical_task_name": canonical_name,
                "member_count": 0,
                "source_labels": [],
                "interval_ids": [],
                "average_confidence": 0.0,
                "primary_objects": [],
            }

    labels_by_interval = {str(row["interval_id"]): row for row in label_rows}
    for bucket in buckets_by_id.values():
        bucket["interval_ids"] = []

    for interval_id_, bucket_id in mapping.items():
        bucket = buckets_by_id.setdefault(
            bucket_id,
            {
                "bucket_id": bucket_id,
                "canonical_task_name": bucket_id,
                "member_count": 0,
                "source_labels": [],
                "interval_ids": [],
                "average_confidence": 0.0,
                "primary_objects": [],
            },
        )
        bucket["interval_ids"].append(interval_id_)

    for bucket_id, bucket in buckets_by_id.items():
        rename = rename_overrides.get(bucket_id)
        if rename and rename.get("canonical_task_name"):
            bucket["canonical_task_name"] = str(rename["canonical_task_name"]).strip() or bucket["canonical_task_name"]

        rows = [labels_by_interval[iid] for iid in bucket.get("interval_ids", []) if iid in labels_by_interval]
        confidences = [float(row.get("confidence", 0.0) or 0.0) for row in rows]
        bucket["member_count"] = len(bucket.get("interval_ids", []))
        bucket["source_labels"] = sorted({str(row.get("normalized_task_name", "")) for row in rows if row.get("normalized_task_name")})
        bucket["primary_objects"] = sorted({str(obj) for row in rows for obj in row.get("primary_objects", [])})
        bucket["average_confidence"] = round(statistics.fmean(confidences), 6) if confidences else 0.0
        bucket["interval_ids"] = sorted(bucket.get("interval_ids", []))

    return {
        "version": bucket_map.get("version", PIPELINE_VERSION),
        "created_at": now_iso(),
        "method": bucket_map.get("method", "override_merge"),
        "similarity_threshold": bucket_map.get("similarity_threshold"),
        "buckets": sorted(
            [row for row in buckets_by_id.values() if row.get("interval_ids")],
            key=lambda row: str(row["bucket_id"]),
        ),
        "mapping": dict(sorted(mapping.items())),
    }


def resolved_episode_status(row: dict[str, Any] | None) -> str | None:
    if not row:
        return None
    value = row.get("effective_episode_status", row.get("episode_status"))
    return str(value) if value is not None else None


def resolved_interval_decision(row: dict[str, Any]) -> str:
    value = row.get("effective_interval_decision", row.get("decision", "keep"))
    return str(value or "keep")


def filter_active_intervals(
    interval_rows: list[dict[str, Any]],
    episode_rows: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    episode_by_source = {str(row.get("source_key")): row for row in episode_rows if row.get("source_key")}
    active_rows = []
    for row in interval_rows:
        if resolved_interval_decision(row) == "reject":
            continue
        episode = episode_by_source.get(str(row.get("source_key")))
        if resolved_episode_status(episode) in {"reject", "invalid"}:
            continue
        active_rows.append(row)
    return active_rows


def filter_active_labels(
    label_rows: list[dict[str, Any]],
    interval_rows: list[dict[str, Any]],
    episode_rows: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    active_interval_ids = {str(row.get("interval_id")) for row in filter_active_intervals(interval_rows, episode_rows)}
    return [row for row in label_rows if str(row.get("interval_id")) in active_interval_ids]


def maybe_materialize_effective_artifacts(args: argparse.Namespace, paths: WorkspacePaths) -> None:
    if paths.review_overrides.exists() and (paths.episodes.exists() or paths.intervals.exists() or paths.labels.exists()):
        cmd_materialize(args)


def cmd_materialize(args: argparse.Namespace) -> None:
    paths = resolve_workspace(args)
    episodes = read_jsonl(paths.episodes)
    intervals = read_jsonl(paths.intervals)
    labels = read_jsonl(paths.labels)
    bucket_map = read_json(paths.bucket_map, {"version": PIPELINE_VERSION, "buckets": [], "mapping": {}})
    overrides = load_review_overrides(paths)

    episode_overrides = overrides.get("episodes", {})
    interval_overrides = overrides.get("intervals", {})
    label_overrides = overrides.get("labels", {})

    effective_episodes = []
    for row in episodes:
        effective = dict(row)
        override = episode_overrides.get(str(row["episode_id"]))
        if override and row.get("validation_status") == "valid":
            effective["effective_episode_status"] = override.get("episode_status", row.get("episode_status"))
            effective["override_note"] = override.get("note")
            effective["override_updated_at"] = override.get("updated_at")
        else:
            effective["effective_episode_status"] = row.get("episode_status")
        effective_episodes.append(effective)

    effective_intervals = []
    for row in intervals:
        effective = dict(row)
        override = interval_overrides.get(str(row["interval_id"]))
        start_s = float(row.get("start_s", 0.0) or 0.0)
        end_s = float(row.get("end_s", start_s) or start_s)
        effective_start_s = start_s
        effective_end_s = end_s
        if override:
            effective["effective_interval_decision"] = override.get("decision", "keep")
            effective["override_note"] = override.get("note")
            effective["override_updated_at"] = override.get("updated_at")
            trim_start_s = override.get("trim_start_s")
            trim_end_s = override.get("trim_end_s")
            if trim_start_s is not None:
                effective_start_s = clamp(float(trim_start_s), start_s, end_s)
            if trim_end_s is not None:
                effective_end_s = clamp(float(trim_end_s), start_s, end_s)
        else:
            effective["effective_interval_decision"] = "keep"
        if effective_end_s <= effective_start_s:
            effective_start_s = start_s
            effective_end_s = end_s
        effective["effective_start_s"] = maybe_round(effective_start_s)
        effective["effective_end_s"] = maybe_round(effective_end_s)
        effective["effective_duration_s"] = maybe_round(max(0.0, effective_end_s - effective_start_s))
        effective_intervals.append(effective)

    effective_labels = []
    for row in labels:
        effective = dict(row)
        override = label_overrides.get(str(row["interval_id"]))
        if override:
            for field in ("is_manipulation", "proposed_task_name", "short_caption", "primary_objects", "confidence", "reason"):
                if field in override:
                    effective[field] = override[field]
            effective["normalized_task_name"] = normalize_open_label(str(effective.get("proposed_task_name", "")))
            effective["override_note"] = override.get("note")
            effective["override_updated_at"] = override.get("updated_at")
        effective_labels.append(effective)

    effective_bucket_map = build_effective_bucket_map(effective_labels, bucket_map, overrides)

    ensure_dir(paths.effective_dir)
    write_jsonl(paths.effective_episodes, sorted(effective_episodes, key=lambda row: str(row["episode_id"])))
    write_jsonl(
        paths.effective_intervals,
        sorted(effective_intervals, key=lambda row: (str(row.get("source_key", "")), float(row.get("start_s", 0.0)))),
    )
    write_jsonl(paths.effective_labels, sorted(effective_labels, key=lambda row: str(row["interval_id"])))
    write_json(paths.effective_bucket_map, effective_bucket_map)
    print(f"Wrote effective artifacts to {paths.effective_dir}")


def cmd_segments(args: argparse.Namespace) -> None:
    paths = resolve_workspace(args)
    ego_qc = resolve_ego_qc_binary(args.ego_qc)

    if not paths.effective_intervals.exists() or not paths.effective_episodes.exists():
        cmd_materialize(args)

    episodes = {str(row["source_key"]): row for row in read_jsonl(paths.effective_episodes)}
    intervals_path = paths.effective_intervals if paths.effective_intervals.exists() else paths.intervals
    intervals = read_jsonl(intervals_path)
    if not intervals:
        raise SystemExit(f"Missing interval file: {intervals_path}")

    ensure_dir(paths.segments_dir)
    manifest_rows = []
    review_rows = []
    write_jsonl(paths.segments_manifest, [])

    for i, row in enumerate(intervals):
        log_progress("segments", i + 1, len(intervals), row.get("interval_id", ""))
        source_key = str(row["source_key"])
        episode = episodes.get(source_key, {})
        if row.get("effective_interval_decision") == "reject":
            continue
        if episode and episode.get("effective_episode_status") not in {None, "keep", "review"}:
            continue

        start_s = float(row.get("effective_start_s", row.get("start_s", 0.0)) or 0.0)
        end_s = float(row.get("effective_end_s", row.get("end_s", start_s)) or start_s)
        duration_s = max(0.0, end_s - start_s)
        if duration_s <= 0.0:
            review_rows.append(
                review_entry(
                    "segments",
                    str(row["interval_id"]),
                    "invalid_effective_interval",
                    interval_id=row["interval_id"],
                    source_key=source_key,
                    start_s=start_s,
                    end_s=end_s,
                )
            )
            continue

        segment_path = paths.segments_dir / f"{row['interval_id']}.egorec"
        proc = run_command(
            [
                ego_qc,
                "clip",
                str(row["local_path"]),
                "--start",
                f"{start_s:.6f}",
                "--end",
                f"{end_s:.6f}",
                "--output",
                str(segment_path),
            ],
            quiet=True,
        )
        if proc.returncode != 0:
            review_rows.append(
                review_entry(
                    "segments",
                    str(row["interval_id"]),
                    "segment_export_failed",
                    interval_id=row["interval_id"],
                    source_key=source_key,
                    details=proc.stderr.strip() or proc.stdout.strip() or "clip export failed",
                )
            )
            continue

        manifest_row = {
            "interval_id": row["interval_id"],
            "source_key": source_key,
            "segment_path": str(segment_path),
            "effective_start_s": maybe_round(start_s),
            "effective_end_s": maybe_round(end_s),
            "duration_s": maybe_round(duration_s),
            "proposal_score": float(row.get("proposal_score", 0.0) or 0.0),
            "note": row.get("override_note"),
        }
        manifest_rows.append(manifest_row)
        append_jsonl(paths.segments_manifest, [manifest_row])

    write_jsonl(paths.segments_manifest, sorted(manifest_rows, key=lambda row: str(row["interval_id"])))
    if review_rows:
        merge_jsonl(paths.review_queue, review_rows, ["review_id"])
    print(f"Exported {len(manifest_rows)} reviewed segments to {paths.segments_dir}")


def cmd_cluster(args: argparse.Namespace) -> None:
    paths = resolve_workspace(args)
    maybe_materialize_effective_artifacts(args, paths)
    episodes_path = paths.effective_episodes if paths.effective_episodes.exists() else paths.episodes
    intervals_path = paths.effective_intervals if paths.effective_intervals.exists() else paths.intervals
    labels_path = paths.effective_labels if paths.effective_labels.exists() else paths.labels
    episodes = read_jsonl(episodes_path)
    intervals = read_jsonl(intervals_path)
    label_rows = filter_active_labels(read_jsonl(labels_path), intervals, episodes)
    if not label_rows:
        raise SystemExit(f"No active labels available for clustering in {labels_path}")

    manipulation_rows = [row for row in label_rows if row.get("is_manipulation")]
    non_manipulation_rows = [row for row in label_rows if not row.get("is_manipulation")]

    review_rows = []
    method = "normalized_label"
    clusters: list[list[dict[str, Any]]] = []

    if manipulation_rows:
        client = None
        vectors = None
        if args.embedding_model or os.environ.get("VLM_EMBEDDING_MODEL") or os.environ.get("OPENAI_EMBEDDING_MODEL"):
            client = OpenAICompatibleClient.from_env(args, require_model=False)
            vectors = client.embed_texts([cluster_text(row) for row in manipulation_rows])
            cluster_indices = cluster_by_similarity(vectors, args.similarity_threshold)
            clusters = [[manipulation_rows[idx] for idx in indices] for indices in cluster_indices]
            method = "embedding_cosine"
        else:
            grouped: dict[str, list[dict[str, Any]]] = defaultdict(list)
            for row in manipulation_rows:
                grouped[row["normalized_task_name"]].append(row)
            clusters = list(grouped.values())

    bucket_rows = []
    mapping: dict[str, str] = {}
    cluster_counter = 0
    for cluster in sorted(clusters, key=lambda rows: (-len(rows), choose_canonical_name(rows))):
        bucket_id = f"bucket_{cluster_counter:03d}"
        cluster_counter += 1
        canonical_name = choose_canonical_name(cluster)
        avg_conf = statistics.fmean(float(row.get("confidence", 0.0) or 0.0) for row in cluster)
        bucket_rows.append(
            {
                "bucket_id": bucket_id,
                "canonical_task_name": canonical_name,
                "member_count": len(cluster),
                "source_labels": sorted({row["normalized_task_name"] for row in cluster}),
                "interval_ids": [row["interval_id"] for row in cluster],
                "average_confidence": round(avg_conf, 6),
                "primary_objects": sorted({obj for row in cluster for obj in row.get("primary_objects", [])}),
            }
        )
        for row in cluster:
            mapping[row["interval_id"]] = bucket_id

        if len(cluster) == 1 or avg_conf < args.low_confidence_threshold:
            review_rows.append(
                review_entry(
                    "cluster",
                    bucket_id,
                    "singleton_or_low_confidence_cluster",
                    bucket_id=bucket_id,
                    canonical_task_name=canonical_name,
                    member_count=len(cluster),
                    average_confidence=avg_conf,
                )
            )

    if non_manipulation_rows:
        bucket_id = "bucket_non_manipulation"
        bucket_rows.append(
            {
                "bucket_id": bucket_id,
                "canonical_task_name": "non_manipulation",
                "member_count": len(non_manipulation_rows),
                "source_labels": ["idle_or_other"],
                "interval_ids": [row["interval_id"] for row in non_manipulation_rows],
                "average_confidence": round(
                    statistics.fmean(float(row.get("confidence", 0.0) or 0.0) for row in non_manipulation_rows),
                    6,
                ),
                "primary_objects": [],
            }
        )
        for row in non_manipulation_rows:
            mapping[row["interval_id"]] = bucket_id

    payload = {
        "version": PIPELINE_VERSION,
        "created_at": now_iso(),
        "method": method,
        "similarity_threshold": args.similarity_threshold if method == "embedding_cosine" else None,
        "buckets": bucket_rows,
        "mapping": mapping,
    }
    ensure_parent(paths.bucket_map)
    paths.bucket_map.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
    if review_rows:
        merge_jsonl(paths.review_queue, review_rows, ["review_id"])
    print(f"Wrote bucket map to {paths.bucket_map}")


def cmd_publish(args: argparse.Namespace) -> None:
    paths = resolve_workspace(args)
    cfg = load_r2_config(Path(args.config))
    s3_client = make_s3_client(cfg)

    if paths.episodes.exists() and paths.intervals.exists() and paths.labels.exists() and paths.bucket_map.exists():
        cmd_materialize(args)

    publish_files = []
    for root in [paths.root / "inventory" / PIPELINE_VERSION, paths.root / "curation" / PIPELINE_VERSION]:
        if not root.exists():
            continue
        publish_files.extend(path for path in root.rglob("*") if path.is_file())

    if not publish_files:
        raise SystemExit("No artifacts to publish")

    prefix = args.remote_prefix.strip("/")
    for path in sorted(publish_files):
        rel = path.relative_to(paths.root).as_posix()
        key = f"{prefix}/{rel}" if prefix else rel
        extra_args = {"ContentType": content_type_for_file(path)}
        print(f"Uploading {path} -> s3://{cfg.bucket}/{key}")
        s3_client.upload_file(str(path), cfg.bucket, key, ExtraArgs=extra_args)

    print(f"Published {len(publish_files)} artifacts to {cfg.bucket}")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workspace", default=str(DEFAULT_WORKSPACE), help="Local workspace for curation artifacts")
    parser.add_argument("--config", default=str(DEFAULT_CONFIG_PATH), help="upload_config.toml with bucket defaults")

    sub = parser.add_subparsers(dest="command", required=True)

    inventory = sub.add_parser("inventory", help="Inventory raw R2 objects into inventory/v1/raw_objects.jsonl")
    inventory.add_argument("--prefix", default="", help="Optional R2 prefix filter")
    inventory.set_defaults(func=cmd_inventory)

    stage = sub.add_parser("stage", help="Download selected inventory rows to the local staging workspace")
    stage.add_argument("--source-prefix", help="Only stage one top-level source prefix")
    stage.add_argument("--include-invalid", action="store_true", help="Stage invalid rows too")
    stage.add_argument("--max-files", type=int, help="Limit the number of staged files")
    stage.set_defaults(func=cmd_stage)

    qc = sub.add_parser("qc", help="Run validation/analyze calibration on staged files and write episodes.jsonl")
    qc.add_argument("--ego-qc", help="Path to ego-qc binary")
    qc.add_argument("--reject-shorter-than-s", type=float, default=SHORT_EPISODE_REJECT_S)
    qc.add_argument("--limit", type=int, help="Only process the first N new episodes")
    qc.add_argument("--skip-existing", action="store_true", help="Skip episodes that already have QC results")
    qc.set_defaults(func=cmd_qc)

    intervals = sub.add_parser("intervals", help="Generate conservative manipulation intervals")
    intervals.add_argument("--ego-qc", help="Path to ego-qc binary")
    intervals.add_argument("--full-episode-seconds", type=float, default=FULL_EPISODE_CLASSIFY_S)
    intervals.add_argument("--min-gap-seconds", type=float, default=INTERVAL_MIN_GAP_S)
    intervals.add_argument("--min-duration-seconds", type=float, default=INTERVAL_MIN_DURATION_S)
    intervals.add_argument("--pad-seconds", type=float, default=INTERVAL_PAD_S)
    intervals.add_argument("--hand-sample-fps", type=float, default=HAND_SAMPLE_FPS)
    intervals.add_argument("--max-segment-seconds", type=float, default=HAND_MAX_SEGMENT_S)
    intervals.add_argument("--vlm-max-window-seconds", type=float, default=VLM_MAX_WINDOW_S)
    intervals.add_argument("--vlm-overlap-seconds", type=float, default=VLM_WINDOW_OVERLAP_S)
    intervals.add_argument("--limit", type=int, help="Only process the first N new episodes")
    intervals.add_argument("--skip-existing", action="store_true", help="Skip episodes that already have intervals")
    intervals.set_defaults(func=cmd_intervals)

    proxies = sub.add_parser("proxies", help="Generate MP4 interval clips and storyboard sheets")
    proxies.add_argument("--ego-qc", help="Path to ego-qc binary")
    proxies.add_argument("--frames-per-storyboard", type=float, default=8.0)
    proxies.set_defaults(func=cmd_proxies)

    label = sub.add_parser("label", help="Run VLM classification on interval storyboards")
    label.add_argument("--base-url", help="OpenAI-compatible API base URL")
    label.add_argument("--api-key", help="OpenAI-compatible API key")
    label.add_argument("--model", help="OpenAI-compatible multimodal model")
    label.add_argument("--embedding-model", help="Optional embedding model for clustering")
    label.add_argument("--limit", type=int, help="Only label the first N intervals")
    label.add_argument("--skip-existing", action="store_true")
    label.add_argument("--low-confidence-threshold", type=float, default=LOW_CONFIDENCE_THRESHOLD)
    label.set_defaults(func=cmd_label)

    cluster = sub.add_parser("cluster", help="Cluster open-ended VLM labels into canonical buckets")
    cluster.add_argument("--base-url", help="OpenAI-compatible API base URL")
    cluster.add_argument("--api-key", help="OpenAI-compatible API key")
    cluster.add_argument("--model", help="Any valid model; only required if embeddings are used")
    cluster.add_argument("--embedding-model", help="Embedding model for semantic clustering")
    cluster.add_argument("--similarity-threshold", type=float, default=CLUSTER_SIMILARITY_THRESHOLD)
    cluster.add_argument("--low-confidence-threshold", type=float, default=LOW_CONFIDENCE_THRESHOLD)
    cluster.set_defaults(func=cmd_cluster)

    materialize = sub.add_parser("materialize", help="Merge review overrides into effective curated artifacts")
    materialize.set_defaults(func=cmd_materialize)

    segments = sub.add_parser("segments", help="Export accepted effective intervals as standalone .egorec clips")
    segments.add_argument("--ego-qc", help="Path to ego-qc binary")
    segments.set_defaults(func=cmd_segments)

    publish = sub.add_parser("publish", help="Upload inventory/curation artifacts back to R2")
    publish.add_argument("--remote-prefix", default="", help="Optional destination prefix in the same bucket")
    publish.set_defaults(func=cmd_publish)

    return parser


def main() -> int:
    load_runtime_env()
    parser = build_parser()
    args = parser.parse_args()
    try:
        args.func(args)
    except (BotoCoreError, ClientError, RuntimeError, FileNotFoundError) as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
