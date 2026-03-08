#!/usr/bin/env python3
"""Export .egorec v2 recordings to RLDS TFRecord format.

Usage:
    python export_rlds.py recording.egorec [recording2.egorec ...]
    python export_rlds.py --output /path/to/output recording.egorec
    python export_rlds.py --quiet recording.egorec

One .egorec file = one RLDS episode. Multiple files create a multi-episode dataset.

Depth values are raw uint16 millimeters (D435 Z16 values preserved exactly).
To convert: depth_meters = depth_mm / 1000.0
"""

import argparse
import os
import sys
import time
import numpy as np
from pathlib import Path

# egorec_reader is built by CMake -- set PYTHONPATH to build dir
try:
    import egorec_reader
except ImportError:
    print("Error: egorec_reader module not found.", file=sys.stderr)
    print("Build with: cmake -B build -DWITH_PYTHON=ON && cmake --build build",
          file=sys.stderr)
    print("Then set: PYTHONPATH=build python export_rlds.py ...",
          file=sys.stderr)
    sys.exit(1)

import tensorflow_datasets as tfds
from tqdm import tqdm

# Module-level quiet flag (checked by _generate_examples)
_quiet = False


class EgoRecDataset(tfds.core.GeneratorBasedBuilder):
    """TFDS builder for .egorec v2 recordings in RLDS format."""

    VERSION = tfds.core.Version('1.0.0')
    RELEASE_NOTES = {'1.0.0': 'Initial release from ego-recorder v2.'}

    @classmethod
    def _get_pkg_dir_path(cls):
        return Path(__file__).parent

    def __init__(self, egorec_paths, dataset_name=None, **kwargs):
        self._egorec_paths = [Path(p) for p in egorec_paths]

        if dataset_name is None:
            reader = egorec_reader.EgorecFile(str(self._egorec_paths[0]))
            header = reader.header()
            dataset_name = header.get('session_name', 'ego_recording')

        self.name = dataset_name
        super().__init__(**kwargs)

    def _info(self):
        return self.dataset_info_from_configs(
            features=tfds.features.FeaturesDict({
                'steps': tfds.features.Dataset({
                    'observation': tfds.features.FeaturesDict({
                        'image': tfds.features.Image(
                            shape=(480, 640, 3), dtype=np.uint8,
                            encoding_format='jpeg'),
                        'depth': tfds.features.Image(
                            shape=(480, 640, 1), dtype=np.uint16,
                            encoding_format='png'),
                        'depth_intrinsics': tfds.features.Tensor(
                            shape=(4,), dtype=np.float32),
                        'color_intrinsics': tfds.features.Tensor(
                            shape=(4,), dtype=np.float32),
                        'extrinsic_R': tfds.features.Tensor(
                            shape=(3, 3), dtype=np.float32),
                        'extrinsic_t': tfds.features.Tensor(
                            shape=(3,), dtype=np.float32),
                    }),
                    'timestamp': tfds.features.Scalar(dtype=np.float64),
                    'is_first': tfds.features.Scalar(dtype=np.bool_),
                    'is_last': tfds.features.Scalar(dtype=np.bool_),
                    'is_terminal': tfds.features.Scalar(dtype=np.bool_),
                }),
                'episode_metadata': tfds.features.FeaturesDict({
                    'file_path': tfds.features.Text(),
                    'session_name': tfds.features.Text(),
                    'duration_s': tfds.features.Scalar(dtype=np.float64),
                }),
            }),
        )

    def _split_generators(self, dl_manager=None):
        return {
            'train': self._generate_examples(self._egorec_paths),
        }

    def _generate_examples(self, paths):
        global _quiet

        for episode_path in paths:
            reader = egorec_reader.EgorecFile(str(episode_path))
            header = reader.header()
            n_frames = reader.frame_count()

            depth_intrinsics = np.array([
                header['depth_fx'], header['depth_fy'],
                header['depth_ppx'], header['depth_ppy']
            ], dtype=np.float32)

            color_intrinsics = np.array([
                header['color_fx'], header['color_fy'],
                header['color_ppx'], header['color_ppy']
            ], dtype=np.float32)

            extrinsic_R = np.array(
                header['extrinsic_R'], dtype=np.float32
            ).reshape(3, 3)

            extrinsic_t = np.array(
                header['extrinsic_t'], dtype=np.float32
            )

            start_ts_us = header['start_ts_us']
            duration_s = header.get('duration_s', 0.0)
            session_name = header.get('session_name', '')

            steps = []
            frame_iter = reader.frames()

            # Progress bar with MB/s throughput (per locked decision)
            if not _quiet:
                frame_iter = tqdm(frame_iter, total=n_frames,
                                  unit='frame',
                                  desc=str(episode_path.name))

            bytes_processed = 0
            t_start = time.monotonic()

            for i, frame in enumerate(frame_iter):
                depth_hwc = frame['depth'][:, :, np.newaxis]

                # Track bytes for MB/s throughput display
                frame_bytes = frame['rgb'].nbytes + frame['depth'].nbytes
                bytes_processed += frame_bytes
                if not _quiet and hasattr(frame_iter, 'set_postfix'):
                    elapsed = time.monotonic() - t_start
                    if elapsed > 0:
                        mb_per_s = (bytes_processed / 1e6) / elapsed
                        frame_iter.set_postfix(
                            {'MB/s': f'{mb_per_s:.1f}'}, refresh=False)

                steps.append({
                    'observation': {
                        'image': frame['rgb'],
                        'depth': depth_hwc,
                        'depth_intrinsics': depth_intrinsics,
                        'color_intrinsics': color_intrinsics,
                        'extrinsic_R': extrinsic_R,
                        'extrinsic_t': extrinsic_t,
                    },
                    'timestamp': frame['timestamp_relative_s'],
                    'is_first': i == 0,
                    'is_last': i == n_frames - 1,
                    'is_terminal': i == n_frames - 1,
                })

            yield str(episode_path), {
                'steps': steps,
                'episode_metadata': {
                    'file_path': str(episode_path),
                    'session_name': session_name,
                    'duration_s': duration_s,
                },
            }


def main():
    global _quiet

    parser = argparse.ArgumentParser(
        description='Export .egorec v2 files to RLDS TFRecord format')
    parser.add_argument('files', nargs='+', help='.egorec v2 file paths')
    parser.add_argument('--output', '-o', default=None,
                        help='Output directory (default: same dir as first input file)')
    parser.add_argument('--name', default=None,
                        help='Dataset name (default: session name from first file)')
    parser.add_argument('--quiet', '-q', action='store_true',
                        help='Suppress progress output')
    args = parser.parse_args()

    _quiet = args.quiet

    for f in args.files:
        if not os.path.exists(f):
            print(f"Error: file not found: {f}", file=sys.stderr)
            sys.exit(1)

    if args.output is None:
        first_file = Path(args.files[0])
        args.output = str(first_file.parent / (first_file.stem + '_rlds'))

    os.makedirs(args.output, exist_ok=True)

    if not _quiet:
        print(f"Exporting {len(args.files)} file(s) to RLDS format")
        print(f"Output: {args.output}")

    builder = EgoRecDataset(
        egorec_paths=args.files,
        dataset_name=args.name,
        data_dir=args.output,
    )

    builder.download_and_prepare()

    if not _quiet:
        print(f"\nExport complete: {args.output}")
        print(f"Load with: tfds.load('{builder.name}', data_dir='{args.output}')")


if __name__ == '__main__':
    main()
