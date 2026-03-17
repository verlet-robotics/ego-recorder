import { useState, useEffect, useCallback } from "react";
import { commands, onUploadProgress } from "@/lib/tauri";
import type { UploadQueueItem } from "@/lib/types";
import { useAppStore } from "@/stores/app-store";
import { ArrowUpCircle, CheckCircle2, XCircle, Clock, Loader2 } from "lucide-react";

export function UploadStatusBar() {
  const setPage = useAppStore((s) => s.setPage);
  const [queue, setQueue] = useState<UploadQueueItem[]>([]);

  const refresh = useCallback(async () => {
    try {
      const items = await commands.getUploadQueue();
      setQueue(items);
    } catch {
      // silently ignore
    }
  }, []);

  useEffect(() => {
    refresh();
    const interval = setInterval(refresh, 5000);
    return () => clearInterval(interval);
  }, [refresh]);

  useEffect(() => {
    const unlisten = onUploadProgress(() => refresh());
    return () => { unlisten.then((fn) => fn()); };
  }, [refresh]);

  const pending = queue.filter((i) => i.status.kind === "pending").length;
  const active = queue.filter((i) => i.status.kind === "uploading" || i.status.kind === "hashing").length;
  const completed = queue.filter((i) => i.status.kind === "completed").length;
  const failed = queue.filter((i) => i.status.kind === "failed").length;

  const total = pending + active + completed + failed;
  if (total === 0) return null;

  // Find a file currently uploading to show progress
  const uploading = queue.find((i) => i.status.kind === "uploading");
  const hashing = queue.find((i) => i.status.kind === "hashing");
  const currentFile = uploading ?? hashing;

  return (
    <button
      onClick={() => setPage("upload")}
      className="w-full flex items-center gap-3 rounded-lg border border-border bg-card px-3 py-2 text-left hover:bg-accent/40 transition-colors cursor-pointer"
    >
      {/* Activity indicator */}
      {active > 0 ? (
        <Loader2 className="size-4 text-highlight animate-spin shrink-0" />
      ) : failed > 0 ? (
        <XCircle className="size-4 text-destructive shrink-0" />
      ) : (
        <CheckCircle2 className="size-4 text-success shrink-0" />
      )}

      {/* Current file progress */}
      <div className="flex-1 min-w-0">
        {currentFile ? (
          <>
            <p className="text-[11px] font-mono truncate text-foreground/80">
              {currentFile.filename.split("/").pop()}
            </p>
            {currentFile.status.kind === "uploading" && (
              <div className="flex items-center gap-2 mt-0.5">
                <div className="flex-1 h-1 rounded-full bg-muted overflow-hidden">
                  <div
                    className="h-full rounded-full bg-highlight transition-all duration-300"
                    style={{ width: `${Math.min(currentFile.status.progress * 100, 100)}%` }}
                  />
                </div>
                <span className="text-[9px] text-muted-foreground tabular-nums shrink-0">
                  {Math.round(currentFile.status.progress * 100)}%
                </span>
              </div>
            )}
            {currentFile.status.kind === "hashing" && (
              <p className="text-[9px] text-muted-foreground mt-0.5">Hashing...</p>
            )}
          </>
        ) : (
          <p className="text-[11px] text-muted-foreground">Uploads idle</p>
        )}
      </div>

      {/* Summary counters */}
      <div className="flex items-center gap-2.5 text-[10px] text-muted-foreground shrink-0">
        {active > 0 && (
          <span className="flex items-center gap-0.5">
            <ArrowUpCircle className="size-3 text-highlight" />
            {active}
          </span>
        )}
        {pending > 0 && (
          <span className="flex items-center gap-0.5">
            <Clock className="size-3" />
            {pending}
          </span>
        )}
        {completed > 0 && (
          <span className="flex items-center gap-0.5">
            <CheckCircle2 className="size-3 text-success" />
            {completed}
          </span>
        )}
        {failed > 0 && (
          <span className="flex items-center gap-0.5">
            <XCircle className="size-3 text-destructive" />
            {failed}
          </span>
        )}
      </div>
    </button>
  );
}
