import { useState, useEffect, useCallback, useMemo } from "react";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { useDatasetStore } from "@/stores/dataset-store";
import { commands, onConversionProgress, onUploadProgress } from "@/lib/tauri";
import { DatasetCard } from "./dataset-card";
import { CreateDatasetForm } from "./create-dataset-form";
import { Plus, Database, Trash2 } from "lucide-react";

export function DatasetsPage() {
  const datasets = useDatasetStore((s) => s.datasets);
  const setDatasets = useDatasetStore((s) => s.setDatasets);
  const conversionProgress = useDatasetStore((s) => s.conversionProgress);
  const setConversionProgress = useDatasetStore((s) => s.setConversionProgress);
  const loading = useDatasetStore((s) => s.loading);
  const setLoading = useDatasetStore((s) => s.setLoading);

  const uploadStats = useDatasetStore((s) => s.uploadStats);
  const updateUploadStats = useDatasetStore((s) => s.updateUploadStats);

  const [showCreateForm, setShowCreateForm] = useState(false);
  const [showClearConfirm, setShowClearConfirm] = useState(false);
  const [clearing, setClearing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refreshUploadQueue = useCallback(async () => {
    try {
      const queue = await commands.getUploadQueue();
      updateUploadStats(queue);
    } catch {}
  }, [updateUploadStats]);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const result = await commands.listDatasets();
      setDatasets(result);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }, [setDatasets, setLoading]);

  // Load datasets on mount
  useEffect(() => {
    refresh();
  }, [refresh]);

  // Poll upload queue + subscribe to progress events for real-time stats
  useEffect(() => {
    refreshUploadQueue();
    const interval = setInterval(refreshUploadQueue, 3000);
    const unlistenUpload = onUploadProgress(() => refreshUploadQueue());
    return () => {
      clearInterval(interval);
      unlistenUpload.then((fn) => fn());
    };
  }, [refreshUploadQueue]);

  // Subscribe to conversion progress events
  useEffect(() => {
    const unsubs: (() => void)[] = [];
    onConversionProgress((progress) => {
      setConversionProgress(progress);
      if (progress.phase === "completed" || progress.phase === "error") {
        // Refresh to update hasLerobot status
        refresh();
        // Clear progress after a delay
        setTimeout(() => setConversionProgress(null), 3000);
      }
    }).then((u) => unsubs.push(u));

    // Check if there's an existing conversion running
    commands.getConversionStatus().then((status) => {
      if (status && status.phase !== "completed" && status.phase !== "error") {
        setConversionProgress(status);
      }
    });

    return () => unsubs.forEach((u) => u());
  }, [setConversionProgress, refresh]);

  const handleCreate = async (name: string, description: string, targetEpisodes: number | null) => {
    setError(null);
    try {
      await commands.createDataset(name, targetEpisodes);
      if (description) {
        // Update with description — the create command returns a summary
        // but we need the dir_name to update. Refresh instead.
      }
      setShowCreateForm(false);
      refresh();
    } catch (err) {
      setError(String(err));
    }
  };

  const handleDelete = async (dirName: string) => {
    setError(null);
    try {
      await commands.deleteDataset(dirName);
      refresh();
    } catch (err) {
      setError(String(err));
    }
  };

  const handleUpload = async (dirName: string) => {
    setError(null);
    try {
      const count = await commands.uploadDataset(dirName);
      if (count === 0) {
        setError("No new files to upload in this dataset.");
      } else {
        refreshUploadQueue();
      }
    } catch (err) {
      setError(String(err));
    }
  };

  const handleConvert = async (dirName: string) => {
    setError(null);
    try {
      await commands.convertDataset(dirName);
    } catch (err) {
      setError(String(err));
    }
  };

  const uploadedDatasets = useMemo(
    () => datasets.filter((ds) => ds.fileCount > 0 && ds.uploadedCount >= ds.fileCount),
    [datasets],
  );

  const handleClearUploaded = async () => {
    setError(null);
    setClearing(true);
    try {
      await commands.clearUploadedDatasets();
      setShowClearConfirm(false);
      refresh();
    } catch (err) {
      setError(String(err));
    } finally {
      setClearing(false);
    }
  };

  return (
    <div className="flex flex-col h-full p-6 gap-4">
      {/* Header */}
      <div className="flex items-center justify-between">
        <h1 className="text-xl font-semibold">Datasets</h1>
        <div className="flex items-center gap-2">
          {uploadedDatasets.length > 0 && (
            <Button
              variant="outline"
              size="sm"
              className="gap-1.5 text-destructive hover:text-destructive"
              onClick={() => setShowClearConfirm(true)}
            >
              <Trash2 className="size-3.5" />
              Clear Uploaded
            </Button>
          )}
          <Button
            size="sm"
            className="gap-1.5"
            onClick={() => setShowCreateForm(true)}
            disabled={showCreateForm}
          >
            <Plus className="size-3.5" />
            New Dataset
          </Button>
        </div>
      </div>

      {/* Error display */}
      {error && (
        <div className="rounded-lg bg-destructive-soft px-4 py-2 text-sm text-destructive">
          {error}
        </div>
      )}

      {/* Create form */}
      {showCreateForm && (
        <CreateDatasetForm
          onSubmit={handleCreate}
          onCancel={() => setShowCreateForm(false)}
        />
      )}

      {/* Dataset list */}
      <div className="flex-1 overflow-y-auto space-y-3">
        {loading && datasets.length === 0 ? (
          <div className="flex items-center justify-center h-32 text-muted-foreground text-sm">
            Loading datasets...
          </div>
        ) : datasets.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-64 text-muted-foreground gap-3">
            <Database className="size-10 opacity-30" />
            <p className="text-sm">No datasets yet</p>
            <Button
              size="sm"
              variant="outline"
              className="gap-1.5"
              onClick={() => setShowCreateForm(true)}
            >
              <Plus className="size-3.5" />
              Create your first dataset
            </Button>
          </div>
        ) : (
          datasets.map((ds) => (
            <DatasetCard
              key={ds.dirName}
              dataset={ds}
              conversionProgress={conversionProgress}
              uploadStats={uploadStats[ds.dirName] ?? null}
              onUpload={() => handleUpload(ds.dirName)}
              onConvert={() => handleConvert(ds.dirName)}
              onDelete={() => handleDelete(ds.dirName)}
            />
          ))
        )}
      </div>

      {/* Clear uploaded datasets confirmation */}
      <ConfirmDialog
        open={showClearConfirm}
        title="Clear uploaded datasets?"
        description={`This will permanently delete ${uploadedDatasets.length} dataset${uploadedDatasets.length !== 1 ? "s" : ""} from this device. All files have already been uploaded to R2.`}
        confirmLabel="Delete All"
        destructive
        loading={clearing}
        onConfirm={handleClearUploaded}
        onCancel={() => setShowClearConfirm(false)}
      >
        <div className="max-h-40 overflow-y-auto space-y-1">
          {uploadedDatasets.map((ds) => (
            <div
              key={ds.dirName}
              className="flex items-center justify-between rounded px-2 py-1 text-sm bg-surface"
            >
              <span className="truncate">{ds.name}</span>
              <span className="text-xs text-muted-foreground shrink-0 ml-2">
                {ds.fileCount} ep{ds.fileCount !== 1 ? "s" : ""}
              </span>
            </div>
          ))}
        </div>
      </ConfirmDialog>
    </div>
  );
}
