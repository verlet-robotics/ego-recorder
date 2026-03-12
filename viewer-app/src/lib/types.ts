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

export type ConversionStatus = "idle" | "streamable" | "error";

/** List item returned by discover_files / list_files. */
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

/** Response from discover_files. */
export interface FilesResponse {
  dir: string;
  files: EgorecListItem[];
}

/** Response from get_file_metadata. */
export interface FileDetailResponse {
  name: string;
  metadata: EgorecMetadata;
  sizeBytes: number;
  conversionStatus: ConversionStatus;
}

// ── Analysis types (mirrors egorec::AnalysisResult) ──────────────────────────

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

// ── Curation types ──────────────────────────────────────────────────────────

export interface CurationWorkspaceInfo {
  root: string | null;
  activeWorkspace: string | null;
  activeName: string | null;
  hasWorkspace: boolean;
}

export interface WorkspaceSummary {
  name: string;
  path: string;
  sourcePrefix: string | null;
  episodeCount: number;
  completedStages: string[];
  hasIntervals: boolean;
  hasLabels: boolean;
  hasBuckets: boolean;
}

export interface RecentWorkspace {
  path: string;
  alias: string | null;
  lastOpenedAt: string;
}
