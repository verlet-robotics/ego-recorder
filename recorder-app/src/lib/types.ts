/** App configuration matching Rust AppConfig */
export interface AppConfig {
  recorder: {
    binary_path: string | null;
    default_crf: number;
    warmup_frames: number;
    h264_preset: string;
  };
  storage: {
    output_dir: string | null;
    disk_threshold_mb: number;
  };
  upload: {
    endpoint: string | null;
    bucket: string | null;
    access_key: string | null;
    secret_key: string | null;
    region: string | null;
    auto_upload: boolean;
    prefix: string | null;
    multipart_chunk_mb: number;
    poll_interval_s: number;
    file_settle_s: number;
  };
}

/** Recorder status from Rust backend */
export interface RecorderStatus {
  state: RecorderState;
  framesWritten: number;
  framesDropped: number;
  captureFps: number;
  writeFps: number;
  fileSizeMb: number;
  elapsedSeconds: number;
  episodeCount: number;
  currentFile: string | null;
}

export type RecorderState = "idle" | "countdown" | "recording" | "stopping" | "error";

/** Disk info from Rust backend */
export interface DiskInfo {
  totalBytes: number;
  freeBytes: number;
  usedBytes: number;
  usagePercent: number;
  freeMb: number;
}

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

export interface FilesResponse {
  dir: string;
  files: EgorecListItem[];
}

export interface FileDetailResponse {
  name: string;
  metadata: EgorecMetadata;
  sizeBytes: number;
  conversionStatus: ConversionStatus;
}

/** Camera info from preview subprocess */
export interface CameraInfo {
  serial: string;
  usb: string;
  hasImu: boolean;
  width: number;
  height: number;
}

/** Preview subprocess state */
export type PreviewState = "off" | "starting" | "previewing" | "recording" | "stopping" | "error";

export type Page = "record" | "library" | "datasets" | "upload" | "settings";

/** Upload queue item from Rust backend */
export interface UploadQueueItem {
  filename: string;
  path: string;
  sizeBytes: number;
  status: UploadQueueStatus;
}

export type UploadQueueStatus =
  | { kind: "pending" }
  | { kind: "hashing"; progress: number }
  | { kind: "uploading"; progress: number; speedBps: number }
  | { kind: "completed"; sha256: string }
  | { kind: "failed"; error: string };

/** Upload progress event from Rust backend */
export interface UploadProgressEvent {
  filename: string;
  bytesTransferred: number;
  totalBytes: number;
  speedBps: number;
  phase: string;
}

/** Dataset summary from Rust backend */
export interface DatasetSummary {
  name: string;
  dirName: string;
  description: string;
  tags: string[];
  fileCount: number;
  totalFrames: number;
  totalDurationS: number;
  totalSizeBytes: number;
  uploadedCount: number;
  hasLerobot: boolean;
  createdAt: string;
  updatedAt: string;
  targetEpisodes: number | null;
}

/** System info from Rust backend */
export interface SystemInfo {
  cpuModel: string;
  cpuCores: number;
  arch: string;
  recommendedPreset: string;
}

/** Conversion progress event from Rust backend */
export interface ConversionProgress {
  datasetName: string;
  currentFile: string;
  fileIndex: number;
  totalFiles: number;
  framesDone: number;
  totalFrames: number;
  phase: string;
  error: string | null;
}
