import { create } from "zustand";
import type { RecorderStatus, DiskInfo, EgorecListItem, ConversionStatus, CameraInfo, PreviewState, DatasetSummary } from "@/lib/types";

interface RecorderState {
  status: RecorderStatus;
  diskInfo: DiskInfo | null;
  countdown: number | null; // null = not counting down, 3/2/1/0

  // Preview state
  previewState: PreviewState;
  cameraInfo: CameraInfo | null;
  previewRgbUrl: string | null;
  previewDepthUrl: string | null;

  // Library state
  libraryDir: string;
  files: EgorecListItem[];
  currentFile: string | null;
  conversionStatus: Record<string, ConversionStatus>;
  videoServerPort: number | null;

  // Dataset selection for recording
  selectedDataset: string | null;
  availableDatasets: DatasetSummary[];

  setStatus: (status: RecorderStatus) => void;
  setDiskInfo: (info: DiskInfo | null) => void;
  setCountdown: (n: number | null) => void;

  setPreviewState: (state: PreviewState) => void;
  setCameraInfo: (info: CameraInfo | null) => void;
  setPreviewUrls: (rgb: string | null, depth: string | null) => void;

  setLibraryDir: (dir: string) => void;
  setFiles: (files: EgorecListItem[]) => void;
  selectFile: (name: string) => void;
  setVideoServerPort: (port: number | null) => void;

  addOrUpdateFile: (file: EgorecListItem) => void;
  removeFile: (name: string) => void;

  setSelectedDataset: (dirName: string | null) => void;
  setAvailableDatasets: (datasets: DatasetSummary[]) => void;
}

const defaultStatus: RecorderStatus = {
  state: "idle",
  framesWritten: 0,
  framesDropped: 0,
  captureFps: 0,
  writeFps: 0,
  fileSizeMb: 0,
  elapsedSeconds: 0,
  episodeCount: 0,
  currentFile: null,
};

export const useRecorderStore = create<RecorderState>((set) => ({
  status: defaultStatus,
  diskInfo: null,
  countdown: null,

  previewState: "off",
  cameraInfo: null,
  previewRgbUrl: null,
  previewDepthUrl: null,

  libraryDir: "",
  files: [],
  currentFile: null,
  conversionStatus: {},
  videoServerPort: null,

  selectedDataset: null,
  availableDatasets: [],

  setStatus: (status) => set({ status }),
  setDiskInfo: (info) => set({ diskInfo: info }),
  setCountdown: (n) => set({ countdown: n }),

  setPreviewState: (state) => set({ previewState: state }),
  setCameraInfo: (info) => set({ cameraInfo: info }),
  setPreviewUrls: (rgb, depth) => set({ previewRgbUrl: rgb, previewDepthUrl: depth }),

  setLibraryDir: (dir) => set({ libraryDir: dir }),
  setFiles: (files) =>
    set({
      files,
      conversionStatus: Object.fromEntries(
        files.map((f) => [f.name, f.conversionStatus]),
      ),
    }),
  selectFile: (name) => set({ currentFile: name }),
  setVideoServerPort: (port) => set({ videoServerPort: port }),

  addOrUpdateFile: (file) =>
    set((state) => {
      const idx = state.files.findIndex((f) => f.name === file.name);
      const newFiles =
        idx >= 0
          ? state.files.map((f, i) => (i === idx ? file : f))
          : [...state.files, file].sort((a, b) => a.name.localeCompare(b.name));
      return {
        files: newFiles,
        conversionStatus: { ...state.conversionStatus, [file.name]: file.conversionStatus },
      };
    }),
  removeFile: (name) =>
    set((state) => {
      const { [name]: _, ...restStatus } = state.conversionStatus;
      return {
        files: state.files.filter((f) => f.name !== name),
        conversionStatus: restStatus,
        currentFile: state.currentFile === name ? null : state.currentFile,
      };
    }),

  setSelectedDataset: (dirName) => set({ selectedDataset: dirName }),
  setAvailableDatasets: (datasets) => set({ availableDatasets: datasets }),
}));
