import { useState, useEffect } from "react";
import { cn } from "@/lib/utils";
import { useAppStore } from "@/stores/app-store";
import { commands } from "@/lib/tauri";
import type { Page } from "@/lib/types";
import { useDatasetStore } from "@/stores/dataset-store";
import {
  Video,
  Library,
  Database,
  Upload,
  Settings,
  Lock,
} from "lucide-react";

const NAV_ITEMS: { page: Page; label: string; icon: typeof Video }[] = [
  { page: "record", label: "Record", icon: Video },
  { page: "library", label: "Library", icon: Library },
  { page: "datasets", label: "Datasets", icon: Database },
  { page: "upload", label: "Upload", icon: Upload },
  { page: "settings", label: "Settings", icon: Settings },
];

export function Sidebar() {
  const page = useAppStore((s) => s.page);
  const setPage = useAppStore((s) => s.setPage);
  const lidSafe = useAppStore((s) => s.lidSafe);
  const [uploadCount, setUploadCount] = useState(0);
  const datasetCount = useDatasetStore((s) => s.datasets.length);

  useEffect(() => {
    const poll = () => {
      commands.getUploadQueue().then((queue) => {
        const active = queue.filter(
          (i) => i.status.kind === "pending" || i.status.kind === "uploading" || i.status.kind === "hashing",
        ).length;
        setUploadCount(active);
      }).catch(() => {});
    };
    poll();
    const interval = setInterval(poll, 5000);
    return () => clearInterval(interval);
  }, []);

  return (
    <aside className="w-14 border-r border-border flex flex-col items-center py-3 gap-1 bg-sidebar shrink-0">
      {NAV_ITEMS.map(({ page: p, label, icon: Icon }) => (
        <button
          key={p}
          onClick={() => setPage(p)}
          className={cn(
            "relative flex flex-col items-center gap-0.5 px-2 py-1.5 rounded-lg transition-colors w-11",
            page === p
              ? "bg-sidebar-accent text-sidebar-primary"
              : "text-sidebar-foreground/60 hover:text-sidebar-foreground hover:bg-sidebar-accent/50",
          )}
          title={label}
        >
          <Icon className="size-4" />
          <span className="text-[9px] font-medium">{label}</span>
          {p === "upload" && uploadCount > 0 && (
            <span className="absolute -top-0.5 -right-0.5 size-3.5 rounded-full bg-highlight text-[8px] font-bold text-highlight-foreground flex items-center justify-center">
              {uploadCount > 9 ? "9+" : uploadCount}
            </span>
          )}
          {p === "datasets" && datasetCount > 0 && (
            <span className="absolute -top-0.5 -right-0.5 size-3.5 rounded-full bg-surface text-[8px] font-bold text-muted-foreground flex items-center justify-center">
              {datasetCount > 9 ? "9+" : datasetCount}
            </span>
          )}
        </button>
      ))}

      <div className="mt-auto">
        {lidSafe && (
          <div className="flex flex-col items-center gap-0.5 px-2 py-1.5 text-warning" title="Lid-close safe mode active">
            <Lock className="size-4" />
            <span className="text-[9px] font-medium">Safe</span>
          </div>
        )}
      </div>
    </aside>
  );
}
