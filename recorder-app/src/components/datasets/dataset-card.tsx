import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import type { DatasetSummary, ConversionProgress } from "@/lib/types";
import type { DatasetUploadStats } from "@/stores/dataset-store";
import { cn } from "@/lib/utils";
import {
  Upload,
  ArrowRightLeft,
  Trash2,
  CheckCircle,
  FileVideo,
  Clock,
  HardDrive,
  ArrowUpCircle,
} from "lucide-react";

interface DatasetCardProps {
  dataset: DatasetSummary;
  conversionProgress: ConversionProgress | null;
  uploadStats: DatasetUploadStats | null;
  onUpload: () => void;
  onConvert: () => void;
  onDelete: () => void;
}

export function DatasetCard({
  dataset,
  conversionProgress,
  uploadStats,
  onUpload,
  onConvert,
  onDelete,
}: DatasetCardProps) {
  const isConverting =
    conversionProgress &&
    conversionProgress.datasetName === dataset.name &&
    (conversionProgress.phase === "converting" || conversionProgress.phase === "finalizing");

  const conversionPercent =
    isConverting && conversionProgress!.totalFrames > 0
      ? Math.round((conversionProgress!.framesDone / conversionProgress!.totalFrames) * 100)
      : 0;

  return (
    <div className="rounded-lg border border-border bg-card p-4 space-y-3">
      {/* Header */}
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h3 className="text-sm font-medium truncate">{dataset.name}</h3>
            {dataset.hasLerobot && (
              <Badge variant="inline" className="bg-highlight/20 text-highlight-foreground shrink-0">
                LeRobot
              </Badge>
            )}
          </div>
          {dataset.description && (
            <p className="text-xs text-muted-foreground mt-0.5 truncate">
              {dataset.description}
            </p>
          )}
        </div>
      </div>

      {/* Tags */}
      {dataset.tags.length > 0 && (
        <div className="flex gap-1 flex-wrap">
          {dataset.tags.map((tag) => (
            <Badge key={tag} variant="inline">
              {tag}
            </Badge>
          ))}
        </div>
      )}

      {/* Episode progress (prominent when target is set) */}
      {dataset.targetEpisodes && (
        <div className="space-y-1.5">
          <div className="flex items-center justify-between text-xs">
            <span className="font-medium text-foreground">
              {dataset.fileCount} / {dataset.targetEpisodes} episodes
            </span>
            <span className={cn(
              "font-mono tabular-nums",
              dataset.fileCount >= dataset.targetEpisodes
                ? "text-highlight-foreground font-semibold"
                : "text-muted-foreground"
            )}>
              {Math.min(Math.round((dataset.fileCount / dataset.targetEpisodes) * 100), 100)}%
            </span>
          </div>
          <div className="h-2 rounded-full bg-surface overflow-hidden">
            <div
              className={cn(
                "h-full rounded-full transition-all duration-300",
                dataset.fileCount >= dataset.targetEpisodes
                  ? "bg-highlight"
                  : "bg-highlight/70",
              )}
              style={{ width: `${Math.min((dataset.fileCount / dataset.targetEpisodes) * 100, 100)}%` }}
            />
          </div>
        </div>
      )}

      {/* Stats row */}
      <div className="flex items-center gap-3 text-xs text-muted-foreground">
        <span className="flex items-center gap-1">
          <FileVideo className="size-3" />
          {dataset.fileCount} ep{dataset.fileCount !== 1 ? "s" : ""}
        </span>
        <span className="flex items-center gap-1">
          <Clock className="size-3" />
          {formatDuration(dataset.totalDurationS)}
        </span>
        <span className="flex items-center gap-1">
          <HardDrive className="size-3" />
          {formatSize(dataset.totalSizeBytes)}
        </span>
        {dataset.fileCount > 0 && (() => {
          const isUploading = uploadStats && uploadStats.totalFiles > 0 && uploadStats.completedFiles < uploadStats.totalFiles;
          const overallProgress = isUploading && uploadStats.totalBytes > 0
            ? uploadStats.bytesUploaded / uploadStats.totalBytes
            : 0;
          const remainingBytes = isUploading ? uploadStats.totalBytes - uploadStats.bytesUploaded : 0;
          const etaSeconds = isUploading && uploadStats.speedBps > 0 ? remainingBytes / uploadStats.speedBps : null;

          return (
            <>
              <span
                className={cn(
                  "flex items-center gap-1",
                  dataset.uploadedCount >= dataset.fileCount
                    ? "text-highlight-foreground"
                    : isUploading
                      ? "text-highlight-foreground"
                      : "",
                )}
              >
                {isUploading ? (
                  <ArrowUpCircle className="size-3 animate-pulse" />
                ) : (
                  <Upload className="size-3" />
                )}
                {dataset.uploadedCount}/{dataset.fileCount} uploaded
                {isUploading && (
                  <span className="font-mono tabular-nums ml-0.5">
                    ({Math.round(overallProgress * 100)}%)
                  </span>
                )}
              </span>
              {isUploading && uploadStats.speedBps > 0 && (
                <span className="flex items-center gap-1 font-mono tabular-nums text-highlight-foreground">
                  {formatSize(uploadStats.speedBps)}/s
                </span>
              )}
              {isUploading && etaSeconds !== null && (
                <span className="font-mono tabular-nums">
                  ETA {formatDuration(etaSeconds)}
                </span>
              )}
              {isUploading && uploadStats.failedFiles > 0 && (
                <span className="text-destructive">
                  {uploadStats.failedFiles} failed
                </span>
              )}
            </>
          );
        })()}
      </div>

      {/* Conversion progress bar */}
      {isConverting && (
        <div className="space-y-1">
          <div className="flex items-center justify-between text-xs text-muted-foreground">
            <span>
              Converting{conversionProgress!.currentFile ? `: ${conversionProgress!.currentFile}` : "..."}
            </span>
            <span>{conversionPercent}%</span>
          </div>
          <div className="h-1.5 rounded-full bg-surface overflow-hidden">
            <div
              className="h-full rounded-full bg-highlight transition-all duration-300"
              style={{ width: `${conversionPercent}%` }}
            />
          </div>
        </div>
      )}

      {/* Actions */}
      <div className="flex items-center gap-2">
        <Button
          variant="outline"
          size="sm"
          className="gap-1.5 text-xs"
          onClick={onUpload}
          disabled={dataset.fileCount === 0 || dataset.uploadedCount >= dataset.fileCount}
        >
          <Upload className="size-3" />
          Upload All
        </Button>
        <Button
          variant="outline"
          size="sm"
          className="gap-1.5 text-xs"
          onClick={onConvert}
          disabled={dataset.fileCount === 0 || !!isConverting}
        >
          <ArrowRightLeft className="size-3" />
          {dataset.hasLerobot ? "Reconvert" : "Convert to LeRobot"}
        </Button>
        <div className="flex-1" />
        <Button
          variant="ghost"
          size="sm"
          className="gap-1.5 text-xs text-destructive hover:text-destructive"
          onClick={onDelete}
        >
          <Trash2 className="size-3" />
        </Button>
      </div>
    </div>
  );
}

function formatDuration(seconds: number): string {
  if (seconds < 60) return `${Math.round(seconds)}s`;
  const m = Math.floor(seconds / 60);
  const s = Math.round(seconds % 60);
  if (m < 60) return `${m}m ${s}s`;
  const h = Math.floor(m / 60);
  return `${h}h ${m % 60}m`;
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}
