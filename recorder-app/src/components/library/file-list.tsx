import { useMemo, useState } from "react";
import { useRecorderStore } from "@/stores/recorder-store";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { Circle, AlertCircle, Play, FolderOpen, Trash2 } from "lucide-react";
import { commands } from "@/lib/tauri";
import type { ConversionStatus, EgorecListItem } from "@/lib/types";

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

/** Extract the display filename (last segment) from a possibly nested name. */
function displayName(name: string): string {
  const idx = name.lastIndexOf("/");
  return idx === -1 ? name : name.slice(idx + 1);
}

function Thumbnail({ file, port }: { file: EgorecListItem; port: number | null }) {
  if (!port || file.conversionStatus !== "streamable") {
    return (
      <div className="w-16 h-10 rounded bg-muted/50 flex items-center justify-center shrink-0">
        <Circle className="size-3 text-muted-foreground/30" />
      </div>
    );
  }

  const src = `http://localhost:${port}/stream/${encodeURIComponent(file.name)}`;

  return (
    <video
      src={src}
      preload="metadata"
      muted
      playsInline
      className="w-16 h-10 rounded bg-black object-cover shrink-0"
    />
  );
}

function FileItem({
  file,
  isSelected,
  status,
  port,
  onSelect,
  onDelete,
}: {
  file: EgorecListItem;
  isSelected: boolean;
  status: ConversionStatus;
  port: number | null;
  onSelect: () => void;
  onDelete: () => void;
}) {
  return (
    <div
      className={cn(
        "group flex items-start gap-2 px-2 py-1.5 rounded-lg transition-colors cursor-pointer",
        isSelected
          ? "bg-accent text-accent-foreground"
          : "hover:bg-hover text-foreground",
      )}
      onClick={onSelect}
    >
      <Thumbnail file={file} port={port} />

      <div className="flex-1 min-w-0 flex flex-col gap-0.5">
        <div className="flex items-center gap-1.5 min-w-0">
          <StatusIcon status={status} />
          <span className="text-[11px] font-medium truncate flex-1">
            {displayName(file.name)}
          </span>
          <Button
            variant="ghost"
            size="icon"
            className="size-5 opacity-0 group-hover:opacity-100 shrink-0 text-muted-foreground hover:text-destructive"
            onClick={(e) => { e.stopPropagation(); onDelete(); }}
          >
            <Trash2 className="size-3" />
          </Button>
        </div>

        <div className="flex items-center gap-1.5 flex-wrap">
          <Badge variant="inline">
            {formatDuration(file.durationS)}
          </Badge>
          <Badge variant="inline">
            {file.totalFrames}f
          </Badge>
          <Badge variant="inline">
            {formatSize(file.sizeBytes)}
          </Badge>
        </div>
      </div>
    </div>
  );
}

export function FileList() {
  const files = useRecorderStore((s) => s.files);
  const currentFile = useRecorderStore((s) => s.currentFile);
  const conversionStatus = useRecorderStore((s) => s.conversionStatus);
  const selectFile = useRecorderStore((s) => s.selectFile);
  const removeFile = useRecorderStore((s) => s.removeFile);
  const videoServerPort = useRecorderStore((s) => s.videoServerPort);
  const [deleting, setDeleting] = useState<string | null>(null);

  const handleDelete = async (file: EgorecListItem) => {
    if (deleting) return;
    setDeleting(file.name);
    try {
      await commands.deleteLibraryFile(file.name);
      removeFile(file.name);
    } catch (err) {
      console.error("Failed to delete:", err);
    } finally {
      setDeleting(null);
    }
  };

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
            {/* Dataset header -- only show when files span multiple directories */}
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
                port={videoServerPort}
                onSelect={() => selectFile(file.name)}
                onDelete={() => handleDelete(file)}
              />
            ))}
          </div>
        ))}
      </div>
    </ScrollArea>
  );
}
