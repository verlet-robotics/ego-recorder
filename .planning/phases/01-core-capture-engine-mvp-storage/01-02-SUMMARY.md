---
phase: 01-core-capture-engine-mvp-storage
plan: 02
subsystem: storage
tags: [binary-format, file-writer, signal-handler, sigwait, atomic, stats, c++17, posix, threading]

# Dependency graph
requires:
  - phase: 01-01
    provides: CapturedFrame/IMUSample structs, compression wrappers, BoundedQueue, CMake build system
provides:
  - binary_format.h: complete .egorec wire format (FileHeader, FrameBlockHeader, IMUSampleWire, IndexEntry, FileFooter) with static_assert size checks
  - FileWriter class: sequential frame appender with in-memory index, finalize writes index table + footer
  - setup_signal_handling(): POSIX sigwait pattern for clean multi-threaded shutdown
  - Stats class: lock-free atomic counters for frame/byte tracking with formatted summary()
affects: [01-03, 01-04]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "POSIX sigwait pattern: block signals in main thread before spawning workers, dedicated sigwait thread handles them synchronously"
    - "In-memory index accumulation: 24 bytes/frame appended-on-close (2.5MB/hour at 30fps -- negligible)"
    - "Best-effort destructor finalize: attempts finalize() on destruction to recover partial files on crash/exception"
    - "Relaxed atomic ordering for stats counters: correctness does not require sequentially consistent reads in reporting"

key-files:
  created:
    - src/storage/binary_format.h
    - src/storage/file_writer.h
    - src/utils/signal_handler.h
  modified:
    - src/storage/file_writer.cpp
    - src/utils/signal_handler.cpp
    - src/utils/stats.cpp

key-decisions:
  - "FileFooter is 36 bytes (4+8+4+8+8+4), not 32 -- static_assert verified"
  - "FRAME_MAGIC=0x46524D45, INDEX_MAGIC=0x58444E49, FOOTER_MAGIC=0x454E4F44 as specified in format spec"
  - "FileWriter uses 256KB write buffer via rdbuf()->pubsetbuf() to reduce syscall frequency"
  - "Signal handler captures shutdown_flag by reference in detached thread -- safe because flag is owned by main() and outlives thread"
  - "Stats counters use memory_order_relaxed: no happens-before relationship required for counters read by monitoring/display code"

patterns-established:
  - "raw_write() helper checks file_.good() after every write -- log error but never throw from writer thread"
  - "finalized_ flag set before write operations in finalize() so destructor re-entry is idempotent"
  - "static_assert on every packed struct size catches packing issues at compile time"

# Metrics
duration: ~4min
completed: 2026-02-19
---

# Phase 1 Plan 02: Binary Format + File Writer + Signal Handler + Stats Summary

**Custom .egorec binary container (magic bytes, packed structs, index table, footer) with POSIX sigwait-based clean shutdown and lock-free atomic stats tracking.**

## Performance

- **Duration:** ~4 min
- **Started:** 2026-02-19T00:05:59Z
- **Completed:** 2026-02-19T00:09:20Z
- **Tasks:** 2/2
- **Files modified:** 6

## Accomplishments

- binary_format.h defines all .egorec wire-format types with #pragma pack(push,1), static_assert size validation (IMUSampleWire=32B, IndexEntry=24B, FrameBlockHeader=36B, FileFooter=36B)
- FileWriter produces valid .egorec files verified by hex inspection: EGOREC magic at offset 0, FRME magic at first frame block, INDX + DONE magic in footer; in-memory index enables seekable random access
- Signal handler uses POSIX sigwait pattern (not signal/sigaction): pthread_sigmask blocks SIGTERM/SIGINT before any threads, detached sigwait thread sets atomic shutdown_flag
- Stats tracker provides lock-free uint64_t atomics for capture/write/drop/bytes counters with elapsed_seconds(), capture_fps(), write_fps(), and formatted summary() string

## Task Commits

Each task was committed atomically:

1. **Task 1: Implement binary format definitions and file writer** - `c6d20f8` (feat)
2. **Task 2: Implement signal handler and stats tracker** - `651a086` (feat)

**Plan metadata:** (see below - docs commit)

## Files Created/Modified

- `src/storage/binary_format.h` - Wire format constants (FILE_MAGIC, FRAME_MAGIC, INDEX_MAGIC, FOOTER_MAGIC) and packed structs (FileHeader ~472B, FrameBlockHeader 36B, IMUSampleWire 32B, IndexEntry 24B, FileFooter 36B) with static_assert size checks
- `src/storage/file_writer.h` - FileWriter class interface: constructor(filepath), write_header(), write_frame(), finalize(), is_finalized()
- `src/storage/file_writer.cpp` - Implementation: 256KB write buffer, per-frame IndexEntry accumulation, atomic finalize with index table + footer write; raw_write() checks file_.good() after each write; destructor best-effort finalize
- `src/utils/signal_handler.h` - setup_signal_handling(atomic<bool>&) declaration with usage docs
- `src/utils/signal_handler.cpp` - pthread_sigmask(SIG_BLOCK) + detached std::thread running sigwait(); sets shutdown_flag with memory_order_release
- `src/utils/stats.h/.cpp` - Stats class with atomic<uint64_t> counters; frame_captured/written/dropped, bytes_written mutators; captured/written/dropped/total_bytes/elapsed_seconds/capture_fps/write_fps accessors; formatted summary() string

## Decisions Made

- **FileFooter size 36B not 32B:** Manual calculation: 4+8+4+8+8+4 = 36 bytes. Static assert catches any future divergence.
- **FRAME_MAGIC as specified (0x46524D45):** Used exact values from format spec. On little-endian x86, these values are stored byte-swapped in the file -- consistent within the format.
- **256KB write buffer:** Reduces syscall frequency on sequential writes without risk of losing data (flush called in finalize before close).
- **Detached sigwait thread captures flag by reference:** The shutdown_flag is declared in main() and outlives the detached thread (process exits when main returns). Safe pattern.
- **memory_order_relaxed for stats counters:** Stats are read by display/monitoring code that does not synchronize with writers. Relaxed ordering is sufficient; stale reads of counters are acceptable within a display interval.

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required. Build with same cmake flags as plan 01.

## Next Phase Readiness

- Plan 03 (capture pipeline) can `#include "storage/file_writer.h"` and use FileWriter directly
- Plan 04 (main orchestration) can `#include "utils/signal_handler.h"` and `#include "utils/stats.h"`
- All storage types defined in binary_format.h are ready for use by any module
- Build continues to work with existing cmake invocation (no new dependencies added)

## Self-Check: PASSED

- All 7 source files exist on disk
- Both task commits verified: c6d20f8, 651a086
- Build succeeds with all new files compiled and linked

---
*Phase: 01-core-capture-engine-mvp-storage*
*Completed: 2026-02-19*
