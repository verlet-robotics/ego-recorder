import { useMemo } from "react";
import { useViewerStore } from "@/stores/viewer-store";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import { Circle, AlertCircle, Play, FolderOpen } from "lucide-react";
import type { ConversionStatus, EgorecListItem, Verdict } from "@/lib/types";

function formatDuration(seconds: number): string {
  if (!isFinite(seconds) || seconds <= 0) return "0:00";
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function StatusIcon({ status }: { status: ConversionStatus }) {
  switch (status) {
    case "streamable":
      return <Play className="size-3 text-success shrink-0" />;
    case "error":
      return <AlertCircle className="size-3 text-destructive shrink-0" />;
    default:
      return <Circle className="size-3 text-muted-foreground/40 shrink-0" />;
  }
}

const VERDICT_BORDER: Record<Verdict, string> = {
  Keep: "border-l-success",
  PruneConfident: "border-l-destructive",
  PruneSuggested: "border-l-warning",
  Review: "border-l-info",
};

function MiniScoreBar({ score }: { score: number }) {
  const pct = Math.round(score * 100);
  const color =
    score >= 0.6
      ? "bg-success"
      : score >= 0.3
        ? "bg-warning"
        : "bg-destructive";
  return (
    <div className="w-12 h-1 bg-surface rounded-full overflow-hidden">
      <div
        className={`h-full rounded-full ${color}`}
        style={{ width: `${pct}%` }}
      />
    </div>
  );
}

/** Extract the display filename (last segment) from a possibly nested name. */
function displayName(name: string): string {
  const idx = name.lastIndexOf("/");
  return idx === -1 ? name : name.slice(idx + 1);
}

function FileItem({
  file,
  isSelected,
  status,
  analysis,
  hasAnalysis,
  onSelect,
}: {
  file: EgorecListItem;
  isSelected: boolean;
  status: ConversionStatus;
  analysis?: { verdict: Verdict; activity_score: number };
  hasAnalysis: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      onClick={onSelect}
      className={cn(
        "flex flex-col gap-1 px-3 py-2 rounded-lg text-left transition-colors w-full",
        hasAnalysis && analysis && "border-l-2",
        hasAnalysis && analysis && VERDICT_BORDER[analysis.verdict],
        isSelected
          ? "bg-accent text-accent-foreground"
          : "hover:bg-hover text-foreground",
      )}
    >
      <div className="flex items-center gap-2 min-w-0">
        <StatusIcon status={status} />
        <span className="text-[12px] font-medium truncate flex-1">
          {displayName(file.name)}
        </span>
        {analysis && <MiniScoreBar score={analysis.activity_score} />}
      </div>

      {file.sessionName && (
        <span className="text-[10px] text-muted-foreground truncate pl-5">
          {file.sessionName}
        </span>
      )}

      <div className="flex items-center gap-2 pl-5 flex-wrap">
        <Badge variant="inline">
          {file.colorWidth}x{file.colorHeight}
        </Badge>
        <Badge variant="inline">
          {formatDuration(file.durationS)}
        </Badge>
        <Badge variant="inline">
          {file.totalFrames} frames
        </Badge>
        <Badge variant="inline">
          {formatSize(file.sizeBytes)}
        </Badge>
      </div>
    </button>
  );
}

export function FileList() {
  const files = useViewerStore((s) => s.files);
  const currentFile = useViewerStore((s) => s.currentFile);
  const conversionStatus = useViewerStore((s) => s.conversionStatus);
  const analysisResults = useViewerStore((s) => s.analysisResults);
  const selectFile = useViewerStore((s) => s.selectFile);

  const hasAnalysis = Object.keys(analysisResults).length > 0;

  // Group files by dataset (subdirectory)
  const groups = useMemo(() => {
    const map = new Map<string, EgorecListItem[]>();
    for (const file of files) {
      const key = file.dataset ?? "";
      const arr = map.get(key) ?? [];
      arr.push(file);
      map.set(key, arr);
    }
    // Sort groups: root ("") first, then alphabetically
    return Array.from(map.entries()).sort((a, b) => {
      if (a[0] === "") return -1;
      if (b[0] === "") return 1;
      return a[0].localeCompare(b[0]);
    });
  }, [files]);

  const hasMultipleGroups = groups.length > 1 || (groups.length === 1 && groups[0]![0] !== "");

  return (
    <ScrollArea className="h-full">
      <div className="flex flex-col gap-0.5 p-1">
        {groups.map(([dataset, groupFiles]) => (
          <div key={dataset}>
            {/* Dataset header — only show when files span multiple directories */}
            {hasMultipleGroups && (
              <div className="flex items-center gap-1.5 px-3 py-1.5 mt-1 first:mt-0">
                <FolderOpen className="size-3 text-muted-foreground shrink-0" />
                <span className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground truncate">
                  {dataset || "Root"}
                </span>
                <span className="text-[10px] text-muted-foreground/60 ml-auto tabular-nums">
                  {groupFiles.length}
                </span>
              </div>
            )}
            {groupFiles.map((file) => (
              <FileItem
                key={file.name}
                file={file}
                isSelected={file.name === currentFile}
                status={conversionStatus[file.name] ?? file.conversionStatus}
                analysis={analysisResults[file.name]}
                hasAnalysis={hasAnalysis}
                onSelect={() => selectFile(file.name)}
              />
            ))}
          </div>
        ))}
      </div>
    </ScrollArea>
  );
}
