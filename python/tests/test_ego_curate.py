import os
import sys
import tempfile
import types
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

boto3_stub = types.ModuleType("boto3")
boto3_stub.client = lambda *args, **kwargs: None

botocore_config_stub = types.ModuleType("botocore.config")


class DummyBotoConfig:
    def __init__(self, *args, **kwargs) -> None:
        self.args = args
        self.kwargs = kwargs


botocore_config_stub.Config = DummyBotoConfig

botocore_exceptions_stub = types.ModuleType("botocore.exceptions")


class DummyBotoError(Exception):
    pass


botocore_exceptions_stub.BotoCoreError = DummyBotoError
botocore_exceptions_stub.ClientError = DummyBotoError

dotenv_stub = types.ModuleType("dotenv")


def fake_load_dotenv(path=None, *args, **kwargs):
    target = Path(path) if path else Path(".env")
    if not target.exists():
        return False
    for line in target.read_text(encoding="utf-8").splitlines():
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        os.environ.setdefault(key.strip(), value.strip())
    return True


dotenv_stub.load_dotenv = fake_load_dotenv

botocore_stub = types.ModuleType("botocore")
botocore_stub.config = botocore_config_stub
botocore_stub.exceptions = botocore_exceptions_stub

sys.modules.setdefault("boto3", boto3_stub)
sys.modules.setdefault("botocore", botocore_stub)
sys.modules.setdefault("botocore.config", botocore_config_stub)
sys.modules.setdefault("botocore.exceptions", botocore_exceptions_stub)
sys.modules.setdefault("dotenv", dotenv_stub)

import ego_curate


class EgoCurateTests(unittest.TestCase):
    def test_load_egorec_reader_uses_local_build_path(self) -> None:
        build_path = str(ego_curate.ROOT / "build")
        import_calls: list[str] = []
        fake_module = object()

        def fake_import_module(name: str):
            import_calls.append(name)
            if len(import_calls) == 1:
                raise ImportError("not on sys.path")
            self.assertIn(build_path, sys.path)
            return fake_module

        with mock.patch.object(ego_curate.importlib, "import_module", side_effect=fake_import_module):
            original_path = list(sys.path)
            try:
                if build_path in sys.path:
                    sys.path.remove(build_path)
                module = ego_curate.load_egorec_reader()
            finally:
                sys.path[:] = original_path

        self.assertIs(module, fake_module)
        self.assertEqual(import_calls, ["egorec_reader", "egorec_reader"])

    def test_detect_hand_presence_samples_streams_egorec_frames(self) -> None:
        class FakeHandLandmarks:
            def __init__(self, area: float) -> None:
                self.landmark = [
                    SimpleNamespace(x=0.1, y=0.1),
                    SimpleNamespace(x=0.1 + area, y=0.1 + area),
                ]

        class FakeHandsContext:
            def __init__(self, outputs: list[SimpleNamespace], processed_frames: list[str]) -> None:
                self.outputs = iter(outputs)
                self.processed_frames = processed_frames

            def __enter__(self) -> "FakeHandsContext":
                return self

            def __exit__(self, exc_type, exc, tb) -> None:
                return None

            def process(self, rgb_frame):
                self.processed_frames.append(rgb_frame)
                return next(self.outputs)

        class FakeEgorecFile:
            def __init__(self, path: str) -> None:
                self.path = path

            def header(self):
                return {"duration_s": 0.3}

            def frame_count(self):
                return 3

            def frames(self):
                yield {"timestamp_relative_s": 18446744073709.52, "rgb": "frame-0"}
                yield {"timestamp_relative_s": 0.05, "rgb": "frame-1"}
                yield {"timestamp_relative_s": 0.21, "rgb": "frame-2"}

        processed_frames: list[str] = []
        fake_mp = SimpleNamespace(
            solutions=SimpleNamespace(
                hands=SimpleNamespace(
                    Hands=lambda **kwargs: FakeHandsContext(
                        [
                            SimpleNamespace(
                                multi_hand_landmarks=[FakeHandLandmarks(0.2)],
                                multi_handedness=[SimpleNamespace(classification=[SimpleNamespace(score=0.9)])],
                            ),
                            SimpleNamespace(
                                multi_hand_landmarks=[FakeHandLandmarks(0.05)],
                                multi_handedness=[SimpleNamespace(classification=[SimpleNamespace(score=0.9)])],
                            ),
                        ],
                        processed_frames,
                    )
                )
            )
        )
        fake_egorec_reader = SimpleNamespace(EgorecFile=FakeEgorecFile)

        with mock.patch.dict(sys.modules, {"mediapipe": fake_mp}):
            with mock.patch.object(ego_curate, "load_egorec_reader", return_value=fake_egorec_reader):
                samples = ego_curate.detect_hand_presence_samples(Path("/tmp/sample.egorec"), sample_fps=5.0)

        self.assertEqual(processed_frames, ["frame-0", "frame-2"])
        self.assertEqual([sample.timestamp_s for sample in samples], [0.0, 0.2])
        self.assertEqual([sample.hand_count for sample in samples], [1, 0])

    def test_vlm_env_prefers_python_env_file(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            default_env = root / "python.env"
            default_env.write_text("VLM_API_KEY=default-key\nVLM_MODEL=default-model\n", encoding="utf-8")
            (root / ".env").write_text("VLM_API_KEY=fallback-key\nVLM_MODEL=fallback-model\n", encoding="utf-8")

            previous_cwd = Path.cwd()
            try:
                os.chdir(root)
                with mock.patch.object(ego_curate, "DEFAULT_ENV_PATH", default_env):
                    with mock.patch.dict(os.environ, {}, clear=True):
                        client = ego_curate.OpenAICompatibleClient.from_env(
                            SimpleNamespace(
                                base_url=None,
                                api_key=None,
                                model=None,
                                embedding_model=None,
                            )
                        )
            finally:
                os.chdir(previous_cwd)

        self.assertEqual(client.api_key, "default-key")
        self.assertEqual(client.model, "default-model")

    def test_vlm_env_falls_back_to_cwd_env(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            missing_default = root / "missing.env"
            (root / ".env").write_text("VLM_API_KEY=fallback-key\nVLM_MODEL=fallback-model\n", encoding="utf-8")

            previous_cwd = Path.cwd()
            try:
                os.chdir(root)
                with mock.patch.object(ego_curate, "DEFAULT_ENV_PATH", missing_default):
                    with mock.patch.dict(os.environ, {}, clear=True):
                        client = ego_curate.OpenAICompatibleClient.from_env(
                            SimpleNamespace(
                                base_url=None,
                                api_key=None,
                                model=None,
                                embedding_model=None,
                            )
                        )
            finally:
                os.chdir(previous_cwd)

        self.assertEqual(client.api_key, "fallback-key")
        self.assertEqual(client.model, "fallback-model")

    def test_interval_id_is_stable(self) -> None:
        first = ego_curate.interval_id("a/b/file.egorec", 1.25, 9.75)
        second = ego_curate.interval_id("a/b/file.egorec", 1.25, 9.75)
        third = ego_curate.interval_id("a/b/file.egorec", 1.25, 9.76)

        self.assertEqual(first, second)
        self.assertNotEqual(first, third)

    def test_make_analysis_windows_splits_with_overlap(self) -> None:
        windows = ego_curate.make_analysis_windows(0.0, 400.0, 180.0, 15.0)

        self.assertEqual(
            windows,
            [
                {"start_s": 0.0, "end_s": 180.0},
                {"start_s": 165.0, "end_s": 345.0},
                {"start_s": 330.0, "end_s": 400.0},
            ],
        )

    def test_normalize_open_label(self) -> None:
        self.assertEqual(
            ego_curate.normalize_open_label("Pick-Up   Blue Cup!!"),
            "pick up blue cup",
        )

    def test_suggest_hand_activity_intervals_requires_overlap(self) -> None:
        samples = [
            ego_curate.HandPresenceSample(timestamp_s=0.0, hand_count=1, max_area_fraction=0.03),
            ego_curate.HandPresenceSample(timestamp_s=0.2, hand_count=1, max_area_fraction=0.03),
            ego_curate.HandPresenceSample(timestamp_s=0.4, hand_count=0, max_area_fraction=0.0),
            ego_curate.HandPresenceSample(timestamp_s=1.0, hand_count=1, max_area_fraction=0.03),
            ego_curate.HandPresenceSample(timestamp_s=1.2, hand_count=1, max_area_fraction=0.03),
        ]
        proposals = [
            {"start_s": 0.0, "end_s": 0.5, "active_fraction": 0.4},
            {"start_s": 0.9, "end_s": 1.5, "active_fraction": 0.8},
        ]

        intervals = ego_curate.suggest_hand_activity_intervals(
            2.0,
            proposals,
            samples,
            sample_fps=5.0,
            min_duration_s=0.1,
            merge_gap_s=0.25,
            pad_s=0.0,
            max_segment_s=10.0,
        )

        self.assertEqual(len(intervals), 2)
        self.assertEqual(intervals[0]["start_s"], 0.0)
        self.assertGreater(intervals[1]["proposal_score"], intervals[0]["proposal_score"])

    def test_suggest_hand_activity_intervals_splits_long_span(self) -> None:
        samples = [
            ego_curate.HandPresenceSample(timestamp_s=float(idx), hand_count=1, max_area_fraction=0.04)
            for idx in range(25)
            if idx not in {10, 11, 12, 13, 14}
        ]
        proposals = [{"start_s": 0.0, "end_s": 24.0, "active_fraction": 0.9}]

        intervals = ego_curate.suggest_hand_activity_intervals(
            24.0,
            proposals,
            samples,
            sample_fps=1.0,
            min_duration_s=1.0,
            merge_gap_s=1.0,
            pad_s=0.0,
            max_segment_s=10.0,
        )

        self.assertGreaterEqual(len(intervals), 2)
        self.assertTrue(all(interval["duration_s"] <= 10.0 for interval in intervals))

    def test_materialize_applies_episode_label_and_bucket_overrides(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            paths = ego_curate.WorkspacePaths.from_root(root)

            ego_curate.write_jsonl(
                paths.episodes,
                [
                    {
                        "episode_id": "ep_1",
                        "source_key": "InHouse2/sample.egorec",
                        "episode_status": "review",
                        "validation_status": "valid",
                    }
                ],
            )
            ego_curate.write_jsonl(
                paths.intervals,
                [
                    {
                        "interval_id": "int_1",
                        "source_key": "InHouse2/sample.egorec",
                        "start_s": 0.0,
                        "end_s": 12.0,
                        "duration_s": 12.0,
                    }
                ],
            )
            ego_curate.write_jsonl(
                paths.labels,
                [
                    {
                        "interval_id": "int_1",
                        "source_key": "InHouse2/sample.egorec",
                        "is_manipulation": True,
                        "proposed_task_name": "pick up cup",
                        "normalized_task_name": "pick up cup",
                        "short_caption": "lifting a cup",
                        "primary_objects": ["cup"],
                        "confidence": 0.8,
                        "reason": "visible grasp",
                    }
                ],
            )
            ego_curate.write_json(
                paths.bucket_map,
                {
                    "version": ego_curate.PIPELINE_VERSION,
                    "buckets": [
                        {
                            "bucket_id": "bucket_000",
                            "canonical_task_name": "pick up cup",
                            "member_count": 1,
                            "source_labels": ["pick up cup"],
                            "interval_ids": ["int_1"],
                            "average_confidence": 0.8,
                            "primary_objects": ["cup"],
                        }
                    ],
                    "mapping": {"int_1": "bucket_000"},
                },
            )
            ego_curate.write_json(
                paths.review_overrides,
                {
                    "version": ego_curate.PIPELINE_VERSION,
                    "updated_at": "2026-03-11T00:00:00+00:00",
                    "episodes": {
                        "ep_1": {
                            "episode_status": "keep",
                            "note": "validated in UI",
                            "updated_at": "2026-03-11T00:00:00+00:00",
                        }
                    },
                    "intervals": {
                        "int_1": {
                            "decision": "reject",
                            "note": "too broad",
                            "trim_start_s": 1.5,
                            "trim_end_s": 9.5,
                            "updated_at": "2026-03-11T00:00:00+00:00",
                        }
                    },
                    "labels": {
                        "int_1": {
                            "proposed_task_name": "stack cup",
                            "short_caption": "cup placed on another cup",
                            "primary_objects": ["cup", "table"],
                            "reason": "manual correction",
                            "updated_at": "2026-03-11T00:00:00+00:00",
                        }
                    },
                    "buckets": {
                        "renames": {
                            "bucket_custom": {
                                "canonical_task_name": "stack cup",
                                "updated_at": "2026-03-11T00:00:00+00:00",
                            }
                        },
                        "interval_assignments": {
                            "int_1": {
                                "bucket_id": "bucket_custom",
                                "canonical_task_name": "stack cup",
                                "updated_at": "2026-03-11T00:00:00+00:00",
                            }
                        },
                    },
                },
            )

            ego_curate.cmd_materialize(SimpleNamespace(workspace=str(root)))

            effective_episodes = ego_curate.read_jsonl(paths.effective_episodes)
            effective_intervals = ego_curate.read_jsonl(paths.effective_intervals)
            effective_labels = ego_curate.read_jsonl(paths.effective_labels)
            effective_bucket_map = ego_curate.read_json(paths.effective_bucket_map, {})

        self.assertEqual(effective_episodes[0]["effective_episode_status"], "keep")
        self.assertEqual(effective_intervals[0]["effective_interval_decision"], "reject")
        self.assertEqual(effective_intervals[0]["effective_start_s"], 1.5)
        self.assertEqual(effective_intervals[0]["effective_end_s"], 9.5)
        self.assertEqual(effective_intervals[0]["effective_duration_s"], 8.0)
        self.assertEqual(effective_labels[0]["proposed_task_name"], "stack cup")
        self.assertEqual(effective_labels[0]["normalized_task_name"], "stack cup")
        self.assertEqual(effective_bucket_map["mapping"]["int_1"], "bucket_custom")
        self.assertEqual(effective_bucket_map["buckets"][0]["canonical_task_name"], "stack cup")

    def test_log_progress_outputs_parseable_json(self) -> None:
        import io
        import json

        buf = io.StringIO()
        with mock.patch("sys.stdout", buf):
            ego_curate.log_progress("qc", 5, 100, "source/file.egorec")

        line = buf.getvalue().strip()
        self.assertTrue(line.startswith("PROGRESS: "))
        payload = json.loads(line[len("PROGRESS: "):])
        self.assertEqual(payload["stage"], "qc")
        self.assertEqual(payload["current"], 5)
        self.assertEqual(payload["total"], 100)
        self.assertEqual(payload["item"], "source/file.egorec")

    def test_log_progress_omits_item_when_empty(self) -> None:
        import io
        import json

        buf = io.StringIO()
        with mock.patch("sys.stdout", buf):
            ego_curate.log_progress("stage", 1, 10)

        payload = json.loads(buf.getvalue().strip()[len("PROGRESS: "):])
        self.assertNotIn("item", payload)

    def test_intervals_skip_existing_preserves_old_rows(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            paths = ego_curate.WorkspacePaths.from_root(root)

            existing_interval = {
                "interval_id": "int_existing",
                "source_key": "InHouse2/existing.egorec",
                "start_s": 0.0,
                "end_s": 5.0,
                "duration_s": 5.0,
                "proposal_source": "hand_activity_v1",
            }
            ego_curate.write_jsonl(paths.intervals, [existing_interval])
            ego_curate.write_jsonl(
                paths.episodes,
                [
                    {
                        "episode_id": "ep_existing",
                        "source_key": "InHouse2/existing.egorec",
                        "local_path": "/tmp/nonexistent.egorec",
                        "episode_status": "keep",
                        "duration_s": 10.0,
                    }
                ],
            )

            args = SimpleNamespace(
                workspace=str(root),
                ego_qc="ego-qc",
                full_episode_seconds=120.0,
                min_gap_seconds=1.0,
                min_duration_seconds=1.5,
                pad_seconds=0.5,
                hand_sample_fps=5.0,
                max_segment_seconds=20.0,
                vlm_max_window_seconds=180.0,
                vlm_overlap_seconds=15.0,
                skip_existing=True,
            )

            # Mock load_activity_proposals + detect_hand_presence_samples
            # to ensure the skipped episode is not processed
            call_count = {"activity": 0, "hand": 0}
            def fake_load_activity_proposals(*a, **kw):
                call_count["activity"] += 1
                return []
            def fake_detect_hand(*a, **kw):
                call_count["hand"] += 1
                return []

            with mock.patch.object(ego_curate, "load_activity_proposals", fake_load_activity_proposals):
                with mock.patch.object(ego_curate, "detect_hand_presence_samples", fake_detect_hand):
                    ego_curate.cmd_intervals(args)

            # Existing episode was skipped
            self.assertEqual(call_count["activity"], 0)
            self.assertEqual(call_count["hand"], 0)

            # Existing interval is preserved
            result = ego_curate.read_jsonl(paths.intervals)
            self.assertEqual(len(result), 1)
            self.assertEqual(result[0]["interval_id"], "int_existing")

    def test_filter_active_intervals_and_labels_skip_rejected_review_state(self) -> None:
        episodes = [
            {
                "episode_id": "ep_keep",
                "source_key": "InHouse2/keep.egorec",
                "effective_episode_status": "keep",
            },
            {
                "episode_id": "ep_reject",
                "source_key": "InHouse2/reject.egorec",
                "effective_episode_status": "reject",
            },
        ]
        intervals = [
            {
                "interval_id": "int_keep",
                "source_key": "InHouse2/keep.egorec",
                "effective_interval_decision": "keep",
            },
            {
                "interval_id": "int_rejected_interval",
                "source_key": "InHouse2/keep.egorec",
                "effective_interval_decision": "reject",
            },
            {
                "interval_id": "int_rejected_episode",
                "source_key": "InHouse2/reject.egorec",
                "effective_interval_decision": "keep",
            },
        ]
        labels = [
            {"interval_id": "int_keep", "proposed_task_name": "pick up cup"},
            {"interval_id": "int_rejected_interval", "proposed_task_name": "put down cup"},
            {"interval_id": "int_rejected_episode", "proposed_task_name": "move bowl"},
        ]

        active_intervals = ego_curate.filter_active_intervals(intervals, episodes)
        active_labels = ego_curate.filter_active_labels(labels, intervals, episodes)

        self.assertEqual([row["interval_id"] for row in active_intervals], ["int_keep"])
        self.assertEqual([row["interval_id"] for row in active_labels], ["int_keep"])


if __name__ == "__main__":
    unittest.main()
