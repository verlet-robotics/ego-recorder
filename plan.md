# Post-Processing Pipeline: EgoScale/EgoDex-Compatible Export

## Context

The EgoScale paper (NVIDIA, 2026) trains a VLA model on egocentric human video with paired 3D hand pose annotations. Its primary data source is the EgoDex dataset (Apple): paired MP4 + HDF5 files containing 1080p 30fps video with SE(3) transforms for 60+ skeletal joints per frame.

Our recorder captures RGB-D at 640x480 30fps but has **no hand tracking, no ego-motion, no action segmentation, and no EgoDex export**. This plan adds a 4-stage post-processing pipeline that takes `.egorec` files and produces EgoDex-compatible output, leveraging our depth advantage (EgoDex doesn't have depth) to get more accurate 3D hand poses than monocular approaches.

---

## Pipeline Overview

```
.egorec
  |
  +--> [Stage 1] Hand Tracking    --> .hands.npz
  +--> [Stage 2] Camera Ego-Motion --> .trajectory.npz
  |         (both feed into)
  +--> [Stage 3] Action Segmentation --> .segments.json
  |
  +--> [Stage 4] EgoDex Export --> task_name/{0..N}.mp4 + {0..N}.hdf5
```

Each stage writes a **sidecar file** next to the source `.egorec`. Stages can be re-run independently. Final export merges all sidecars into EgoDex format.

---

## Stage 1: Hand Pose Estimation

**File:** `python/pipeline/hand_tracker.py`
**Output:** `{basename}.hands.npz`
**Deps:** `mediapipe>=0.10.0`, `opencv-python>=4.8.0`, `scipy`

### Algorithm
1. Run MediaPipe Hands on each RGB frame (max 2 hands, `min_detection_confidence=0.5`, `min_tracking_confidence=0.3`)
2. For each detected hand, get 21 2D landmarks
3. **Depth-lift to 3D**: For each landmark at pixel (u, v):
   - Read depth with 5x5 median filter around (u, v) to handle edge noise
   - Deproject using **color intrinsics** (depth is aligned to color): `X = (u - ppx) * Z * depth_scale / fx`, etc.
   - If depth=0: interpolate from adjacent frames (up to 10-frame gap), else confidence=0
4. **Build SE(3) per joint**:
   - Wrist transform: position from landmark 0, orientation from wrist/index-MCP/pinky-MCP triangle
   - Fingertip/joint transforms: position from landmark, orientation from parent bone direction
5. Confidence = MediaPipe visibility * depth validity (1.0 if valid depth, 0.0 if interpolated/missing)

### Output schema
```python
{
    "left_landmarks_3d":  (N, 21, 3) float32,   # camera-frame meters
    "right_landmarks_3d": (N, 21, 3) float32,
    "left_confidence":    (N, 21) float32,
    "right_confidence":   (N, 21) float32,
    "left_wrist_se3":     (N, 4, 4) float64,
    "right_wrist_se3":    (N, 4, 4) float64,
    "left_detected":      (N,) bool,
    "right_detected":     (N,) bool,
    "timestamps_s":       (N,) float64,
}
```

---

## Stage 2: Camera Ego-Motion (RGB-D SLAM)

**File:** `python/pipeline/ego_motion.py`
**Output:** `{basename}.trajectory.npz`
**Deps:** `open3d>=0.17.0`

### Algorithm
1. For each consecutive frame pair, run Open3D `compute_rgbd_odometry()` with hybrid (intensity + point-to-plane) objective
2. Build camera intrinsic matrix from header: `[[fx, 0, ppx], [0, fy, ppy], [0, 0, 1]]`
3. Chain relative transforms: `T_t = T_{t-1} @ delta_T`
4. Store per-frame fitness score as quality metric
5. If fitness < 0.3 on a frame: use identity delta (assume no motion), flag quality=0

### Output schema
```python
{
    "camera_poses": (N, 4, 4) float64,  # camera-to-world SE(3)
    "timestamps_s": (N,) float64,
    "quality":      (N,) float32,        # 0-1 per-frame confidence
    "method":       str,                 # "open3d_rgbd"
}
```

Open3D is pip-installable, no CUDA needed. For tabletop manipulation with a relatively stable head-mounted camera, this is sufficient. Drift is manageable because exported segments are short (<30s).

---

## Stage 3: Action Segmentation

**File:** `python/pipeline/segmenter.py`
**Output:** `{basename}.segments.json`
**Deps:** `scipy` (Gaussian smoothing)

### Signals (ordered by reliability)
1. **Hand velocity** (primary): 3D wrist velocity from Stage 1. Active manipulation = velocity > `30 mm/s`
2. **Hand presence** (secondary): No hands detected for >1s = transition/idle
3. **Pinch detection** (tertiary): Thumb-tip to index-tip distance. Pinch (<30mm) followed by release (>60mm) = grasp/release boundary within long active intervals
4. **Camera motion** (quaternary): Rotation >30 deg or translation >0.5m in 1s window = repositioning, not manipulation

### Algorithm
1. Compute all signals from Stage 1 + Stage 2 outputs
2. Gaussian smooth (sigma=5 frames / ~167ms)
3. Detect "active" intervals where hand velocity > threshold AND hands detected
4. Merge intervals separated by <0.5s (brief pauses mid-task)
5. Split intervals >10s at grasp/release events (likely multi-action)
6. Discard segments <1s (noise)
7. Pad each segment by 0.5s on both sides for context

### Optional: LLM annotation (`--annotate`)
For each segment, extract the middle frame and send to a vision-language model with prompt: "Describe the manipulation action in this ego-centric image in one sentence." Stored as `llm_description` attribute. Requires `--annotate --api-key KEY`.

### Output schema
```json
{
  "segments": [
    {
      "id": 0,
      "start_frame": 45, "end_frame": 120,
      "start_time_s": 1.5, "end_time_s": 4.0,
      "description": "",
      "confidence": 0.85,
      "signals": { "mean_hand_velocity_mm_s": 145.3, "grasp_events": 1 }
    }
  ],
  "metadata": {
    "total_frames": 900, "active_ratio": 0.65,
    "method": "velocity_threshold", "threshold_mm_per_s": 30.0
  }
}
```

---

## Stage 4: EgoDex Export

**File:** `python/pipeline/egodex_exporter.py`
**Output:** `{output_dir}/{task_name}/{idx}.mp4` + `{idx}.hdf5`
**Deps:** `h5py`, `opencv-python`

### Per segment:

**MP4**: Re-encode RGB frames for the segment range via OpenCV VideoWriter (H.264, native resolution)

**HDF5** (matching EgoDex schema):
```
camera/intrinsic              (3, 3) float64     # from header color intrinsics
transforms/camera             (M, 4, 4) float64  # from .trajectory.npz, segment slice
transforms/leftHand           (M, 4, 4) float64  # wrist SE(3) from .hands.npz
transforms/rightHand          (M, 4, 4) float64
transforms/{joint_name}       (M, 4, 4) float64  # 21 joints x 2 hands = 42 entries
confidences/{joint_name}      (M,) float64
attrs["llm_description"]      str                # from .segments.json
```

**Extra (not in EgoDex but we have it):**
```
depth/frames                  (M, H, W) uint16   # raw Z16 depth (optional, --include-depth)
camera/depth_intrinsic        (3, 3) float64
camera/depth_scale            float64
```

Joint name mapping: MediaPipe landmark indices to EgoDex-style anatomical names (e.g., `leftThumbTip`, `rightIndexMCP`). Defined in `python/pipeline/joint_mapping.py`.

Camera poses are re-centered per segment (first frame = identity) to avoid drift accumulation.

---

## File Structure

```
python/
    pipeline/
        __init__.py
        hand_tracker.py        # Stage 1
        ego_motion.py          # Stage 2
        segmenter.py           # Stage 3
        egodex_exporter.py     # Stage 4
        joint_mapping.py       # MediaPipe idx -> EgoDex joint names
        se3_utils.py           # SE(3) math helpers
        depth_utils.py         # Depth deprojection, median filter, interpolation
    export_egodex.py           # CLI entry point
    requirements-egodex.txt    # Dependencies
```

## CLI Interface

```bash
# Full pipeline:
python export_egodex.py recording.egorec -o ./output --task-name "kitchen"

# Individual stages:
python export_egodex.py --stage hands recording.egorec
python export_egodex.py --stage slam recording.egorec
python export_egodex.py --stage segment recording.egorec
python export_egodex.py --stage export recording.egorec -o ./output

# Skip stages if sidecars already exist:
python export_egodex.py --skip-existing recording.egorec -o ./output

# With LLM annotation:
python export_egodex.py --annotate --api-key $KEY recording.egorec -o ./output

# Include depth in HDF5 (not in EgoDex spec but useful):
python export_egodex.py --include-depth recording.egorec -o ./output
```

## C++ Integration

Add `"egodex"` case in `src/main.cpp` export dispatch (line 324-333), routing to `python/export_egodex.py`. Identical pattern to existing `"rlds"` and `"lerobot"` cases.

---

## Implementation Order

### 1. Utility modules
- `se3_utils.py` — rotation matrix from 3 points, SE(3) construction, interpolation
- `depth_utils.py` — median-filtered depth lookup, deprojection, temporal interpolation
- `joint_mapping.py` — MediaPipe index to EgoDex name table

### 2. Hand tracker (Stage 1)
- MediaPipe integration, depth lifting, SE(3) per joint, .hands.npz writer
- Test: run on existing recording, visualize 3D hand skeleton overlaid on depth

### 3. Ego-motion (Stage 2)
- Open3D RGB-D odometry, pose chaining, quality scoring, .trajectory.npz writer
- Test: plot camera trajectory as 3D path

### 4. Segmenter (Stage 3)
- Velocity computation, pinch detection, interval merging, .segments.json writer
- Test: inspect segment boundaries against video

### 5. EgoDex exporter (Stage 4) + CLI
- HDF5 + MP4 writer, CLI entry point, C++ dispatch integration
- Test: load output with `h5py`, verify schema matches EgoDex

---

## Verification

1. **Hand tracking accuracy**: Visualize projected 3D landmarks back onto RGB — they should align with visible hand positions
2. **Ego-motion sanity**: For a static camera, all poses should be near-identity. For moving camera, trajectory should be smooth
3. **Segmentation quality**: Manually review 5+ segments against video — boundaries should align with action start/stop
4. **EgoDex compatibility**: Load output HDF5 with the EgoDex `simple_dataset.py` loader from `github.com/apple/ml-egodex` — should parse without errors
5. **Round-trip**: Full pipeline on a test recording, inspect all sidecar files + final output

## Dependencies

**`python/requirements-egodex.txt`:**
```
mediapipe>=0.10.0
open3d>=0.17.0
opencv-python>=4.8.0
h5py>=3.9.0
scipy>=1.11.0
numpy>=1.24.0
tqdm>=4.60.0
```
