import type { DiskInfo } from "@/lib/types";
import { cn } from "@/lib/utils";

interface DiskBarProps {
  diskInfo: DiskInfo;
  threshold: number;
}

export function DiskBar({ diskInfo, threshold }: DiskBarProps) {
  const isBelowThreshold = diskInfo.freeMb < threshold;
  const usagePct = Math.min(diskInfo.usagePercent, 100);

  return (
    <div className="space-y-1.5">
      <div className="flex items-center justify-between text-[10px] text-muted-foreground">
        <span>Disk Space</span>
        <span className={cn("font-mono", isBelowThreshold && "text-destructive font-semibold")}>
          {formatSize(diskInfo.freeBytes)} free of {formatSize(diskInfo.totalBytes)}
        </span>
      </div>
      <div className="h-2 bg-surface rounded-full overflow-hidden">
        <div
          className={cn(
            "h-full rounded-full transition-[width] duration-500",
            isBelowThreshold ? "bg-destructive" : usagePct > 80 ? "bg-warning" : "bg-success",
          )}
          style={{ width: `${usagePct}%` }}
        />
      </div>
      {isBelowThreshold && (
        <div className="text-[10px] text-destructive font-medium">
          Low disk space! Recording may stop below {threshold} MB free.
        </div>
      )}
    </div>
  );
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}
