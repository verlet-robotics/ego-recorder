#!/usr/bin/env python3
"""Export .egorec v2 recordings to LeRobot v3 dataset format.

Usage:
    python export_lerobot.py recording.egorec [recording2.egorec ...]
    python export_lerobot.py --output /path/to/output recording.egorec
    python export_lerobot.py --separate recording1.egorec recording2.egorec
    python export_lerobot.py --quiet recording.egorec

Multiple recordings merge into one dataset by default. Use --separate to
create individual datasets per recording.

RGB is exported as video (LeRobot handles MP4 encoding internally).
To control the video codec and CRF, use LeRobot's configuration options
(e.g., lerobot-edit-dataset --operation.vcodec h264 --operation.crf 23).

Depth is stored as float32 mm (original D435 Z16 values preserved exactly).

# To push to HuggingFace Hub after export:
#   from huggingface_hub import HfApi
#   api = HfApi()
#   api.upload_folder(folder_path=output_dir, repo_id="your-org/dataset-name",
#                     repo_type="dataset")
"""

import argparse
import os
import sys
import time
import numpy as np
from pathlib import Path

try:
    import egorec_reader
except ImportError:
    print("Error: egorec_reader module not found.", file=sys.stderr)
    print("Build with: cmake -B build -DWITH_PYTHON=ON && cmake --build build", file=sys.stderr)
    print("Then set: PYTHONPATH=build python export_lerobot.py ...", file=sys.stderr)
    sys.exit(1)

from tqdm import tqdm

try:
    from lerobot.common.datasets.lerobot_dataset import LeRobotDataset
except ImportError:
    try:
        from lerobot.datasets.lerobot_dataset import LeRobotDataset
    except ImportError:
        print("Error: LeRobotDataset not found in lerobot package.", file=sys.stderr)
        print("Install lerobot v3: pip install 'git+https://github.com/huggingface/lerobot.git'",
              file=sys.stderr)
        sys.exit(1)


def export_single_dataset(egorec_paths, output_dir, repo_id, quiet=False):
    """Export one or more .egorec files as episodes in a single LeRobot v3 dataset."""

    features = {
        "observation.images.rgb": {
            "dtype": "video",
            "shape": (480, 640, 3),
            "names": ["height", "width", "channel"],
        },
        "observation.depth_mm": {
            "dtype": "float32",
            "shape": (480, 640),
            "names": ["height", "width"],
        },
        "timestamp": {
            "dtype": "float64",
            "shape": (1,),
            "names": None,
        },
        "task_index": {
            "dtype": "int64",
            "shape": (1,),
            "names": None,
        },
    }

    # Handle API differences between lerobot versions
    try:
        dataset = LeRobotDataset.create(
            repo_id=repo_id, fps=30, features=features,
            robot_type="realsense_d435", root=output_dir)
    except TypeError as e:
        if 'root' in str(e):
            dataset = LeRobotDataset.create(
                repo_id=repo_id, fps=30, features=features,
                robot_type="realsense_d435", local_dir=output_dir)
        else:
            raise

    for path in egorec_paths:
        reader = egorec_reader.EgorecFile(str(path))
        header = reader.header()
        n_frames = reader.frame_count()

        frame_iter = reader.frames()

        # Progress bar with MB/s throughput (per locked decision)
        if not quiet:
            frame_iter = tqdm(frame_iter, total=n_frames,
                              unit='frame',
                              desc=Path(path).name)

        bytes_processed = 0
        t_start = time.monotonic()

        for frame in frame_iter:
            dataset.add_frame({
                "observation.images.rgb": frame["rgb"],
                "observation.depth_mm": frame["depth"].astype(np.float32),
                "timestamp": np.array([frame["timestamp_relative_s"]]),
                "task_index": np.array([0], dtype=np.int64),
            })

            # Track bytes for MB/s throughput display
            frame_bytes = frame['rgb'].nbytes + frame['depth'].nbytes
            bytes_processed += frame_bytes
            if not quiet and hasattr(frame_iter, 'set_postfix'):
                elapsed = time.monotonic() - t_start
                if elapsed > 0:
                    mb_per_s = (bytes_processed / 1e6) / elapsed
                    frame_iter.set_postfix(
                        {'MB/s': f'{mb_per_s:.1f}'}, refresh=False)

        dataset.save_episode()

    # CRITICAL: finalize() MUST be called to write Parquet footers
    # Without this, the dataset files are corrupt
    dataset.finalize()

    if not quiet:
        print(f"\nDataset created: {output_dir}")
        print(f"Episodes: {len(egorec_paths)}")


def main():
    parser = argparse.ArgumentParser(
        description='Export .egorec v2 files to LeRobot v3 dataset format')
    parser.add_argument('files', nargs='+', help='.egorec v2 file paths')
    parser.add_argument('--output', '-o', default=None,
                        help='Output directory (default: same dir as first input file)')
    parser.add_argument('--name', default=None,
                        help='Dataset name / repo_id (default: session name from file)')
    parser.add_argument('--separate', action='store_true',
                        help='Create separate dataset per recording (default: merge into one)')
    parser.add_argument('--quiet', '-q', action='store_true',
                        help='Suppress progress output')
    args = parser.parse_args()

    for f in args.files:
        if not os.path.exists(f):
            print(f"Error: file not found: {f}", file=sys.stderr)
            sys.exit(1)

    if args.separate:
        for f in args.files:
            reader = egorec_reader.EgorecFile(f)
            header = reader.header()
            name = args.name or header.get('session_name', Path(f).stem)
            repo_id = f"local/{name}"

            if args.output:
                out_dir = os.path.join(args.output, name)
            else:
                out_dir = str(Path(f).parent / (Path(f).stem + '_lerobot'))

            if not args.quiet:
                print(f"Exporting {f} -> {out_dir}")

            export_single_dataset([f], out_dir, repo_id, quiet=args.quiet)
    else:
        first_reader = egorec_reader.EgorecFile(args.files[0])
        first_header = first_reader.header()
        name = args.name or first_header.get('session_name', 'ego_recording')
        repo_id = f"local/{name}"

        if args.output:
            out_dir = args.output
        else:
            out_dir = str(Path(args.files[0]).parent / (name + '_lerobot'))

        if not args.quiet:
            print(f"Exporting {len(args.files)} file(s) to LeRobot v3 format")
            print(f"Output: {out_dir}")

        export_single_dataset(args.files, out_dir, repo_id, quiet=args.quiet)

    if not args.quiet:
        print("\nDone.")


if __name__ == '__main__':
    main()
