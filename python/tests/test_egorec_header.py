import struct
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import egorec_header


def _fixed_cstring(value: str, size: int) -> bytes:
    raw = value.encode("utf-8")
    if len(raw) >= size:
        raw = raw[: size - 1]
    return raw + b"\x00" + b"\x00" * (size - len(raw) - 1)


def _build_header_bytes() -> bytes:
    parts = [
        egorec_header.FILE_MAGIC,
        struct.pack("<II", egorec_header.HEADER_SIZE, 0x03),
        _fixed_cstring("SERIAL-123", 32),
        struct.pack("<f", 0.001),
        struct.pack("<I", 640),
        struct.pack("<I", 480),
        struct.pack("<f", 100.0),
        struct.pack("<f", 101.0),
        struct.pack("<f", 320.0),
        struct.pack("<f", 240.0),
        struct.pack("<I", 4),
        struct.pack("<5f", 1.0, 2.0, 3.0, 4.0, 5.0),
        struct.pack("<I", 1280),
        struct.pack("<I", 720),
        struct.pack("<f", 200.0),
        struct.pack("<f", 201.0),
        struct.pack("<f", 640.0),
        struct.pack("<f", 360.0),
        struct.pack("<I", 2),
        struct.pack("<5f", 6.0, 7.0, 8.0, 9.0, 10.0),
        struct.pack("<9f", *range(1, 10)),
        struct.pack("<3f", 0.1, 0.2, 0.3),
        _fixed_cstring("session-abc", 128),
        struct.pack("<Q", 1_700_000_123_456_789),
        _fixed_cstring("USB 3.2", 8),
        struct.pack("<BBBB", 2, 1, 23, 7),
        bytes(range(128)),
    ]
    data = b"".join(parts)
    assert len(data) == egorec_header.HEADER_SIZE
    return data


def _build_footer_bytes() -> bytes:
    data = struct.pack(
        "<IQIQQI",
        egorec_header.INDEX_MAGIC,
        123456,
        42,
        600,
        20_000_000,
        egorec_header.FOOTER_MAGIC,
    )
    assert len(data) == egorec_header.FOOTER_SIZE
    return data


class EgorecHeaderTests(unittest.TestCase):
    def test_constants_match_wire_layout(self) -> None:
        self.assertEqual(egorec_header.HEADER_SIZE, 472)
        self.assertEqual(egorec_header.FOOTER_SIZE, 36)

    def test_read_header_parses_real_layout(self) -> None:
        header = egorec_header.read_header(_build_header_bytes())

        self.assertEqual(header.header_size, egorec_header.HEADER_SIZE)
        self.assertTrue(header.has_imu)
        self.assertTrue(header.has_index)
        self.assertEqual(header.serial_number, "SERIAL-123")
        self.assertEqual(header.depth_width, 640)
        self.assertEqual(header.depth_height, 480)
        self.assertEqual(header.color_width, 1280)
        self.assertEqual(header.color_height, 720)
        self.assertEqual(header.depth_distortion_coeffs, [1.0, 2.0, 3.0, 4.0, 5.0])
        self.assertEqual(header.color_distortion_coeffs, [6.0, 7.0, 8.0, 9.0, 10.0])
        self.assertEqual(header.session_name, "session-abc")
        self.assertEqual(header.usb_type, "USB 3.2")
        self.assertEqual(header.rgb_codec, 2)
        self.assertEqual(header.depth_codec, 1)
        self.assertEqual(header.rgb_quality, 23)
        self.assertEqual(header.zstd_level, 7)
        self.assertEqual(header.reserved, bytes(range(128)))

    def test_read_metadata_reads_footer_if_present(self) -> None:
        header_bytes = _build_header_bytes()
        footer_bytes = _build_footer_bytes()

        with tempfile.TemporaryDirectory() as tmpdir:
            path = Path(tmpdir) / "sample.egorec"
            path.write_bytes(header_bytes + b"\x00" * 64 + footer_bytes)

            meta = egorec_header.read_metadata(path)

        self.assertIsNotNone(meta.footer)
        self.assertEqual(meta.frame_count, 600)
        self.assertAlmostEqual(meta.duration_s, 20.0)
        self.assertAlmostEqual(meta.fps, 30.0)
        self.assertEqual(meta.header.session_name, "session-abc")

    def test_read_metadata_tolerates_missing_footer(self) -> None:
        header_bytes = _build_header_bytes()

        with tempfile.TemporaryDirectory() as tmpdir:
            path = Path(tmpdir) / "sample.egorec"
            path.write_bytes(header_bytes + b"\x00" * 16 + b"not-a-footer")

            meta = egorec_header.read_metadata(path)

        self.assertIsNone(meta.footer)
        self.assertEqual(meta.frame_count, 0)
        self.assertEqual(meta.duration_s, 0.0)


if __name__ == "__main__":
    unittest.main()
