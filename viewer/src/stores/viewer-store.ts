import { create } from "zustand";
import type {
  AnalysisResult,
  AnalysisStatus,
  ConversionStatus,
  EgorecListItem,
} from "@/lib/types";

interface ViewerState {
  dir: string;
  files: EgorecListItem[];
  currentFile: string | null;
  conversionStatus: Record<string, ConversionStatus>;

  // Analysis state
  analysisStatus: AnalysisStatus;
  analysisResults: Record<string, AnalysisResult>;
  analysisError: string | null;

  setDir(dir: string): void;
  setFiles(files: EgorecListItem[]): void;
  selectFile(name: string): void;
  setConversionStatus(name: string, status: ConversionStatus): void;

  setAnalysisStatus(status: AnalysisStatus): void;
  setAnalysisResults(results: AnalysisResult[]): void;
  setAnalysisError(error: string | null): void;
  clearAnalysis(): void;
  removeFile(name: string): void;
  addFiles(files: EgorecListItem[]): void;
}

export const useViewerStore = create<ViewerState>((set) => ({
  dir: "",
  files: [],
  currentFile: null,
  conversionStatus: {},

  analysisStatus: "idle",
  analysisResults: {},
  analysisError: null,

  setDir: (dir) => set({ dir }),

  setFiles: (files) =>
    set({
      files,
      conversionStatus: Object.fromEntries(
        files.map((f) => [f.name, f.conversionStatus]),
      ),
    }),

  selectFile: (name) => set({ currentFile: name }),

  setConversionStatus: (name, status) =>
    set((state) => ({
      conversionStatus: { ...state.conversionStatus, [name]: status },
    })),

  setAnalysisStatus: (status) => set({ analysisStatus: status }),

  setAnalysisResults: (results) =>
    set({
      analysisResults: Object.fromEntries(
        results.map((r) => [r.filename, r]),
      ),
    }),

  setAnalysisError: (error) => set({ analysisError: error }),

  clearAnalysis: () =>
    set({ analysisStatus: "idle", analysisResults: {}, analysisError: null }),

  removeFile: (name) =>
    set((state) => {
      const files = state.files.filter((f) => f.name !== name);
      const { [name]: _, ...conversionStatus } = state.conversionStatus;
      const { [name]: __, ...analysisResults } = state.analysisResults;
      const currentFile =
        state.currentFile === name
          ? files.length > 0
            ? files[0]!.name
            : null
          : state.currentFile;
      return { files, conversionStatus, analysisResults, currentFile };
    }),

  addFiles: (newFiles) =>
    set((state) => {
      const existingNames = new Set(state.files.map((f) => f.name));
      const toAdd = newFiles.filter((f) => !existingNames.has(f.name));
      return {
        files: [...state.files, ...toAdd].sort((a, b) =>
          a.name.localeCompare(b.name),
        ),
        conversionStatus: {
          ...state.conversionStatus,
          ...Object.fromEntries(
            toAdd.map((f) => [f.name, f.conversionStatus]),
          ),
        },
      };
    }),
}));
