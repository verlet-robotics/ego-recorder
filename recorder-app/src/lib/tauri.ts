import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppConfig,
  RecorderStatus,
  DiskInfo,
  FilesResponse,
  FileDetailResponse,
  UploadQueueItem,
  UploadProgressEvent,
  CameraInfo,
  PreviewState,
  DatasetSummary,
  ConversionProgress,
  EgorecListItem,
  SystemInfo,
} from "./types";

export const commands = {
  // Preview (unified preview + recording)
  startPreview: () => invoke<CameraInfo>("start_preview"),
  stopPreview: () => invoke<void>("stop_preview"),
  startRecording: (outputDir: string, sessionName: string, crf: number) =>
    invoke<void>("start_recording", { outputDir, sessionName, crf }),
  stopRecording: () => invoke<void>("stop_recording"),
  discardLastRecording: () => invoke<string>("discard_last_recording"),
  getPreviewState: () => invoke<PreviewState>("get_preview_state"),
  getCameraInfo: () => invoke<CameraInfo | null>("get_camera_info"),
  getPreviewUrl: (streamType: string) => invoke<string>("get_preview_url", { streamType }),
  checkCamera: () => invoke<boolean>("check_camera"),

  // Recorder (status/stats/lid-safe)
  getRecorderStatus: () => invoke<string>("get_recorder_status"),
  getRecorderStats: () => invoke<RecorderStatus>("get_recorder_stats"),
  toggleLidSafe: (enable: boolean) => invoke<boolean>("toggle_lid_safe", { enable }),

  // Library
  discoverFiles: (dir: string) => invoke<FilesResponse>("discover_files", { dir }),
  getFileMetadata: (fileName: string) => invoke<FileDetailResponse>("get_file_metadata", { fileName }),
  getVideoServerPort: () => invoke<number | null>("get_video_server_port"),
  getStreamUrl: (fileName: string) => invoke<string>("get_stream_url", { fileName }),
  watchDirectory: (dir: string) => invoke<void>("watch_directory", { dir }),
  getWatchedDir: () => invoke<string | null>("get_watched_dir"),

  // Settings
  getConfig: () => invoke<AppConfig>("get_config"),
  saveConfig: (config: AppConfig) => invoke<void>("save_config", { config }),
  isFirstRun: () => invoke<boolean>("is_first_run"),
  completeFirstRun: () => invoke<void>("complete_first_run"),
  locateBinary: () => invoke<string | null>("locate_binary"),
  testCamera: (binaryPath: string) => invoke<string>("test_camera", { binaryPath }),
  getDiskInfo: (path: string) => invoke<DiskInfo>("get_disk_info", { path }),
  getSystemInfo: () => invoke<SystemInfo>("get_system_info"),

  // Dialogs
  openDirectory: () => invoke<string | null>("open_directory"),
  selectFile: (title: string) => invoke<string | null>("select_file", { title }),

  // Upload
  queueUpload: (path: string) => invoke<void>("queue_upload", { path }),
  getUploadQueue: () => invoke<UploadQueueItem[]>("get_upload_queue"),
  retryFailed: () => invoke<void>("retry_failed"),
  cancelUpload: (filename: string) => invoke<void>("cancel_upload", { filename }),
  testUploadConnection: () => invoke<string>("test_upload_connection"),
  toggleAutoUpload: (enable: boolean) => invoke<void>("toggle_auto_upload", { enable }),

  // Datasets
  listDatasets: () => invoke<DatasetSummary[]>("list_datasets"),
  createDataset: (name: string, targetEpisodes?: number | null) =>
    invoke<DatasetSummary>("create_dataset", { name, targetEpisodes: targetEpisodes ?? null }),
  updateDataset: (dirName: string, name: string, description: string, tags: string[]) =>
    invoke<void>("update_dataset", { dirName, name, description, tags }),
  deleteDataset: (dirName: string) => invoke<void>("delete_dataset", { dirName }),
  getDatasetFiles: (dirName: string) => invoke<EgorecListItem[]>("get_dataset_files", { dirName }),
  uploadDataset: (dirName: string) => invoke<number>("upload_dataset", { dirName }),
  convertDataset: (dirName: string) => invoke<void>("convert_dataset", { dirName }),
  getConversionStatus: () => invoke<ConversionProgress | null>("get_conversion_status"),
};

// Event listeners
export function onRecorderStats(callback: (stats: RecorderStatus) => void): Promise<UnlistenFn> {
  return listen<RecorderStatus>("recorder:stats", (event) => callback(event.payload));
}

export function onRecorderStopped(callback: (reason: string) => void): Promise<UnlistenFn> {
  return listen<string>("recorder:stopped", (event) => callback(event.payload));
}

export function onPreviewStateChanged(callback: (state: PreviewState) => void): Promise<UnlistenFn> {
  return listen<PreviewState>("preview:state-changed", (event) => callback(event.payload));
}

export function onPreviewDisconnected(callback: () => void): Promise<UnlistenFn> {
  return listen("preview:disconnected", () => callback());
}

export function onCameraInfo(callback: (info: CameraInfo) => void): Promise<UnlistenFn> {
  return listen<CameraInfo>("preview:camera-info", (event) => callback(event.payload));
}

export function onUsbWarning(callback: (message: string) => void): Promise<UnlistenFn> {
  return listen<string>("preview:usb-warning", (event) => callback(event.payload));
}

export function onUploadProgress(callback: (progress: UploadProgressEvent) => void): Promise<UnlistenFn> {
  return listen<UploadProgressEvent>("upload:progress", (event) => callback(event.payload));
}

export function onConversionProgress(callback: (progress: ConversionProgress) => void): Promise<UnlistenFn> {
  return listen<ConversionProgress>("dataset:convert_progress", (event) => callback(event.payload));
}

export function onFileAdded(callback: (item: EgorecListItem) => void): Promise<UnlistenFn> {
  return listen<EgorecListItem>("library:file-added", (event) => callback(event.payload));
}

export function onFileRemoved(callback: (name: string) => void): Promise<UnlistenFn> {
  return listen<string>("library:file-removed", (event) => callback(event.payload));
}

export function onCameraConnected(callback: (connected: boolean) => void): Promise<UnlistenFn> {
  return listen<boolean>("camera:connected", (event) => callback(event.payload));
}
