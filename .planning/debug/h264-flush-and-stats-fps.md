---
status: diagnosed
trigger: "Two unit tests failing: H264EncoderTest.FlushDrainsBufferedFrames and Stats.CaptureFpsCalculation"
created: 2026-03-08T00:00:00Z
updated: 2026-03-08T00:10:00Z
---

## Current Focus

hypothesis: Both root causes confirmed with evidence
test: N/A - diagnosis complete
expecting: N/A
next_action: Report findings

## Symptoms

expected: Both tests should pass
actual: FlushDrainsBufferedFrames fails 100%. CaptureFpsCalculation fails intermittently (~2% of runs).
errors: |
  test_compression.cpp:263 - Expected (fsize) > (0u), actual: 0 vs 0
  test_stats.cpp:98 - Expected (fps) > (0.0), actual: 0 vs 0
reproduction: Run tests individually or in suite. H264 always fails. Stats fails ~1/50 runs.
started: Since zerolatency tune was added to H264Encoder

## Eliminated

## Evidence

- timestamp: 2026-03-08T00:01:00Z
  checked: Stats.CaptureFpsCalculation in isolation (5 runs)
  found: Passes all 5 times
  implication: Not a consistent failure, may be intermittent

- timestamp: 2026-03-08T00:02:00Z
  checked: H264EncoderTest.FlushDrainsBufferedFrames (3 runs)
  found: Fails 100% of runs, always fsize==0
  implication: Deterministic failure in flush()

- timestamp: 2026-03-08T00:03:00Z
  checked: Full test suite
  found: Only H264EncoderTest.FlushDrainsBufferedFrames fails, Stats passes
  implication: No test-interaction issue

- timestamp: 2026-03-08T00:04:00Z
  checked: Per-frame encode() output with zerolatency
  found: encode(0)=5676, encode(1)=930, encode(2)=1761, encode(3)=1480, encode(4)=1734, flush()=0
  implication: zerolatency makes x264 emit every frame immediately. flush() has nothing to drain. Test assumption is wrong.

- timestamp: 2026-03-08T00:05:00Z
  checked: Stats.CaptureFpsCalculation over 50 runs
  found: Failed 1/50 times (intermittent ~2% rate)
  implication: Race condition between construction time and fps calculation

- timestamp: 2026-03-08T00:06:00Z
  checked: capture_fps() implementation (stats.cpp:72-76)
  found: Returns 0.0 when elapsed_seconds() < 1e-6. Test creates Stats, does 100 atomic increments, then calls capture_fps(). On fast CPUs, this can complete in <1 microsecond.
  implication: The guard clause (elapsed < 1e-6) triggers when the test runs too fast, returning 0.0 instead of a positive value.

## Resolution

root_cause: |
  BUG 1 (H264EncoderTest.FlushDrainsBufferedFrames):
  The test expects flush() to return >0 bytes after encoding 5 frames. However, the encoder
  is configured with tune=zerolatency (h264_encoder.cpp:56), which disables x264's internal
  lookahead buffer. This means every encode() call produces output immediately (no buffering).
  When flush() is called, there are zero buffered frames to drain, so it correctly returns 0.
  The test's assumption that flush() must produce data is wrong when zerolatency is enabled.

  BUG 2 (Stats.CaptureFpsCalculation):
  The test creates a Stats object and immediately calls frame_captured() 100 times, then
  checks capture_fps() > 0.0. The capture_fps() method (stats.cpp:72-76) has a guard:
  if elapsed_seconds() < 1e-6, return 0.0. On fast machines, the entire test (construction +
  100 atomic increments + fps call) completes in under 1 microsecond, hitting the guard and
  returning 0.0. This is an intermittent failure (~2%) dependent on CPU speed/scheduling.

fix: |
  BUG 1: Change the test to match the zerolatency encoder behavior. Instead of asserting
  flush() returns >0 bytes, assert that the total bytes from encode() + flush() is >0.
  Alternatively, check that flush() returns >=0 (does not error) since with zerolatency
  all output is produced during encode().

  BUG 2: Either (a) add a small sleep (e.g. 1ms) after constructing Stats so elapsed is
  never near-zero, or (b) change the test to check that captured() == 100 && elapsed > 0
  instead of checking fps directly, or (c) change capture_fps() to use a smaller epsilon
  guard or return frames/elapsed when elapsed > 0 with no guard.

verification:
files_changed: []
