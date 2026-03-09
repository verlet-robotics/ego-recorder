# Post-Processing Pipeline: EgoScale/EgoDex-Compatible Export

## Context

The EgoScale paper (NVIDIA, 2026, arxiv:2602.16710) trains a VLA model on 20,854 hours of egocentric human video with paired 3D hand pose annotations. Its primary data source is the EgoDex dataset (Apple, arxiv:2505.11709): paired MP4 + HDF5 files containing 1080p 30fps video with SE(3) transforms for 68 skeletal joints per frame (camera, 25 per hand, upper body, spine, neck).

Our recorder captures RGB-D at 640x480 30fps in `.egorec` format. This plan adds a 4-stage post-processing pipeline that takes `.egorec` files and produces EgoDex-compatible output. Our depth channel gives us an advantage over monocular approaches for absolute 3D positioning, and we use HaMeR (CVPR 2024) for state-of-the-art MANO mesh recovery with depth refinement for anatomically correct hand poses.

### EgoScale Data Tiers

EgoScale uses three tiers of training data with different fidelity requirements:

| Tier | Hours | Source | Hand Tracking | Our Equivalent |
|------|-------|--------|--------------|----------------|
| 1 — Bulk pretraining | 20,000 | In-the-wild egocentric RGB | Off-the-shelf pipelines (noisy) | **This pipeline** — matches or exceeds via depth |
| 2 — EgoDex | 829 | Apple Vision Pro + ARKit | Hardware-accelerated (~2-4mm) | Not achievable without Vision Pro |
| 3 — Aligned mid-training | 54 | Vive trackers + Manus gloves | Motion capture grade (<1mm) | Existing teleop recorder (leader/follower) |

This pipeline targets **Tier 1+ quality** with HaMeR + depth-enhanced accuracy (~3-5mm), landing between Tier 1 and Tier 2.

### EgoDex HDF5 Schema (canonical reference)

From `apple/ml-egodex` repository:

```
camera/
  intrinsic                         (3, 3) float64

transforms/                         all (N, 4, 4) float64, in world frame
  camera                            # camera-to-world extrinsics
  leftHand                          # left wrist
  rightHand                         # right wrist
  # Per finger: Metacarpal, Knuckle, IntermediateBase, IntermediateTip, Tip
  leftIndexFingerMetacarpal
  leftIndexFingerKnuckle
  leftIndexFingerIntermediateBase
  leftIndexFingerIntermediateTip
  leftIndexFingerTip
  leftMiddleFingerMetacarpal        # same 5-joint pattern
  leftMiddleFingerKnuckle
  leftMiddleFingerIntermediateBase
  leftMiddleFingerIntermediateTip
  leftMiddleFingerTip
  leftRingFingerMetacarpal
  leftRingFingerKnuckle
  leftRingFingerIntermediateBase
  leftRingFingerIntermediateTip
  leftRingFingerTip
  leftLittleFingerMetacarpal
  leftLittleFingerKnuckle
  leftLittleFingerIntermediateBase
  leftLittleFingerIntermediateTip
  leftLittleFingerTip
  leftThumbKnuckle                  # thumb has 4 joints (no metacarpal)
  leftThumbIntermediateBase
  leftThumbIntermediateTip
  leftThumbTip
  right...                          # same 24 joints for right hand
  # Upper body (optional — zero-confidence if unavailable)
  leftShoulder, leftArm, leftForearm
  rightShoulder, rightArm, rightForearm
  hip, spine1-7, neck1-4

confidences/                        all (N,) float64 per joint
  leftHand, rightHand, ...          # 0.0 = occluded/undetected, 1.0 = confident

attrs:
  llm_description                   # natural language task description
  llm_description2                  # (optional, for reversible tasks)
  which_llm_description             # "1" or "2"
  llm_type                          # "reversible" or other
```

Total: 25 hand joints per side (1 wrist + 24 finger) = 50 hand joints + camera + upper body.

---

## Pipeline Overview

```
.egorec
  |
  +--> [Stage 1] Hand Tracking (HaMeR + depth refinement)  --> .hands.npz
  +--> [Stage 2] Camera Ego-Motion (RGB-D SLAM)             --> .trajectory.npz
  |         (both feed into)
  +--> [Stage 3] Action Segmentation                        --> .segments.json
  |
  +--> [Stage 4] EgoDex Export                              --> task_name/{0..N}.mp4 + {0..N}.hdf5
```

Each stage writes a **sidecar file** next to the source `.egorec`. Stages can be re-run independently. Final export merges all sidecars into EgoDex format.

---

## Stage 1: Hand Pose Estimation (HaMeR + Depth Refinement)

**File:** `python/pipeline/hand_tracker.py`
**Output:** `{basename}.hands.npz`
**Deps:** `torch>=2.0.0`, `hamer`, `opencv-python>=4.8.0`, `scipy`

### Architecture

HaMeR (Hand Mesh Recovery, CVPR 2024) is the current state-of-the-art for monocular 3D hand pose estimation. It takes an RGB image crop and directly outputs MANO parameters — no intermediate 2D landmark detection needed. We add depth refinement and temporal smoothing on top.

```
RGB frame ──► ViTDet hand detector ──► hand bounding boxes + handedness
                                              │
                                              ▼
                                        HaMeR backbone ──► MANO params (β, θ, t)
                                        (ViT-H encoder,     per detected hand
                                         MANO decoder)
                                              │
                                              ▼
                                    MANO forward kinematics ──► 25 joints with SE(3)
                                    (anatomically constrained,
                                     includes metacarpals)
                                              │
                                              ▼
                                    Depth refinement ──► Corrected absolute 3D
                                    (anchor wrist Z       (eliminates monocular
                                     from depth map)       depth ambiguity)
                                              │
                                              ▼
                                    Temporal smoothing ──► Jitter-free trajectories
                                    (one-euro filter)
```

### Phase 1: HaMeR Inference

1. Run ViTDet hand detector on RGB frame → bounding boxes for up to 2 hands + left/right classification
2. Crop and resize each hand region to 256×256
3. Feed through HaMeR's ViT-H backbone → directly outputs MANO parameters:
   - `β` (10 dims): Hand shape (bone lengths, hand size)
   - `θ` (15 × 3 = 45 dims): Joint rotations as axis-angle
   - `cam` (3 dims): Weak-perspective camera parameters (scale + 2D translation)
4. Run MANO forward kinematics to get 3D joint positions and mesh vertices

HaMeR outputs are already anatomically constrained by the MANO model — joints respect bone lengths, angle limits, and kinematic chain structure. No post-hoc fitting or optimization needed.

**Why HaMeR over MediaPipe:**
- Outputs a full MANO mesh (778 vertices, 1538 faces), not just sparse landmarks
- Handles occlusion significantly better — the model hallucinates plausible occluded finger positions
- Joint orientations come from the MANO kinematic chain (proper SE(3)), not ad-hoc bone direction heuristics
- Metacarpal joints are native to the MANO skeleton
- Trained on 10× more data than MediaPipe Hands (FreiHAND, HO3D, InterHand2.6M, etc.)

### Phase 2: Depth Refinement

HaMeR uses weak-perspective projection, so its absolute depth (distance from camera) is ambiguous. Our RealSense depth resolves this:

1. Project the MANO wrist joint to pixel coordinates using the weak-perspective camera params
2. Sample depth from the depth map using a 7×7 median filter centered on the wrist pixel
3. If valid depth (>0):
   - Convert HaMeR's weak-perspective output to full perspective using color intrinsics
   - Replace the estimated wrist Z with measured depth Z
   - The entire hand mesh shifts accordingly (relative joint geometry preserved)
   - Cross-check: if depth is available at fingertip pixels (thumb, index — large enough for clean readings), verify they're within 30mm of the MANO prediction. Flag discrepancies.
4. If depth invalid: keep HaMeR's monocular estimate, set wrist confidence to 0.5

### Phase 3: Temporal Smoothing

Apply a one-euro filter to each joint's position and orientation trajectory:
- `min_cutoff=1.0` (Hz) — smooths slow movements
- `beta=0.5` — allows fast movements to pass through
- `d_cutoff=1.0` — derivative cutoff

The one-euro filter is adaptive: it smooths more during slow motion and less during fast motion, avoiding both jitter and lag. Applied independently to position (3D) and orientation (quaternion SLERP variant).

### MANO → EgoDex Joint Extraction

The MANO model has a 16-joint kinematic chain. Combined with 5 fingertip vertices from the mesh, this gives 21 joints. We extract the full 25 EgoDex joints per hand:

```python
MANO_TO_EGODEX = {
    # MANO joint index → EgoDex name ('{side}' replaced with left/right)
    0:  '{side}Hand',                          # Wrist
    1:  '{side}IndexFingerKnuckle',            # Index MCP
    2:  '{side}IndexFingerIntermediateBase',    # Index PIP
    3:  '{side}IndexFingerIntermediateTip',     # Index DIP
    4:  '{side}MiddleFingerKnuckle',           # Middle MCP
    5:  '{side}MiddleFingerIntermediateBase',   # Middle PIP
    6:  '{side}MiddleFingerIntermediateTip',    # Middle DIP
    7:  '{side}LittleFingerKnuckle',           # Little MCP
    8:  '{side}LittleFingerIntermediateBase',   # Little PIP
    9:  '{side}LittleFingerIntermediateTip',    # Little DIP
    10: '{side}RingFingerKnuckle',             # Ring MCP
    11: '{side}RingFingerIntermediateBase',     # Ring PIP
    12: '{side}RingFingerIntermediateTip',      # Ring DIP
    13: '{side}ThumbKnuckle',                  # Thumb CMC
    14: '{side}ThumbIntermediateBase',          # Thumb MCP
    15: '{side}ThumbIntermediateTip',           # Thumb IP
}

# Fingertips from MANO mesh vertices (not skeleton joints)
MANO_FINGERTIP_VERTICES = {
    744: '{side}IndexFingerTip',
    320: '{side}MiddleFingerTip',
    443: '{side}LittleFingerTip',
    555: '{side}RingFingerTip',
    745: '{side}ThumbTip',
}

# Metacarpals: interpolated from wrist and knuckle
# Position: lerp(wrist, knuckle, 0.3)
# Orientation: same as wrist
INTERPOLATED_METACARPALS = [
    ('{side}IndexFingerMetacarpal',  0, 1),   # (name, wrist_idx, knuckle_idx)
    ('{side}MiddleFingerMetacarpal', 0, 4),
    ('{side}RingFingerMetacarpal',   0, 10),
    ('{side}LittleFingerMetacarpal', 0, 7),
]
# Thumb has no metacarpal in EgoDex (only 4 thumb joints)
```

### SE(3) Construction per Joint

For each MANO joint, the SE(3) transform is constructed from the forward kinematics:
- **Position**: Joint's 3D location in camera frame (meters)
- **Orientation**: Accumulated rotation through the kinematic chain from root to joint

The MANO model's forward kinematics naturally produce these — no ad-hoc frame construction needed.

All SE(3)s are output in **camera frame**. Stage 4 transforms them to world frame using Stage 2 ego-motion.

### Output Schema

```python
{
    # Raw MANO parameters (for re-fitting, retargeting, or analysis)
    "left_mano_shape":    (10,) float32,         # β — constant per recording
    "right_mano_shape":   (10,) float32,
    "left_mano_pose":     (N, 48) float32,        # θ (45 joint + 3 global rotation)
    "right_mano_pose":    (N, 48) float32,
    "left_mano_trans":    (N, 3) float32,         # global translation (depth-corrected)
    "right_mano_trans":   (N, 3) float32,

    # HaMeR detection metadata
    "left_bbox":          (N, 4) float32,         # hand bounding boxes [x1, y1, x2, y2]
    "right_bbox":         (N, 4) float32,
    "left_hamer_score":   (N,) float32,           # HaMeR detection confidence
    "right_hamer_score":  (N,) float32,

    # Derived joint transforms — 25 joints per hand in camera frame
    "left_joint_se3":     (N, 25, 4, 4) float64,  # SE(3) per joint
    "right_joint_se3":    (N, 25, 4, 4) float64,
    "left_confidence":    (N, 25) float32,         # per-joint confidence
    "right_confidence":   (N, 25) float32,
    "left_detected":      (N,) bool,
    "right_detected":     (N,) bool,

    # Metadata
    "timestamps_s":       (N,) float64,
    "joint_names":        list[str],               # 25 EgoDex joint names in order
    "depth_refined":      (N,) bool,               # True if depth was used for this frame
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

### Output Schema

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
1. **Hand velocity** (primary): 3D wrist velocity from Stage 1 HaMeR/MANO output. Active manipulation = velocity > `30 mm/s`
2. **Hand presence** (secondary): No hands detected for >1s = transition/idle
3. **Pinch detection** (tertiary): Thumb-tip to index-tip distance from MANO mesh. Pinch (<30mm) followed by release (>60mm) = grasp/release boundary
4. **Camera motion** (quaternary): Rotation >30° or translation >0.5m in 1s window from Stage 2 = repositioning, not manipulation

### Algorithm
1. Compute all signals from Stage 1 + Stage 2 outputs
2. Gaussian smooth (sigma=5 frames / ~167ms)
3. Detect "active" intervals where hand velocity > threshold AND hands detected
4. Merge intervals separated by <0.5s (brief pauses mid-task)
5. Split intervals >10s at grasp/release events (likely multi-action)
6. Discard segments <1s (noise)
7. Pad each segment by 0.5s on both sides for context

### Optional: LLM annotation (`--annotate`)
For each segment, extract the middle frame and send to a vision-language model with prompt: "Describe the manipulation action in this egocentric image in one sentence." Stored as `llm_description` attribute. Requires `--annotate --api-key KEY`.

### Output Schema

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

**MP4**: Re-encode RGB frames for the segment range via OpenCV VideoWriter (H.264, native resolution).

**HDF5** (matching EgoDex schema exactly):

```
camera/intrinsic              (3, 3) float64     # from .egorec header color intrinsics

transforms/                   all (M, 4, 4) float64, in WORLD FRAME
  camera                      # camera-to-world SE(3) from .trajectory.npz
  leftHand                    # wrist SE(3), transformed to world frame
  rightHand
  leftIndexFingerMetacarpal   # 24 finger joints per hand
  leftIndexFingerKnuckle
  leftIndexFingerIntermediateBase
  leftIndexFingerIntermediateTip
  leftIndexFingerTip
  leftMiddleFingerMetacarpal
  ...                         # (same pattern for all fingers)
  leftThumbKnuckle
  leftThumbIntermediateBase
  leftThumbIntermediateTip
  leftThumbTip
  rightIndexFingerMetacarpal  # same 24 for right hand
  ...
  rightThumbTip

confidences/                  all (M,) float64
  leftHand                    # per-joint scalar confidence
  rightHand
  leftIndexFingerMetacarpal
  ...                         # one entry per joint in transforms/

attrs:
  llm_description             # from .segments.json (or empty string)
```

**Reference frame transformation:**

All Stage 1 hand poses are in camera frame. At export time, transform each joint to world frame:
```
T_joint_world = T_camera_to_world @ T_joint_camera
```
where `T_camera_to_world` comes from Stage 2.

Camera poses are re-centered per segment: first frame of each segment becomes identity. This matches EgoDex convention (ARKit origin is arbitrary per session) and avoids drift accumulation.

**Upper body joints:**

Not exported by default (our pipeline doesn't estimate them). If `--include-upper-body` is passed and a body pose sidecar exists (from an optional MediaPipe Holistic or WHAM pass), those joints are included. Otherwise, upper body joints are omitted entirely (EgoDex readers handle missing joints gracefully — they only query what they need).

**Extra data (not in EgoDex but we have it):**

```
depth/frames                  (M, H, W) uint16   # raw Z16 depth (optional, --include-depth)
camera/depth_intrinsic        (3, 3) float64
camera/depth_scale            float64
mano/left_shape               (10,) float32       # MANO β (optional, --include-mano)
mano/right_shape              (10,) float32
mano/left_pose                (M, 48) float32     # MANO θ
mano/right_pose               (M, 48) float32
```

---

## File Structure

```
python/
    pipeline/
        __init__.py
        hand_tracker.py        # Stage 1: HaMeR inference → depth refinement → smoothing
        hamer_wrapper.py       # HaMeR model loading, batched inference, caching
        ego_motion.py          # Stage 2: RGB-D SLAM
        segmenter.py           # Stage 3: velocity/pinch segmentation
        egodex_exporter.py     # Stage 4: HDF5 + MP4 export
        joint_mapping.py       # MANO ↔ EgoDex joint name mapping
        se3_utils.py           # SE(3) math: rotation, interpolation, smoothing
        depth_utils.py         # Depth: median filter, deprojection, scale correction
        temporal_filter.py     # One-euro filter implementation
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

# Include raw MANO parameters:
python export_egodex.py --include-mano recording.egorec -o ./output

# Batch process a directory:
python export_egodex.py recordings/*.egorec -o ./output --task-name "tabletop"
```

## C++ Integration

Add `"egodex"` case in `src/main.cpp` export dispatch (line 324-333), routing to `python/export_egodex.py`. Identical pattern to existing `"rlds"` and `"lerobot"` cases.

---

## Implementation Order

### 1. Utility modules
- `se3_utils.py` — SE(3) construction, multiplication, interpolation, camera projection
- `depth_utils.py` — median-filtered depth lookup, deprojection, scale factor computation
- `joint_mapping.py` — MANO ↔ EgoDex name tables, metacarpal interpolation
- `temporal_filter.py` — one-euro filter for position and quaternion smoothing

### 2. HaMeR wrapper
- `hamer_wrapper.py` — model loading (ViTDet detector + HaMeR backbone), batched inference, GPU memory management
- Download pretrained weights from https://github.com/geopavlakos/hamer
- Test: run on single image, verify MANO output has correct shapes

### 3. Hand tracker (Stage 1)
- `hand_tracker.py` — HaMeR inference → depth refinement → temporal smoothing → sidecar writer
- Test: run on existing recording, visualize MANO mesh overlaid on RGB. Verify depth-corrected wrist matches depth map.

### 4. Ego-motion (Stage 2)
- `ego_motion.py` — Open3D RGB-D odometry, pose chaining, quality scoring
- Test: plot camera trajectory as 3D path. Static camera → near-identity poses.

### 5. Segmenter (Stage 3)
- `segmenter.py` — velocity/pinch/presence signals, interval detection, merging, splitting
- Test: inspect segment boundaries against video playback

### 6. EgoDex exporter (Stage 4) + CLI
- `egodex_exporter.py` — HDF5 + MP4 writer, camera→world transform, per-segment re-centering
- `export_egodex.py` — CLI entry point with stage selection
- Test: load output with EgoDex `simple_dataset.py`, verify schema matches

---

## Verification

1. **HaMeR mesh quality**: Reproject fitted MANO mesh onto RGB — mesh silhouette should align with visible hand. HaMeR's published benchmark: ~6mm PA-MPJPE on FreiHAND. Verify visually on our egocentric recordings.
2. **Depth refinement**: Compare HaMeR-only wrist Z vs depth-corrected wrist Z against ground truth depth. Depth-corrected should be within ±5mm for valid depth frames.
3. **Joint SE(3) sanity**: Visualize SE(3) frames on a 3D hand skeleton. Orientations should follow bone directions. No flipped axes.
4. **Temporal smoothness**: Plot any joint's X/Y/Z over time. Should be smooth without jitter spikes. One-euro filter should eliminate >90% of frame-to-frame noise.
5. **Ego-motion sanity**: For a static camera, all poses should be near-identity. For moving camera, trajectory should be smooth without jumps.
6. **Segmentation quality**: Manually review 5+ segments against video — boundaries should align with action start/stop within ±0.3s.
7. **EgoDex compatibility**: Load output HDF5 with EgoDex `simple_dataset.py` from `github.com/apple/ml-egodex`. Must parse without errors. Run `visualize_2d.py` to verify reprojected joints align with video.
8. **Round-trip**: Full pipeline on a test recording → inspect all sidecar files → load final HDF5/MP4 → verify all 50 hand joints + camera present with correct shapes and dtypes.

## Dependencies

**`python/requirements-egodex.txt`:**
```
# Hand tracking — HaMeR
torch>=2.0.0
torchvision>=0.15.0
hamer @ git+https://github.com/geopavlakos/hamer.git
vitdet @ git+https://github.com/geopavlakos/hamer.git#subdirectory=third-party/ViTDet

# Ego-motion
open3d>=0.17.0

# Export
h5py>=3.9.0
opencv-python>=4.8.0

# Processing
scipy>=1.11.0
numpy>=1.24.0
tqdm>=4.60.0
```

**Model weights (automatic on first run):**
HaMeR downloads pretrained weights automatically from the authors' server (~400MB). MANO model files (`MANO_RIGHT.pkl`, `MANO_LEFT.pkl`) must be downloaded manually from https://mano.is.tue.mpg.de/ (requires registration) and placed in `$HOME/.mano/` or `python/pipeline/models/mano/`.

## Performance Estimates

**With GPU (RTX 3060 or better):**
- ViTDet hand detection: ~25fps
- HaMeR MANO recovery: ~15fps per hand crop (batched: ~10fps for 2 hands)
- Depth refinement: ~1000fps (trivial)
- One-euro filtering: ~10000fps (trivial)
- **Total Stage 1: ~10-15fps** (bottleneck: HaMeR ViT-H backbone)

**Optimization options to reach ~25-30fps:**
- TensorRT export: compile HaMeR to TensorRT FP16 → ~2-3× speedup
- ONNX Runtime: export to ONNX with FP16 → ~1.5-2× speedup
- Batch consecutive frames: process 4 frames through ViTDet simultaneously → better GPU utilization
- Skip detection every other frame: run ViTDet at 15fps, track bounding boxes between detections using optical flow or simple IoU tracking. HaMeR runs on tracked crops at 30fps.
- Resolution: detector runs on 640×480 (our native res) not 1280×960 — already optimized

**CPU only: not recommended.** HaMeR's ViT-H backbone is 632M parameters. CPU inference is ~1-2fps. Use GPU.

Stage 2 (Open3D SLAM): ~15-20fps (CPU)
Stage 3 (segmentation): instant (operates on precomputed signals)
Stage 4 (export): ~30fps (bottleneck: H.264 encoding)

**Full pipeline for a 1-minute recording (~1800 frames): ~3-5 minutes with GPU.**
