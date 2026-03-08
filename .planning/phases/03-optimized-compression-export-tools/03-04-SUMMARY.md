---
phase: 03-optimized-compression-export-tools
plan: 04
subsystem: export
tags: [rlds, tfrecord, tensorflow-datasets, python, ml-export, tqdm, numpy]

# Dependency graph
requires:
  - phase: 03-optimized-compression-export-tools
    plan: 03
    provides: "egorec_reader.so pybind11 module with EgorecFile, header(), frame_count(), frames() API"
provides:
  - "python/export_rlds.py -- RLDS TFRecord export CLI script"
  - "EgoRecDataset TFDS GeneratorBasedBuilder for .egorec v2 to RLDS conversion"
  - "1:1 egorec-to-episode mapping with multi-file batch support"
  - "uint16 mm depth lossless via PNG encoding, camera intrinsics/extrinsics per step"
  - "tqdm progress bar with MB/s throughput tracking"
  - "python/requirements-rlds.txt with tensorflow-datasets, numpy, tqdm"
affects: [03-05, export-lerobot, ml-training-pipeline]

# Tech tracking
tech-stack:
  added: [tensorflow-datasets, tqdm]
  patterns: [tfds-generator-based-builder, rlds-episode-schema, tqdm-throughput-postfix]

key-files:
  created:
    - python/export_rlds.py
    - python/requirements-rlds.txt

key-decisions:
  - "TFDS GeneratorBasedBuilder subclass for standard RLDS TFRecord generation pipeline"
  - "Depth as Image(shape=(480,640,1), dtype=uint16, encoding_format='png') for lossless preservation per locked decision"
  - "Per-step camera intrinsics/extrinsics as Tensor features for RLDS convention compatibility"
  - "tqdm set_postfix with MB/s throughput for real-time progress monitoring per locked decision"

patterns-established:
  - "RLDS episode schema: steps with observation (image, depth, intrinsics, extrinsics), timestamp, is_first/is_last/is_terminal + episode_metadata"
  - "egorec_reader integration pattern: EgorecFile -> header() for metadata, frames() iterator for decoded numpy arrays"
  - "Module-level _quiet flag for suppressing progress output across generator methods"

# Metrics
duration: 1min
completed: 2026-03-08
---

# Phase 3 Plan 4: RLDS TFRecord Export Summary

**RLDS TFRecord export script using TFDS GeneratorBasedBuilder, converting .egorec v2 files to ML-ready episodes with lossless uint16 depth, camera intrinsics/extrinsics, and tqdm MB/s throughput tracking**

## Performance

- **Duration:** 1 min
- **Started:** 2026-03-08T05:34:34Z
- **Completed:** 2026-03-08T05:35:57Z
- **Tasks:** 1/1
- **Files modified:** 2

## Accomplishments
- RLDS TFRecord export script with TFDS GeneratorBasedBuilder that reads .egorec v2 files via egorec_reader
- 1:1 egorec-to-episode mapping; multiple input files create a multi-episode dataset
- Depth preserved as raw uint16 mm values with PNG encoding (lossless per locked decision)
- Camera depth/color intrinsics (fx, fy, ppx, ppy) and extrinsics (R 3x3, t 3-vec) included per step
- Relative timestamps from egorec_reader's timestamp_relative_s field
- tqdm progress bar with frame count and MB/s throughput via set_postfix
- CLI supports --output, --quiet, --name flags per locked decisions

## Task Commits

Each task was committed atomically:

1. **Task 1: RLDS TFRecord export script with TFDS GeneratorBasedBuilder and MB/s progress tracking** - `1df3420` (feat)

## Files Created/Modified
- `python/export_rlds.py` - RLDS TFRecord export CLI: EgoRecDataset TFDS builder, argparse CLI, tqdm progress with MB/s
- `python/requirements-rlds.txt` - Python dependencies: tensorflow-datasets>=4.9.0, numpy>=1.24.0, tqdm>=4.60.0

## Decisions Made
- **TFDS GeneratorBasedBuilder:** Standard TFDS pipeline handles serialization, sharding, and metadata. No custom TFRecord writing needed.
- **Depth as Image with PNG encoding:** TFDS Image feature with encoding_format='png' preserves uint16 values losslessly. Channel axis added via np.newaxis for shape (H,W,1) compatibility.
- **Per-step intrinsics/extrinsics:** Included as Tensor features in every step observation for RLDS convention compatibility (consumers expect self-contained steps).
- **Module-level _quiet flag:** Global flag checked by _generate_examples since TFDS calls it internally -- no way to pass builder constructor args through to the generator.

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required. Users need tensorflow-datasets installed (`pip install -r python/requirements-rlds.txt`) and PYTHONPATH set to the build directory containing egorec_reader.so.

## Next Phase Readiness
- RLDS export script ready for end-to-end testing once a v2 .egorec recording is made with the complete compression pipeline
- Plan 03-05 (LeRobot export) can follow the same egorec_reader integration pattern established here
- No blockers for next plan

## Self-Check: PASSED

All 2 created files verified present. Task commit 1df3420 verified in git log.

---
*Phase: 03-optimized-compression-export-tools*
*Completed: 2026-03-08*
