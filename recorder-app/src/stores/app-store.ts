import { create } from "zustand";
import type { AppConfig, Page } from "@/lib/types";

interface AppState {
  page: Page;
  config: AppConfig | null;
  firstRun: boolean;
  lidSafe: boolean;
  uploadEnabled: boolean;

  setPage: (page: Page) => void;
  setConfig: (config: AppConfig) => void;
  setFirstRun: (firstRun: boolean) => void;
  setLidSafe: (lidSafe: boolean) => void;
  setUploadEnabled: (enabled: boolean) => void;
}

export const useAppStore = create<AppState>((set) => ({
  page: "record",
  config: null,
  firstRun: false,
  lidSafe: false,
  uploadEnabled: false,

  setPage: (page) => set({ page }),
  setConfig: (config) => set({ config, uploadEnabled: config?.upload.auto_upload ?? false }),
  setFirstRun: (firstRun) => set({ firstRun }),
  setLidSafe: (lidSafe) => set({ lidSafe }),
  setUploadEnabled: (uploadEnabled) => set({ uploadEnabled }),
}));
