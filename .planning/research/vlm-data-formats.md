# VLM Data Formats for Robotics: Market Research

**Researched:** 2026-02-19
**Focus:** What robotics companies actually use for VLM fine-tuning data
**Overall Confidence:** HIGH (sourced from official repos, papers, and verified documentation)

---

## Executive Summary

The robotics VLM ecosystem has converged on two dominant data formats: **RLDS/TFRecord** (the Google DeepMind standard used by Open X-Embodiment, RT-X, Octo, OpenVLA) and **LeRobot v3** (the Hugging Face/PyTorch standard gaining rapid adoption in 2025). Any commercial dataset targeting VLM fine-tuning buyers MUST ship in at least RLDS format, ideally both.

The critical finding for this project: **current VLM models overwhelmingly train on RGB only, not depth**. OpenVLA, Octo, RT-2, and Pi0 all use single RGB images at 224x224 or 256x256 resolution. However, depth data is stored by several foundational datasets (DROID, Bridge V2, ManiSkill) and is increasingly valued as 3D-aware models emerge. Depth is a differentiator, not a table-stakes feature.

The most valuable data product is NOT raw ego-centric video -- it is **teleoperated robot manipulation episodes with action labels, language annotations, and camera calibration metadata**, formatted as RLDS-compliant TFRecords. Ego-centric human video without robot actions sits lower in the data hierarchy and commands significantly less value.

---

## 1. Data Formats Used by Major Robotics VLM Projects

### Format Adoption Matrix

| Project | Primary Format | Image Resolution | Framerate | Depth? | Language? |
|---------|---------------|-----------------|-----------|--------|-----------|
| **Open X-Embodiment** | RLDS/TFRecord | 256x256 (canonical) | Varies (5-30Hz) | Optional | Yes |
| **RT-2 / RT-X** | RLDS/TFRecord | 256x256 | ~10Hz | No | Yes |
| **Octo** | RLDS/TFRecord | 256x256 (3rd person), 128x128 (wrist) | Varies | No | Yes (56% of data) |
| **OpenVLA** | RLDS/TFRecord | 224x224 | Varies | No | Yes |
| **Pi0 / OpenPI** | RLDS (large), LeRobot v2.1 (small) | 224x224 | Varies | No | Yes |
| **DROID** | RLDS/TFRecord + raw SVO | 1280x720 (raw), downsampled for training | 15Hz control | Yes (stereo) | Yes (1-3 per episode) |
| **Bridge V2** | RLDS/TFRecord + raw JPEG/PNG | 640x480 (raw) | ~5Hz | Yes (RGBD camera) | Yes |
| **RoboCasa** | HDF5 (robomimic) | 224x224 | N/A (sim) | Sim depth available | Yes |
| **ManiSkill** | RLDS/TFRecord | 256x256 | N/A (sim) | Yes (uint16) | Yes |

**Confidence: HIGH** -- verified against arxiv papers and official GitHub repos.

### Key Takeaway

RLDS/TFRecord is the undisputed standard for large-scale cross-embodiment datasets. Every major VLM project either natively uses RLDS or has RLDS conversion available. LeRobot v3 is the rising alternative for PyTorch-first teams, but Pi0 (the most commercially relevant model) explicitly uses RLDS for large datasets.

---

## 2. RLDS Format Deep Dive

### What is RLDS?

RLDS (Reinforcement Learning Datasets) is a Google Research ecosystem for storing episodic sequential decision-making data. It stores data as TFRecord files (protobuf-serialized) organized through TensorFlow Datasets (TFDS).

**Source:** [github.com/google-research/rlds](https://github.com/google-research/rlds)

### Structure

```
Dataset
  -> Episode 1
       -> episode_metadata: {episode_id, file_path, ...}
       -> steps: [
            Step 0: {is_first=True,  observation, action, reward, discount, ...}
            Step 1: {is_first=False, observation, action, reward, discount, ...}
            ...
            Step N: {is_last=True,   observation, (action/reward invalid)}
          ]
  -> Episode 2
       -> ...
```

### Step Schema (Required Fields)

| Field | Type | Description |
|-------|------|-------------|
| `is_first` | bool | First step of episode |
| `is_last` | bool | Last step of episode |
| `observation` | Dict | Current state (images, proprioception, etc.) |
| `action` | Tensor | Action applied at this step |
| `reward` | Scalar(float32) | Reward signal |
| `discount` | Scalar(float32) | Discount factor |
| `is_terminal` | bool | Whether this is a terminal state |

### Open X-Embodiment Standard Action Space

The canonical OXE action format is an **8-dimensional vector**:

```
action[0:3] = end-effector position (x, y, z) -- absolute or delta
action[3:6] = end-effector orientation (roll, pitch, yaw) -- absolute or delta
action[6]   = gripper open/close
action[7]   = episode termination flag
```

### Observation Fields (OXE Convention)

| Field | Shape | Type | Notes |
|-------|-------|------|-------|
| `image` | (H, W, 3) | uint8 | Primary 3rd-person camera RGB |
| `wrist_image` | (H, W, 3) | uint8 | Wrist-mounted camera (optional) |
| `depth` | (H, W, 1) | uint16 | Depth map (optional, divide by scale for meters) |
| `state` | (N,) | float32 | Robot proprioception (joint angles, etc.) |
| `language_instruction` | string | Text | Natural language task description |
| `language_embedding` | (512,) | float32 | Pre-computed embedding (USE or similar) |

### How Widely Adopted is RLDS?

**Extremely.** As of 2025:
- Open X-Embodiment: 60+ datasets, 1M+ trajectories, all in RLDS
- Every Google DeepMind robotics project uses RLDS natively
- Pi0/OpenPI uses RLDS for large-scale training (explicitly chosen over LeRobot for scale)
- Octo, OpenVLA, RT-X all train directly from RLDS
- Multiple conversion tools exist to/from RLDS

**Confidence: HIGH** -- directly verified from official repositories and papers.

---

## 3. Open X-Embodiment Dataset Format Details

### Episode Structure

Each OXE-compatible dataset defines:

```python
FeaturesDict({
    'episode_metadata': FeaturesDict({
        'episode_id': Text,
        'file_path': Text,
        # Custom metadata (collector_id, scene_type, etc.)
    }),
    'steps': tfds.features.Dataset({
        'observation': FeaturesDict({
            'image': Image(shape=(H, W, 3), dtype=uint8),
            # Optional additional cameras, depth, state
        }),
        'action': Tensor(shape=(N,), dtype=float32),
        'reward': Scalar(dtype=float32),
        'is_first': bool,
        'is_last': bool,
        'is_terminal': bool,
        'language_instruction': Text,
        'language_embedding': Tensor(shape=(512,), dtype=float32),
    }),
})
```

### What the RT-X Models Actually Consume

The RT-X models select ONE canonical camera view per dataset, resize to a common resolution (256x256), and use the standardized 8D action space. They explicitly **exclude**:
- Wrist camera images
- Depth sensor data
- Point cloud data

This is a training-time limitation, not a data format limitation. The RLDS format stores all modalities; models just pick what they use.

**Confidence: HIGH** -- from the OXE paper (arxiv 2310.08864) and official GitHub.

---

## 4. Resolution and Framerate Analysis

### Training Resolution

| Model | Training Resolution | Notes |
|-------|-------------------|-------|
| OpenVLA | 224x224 | Tested 384x384, no improvement, 3x slower |
| Octo | 256x256 (main), 128x128 (wrist) | Tokenized to 16x16 patches |
| RT-2 / RT-X | 256x256 | Canonical OXE resolution |
| Pi0 | 224x224 | "Uncalibrated monocular RGB" |
| RoboCasa policies | 224x224 | Simulation standard |

**Bottom line:** Models train at 224x224 or 256x256. Storing at higher resolution (640x480) is correct -- it preserves information for future higher-res models and allows the buyer to crop/resize.

### Collection Framerate

| Dataset | Collection Rate | Control Rate | Notes |
|---------|----------------|-------------|-------|
| DROID | 30fps video | 15Hz actions | Actions at half frame rate |
| Bridge V2 | ~5fps | ~5Hz | Lower framerate |
| OXE (estimated) | ~30Hz | Varies | Paper estimates assume 30Hz |
| OpenVLA inference | N/A | ~6Hz | RTX 4090 inference speed |

**Bottom line:** 30fps capture is appropriate and matches the highest-quality datasets (DROID). Control frequency is typically 10-15Hz for real robots. 30fps gives flexibility for temporal subsampling.

**Confidence: HIGH** -- verified from papers.

---

## 5. Essential Metadata for Robotics VLM Training

### Tier 1: Required (without these, data has minimal value)

| Metadata | Why Essential | Format |
|----------|--------------|--------|
| **Language annotations** | All VLMs are language-conditioned; this is how tasks are specified | 1-3 natural language strings per episode |
| **Action labels** | The entire point of VLA training; maps observations to robot actions | Float32 tensor per step (typically 7-8 dim) |
| **Camera type/position** | Buyers need to know viewing angle for domain matching | String metadata (e.g., "ego", "over-shoulder", "wrist") |
| **Episode boundaries** | RLDS requires clear episode start/end | is_first, is_last, is_terminal flags |
| **Success/failure labels** | Filtering bad demonstrations is critical | Boolean per episode |

### Tier 2: Strongly Recommended (significantly increases value)

| Metadata | Why Valuable | Format |
|----------|-------------|--------|
| **Camera intrinsics** | Enables 3D reconstruction, depth reprojection, sim-to-real | 3x3 float32 matrix (fx, fy, cx, cy) |
| **Camera extrinsics** | Enables multi-view fusion, coordinate transforms | 4x4 float32 homogeneous transform |
| **Robot proprioception** | Joint states enable action-conditioned models | Float32 tensor (joint angles, velocities) |
| **Timestamps** | Temporal alignment, framerate verification | Float64 per step |
| **Pre-computed language embeddings** | Saves buyers compute; standard in OXE | 512-dim float32 (Universal Sentence Encoder) |

### Tier 3: Differentiators (nice-to-have)

| Metadata | Why | Format |
|----------|-----|--------|
| **Scene description** | GPT-4V scene classification (DROID does this) | String |
| **Collector ID** | Quality tracking, bias analysis | String |
| **Environment/location** | Domain diversity metrics | String |
| **Gripper state** | Binary open/close or continuous | Float32 or bool |

### Critical Insight: Ego-Centric Video WITHOUT Action Labels

If this product is ego-centric human video (no robot, no teleoperation), it falls into the "internet video" tier of the data hierarchy:

```
Most valuable:  Robot teleoperation data with actions + language
                Robot teleoperation data with actions only
                Ego-centric video with language + hand tracking
                Ego-centric video with language only
Least valuable: Raw ego-centric video without annotations
```

**Scale AI hires hundreds of contractors to collect teleoperated robot data specifically because ego video alone is insufficient for VLA training.** The data hierarchy matters enormously for pricing.

**Confidence: HIGH** -- synthesized from DROID, OXE, OpenVLA, and Scale AI documentation.

---

## 6. Depth Data Usage in VLM Training

### Current State: Depth is Stored but Rarely Used

| Project | Uses Depth in Training? | Stores Depth? |
|---------|------------------------|---------------|
| OpenVLA | No | N/A (does not store) |
| Octo | No | N/A |
| RT-2 / RT-X | No | Format supports it |
| Pi0 | No | N/A |
| DROID | No (for VLM training) | Yes (stereo cameras) |
| Bridge V2 | No (for VLM training) | Yes (RGBD camera, 640x480) |
| ManiSkill (RLDS) | Available | Yes (uint16 depth maps) |

### How Depth IS Stored When Present

**RLDS/TFRecord format for depth:**
```python
'depth': Image(shape=(H, W, 1), dtype=uint16)
# To convert to meters: depth_meters = depth_raw / 1024.0 (dataset-specific scale)
```

- **ManiSkill convention:** uint16, divide by 2^10 (1024) for meters
- **Bridge V2:** Single-channel depth at 640x480, uint8 in TFDS catalog (likely lossy)
- **DROID:** Stereo cameras (Zed 2) with native depth, stored in SVO format originally

### Why Depth Still Has Value

1. **3D-aware VLAs are emerging:** Dual-system architectures (e.g., 3D Diffusion Actor) use point clouds
2. **Depth enables reconstruction:** Buyers can generate point clouds, mesh reconstructions
3. **Sim-to-real transfer:** Depth is crucial for bridging simulation and reality
4. **Future-proofing:** As VLMs evolve beyond 2D, depth becomes training signal

### Recommendation for This Project

Store depth as **uint16 PNG in RLDS** (matching ManiSkill convention). Include the depth-to-meters scale factor in metadata. This costs almost nothing to capture with D435 and significantly differentiates the dataset.

**Confidence: HIGH** for current non-use; MEDIUM for future value claim.

---

## 7. Conversion Tools and Pipeline

### RLDS Dataset Builder (Official)

**Source:** [github.com/kpertsch/rlds_dataset_builder](https://github.com/kpertsch/rlds_dataset_builder)

The standard template for converting custom data to RLDS format for Open X-Embodiment:

1. Define features in `_info()` matching your data schema
2. Implement `_generate_examples()` to yield episodes from raw data
3. Build with `tfds build --overwrite`
4. Create a transform function mapping to the standardized 8D action spec
5. Upload to Google Cloud or distribute

### Forge (Multi-Format Converter)

**Source:** [github.com/arpitg1304/forge](https://github.com/arpitg1304/forge)

Hub-and-spoke architecture supporting bidirectional conversion:
- **Read + Write:** RLDS, LeRobot v2/v3, RoboDM
- **Read only:** GR00T (NVIDIA), Zarr, HDF5, Rosbag
- Includes quality scoring and filtering

```bash
forge convert /path/to/source ./output --format rlds
forge convert /path/to/source ./output --format lerobot-v3
```

### LeRobot v2.1 to v3.0 Migration

```bash
python -m lerobot.datasets.v30.convert_dataset_v21_to_v30 --repo-id=<HF_USER/DATASET_ID>
```

### OXE EnvLogger (For Live Collection)

**Source:** [github.com/rail-berkeley/oxe_envlogger](https://github.com/rail-berkeley/oxe_envlogger)

Wraps a dm_env Environment to record interactions directly into RLDS-compatible TFRecords during data collection. Best for real-time recording pipelines.

### Recommended Pipeline for This Project

```
RealSense D435 capture (640x480 RGB + depth + IMU @ 30fps)
    |
    v
Raw storage: per-frame PNG (RGB) + uint16 PNG (depth) + JSON (IMU/metadata)
    |
    v
Episode annotation: language labels + success/failure
    |
    v
RLDS Dataset Builder: convert to TFRecord
    |
    v
(Optional) Forge: convert to LeRobot v3 for PyTorch users
    |
    v
Ship both formats
```

**Confidence: HIGH** -- tools verified from official repos.

---

## 8. What Makes This Data Product Most Valuable

### The Hard Truth About Ego-Centric RGBD

Pure ego-centric video from a RealSense D435 mounted on a person's head is **NOT directly usable for VLA training**. VLA models need:
- Robot actions (joint positions, end-effector poses) per timestep
- These map observations to motor commands
- Human hand movements are not robot actions

### Value Tiers (Realistic Assessment)

#### Tier S: Maximum Value -- Teleoperated Robot Data ($$$)
- D435 mounted on robot (or robot workspace)
- Teleoperation actions recorded per step
- Language annotations per episode
- Camera intrinsics + extrinsics calibrated
- RLDS + LeRobot format
- **This is what OpenVLA, Pi0, Octo fine-tune on**

#### Tier A: High Value -- Ego Video with Dense Annotations ($$)
- D435 worn by human
- Dense hand tracking / pose estimation per frame
- Object interaction labels (grasping, placing, etc.)
- Language descriptions per segment
- Diverse environments (kitchens, workshops, labs)
- Camera intrinsics + depth calibrated
- **Useful for pretraining, representation learning, Ego4D-style research**

#### Tier B: Moderate Value -- Ego Video with Language Only ($)
- D435 worn by human
- Language annotations per segment
- Scene diversity
- RGBD with calibration
- **Useful for video-language pretraining, scene understanding**

#### Tier C: Low Value -- Raw Ego RGBD (minimal $)
- Just video streams
- No annotations
- **Too expensive to annotate for most buyers vs. collecting their own**

### What Buyers Actually Want

Based on the ecosystem analysis:

1. **Diverse manipulation scenes** -- kitchens, workshops, labs (not just one environment)
2. **Multiple viewpoints** -- both ego and third-person (DROID uses 3 cameras)
3. **Language descriptions** -- "pick up the red cup and place it on the shelf"
4. **Calibrated cameras** -- intrinsics and extrinsics allow reprojection
5. **Consistent quality** -- 640x480+ resolution, good lighting, no occlusion
6. **Standard format** -- RLDS TFRecord, loadable with `tfds.load()`
7. **Scale** -- hundreds to thousands of episodes (DROID has 76K, Bridge V2 has 60K)

### Specific Recommendations

1. **Do NOT sell raw ego video.** The market for unannotated ego RGBD is near zero for VLM fine-tuning. Scale AI already collects this at massive scale.

2. **Add robot actions if possible.** Mount the D435 as a workspace camera for a robot arm (even a cheap WidowX 250 like Bridge V2). This instantly moves value from Tier C to Tier S.

3. **If staying ego-only, invest heavily in annotations.** Language descriptions, hand pose tracking (MediaPipe), object bounding boxes, and grasp labels. This is what makes Ego4D valuable.

4. **Always include depth + calibration.** This is your differentiator vs. smartphone-collected ego video. D435 depth + factory intrinsics + proper extrinsic calibration = genuine value.

5. **Ship in RLDS format.** Use the `rlds_dataset_builder` template. Also convert to LeRobot v3 via Forge. Buyers should be able to `tfds.load("your_dataset")` immediately.

6. **Include pre-computed language embeddings.** Use the Universal Sentence Encoder (512-dim) matching the OXE convention.

7. **Target 1000+ episodes minimum.** Below this, the dataset is too small to matter for fine-tuning.

---

## Appendix A: LeRobot v3 Format (Alternative Standard)

### Structure
```
dataset/
  meta/
    info.json       # Schema, fps, features
    stats.json      # Per-feature normalization stats (mean, std, min, max)
    tasks.jsonl     # Language task descriptions -> indices
    episodes/       # Per-episode metadata in chunked Parquet
  data/             # Tabular data (actions, states) in Parquet
  videos/           # MP4 files organized by camera key
    front_cam/
      chunk_000/
        file_000.mp4
```

### Key Differences from RLDS

| Aspect | RLDS/TFRecord | LeRobot v3 |
|--------|---------------|------------|
| Ecosystem | TensorFlow | PyTorch |
| Image storage | Encoded in TFRecord | MP4 video files |
| Tabular data | TFRecord | Parquet |
| Metadata | TFDS metadata | JSON + Parquet |
| Streaming | TFDS cloud loading | HuggingFace Hub streaming |
| Scale | Proven at 1M+ episodes | v3 designed for millions |
| Used by | Google DeepMind, Octo, OpenVLA, Pi0 | HuggingFace community, growing |
| Depth support | Native (uint16 images) | Not explicitly documented |

**Recommendation:** Ship both. RLDS for research labs using OXE/Octo/OpenVLA/Pi0. LeRobot v3 for PyTorch-first teams.

---

## Appendix B: Complete RLDS Feature Spec Example (Reference)

This is based on the ManiSkill RLDS dataset, which is the gold standard for including depth + camera calibration:

```python
features = tfds.features.FeaturesDict({
    'episode_metadata': tfds.features.FeaturesDict({
        'episode_id': tfds.features.Text(),
        'file_path': tfds.features.Text(),
    }),
    'steps': tfds.features.Dataset({
        'observation': tfds.features.FeaturesDict({
            # RGB images
            'image': tfds.features.Image(shape=(256, 256, 3), dtype=np.uint8),
            'wrist_image': tfds.features.Image(shape=(256, 256, 3), dtype=np.uint8),
            # Depth images
            'depth': tfds.features.Image(shape=(256, 256, 1), dtype=np.uint16),
            'wrist_depth': tfds.features.Image(shape=(256, 256, 1), dtype=np.uint16),
            # Camera calibration
            'main_camera_intrinsic_cv': tfds.features.Tensor(shape=(3,3), dtype=np.float32),
            'main_camera_extrinsic_cv': tfds.features.Tensor(shape=(4,4), dtype=np.float32),
            # Robot state
            'state': tfds.features.Tensor(shape=(18,), dtype=np.float32),
        }),
        'action': tfds.features.Tensor(shape=(7,), dtype=np.float32),
        'reward': tfds.features.Scalar(dtype=np.float32),
        'discount': tfds.features.Scalar(dtype=np.float32),
        'is_first': tf.bool,
        'is_last': tf.bool,
        'is_terminal': tf.bool,
        'language_instruction': tfds.features.Text(),
        'language_embedding': tfds.features.Tensor(shape=(512,), dtype=np.float32),
    }),
})
```

---

## Appendix C: Sources

### Official Repositories
- [Open X-Embodiment](https://github.com/google-deepmind/open_x_embodiment) -- dataset specification and model code
- [RLDS](https://github.com/google-research/rlds) -- format specification and ecosystem
- [RLDS Dataset Builder](https://github.com/kpertsch/rlds_dataset_builder) -- conversion template for custom datasets
- [OXE EnvLogger](https://github.com/rail-berkeley/oxe_envlogger) -- real-time RLDS recording
- [Forge](https://github.com/arpitg1304/forge) -- multi-format conversion tool
- [OpenVLA](https://github.com/openvla/openvla) -- VLA model and training code
- [Octo](https://github.com/octo-models/octo) -- generalist robot policy
- [OpenPI (Pi0)](https://github.com/Physical-Intelligence/openpi) -- Physical Intelligence open-source
- [LeRobot](https://github.com/huggingface/lerobot) -- Hugging Face robotics framework
- [Bridge V2](https://github.com/rail-berkeley/bridge_data_v2) -- dataset and conversion scripts

### Papers
- Open X-Embodiment: [arxiv 2310.08864](https://arxiv.org/abs/2310.08864)
- OpenVLA: [arxiv 2406.09246](https://arxiv.org/abs/2406.09246)
- Octo: [arxiv 2405.12213](https://arxiv.org/abs/2405.12213)
- DROID: [arxiv 2403.12945](https://arxiv.org/abs/2403.12945)
- Bridge V2: [arxiv 2308.12952](https://arxiv.org/abs/2308.12952)
- Pi0: [arxiv 2410.24164](https://arxiv.org/abs/2410.24164)
- RLDS: [arxiv 2111.02767](https://arxiv.org/abs/2111.02767)

### Documentation
- [LeRobot v3 Dataset Format](https://huggingface.co/blog/lerobot-datasets-v3)
- [ManiSkill RLDS Catalog](https://www.tensorflow.org/datasets/catalog/maniskill_dataset_converted_externally_to_rlds) -- reference for depth + intrinsics spec
- [Scale AI Physical AI](https://scale.com/physical-ai) -- commercial data collection at scale
- [DROID Camera Calibration](https://medium.com/@zubair_irshad/scaling-up-automatic-camera-calibration-for-droid-dataset-4ddfc45361d3)
