/** Parsed metadata from .egorec header + footer. */
export interface EgorecMetadata {
  sessionName: string;
  serialNumber: string;
  usbType: string;
  colorWidth: number;
  colorHeight: number;
  depthWidth: number;
  depthHeight: number;
  depthScale: number;
  fps: number;
  totalFrames: number;
  durationS: number;
  startTimestampUs: number;
  hasImu: boolean;
  rgbCodec: number;
  depthCodec: number;
  rgbQuality: number;
  zstdLevel: number;
  intrinsics: {
    color: CameraIntrinsics;
    depth: CameraIntrinsics & { scale: number };
  };
  extrinsics: {
    rotation: number[];
    translation: number[];
  };
}

export interface CameraIntrinsics {
  width: number;
  height: number;
  fx: number;
  fy: number;
  ppx: number;
  ppy: number;
  distortionModel: number;
  distortionCoeffs: number[];
}

export type ConversionStatus = "idle" | "queued" | "converting" | "ready" | "streamable" | "error";

/** File entry in the server's in-memory index. */
export interface EgorecFileEntry {
  name: string;
  path: string;
  sizeBytes: number;
  metadata: EgorecMetadata;
  conversionStatus: ConversionStatus;
  error?: string;
}

/** API response for GET /api/files */
export interface FilesResponse {
  dir: string;
  files: EgorecListItem[];
}

/** List item returned by the API. */
export interface EgorecListItem {
  name: string;
  dataset: string | null;
  sessionName: string;
  rgbCodec: number;
  colorWidth: number;
  colorHeight: number;
  fps: number;
  totalFrames: number;
  durationS: number;
  sizeBytes: number;
  conversionStatus: ConversionStatus;
  hasImu: boolean;
}

/** Detailed file response for GET /api/files/:name */
export interface FileDetailResponse {
  name: string;
  metadata: EgorecMetadata;
  sizeBytes: number;
  conversionStatus: ConversionStatus;
  error?: string;
}

/** Response for GET /api/files/:name/status */
export interface ConversionStatusResponse {
  name: string;
  status: ConversionStatus;
  error?: string;
}

// ── Analysis types (mirrors ego-qc AnalysisResult) ──────────────────────────

export type Verdict = "Keep" | "PruneConfident" | "PruneSuggested" | "Review";

export interface IdleBaseline {
  median: number;
  mad: number;
  threshold: number;
}

export interface EpisodeFeatures {
  active_frame_fraction: number;
  burst_count: number;
  p95_p50_ratio: number;
  final_third_activity: number;
  active_window_fraction: number;
  depth_cv: number;
  depth_active_frame_fraction: number;
  window_depth_cv_mean: number;
  window_depth_cv_max: number;
  ego_motion_window_fraction: number;
}

export interface AnalysisResult {
  filename: string;
  verdict: Verdict;
  activity_score: number;
  reasons_keep: string[];
  reasons_prune: string[];
  features: EpisodeFeatures;
  total_frames: number;
  duration_s: number;
  idle_baseline: IdleBaseline;
  depth_idle_baseline: IdleBaseline;
  used_profile: boolean;
}

export type AnalysisStatus = "idle" | "running" | "done" | "error";

export interface SpliceResult {
  file: string;
  segments: string[];
}
