import { useState } from "react";
import { useViewerStore } from "@/stores/viewer-store";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import { ScrollArea } from "@/components/ui/scroll-area";
import { commands } from "@/lib/tauri";
import {
  Scissors,
  Trash2,
  Undo2,
  Loader2,
  ChevronDown,
  ChevronRight,
} from "lucide-react";
import type { Verdict, EgorecListItem } from "@/lib/types";

const VERDICT_CONFIG: Record<
  Verdict,
  { label: string; className: string; dotClass: string }
> = {
  Keep: {
    label: "Keep",
    className: "bg-success-soft text-success",
    dotClass: "bg-success",
  },
  PruneConfident: {
    label: "Prune",
    className: "bg-destructive-soft text-destructive",
    dotClass: "bg-destructive",
  },
  PruneSuggested: {
    label: "Prune?",
    className: "bg-warning-soft text-warning",
    dotClass: "bg-warning",
  },
  Review: {
    label: "Review",
    className: "bg-info-soft text-info",
    dotClass: "bg-info",
  },
};

function VerdictBadge({ verdict }: { verdict: Verdict }) {
  const cfg = VERDICT_CONFIG[verdict];
  return (
    <span
      className={`inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[10px] font-semibold ${cfg.className}`}
    >
      <span className={`size-1.5 rounded-full ${cfg.dotClass}`} />
      {cfg.label}
    </span>
  );
}

function ScoreBar({ score }: { score: number }) {
  const pct = Math.round(score * 100);
  const color =
    score >= 0.6
      ? "bg-success"
      : score >= 0.3
        ? "bg-warning"
        : "bg-destructive";
  return (
    <div className="flex items-center gap-2">
      <div className="flex-1 h-1.5 bg-surface rounded-full overflow-hidden">
        <div
          className={`h-full rounded-full ${color}`}
          style={{ width: `${pct}%` }}
        />
      </div>
      <span className="text-[10px] font-mono text-muted-foreground tabular-nums w-8 text-right">
        {score.toFixed(2)}
      </span>
    </div>
  );
}

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-2">
      <h3 className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
        {title}
      </h3>
      <div className="space-y-1">{children}</div>
    </div>
  );
}

function Field({
  label,
  value,
}: {
  label: string;
  value: string | number;
}) {
  return (
    <div className="flex items-baseline justify-between gap-2">
      <span className="text-[11px] text-muted-foreground shrink-0">
        {label}
      </span>
      <span className="text-[11px] font-mono text-foreground text-right truncate">
        {value}
      </span>
    </div>
  );
}

interface AnalysisPanelProps {
  fileName: string;
}

export function AnalysisPanel({ fileName }: AnalysisPanelProps) {
  const result = useViewerStore((s) => s.analysisResults[fileName]);
  const removeFile = useViewerStore((s) => s.removeFile);
  const addFiles = useViewerStore((s) => s.addFiles);

  const [pruneLoading, setPruneLoading] = useState(false);
  const [spliceLoading, setSpliceLoading] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [showReasons, setShowReasons] = useState(false);
  const [showFeatures, setShowFeatures] = useState(false);

  if (!result) {
    return (
      <div className="flex items-center justify-center h-full text-muted-foreground text-[12px]">
        Run analysis to see activity data
      </div>
    );
  }

  const handlePrune = async () => {
    setPruneLoading(true);
    setActionError(null);
    try {
      await commands.pruneFile(fileName);
      removeFile(fileName);
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      setPruneLoading(false);
    }
  };

  const handleSplice = async (replaceOriginal: boolean) => {
    setSpliceLoading(true);
    setActionError(null);
    try {
      const data = await commands.spliceFile(fileName, { replaceOriginal });
      if (data.newFiles?.length > 0) {
        addFiles(data.newFiles as EgorecListItem[]);
      }
      if (data.originalRemoved) {
        removeFile(fileName);
      }
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      setSpliceLoading(false);
    }
  };

  const isPrunable =
    result.verdict === "PruneConfident" || result.verdict === "PruneSuggested";
  const f = result.features;

  return (
    <ScrollArea className="h-full">
      <div className="space-y-4 p-3">
        <Section title="Activity Analysis">
          <div className="flex items-center justify-between">
            <VerdictBadge verdict={result.verdict} />
            <span className="text-[10px] text-muted-foreground">
              {result.total_frames.toLocaleString()} frames
            </span>
          </div>
          <div className="space-y-0.5">
            <span className="text-[10px] text-muted-foreground">
              Activity score
            </span>
            <ScoreBar score={result.activity_score} />
          </div>
        </Section>

        <Separator />

        <Section title="Actions">
          <div className="flex flex-col gap-1.5">
            {isPrunable && (
              <Button
                variant="outline"
                size="xs"
                className="w-full justify-start text-destructive hover:text-destructive"
                onClick={handlePrune}
                disabled={pruneLoading || spliceLoading}
              >
                {pruneLoading ? (
                  <Loader2 className="size-3 animate-spin" />
                ) : (
                  <Trash2 className="size-3" />
                )}
                Move to .pruned/
              </Button>
            )}
            <Button
              variant="outline"
              size="xs"
              className="w-full justify-start"
              onClick={() => handleSplice(false)}
              disabled={pruneLoading || spliceLoading}
            >
              {spliceLoading ? (
                <Loader2 className="size-3 animate-spin" />
              ) : (
                <Scissors className="size-3" />
              )}
              Extract active segments
            </Button>
            <Button
              variant="outline"
              size="xs"
              className="w-full justify-start"
              onClick={() => handleSplice(true)}
              disabled={pruneLoading || spliceLoading}
            >
              {spliceLoading ? (
                <Loader2 className="size-3 animate-spin" />
              ) : (
                <Scissors className="size-3" />
              )}
              Splice + replace original
            </Button>
          </div>
          {actionError && (
            <p className="text-[10px] text-destructive mt-1">{actionError}</p>
          )}
        </Section>

        <Separator />

        <div>
          <button
            onClick={() => setShowReasons((v) => !v)}
            className="flex items-center gap-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground hover:text-foreground transition-colors w-full"
          >
            {showReasons ? (
              <ChevronDown className="size-3" />
            ) : (
              <ChevronRight className="size-3" />
            )}
            Reasons
          </button>
          {showReasons && (
            <div className="mt-2 space-y-1.5">
              {result.reasons_keep.map((r, i) => (
                <div
                  key={`k${i}`}
                  className="text-[10px] text-success flex gap-1"
                >
                  <span className="shrink-0">+</span>
                  <span>{r}</span>
                </div>
              ))}
              {result.reasons_prune.map((r, i) => (
                <div
                  key={`p${i}`}
                  className="text-[10px] text-destructive flex gap-1"
                >
                  <span className="shrink-0">-</span>
                  <span>{r}</span>
                </div>
              ))}
              {result.reasons_keep.length === 0 &&
                result.reasons_prune.length === 0 && (
                  <span className="text-[10px] text-muted-foreground">
                    No reasons recorded
                  </span>
                )}
            </div>
          )}
        </div>

        <Separator />

        <div>
          <button
            onClick={() => setShowFeatures((v) => !v)}
            className="flex items-center gap-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground hover:text-foreground transition-colors w-full"
          >
            {showFeatures ? (
              <ChevronDown className="size-3" />
            ) : (
              <ChevronRight className="size-3" />
            )}
            Features
          </button>
          {showFeatures && (
            <div className="mt-2 space-y-1">
              <Field
                label="Active frames"
                value={`${(f.active_frame_fraction * 100).toFixed(1)}%`}
              />
              <Field label="Burst count" value={f.burst_count} />
              <Field
                label="P95/P50 ratio"
                value={f.p95_p50_ratio.toFixed(2)}
              />
              <Field
                label="Final third"
                value={`${(f.final_third_activity * 100).toFixed(1)}%`}
              />
              <Field
                label="Depth CV"
                value={f.depth_cv.toFixed(3)}
              />
              <Field
                label="Depth active"
                value={`${(f.depth_active_frame_fraction * 100).toFixed(1)}%`}
              />
              <Field
                label="Window depth CV max"
                value={f.window_depth_cv_max.toFixed(3)}
              />
              <Field
                label="Ego motion"
                value={`${(f.ego_motion_window_fraction * 100).toFixed(1)}%`}
              />
            </div>
          )}
        </div>
      </div>
    </ScrollArea>
  );
}

export function PrunedFilesPanel() {
  const [prunedFiles, setPrunedFiles] = useState<string[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [restoring, setRestoring] = useState<string | null>(null);
  const addFiles = useViewerStore((s) => s.addFiles);

  const loadPruned = async () => {
    try {
      const files = await commands.listPruned();
      setPrunedFiles(files);
      setLoaded(true);
    } catch {
      setPrunedFiles([]);
      setLoaded(true);
    }
  };

  const handleRestore = async (name: string) => {
    setRestoring(name);
    try {
      const data = await commands.restoreFile(name);
      if (data.file) {
        addFiles([data.file]);
        setPrunedFiles((prev) => prev.filter((f) => f !== name));
      }
    } catch {
      // ignore
    } finally {
      setRestoring(null);
    }
  };

  if (!loaded) {
    return (
      <div className="p-3">
        <Button variant="outline" size="xs" className="w-full" onClick={loadPruned}>
          Show pruned files
        </Button>
      </div>
    );
  }

  if (prunedFiles.length === 0) {
    return (
      <div className="p-3 text-[11px] text-muted-foreground text-center">
        No pruned files
      </div>
    );
  }

  return (
    <ScrollArea className="h-full">
      <div className="p-2 space-y-0.5">
        {prunedFiles.map((name) => (
          <div
            key={name}
            className="flex items-center gap-2 px-2 py-1.5 rounded-md hover:bg-hover"
          >
            <span className="text-[11px] text-muted-foreground truncate flex-1">
              {name}
            </span>
            <Button
              variant="ghost"
              size="xs"
              onClick={() => handleRestore(name)}
              disabled={restoring === name}
            >
              {restoring === name ? (
                <Loader2 className="size-3 animate-spin" />
              ) : (
                <Undo2 className="size-3" />
              )}
            </Button>
          </div>
        ))}
      </div>
    </ScrollArea>
  );
}
