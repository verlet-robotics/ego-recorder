import { create } from "zustand";
import type { DatasetSummary, ConversionProgress } from "@/lib/types";

interface DatasetState {
  datasets: DatasetSummary[];
  selectedDataset: string | null;
  conversionProgress: ConversionProgress | null;
  loading: boolean;

  setDatasets: (datasets: DatasetSummary[]) => void;
  selectDataset: (dirName: string | null) => void;
  setConversionProgress: (progress: ConversionProgress | null) => void;
  setLoading: (loading: boolean) => void;
}

export const useDatasetStore = create<DatasetState>((set) => ({
  datasets: [],
  selectedDataset: null,
  conversionProgress: null,
  loading: false,

  setDatasets: (datasets) => set({ datasets }),
  selectDataset: (dirName) => set({ selectedDataset: dirName }),
  setConversionProgress: (progress) => set({ conversionProgress: progress }),
  setLoading: (loading) => set({ loading }),
}));
