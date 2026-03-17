import { create } from "zustand";
import type { DatasetSummary, ConversionProgress, UploadQueueItem } from "@/lib/types";

/** Per-dataset upload stats derived from the upload queue */
export interface DatasetUploadStats {
  totalFiles: number;
  completedFiles: number;
  failedFiles: number;
  activeFile: string | null;
  activeProgress: number;
  speedBps: number;
  /** Total bytes across all queued files for this dataset */
  totalBytes: number;
  /** Bytes already uploaded (completed files + active progress) */
  bytesUploaded: number;
}

interface DatasetState {
  datasets: DatasetSummary[];
  selectedDataset: string | null;
  conversionProgress: ConversionProgress | null;
  loading: boolean;
  /** Upload stats keyed by dataset dirName */
  uploadStats: Record<string, DatasetUploadStats>;

  setDatasets: (datasets: DatasetSummary[]) => void;
  selectDataset: (dirName: string | null) => void;
  setConversionProgress: (progress: ConversionProgress | null) => void;
  setLoading: (loading: boolean) => void;
  updateUploadStats: (queue: UploadQueueItem[]) => void;
}

export const useDatasetStore = create<DatasetState>((set) => ({
  datasets: [],
  selectedDataset: null,
  conversionProgress: null,
  loading: false,
  uploadStats: {},

  setDatasets: (datasets) => set({ datasets }),
  selectDataset: (dirName) => set({ selectedDataset: dirName }),
  setConversionProgress: (progress) => set({ conversionProgress: progress }),
  setLoading: (loading) => set({ loading }),
  updateUploadStats: (queue) => {
    const stats: Record<string, DatasetUploadStats> = {};

    for (const item of queue) {
      // Extract dataset dir from filename (e.g. "Bathroom-cleaning/ep_001.egorec")
      const slashIdx = item.filename.indexOf("/");
      if (slashIdx < 0) continue;
      const dirName = item.filename.slice(0, slashIdx);

      if (!stats[dirName]) {
        stats[dirName] = {
          totalFiles: 0,
          completedFiles: 0,
          failedFiles: 0,
          activeFile: null,
          activeProgress: 0,
          speedBps: 0,
          totalBytes: 0,
          bytesUploaded: 0,
        };
      }

      const s = stats[dirName];
      s.totalFiles++;
      s.totalBytes += item.sizeBytes;

      if (item.status.kind === "completed") {
        s.completedFiles++;
        s.bytesUploaded += item.sizeBytes;
      } else if (item.status.kind === "failed") {
        s.failedFiles++;
      } else if (item.status.kind === "uploading") {
        s.activeFile = item.filename.slice(slashIdx + 1);
        s.activeProgress = item.status.progress;
        s.speedBps = item.status.speedBps;
        s.bytesUploaded += Math.round(item.sizeBytes * item.status.progress);
      } else if (item.status.kind === "hashing") {
        s.activeFile = item.filename.slice(slashIdx + 1);
        s.activeProgress = 0;
        s.speedBps = 0;
      }
    }

    set({ uploadStats: stats });
  },
}));
