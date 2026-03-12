import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AnalysisResult,
  ConversionStatus,
  CurationWorkspaceInfo,
  EgorecListItem,
  EgorecMetadata,
  FileDetailResponse,
  FilesResponse,
  RecentWorkspace,
  WorkspaceSummary,
} from "./types";

// ── Tauri Commands ───────────────────────────────────────────────────────────

export const commands = {
  openDirectory: () => invoke<string | null>("open_directory"),

  getRecordingsDir: () => invoke<string | null>("get_recordings_dir"),

  setRecordingsDir: (dir: string) =>
    invoke<void>("set_recordings_dir", { dir }),

  discoverFiles: () => invoke<FilesResponse>("discover_files"),

  listFiles: () => invoke<EgorecListItem[]>("list_files"),

  getFileMetadata: (name: string) =>
    invoke<FileDetailResponse>("get_file_metadata", { name }),

  runAnalysis: () =>
    invoke<{ status: string; results?: AnalysisResult[]; error?: string }>(
      "run_analysis",
    ),

  getAnalysis: () =>
    invoke<{ status: string; results?: AnalysisResult[]; error?: string }>(
      "get_analysis",
    ),

  pruneFile: (name: string) =>
    invoke<{ status: string; name: string }>("prune_file", { name }),

  spliceFile: (
    name: string,
    opts?: {
      minGap?: number;
      minDuration?: number;
      replaceOriginal?: boolean;
    },
  ) =>
    invoke<{
      status: string;
      name: string;
      segments: string[];
      newFiles: EgorecListItem[];
      originalRemoved: boolean;
    }>("splice_file", {
      name,
      minGap: opts?.minGap ?? null,
      minDuration: opts?.minDuration ?? null,
      replaceOriginal: opts?.replaceOriginal ?? false,
    }),

  restoreFile: (name: string) =>
    invoke<{ status: string; file: EgorecListItem }>("restore_file", { name }),

  listPruned: () => invoke<string[]>("list_pruned"),

  getVideoServerPort: () => invoke<number | null>("get_video_server_port"),

  getStreamUrl: (name: string) =>
    invoke<string | null>("get_stream_url", { name }),

  getCurationStreamUrl: (sourceKey: string) =>
    invoke<string | null>("get_curation_stream_url", { sourceKey }),

  getCurationWorkspace: () =>
    invoke<CurationWorkspaceInfo>("get_curation_workspace"),

  setCurationRoot: (dir: string) =>
    invoke<CurationWorkspaceInfo>("set_curation_root", { dir }),

  listWorkspaces: () => invoke<WorkspaceSummary[]>("list_workspaces"),

  setActiveWorkspace: (name: string) =>
    invoke<CurationWorkspaceInfo>("set_active_workspace", { name }),

  setCurationWorkspace: (workspace: string) =>
    invoke<void>("set_curation_workspace", { workspace }),

  runCurationJob: (
    stage: string,
    sourcePrefix?: string | null,
    publishPrefix?: string | null,
  ) =>
    invoke<string>("run_curation_job", {
      stage,
      sourcePrefix: sourcePrefix ?? null,
      publishPrefix: publishPrefix ?? null,
    }),

  readCurationData: (dataType: string) =>
    invoke<unknown>("read_curation_data", { dataType }),

  writeCurationOverride: (
    overrideType: string,
    id: string,
    data: unknown,
  ) =>
    invoke<void>("write_curation_override", { overrideType, id, data }),

  getRecentWorkspaces: () =>
    invoke<RecentWorkspace[]>("get_recent_workspaces"),

  removeRecentWorkspace: (path: string) =>
    invoke<void>("remove_recent_workspace", { path }),

  updateRecentWorkspaceAlias: (path: string, alias: string | null) =>
    invoke<void>("update_recent_workspace_alias", { path, alias }),
};

// ── Tauri Events ─────────────────────────────────────────────────────────────

export interface AnalysisProgressPayload {
  current: number;
  total: number;
  file: string;
}

export const EVENTS = {
  ANALYSIS_PROGRESS: "analysis:progress",
  PIPELINE_PROGRESS: "pipeline:progress",
  MENU_OPEN_DIRECTORY: "menu:open_directory",
} as const;

export function onAnalysisProgress(
  callback: (payload: AnalysisProgressPayload) => void,
): Promise<UnlistenFn> {
  return listen<AnalysisProgressPayload>(EVENTS.ANALYSIS_PROGRESS, (event) => {
    callback(event.payload);
  });
}

export interface PipelineProgressPayload {
  stage: string;
  current: number;
  total: number;
  file: string;
}

export function onPipelineProgress(
  callback: (payload: PipelineProgressPayload) => void,
): Promise<UnlistenFn> {
  return listen<PipelineProgressPayload>(EVENTS.PIPELINE_PROGRESS, (event) => {
    callback(event.payload);
  });
}

export function onMenuOpenDirectory(callback: () => void): Promise<UnlistenFn> {
  return listen(EVENTS.MENU_OPEN_DIRECTORY, () => {
    callback();
  });
}
