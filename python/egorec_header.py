"""Lightweight .egorec v2 header and footer parser.

Reads the fixed-size binary structures without requiring ffmpeg, Zdepth,
or any compiled extensions. Matches the format defined in
ego-recorder/rust/egorec/src/format.rs.

All multi-byte values are little-endian (#pragma pack(push, 1) in C++).
"""

import struct
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

# Magic bytes: ASCII "EGOREC" + version 2.0
FILE_MAGIC = b"EGOREC\x02\x00"
FOOTER_MAGIC = 0x454E4F44  # 'DONE'
INDEX_MAGIC = 0x58444E49   # 'INDX'
BROWSER_STREAMABLE_RGB_CODEC = 2

# See rust/egorec/src/format.rs FILE_HEADER_SIZE / FileFooter.
HEADER_SIZE = 472
FOOTER_SIZE = 36


@dataclass
class EgorecHeader:
    """Parsed .egorec v2 file header."""

    magic: bytes
    header_size: int
    flags: int
    serial_number: str
    depth_scale: float
    depth_width: int
    depth_height: int
    depth_fx: float
    depth_fy: float
    depth_ppx: float
    depth_ppy: float
    depth_distortion_model: int
    depth_distortion_coeffs: list[float]
    color_width: int
    color_height: int
    color_fx: float
    color_fy: float
    color_ppx: float
    color_ppy: float
    color_distortion_model: int
    color_distortion_coeffs: list[float]
    extrinsic_rotation: list[float]
    extrinsic_translation: list[float]
    session_name: str
    start_timestamp_us: int
    usb_type: str
    rgb_codec: int
    depth_codec: int
    rgb_quality: int
    zstd_level: int
    reserved: bytes

    @property
    def has_imu(self) -> bool:
        return bool(self.flags & 0x01)

    @property
    def has_index(self) -> bool:
        return bool(self.flags & 0x02)

    @property
    def recorded_at(self) -> datetime | None:
        if self.start_timestamp_us > 0:
            return datetime.fromtimestamp(
                self.start_timestamp_us / 1_000_000, tz=timezone.utc
            )
        return None

    def to_intrinsics_dict(self) -> dict:
        """Return camera intrinsics as a JSON-serializable dict."""

        return {
            "color": {
                "width": self.color_width,
                "height": self.color_height,
                "fx": self.color_fx,
                "fy": self.color_fy,
                "ppx": self.color_ppx,
                "ppy": self.color_ppy,
                "distortion_model": self.color_distortion_model,
                "distortion_coeffs": self.color_distortion_coeffs,
            },
            "depth": {
                "width": self.depth_width,
                "height": self.depth_height,
                "fx": self.depth_fx,
                "fy": self.depth_fy,
                "ppx": self.depth_ppx,
                "ppy": self.depth_ppy,
                "distortion_model": self.depth_distortion_model,
                "distortion_coeffs": self.depth_distortion_coeffs,
                "scale": self.depth_scale,
            },
            "extrinsics": {
                "rotation": self.extrinsic_rotation,
                "translation": self.extrinsic_translation,
            },
        }


@dataclass
class EgorecFooter:
    """Parsed .egorec v2 file footer."""

    index_magic: int
    index_offset: int
    index_entry_count: int
    total_frames: int
    total_duration_us: int
    footer_magic: int

    @property
    def duration_s(self) -> float:
        return self.total_duration_us / 1_000_000

    @property
    def fps(self) -> float:
        if self.total_duration_us > 0 and self.total_frames > 0:
            return self.total_frames / (self.total_duration_us / 1_000_000)
        return 0.0


@dataclass
class EgorecMetadata:
    """Combined header + footer metadata for an .egorec file."""

    header: EgorecHeader
    footer: EgorecFooter | None
    file_size: int
    file_path: str

    @property
    def frame_count(self) -> int:
        return self.footer.total_frames if self.footer else 0

    @property
    def duration_s(self) -> float:
        return self.footer.duration_s if self.footer else 0.0

    @property
    def fps(self) -> float:
        return self.footer.fps if self.footer else 0.0

    @property
    def video_streamable(self) -> bool:
        return self.header.rgb_codec == BROWSER_STREAMABLE_RGB_CODEC

    @property
    def video_stream_error(self) -> str | None:
        if self.video_streamable:
            return None
        return (
            f"rgb_codec={self.header.rgb_codec} is not browser-streamable "
            f"(need {BROWSER_STREAMABLE_RGB_CODEC} for H.264)"
        )

    def to_episode_dict(self) -> dict:
        """Return metadata as a dict suitable for the facility API registration."""

        return {
            "session_name": self.header.session_name or None,
            "frame_count": self.frame_count,
            "duration_s": self.duration_s,
            "resolution_w": self.header.color_width,
            "resolution_h": self.header.color_height,
            "fps": round(self.fps, 2) if self.fps else None,
            "file_size_bytes": self.file_size,
            "recorded_at": (
                self.header.recorded_at.isoformat() if self.header.recorded_at else None
            ),
            "camera_serial": self.header.serial_number or None,
            "has_depth": True,
            "has_imu": self.header.has_imu,
            "intrinsics": self.header.to_intrinsics_dict(),
            "rgb_codec": self.header.rgb_codec,
            "depth_codec": self.header.depth_codec,
            "video_streamable": self.video_streamable,
            "video_stream_error": self.video_stream_error,
        }


def _read_cstring(data: bytes, max_len: int) -> str:
    """Read a null-terminated C string from a fixed-width byte field."""

    end = data.find(b"\x00")
    if end == -1:
        end = max_len
    return data[:end].decode("utf-8", errors="replace")


def read_header(data: bytes) -> EgorecHeader:
    """Parse an EgorecHeader from raw bytes (must be >= HEADER_SIZE bytes)."""

    if len(data) < HEADER_SIZE:
        raise ValueError(f"Need at least {HEADER_SIZE} bytes, got {len(data)}")

    off = 0
    magic = data[off:off + 8]
    if magic != FILE_MAGIC:
        raise ValueError(f"Invalid .egorec magic: {magic!r}")
    off += 8

    header_size, flags = struct.unpack_from("<II", data, off)
    off += 8

    serial_number = _read_cstring(data[off:off + 32], 32)
    off += 32

    depth_scale = struct.unpack_from("<f", data, off)[0]
    off += 4
    depth_w = struct.unpack_from("<I", data, off)[0]
    off += 4
    depth_h = struct.unpack_from("<I", data, off)[0]
    off += 4
    depth_fx = struct.unpack_from("<f", data, off)[0]
    off += 4
    depth_fy = struct.unpack_from("<f", data, off)[0]
    off += 4
    depth_ppx = struct.unpack_from("<f", data, off)[0]
    off += 4
    depth_ppy = struct.unpack_from("<f", data, off)[0]
    off += 4
    depth_dist_model = struct.unpack_from("<I", data, off)[0]
    off += 4
    depth_dist_coeffs = list(struct.unpack_from("<5f", data, off))
    off += 20

    color_w = struct.unpack_from("<I", data, off)[0]
    off += 4
    color_h = struct.unpack_from("<I", data, off)[0]
    off += 4
    color_fx = struct.unpack_from("<f", data, off)[0]
    off += 4
    color_fy = struct.unpack_from("<f", data, off)[0]
    off += 4
    color_ppx = struct.unpack_from("<f", data, off)[0]
    off += 4
    color_ppy = struct.unpack_from("<f", data, off)[0]
    off += 4
    color_dist_model = struct.unpack_from("<I", data, off)[0]
    off += 4
    color_dist_coeffs = list(struct.unpack_from("<5f", data, off))
    off += 20

    extrinsic_rotation = list(struct.unpack_from("<9f", data, off))
    off += 36
    extrinsic_translation = list(struct.unpack_from("<3f", data, off))
    off += 12

    session_name = _read_cstring(data[off:off + 128], 128)
    off += 128
    start_timestamp_us = struct.unpack_from("<Q", data, off)[0]
    off += 8
    usb_type = _read_cstring(data[off:off + 8], 8)
    off += 8
    rgb_codec, depth_codec, rgb_quality, zstd_level = struct.unpack_from("<BBBB", data, off)
    off += 4

    reserved = data[off:off + 128]
    off += 128

    if off != HEADER_SIZE:
        raise ValueError(f"Header parse ended at {off}, expected {HEADER_SIZE}")

    return EgorecHeader(
        magic=magic,
        header_size=header_size,
        flags=flags,
        serial_number=serial_number,
        depth_scale=depth_scale,
        depth_width=depth_w,
        depth_height=depth_h,
        depth_fx=depth_fx,
        depth_fy=depth_fy,
        depth_ppx=depth_ppx,
        depth_ppy=depth_ppy,
        depth_distortion_model=depth_dist_model,
        depth_distortion_coeffs=depth_dist_coeffs,
        color_width=color_w,
        color_height=color_h,
        color_fx=color_fx,
        color_fy=color_fy,
        color_ppx=color_ppx,
        color_ppy=color_ppy,
        color_distortion_model=color_dist_model,
        color_distortion_coeffs=color_dist_coeffs,
        extrinsic_rotation=extrinsic_rotation,
        extrinsic_translation=extrinsic_translation,
        session_name=session_name,
        start_timestamp_us=start_timestamp_us,
        usb_type=usb_type,
        rgb_codec=rgb_codec,
        depth_codec=depth_codec,
        rgb_quality=rgb_quality,
        zstd_level=zstd_level,
        reserved=reserved,
    )


def read_footer(data: bytes) -> EgorecFooter:
    """Parse an EgorecFooter from raw bytes (must be exactly FOOTER_SIZE bytes)."""

    if len(data) < FOOTER_SIZE:
        raise ValueError(f"Need at least {FOOTER_SIZE} bytes for footer, got {len(data)}")

    index_magic, index_offset, index_entry_count, total_frames, total_duration_us, footer_magic = (
        struct.unpack_from("<IQIQQI", data, 0)
    )

    if index_magic != INDEX_MAGIC:
        raise ValueError(f"Invalid index magic: 0x{index_magic:08X}")
    if footer_magic != FOOTER_MAGIC:
        raise ValueError(f"Invalid footer magic: 0x{footer_magic:08X}")

    return EgorecFooter(
        index_magic=index_magic,
        index_offset=index_offset,
        index_entry_count=index_entry_count,
        total_frames=total_frames,
        total_duration_us=total_duration_us,
        footer_magic=footer_magic,
    )


def read_metadata(path: str | Path) -> EgorecMetadata:
    """Read header and footer metadata from an .egorec file.

    Only reads the first HEADER_SIZE bytes and last FOOTER_SIZE bytes,
    so it's fast even for multi-GB files.
    """

    path = Path(path)
    file_size = path.stat().st_size

    with open(path, "rb") as f:
        header_data = f.read(HEADER_SIZE)
        header = read_header(header_data)

        footer = None
        if file_size >= HEADER_SIZE + FOOTER_SIZE:
            f.seek(file_size - FOOTER_SIZE)
            footer_data = f.read(FOOTER_SIZE)
            try:
                footer = read_footer(footer_data)
            except ValueError:
                pass

    return EgorecMetadata(
        header=header,
        footer=footer,
        file_size=file_size,
        file_path=str(path),
    )
