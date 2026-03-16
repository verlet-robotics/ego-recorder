import { useEffect } from "react";
import { useAppStore } from "@/stores/app-store";
import { commands } from "@/lib/tauri";
import { Sidebar } from "@/components/layout/sidebar";
import { RecordPage } from "@/components/record/record-page";
import { LibraryPage } from "@/components/library/library-page";
import { DatasetsPage } from "@/components/datasets/datasets-page";
import { UploadPage } from "@/components/upload/upload-page";
import { SettingsPage } from "@/components/settings/settings-page";

export default function App() {
  const page = useAppStore((s) => s.page);
  const setPage = useAppStore((s) => s.setPage);
  const setConfig = useAppStore((s) => s.setConfig);
  const firstRun = useAppStore((s) => s.firstRun);
  const setFirstRun = useAppStore((s) => s.setFirstRun);

  // Load config on mount
  useEffect(() => {
    commands.getConfig().then(setConfig);
    commands.isFirstRun().then((isFirst) => {
      setFirstRun(isFirst);
      if (isFirst) setPage("settings");
    });
  }, [setConfig, setFirstRun, setPage]);

  // Block all navigation during first-run wizard
  if (firstRun) {
    return (
      <div className="flex h-screen bg-background app-gradient">
        <main className="flex-1 min-w-0 overflow-hidden">
          <SettingsPage />
        </main>
      </div>
    );
  }

  return (
    <div className="flex h-screen bg-background app-gradient">
      <Sidebar />
      <main className="flex-1 min-w-0 overflow-hidden">
        {page === "record" && <RecordPage />}
        {page === "library" && <LibraryPage />}
        {page === "datasets" && <DatasetsPage />}
        {page === "upload" && <UploadPage />}
        {page === "settings" && <SettingsPage />}
      </main>
    </div>
  );
}


