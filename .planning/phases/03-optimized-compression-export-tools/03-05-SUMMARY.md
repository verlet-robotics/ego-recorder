---
phase: 03-optimized-compression-export-tools
plan: 05
subsystem: export-lerobot
tags: [lerobot, huggingface, python, export, dataset, video, depth, tqdm]

# Dependency graph
requires:
  - phase: 03-optimized-compression-export-tools
    plan: 03
    provides: "egorec_reader.so Python module with EgorecFile, header(), frame_count(), frames()"
provides:
  - "python/export_lerobot.py CLI tool for .egorec v2 to LeRobot v3 dataset conversion"
  - "RGB exported as dtype video (LeRobot handles MP4 encoding internally)"
  - "Depth exported as float32 mm (exact D435 Z16 values preserved)"
  - "Multi-recording merge (default) and --separate per-file split modes"
  - "Progress bar with frame count and MB/s throughput via tqdm"
  - "dataset.finalize() always called for valid Parquet output"
affects: [huggingface-hub-upload, dataset-export-workflow]

# Tech tracking
tech-stack:
  added: [lerobot-v3-sdk, tqdm]
  patterns: [lerobot-dataset-create-api, tqdm-set-postfix-throughput]

key-files:
  created:
    - python/export_lerobot.py
    - python/requirements-lerobot.txt

key-decisions:
  - "LeRobot API fallback: try root= then local_dir= for version compatibility"
  - "Depth as float32 mm: preserves exact D435 Z16 values, field name observation.depth_mm makes unit explicit"
  - "RGB as dtype video: LeRobot handles MP4 encoding internally, codec/CRF controlled by LeRobot config not this script"
  - "repo_id uses local/ prefix for local datasets, user renames for Hub upload"

patterns-established:
  - "LeRobot v3 create pattern: features dict with dtype/shape/names, then add_frame() per frame, save_episode() per recording, finalize() at end"
  - "tqdm throughput: set_postfix with MB/s calculated from cumulative bytes / elapsed time"
  - "egorec_reader import guard: ImportError with build instructions to stderr"

# Metrics
duration: 1min
completed: 2026-03-08
---

# Phase 3 Plan 5: LeRobot v3 Export Summary

**LeRobot v3 export CLI converting .egorec v2 recordings to HuggingFace-compatible datasets with video RGB, float32 depth, multi-file merge, and MB/s progress tracking**

## Performance

- **Duration:** 1 min
- **Started:** 2026-03-08T05:34:36Z
- **Completed:** 2026-03-08T05:35:53Z
- **Tasks:** 1/1
- **Files modified:** 2

## Accomplishments
- export_lerobot.py CLI converts .egorec v2 files to LeRobot v3 datasets via LeRobotDataset.create API
- RGB exported as dtype "video" (LeRobot handles MP4 encoding internally; codec/CRF via LeRobot config)
- Depth stored as float32 mm preserving exact D435 Z16 values (field named observation.depth_mm)
- Multiple recordings merge into one dataset by default; --separate creates individual datasets
- Progress bar with tqdm shows frame X/Y, ETA, and MB/s throughput via set_postfix
- dataset.finalize() always called (critical for valid Parquet files)
- CLI supports --output, --quiet, --separate, --name flags with helpful error messages
- Fallback import handles lerobot API differences between versions

## Task Commits

Each task was committed atomically:

1. **Task 1: LeRobot v3 export script with LeRobotDataset.create API and MB/s progress tracking** - `9c1c90e` (feat)

## Files Created/Modified
- `python/export_lerobot.py` - LeRobot v3 export CLI: reads .egorec v2 via egorec_reader, produces LeRobot datasets with video RGB and float32 depth
- `python/requirements-lerobot.txt` - Python dependencies: lerobot>=0.4.0, numpy>=1.24.0, tqdm>=4.60.0

## Decisions Made
- **LeRobot API fallback:** Try `root=` parameter first, then `local_dir=` on TypeError -- handles API differences between lerobot versions without requiring a specific version.
- **Depth as float32 mm:** Rather than converting to meters (dividing by 1000), store exact D435 Z16 values as float32. The field name `observation.depth_mm` makes the unit explicit.
- **RGB as dtype video:** LeRobot handles MP4 encoding internally for video features. No attempt to set CRF or codec directly -- those are controlled via lerobot-edit-dataset or LeRobot config.
- **repo_id local/ prefix:** Using `local/{name}` for local datasets. Users rename via `--name` or update repo_id for Hub upload.

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required. To use the export script:
1. Build egorec_reader.so: `cmake -B build -DWITH_PYTHON=ON && cmake --build build`
2. Install lerobot v3: `pip install "git+https://github.com/huggingface/lerobot.git"`
3. Run: `PYTHONPATH=build python python/export_lerobot.py recording.egorec`

## Next Phase Readiness
- LeRobot export is ready for end-to-end testing once a v2 .egorec recording is available
- Phase 03 is now complete: all 5 plans (compression wrappers, pipeline integration, Python reader, RLDS export, LeRobot export) are done
- E2E verification (actually converting a .egorec v2 file) is deferred to manual testing

## Self-Check: PASSED

All 2 created files verified present. Task commit 9c1c90e verified in git log. export_lerobot.py is 196 lines (exceeds min_lines: 120).

---
*Phase: 03-optimized-compression-export-tools*
*Completed: 2026-03-08*
