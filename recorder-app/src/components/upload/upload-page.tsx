import { useState, useEffect, useCallback } from "react";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Badge } from "@/components/ui/badge";
import { useAppStore } from "@/stores/app-store";
import { commands, onUploadProgress } from "@/lib/tauri";
import type { UploadQueueItem, UploadQueueStatus } from "@/lib/types";
import {
  Upload,
  RefreshCw,
  Trash2,
  Wifi,
  WifiOff,
  Loader2,
  CheckCircle2,
  XCircle,
  Clock,
  Hash,
  ArrowUpCircle,
  ToggleLeft,
  ToggleRight,
  FilePlus,
} from "lucide-react";

type FilterTab = "all" | "pending" | "uploading" | "completed" | "failed";

export function UploadPage() {
  const config = useAppStore((s) => s.config);
  const uploadEnabled = useAppStore((s) => s.uploadEnabled);
  const setUploadEnabled = useAppStore((s) => s.setUploadEnabled);

  const [queue, setQueue] = useState<UploadQueueItem[]>([]);
  const [filter, setFilter] = useState<FilterTab>("all");
  const [loading, setLoading] = useState(true);
  const [testResult, setTestResult] = useState<{ ok: boolean; message: string } | null>(null);
  const [testing, setTesting] = useState(false);

  const refreshQueue = useCallback(async () => {
    try {
      const items = await commands.getUploadQueue();
      setQueue(items);
    } catch (err) {
      console.error("Failed to fetch upload queue:", err);
    }
  }, []);

  // Initial load + polling
  useEffect(() => {
    refreshQueue().then(() => setLoading(false));
    const interval = setInterval(refreshQueue, 3000);
    return () => clearInterval(interval);
  }, [refreshQueue]);

  // Listen for progress events
  useEffect(() => {
    const unlisten = onUploadProgress(() => {
      refreshQueue();
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [refreshQueue]);

  const handleToggleAutoUpload = useCallback(async () => {
    const newState = !uploadEnabled;
    await commands.toggleAutoUpload(newState);
    setUploadEnabled(newState);
  }, [uploadEnabled, setUploadEnabled]);

  const handleRetryFailed = useCallback(async () => {
    await commands.retryFailed();
    refreshQueue();
  }, [refreshQueue]);

  const handleClearCompleted = useCallback(() => {
    setQueue((prev) => prev.filter((item) => item.status.kind !== "completed"));
  }, []);

  const handleTestConnection = useCallback(async () => {
    setTesting(true);
    setTestResult(null);
    try {
      const msg = await commands.testUploadConnection();
      setTestResult({ ok: true, message: msg });
    } catch (err) {
      setTestResult({ ok: false, message: err instanceof Error ? err.message : String(err) });
    } finally {
      setTesting(false);
    }
  }, []);

  const handleManualUpload = useCallback(async () => {
    const file = await commands.selectFile("Select .egorec file to upload");
    if (file) {
      try {
        await commands.queueUpload(file);
        refreshQueue();
      } catch (err) {
        console.error("Failed to queue upload:", err);
      }
    }
  }, [refreshQueue]);

  const handleCancel = useCallback(
    async (filename: string) => {
      await commands.cancelUpload(filename);
      refreshQueue();
    },
    [refreshQueue],
  );

  const statusKind = (s: UploadQueueStatus) => s.kind;

  const filtered = queue.filter((item) => {
    if (filter === "all") return true;
    return statusKind(item.status) === filter;
  });

  const counts = {
    all: queue.length,
    pending: queue.filter((i) => i.status.kind === "pending").length,
    uploading: queue.filter((i) => i.status.kind === "uploading" || i.status.kind === "hashing").length,
    completed: queue.filter((i) => i.status.kind === "completed").length,
    failed: queue.filter((i) => i.status.kind === "failed").length,
  };

  const hasCredentials =
    config?.upload.endpoint && config?.upload.bucket && config?.upload.access_key && config?.upload.secret_key;

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full text-muted-foreground">
        <Loader2 className="size-5 animate-spin" />
      </div>
    );
  }

  return (
    <ScrollArea className="h-full">
      <div className="max-w-4xl mx-auto p-6 space-y-6">
        {/* Header */}
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-3">
            <Upload className="size-5 text-muted-foreground" />
            <h1 className="text-xl font-semibold">Uploads</h1>
          </div>
          <div className="flex items-center gap-2">
            {/* Auto-upload toggle */}
            <button
              onClick={handleToggleAutoUpload}
              className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-md text-[11px] font-medium transition-colors hover:bg-accent"
              disabled={!hasCredentials}
              title={hasCredentials ? "Toggle auto-upload" : "Configure credentials in Settings first"}
            >
              {uploadEnabled ? (
                <ToggleRight className="size-4 text-success" />
              ) : (
                <ToggleLeft className="size-4 text-muted-foreground" />
              )}
              Auto-upload
            </button>

            <Button variant="outline" size="sm" onClick={handleManualUpload} disabled={!hasCredentials}>
              <FilePlus className="size-3.5" />
              Upload File
            </Button>
          </div>
        </div>

        {/* No credentials warning */}
        {!hasCredentials && (
          <div className="rounded-lg border border-warning/30 bg-warning/5 px-4 py-3">
            <p className="text-[12px] text-warning">
              Upload credentials not configured. Go to Settings to enter your R2/S3 endpoint, bucket, and keys.
            </p>
          </div>
        )}

        {/* Connection test + stats bar */}
        <div className="flex items-center gap-3">
          <Button
            variant="outline"
            size="sm"
            onClick={handleTestConnection}
            disabled={testing || !hasCredentials}
          >
            {testing ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : testResult?.ok ? (
              <Wifi className="size-3.5 text-success" />
            ) : testResult ? (
              <WifiOff className="size-3.5 text-destructive" />
            ) : (
              <Wifi className="size-3.5" />
            )}
            Test Connection
          </Button>
          {testResult && (
            <span className={`text-[11px] ${testResult.ok ? "text-success" : "text-destructive"}`}>
              {testResult.message}
            </span>
          )}
          <div className="ml-auto flex items-center gap-3 text-[11px] text-muted-foreground">
            {counts.uploading > 0 && (
              <span className="flex items-center gap-1">
                <ArrowUpCircle className="size-3 text-highlight" />
                {counts.uploading} uploading
              </span>
            )}
            {counts.pending > 0 && (
              <span className="flex items-center gap-1">
                <Clock className="size-3" />
                {counts.pending} pending
              </span>
            )}
            {counts.completed > 0 && (
              <span className="flex items-center gap-1">
                <CheckCircle2 className="size-3 text-success" />
                {counts.completed} done
              </span>
            )}
          </div>
        </div>

        {/* Filter tabs */}
        <div className="flex items-center gap-1 border-b border-border">
          {(["all", "pending", "uploading", "completed", "failed"] as FilterTab[]).map((tab) => (
            <button
              key={tab}
              onClick={() => setFilter(tab)}
              className={`px-3 py-1.5 text-[11px] font-medium capitalize border-b-2 transition-colors ${
                filter === tab
                  ? "border-highlight text-foreground"
                  : "border-transparent text-muted-foreground hover:text-foreground"
              }`}
            >
              {tab}
              {counts[tab] > 0 && (
                <span className="ml-1.5 text-[10px] text-muted-foreground">({counts[tab]})</span>
              )}
            </button>
          ))}
          <div className="ml-auto flex gap-1.5 pb-1">
            {counts.failed > 0 && (
              <Button variant="ghost" size="sm" className="text-[11px] gap-1" onClick={handleRetryFailed}>
                <RefreshCw className="size-3" />
                Retry Failed
              </Button>
            )}
            {counts.completed > 0 && (
              <Button variant="ghost" size="sm" className="text-[11px] gap-1" onClick={handleClearCompleted}>
                <Trash2 className="size-3" />
                Clear Done
              </Button>
            )}
          </div>
        </div>

        {/* Queue list */}
        {filtered.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-16 text-muted-foreground gap-2">
            <Upload className="size-8 opacity-30" />
            <p className="text-sm">
              {queue.length === 0
                ? "No uploads yet. Record some episodes or use Upload File."
                : "No items match this filter."}
            </p>
          </div>
        ) : (
          <div className="space-y-1">
            {filtered.map((item) => (
              <UploadRow key={item.filename} item={item} onCancel={handleCancel} />
            ))}
          </div>
        )}
      </div>
    </ScrollArea>
  );
}

function UploadRow({ item, onCancel }: { item: UploadQueueItem; onCancel: (f: string) => void }) {
  const status = item.status;

  return (
    <div className="flex items-center gap-3 px-3 py-2.5 rounded-lg hover:bg-accent/30 transition-colors group">
      {/* Status icon */}
      <div className="shrink-0">
        {status.kind === "pending" && <Clock className="size-4 text-muted-foreground" />}
        {status.kind === "hashing" && <Hash className="size-4 text-highlight animate-pulse" />}
        {status.kind === "uploading" && <ArrowUpCircle className="size-4 text-highlight" />}
        {status.kind === "completed" && <CheckCircle2 className="size-4 text-success" />}
        {status.kind === "failed" && <XCircle className="size-4 text-destructive" />}
      </div>

      {/* Filename + details */}
      <div className="flex-1 min-w-0">
        <p className="text-sm font-mono truncate">{item.filename}</p>
        <div className="flex items-center gap-2 text-[10px] text-muted-foreground">
          <span>{formatBytes(item.sizeBytes)}</span>
          {status.kind === "uploading" && status.speedBps > 0 && (
            <span>{formatBytes(status.speedBps)}/s</span>
          )}
          {status.kind === "completed" && (
            <span className="font-mono truncate max-w-[200px]" title={status.sha256}>
              SHA-256: {status.sha256.slice(0, 12)}...
            </span>
          )}
          {status.kind === "failed" && (
            <span className="text-destructive truncate max-w-[300px]" title={status.error}>
              {status.error}
            </span>
          )}
        </div>
      </div>

      {/* Progress bar */}
      {(status.kind === "uploading" || status.kind === "hashing") && (
        <div className="w-32 shrink-0">
          <div className="h-1.5 rounded-full bg-muted overflow-hidden">
            <div
              className="h-full rounded-full bg-highlight transition-all duration-300"
              style={{ width: `${Math.min(status.progress * 100, 100)}%` }}
            />
          </div>
          <p className="text-[9px] text-muted-foreground mt-0.5 text-right">
            {status.kind === "hashing" ? "Hashing" : `${Math.round(status.progress * 100)}%`}
          </p>
        </div>
      )}

      {/* Status badge */}
      <div className="shrink-0">
        {status.kind === "pending" && <Badge variant="inline">Pending</Badge>}
        {status.kind === "hashing" && <Badge variant="inline" className="text-highlight">Hashing</Badge>}
        {status.kind === "uploading" && <Badge variant="inline" className="text-highlight">Uploading</Badge>}
        {status.kind === "completed" && <Badge variant="inline" className="text-success">Done</Badge>}
        {status.kind === "failed" && <Badge variant="inline" className="text-destructive">Failed</Badge>}
      </div>

      {/* Cancel button */}
      {(status.kind === "uploading" || status.kind === "hashing" || status.kind === "pending") && (
        <Button
          variant="ghost"
          size="sm"
          className="opacity-0 group-hover:opacity-100 transition-opacity size-7 p-0"
          onClick={() => onCancel(item.filename)}
          title="Cancel upload"
        >
          <XCircle className="size-3.5" />
        </Button>
      )}
    </div>
  );
}

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  return `${(bytes / Math.pow(1024, i)).toFixed(i > 0 ? 1 : 0)} ${units[i]}`;
}
