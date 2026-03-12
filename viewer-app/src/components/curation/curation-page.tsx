import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { VideoPlayer } from "@/components/video/video-player";
import { commands, onPipelineProgress } from "@/lib/tauri";
import type {
  CurationWorkspaceInfo,
  RecentWorkspace,
  WorkspaceSummary,
} from "@/lib/types";
import { cn } from "@/lib/utils";
import {
  BarChart3,
  CheckCircle2,
  ChevronRight,
  Clock,
  Database,
  Film,
  FolderOpen,
  Layers,
  Loader2,
  Pencil,
  Play,
  RefreshCw,
  Tags,
  Trash2,
  XCircle,
  Check,
  X,
} from "lucide-react";

type RightTab = "qc" | "intervals" | "label" | "bucket";
type EpisodeStatus = "keep" | "review" | "reject" | "invalid";
type IntervalDecision = "keep" | "reject";

interface EpisodeRow {
  episode_id: string;
  source_key: string;
  local_path: string;
  session_name?: string | null;
  duration_s: number;
  frame_count: number;
  fps: number;
  size_bytes: number;
  validate_ok: boolean;
  analyze_verdict?: string | null;
  activity_score?: number | null;
  episode_status: EpisodeStatus;
}

interface IntervalRow {
  interval_id: string;
  source_key: string;
  start_s: number;
  end_s: number;
  duration_s: number;
  activity_fraction?: number;
  active_fraction?: number;
  effective_interval_decision?: IntervalDecision;
}

interface LabelRow {
  interval_id: string;
  source_key: string;
  is_manipulation: boolean;
  proposed_task_name: string;
  short_caption: string;
  confidence: number;
}

interface BucketRow {
  bucket_id: string;
  canonical_task_name: string;
  member_count: number;
  interval_ids: string[];
}

interface BucketMap {
  buckets: BucketRow[];
  mapping: Record<string, string>;
}

interface CurationSummary {
  episodes: { total: number; keep: number; review: number; reject: number };
  intervals: {
    total: number;
    kept: number;
    totalDurationS: number;
    keptDurationS: number;
  };
  labels: { total: number; manipulation: number; avgConfidence: number };
  buckets: { total: number };
}

function formatDuration(seconds: number): string {
  if (!isFinite(seconds) || seconds <= 0) return "0:00";
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}

function formatSize(bytes: number): string {
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  if (bytes < 1024 * 1024 * 1024)
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

const STATUS_COLORS: Record<EpisodeStatus, string> = {
  keep: "text-success bg-success-soft",
  review: "text-info bg-info-soft",
  reject: "text-destructive bg-destructive-soft",
  invalid: "text-muted-foreground bg-muted",
};

const STAGE_ORDER = ["stage", "qc", "intervals", "segments", "label", "cluster"];

export function CurationPage() {
  const [wsInfo, setWsInfo] = useState<CurationWorkspaceInfo | null>(null);
  const [workspaces, setWorkspaces] = useState<WorkspaceSummary[]>([]);
  const [episodes, setEpisodes] = useState<EpisodeRow[]>([]);
  const [intervals, setIntervals] = useState<IntervalRow[]>([]);
  const [labels, setLabels] = useState<LabelRow[]>([]);
  const [bucketMap, setBucketMap] = useState<BucketMap | null>(null);
  const [selectedEpisodeId, setSelectedEpisodeId] = useState<string | null>(
    null,
  );
  const [rightTab, setRightTab] = useState<RightTab>("qc");
  const [loading, setLoading] = useState(true);
  const [runningStage, setRunningStage] = useState<string | null>(null);
  const [stageError, setStageError] = useState<string | null>(null);
  const [stageProgress, setStageProgress] = useState<string | null>(null);
  const [streamUrl, setStreamUrl] = useState<string | null>(null);

  const loadWorkspaceList = useCallback(async () => {
    try {
      const info = await commands.getCurationWorkspace();
      setWsInfo(info);
      if (info.root || info.hasWorkspace) {
        const list = await commands.listWorkspaces();
        setWorkspaces(list);
      }
    } catch (err) {
      console.error("Failed to load workspace info:", err);
    }
  }, []);

  const loadActiveWorkspaceData = useCallback(async () => {
    setLoading(true);
    setEpisodes([]);
    setIntervals([]);
    setLabels([]);
    setBucketMap(null);
    setSelectedEpisodeId(null);
    setStreamUrl(null);

    try {
      const [epData, intData, lblData, bktData] = await Promise.all([
        commands.readCurationData("episodes").catch(() => null),
        commands.readCurationData("intervals").catch(() => null),
        commands.readCurationData("labels").catch(() => null),
        commands.readCurationData("buckets").catch(() => null),
      ]);

      if (Array.isArray(epData)) {
        setEpisodes(epData as EpisodeRow[]);
      }
      if (Array.isArray(intData)) {
        setIntervals(intData as IntervalRow[]);
      }
      if (Array.isArray(lblData)) {
        setLabels(lblData as LabelRow[]);
      }
      if (
        bktData &&
        typeof bktData === "object" &&
        "buckets" in (bktData as Record<string, unknown>)
      ) {
        setBucketMap(bktData as BucketMap);
      }
    } catch (err) {
      console.error("Failed to load workspace data:", err);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    const unlisten = onPipelineProgress((payload) => {
      setStageProgress(`${payload.current}/${payload.total}: ${payload.file}`);
    });
    return () => {
      unlisten.then((u) => u());
    };
  }, []);

  useEffect(() => {
    (async () => {
      await loadWorkspaceList();
      const info = await commands.getCurationWorkspace().catch(() => null);
      if (info?.hasWorkspace) {
        await loadActiveWorkspaceData();
      }
      setLoading(false);
    })();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const handleOpenRoot = useCallback(async () => {
    const dir = await commands.openDirectory();
    if (!dir) return;
    try {
      const info = await commands.setCurationRoot(dir);
      setWsInfo(info);
      const list = await commands.listWorkspaces();
      setWorkspaces(list);
      if (info.hasWorkspace) {
        await loadActiveWorkspaceData();
      } else {
        setEpisodes([]);
        setIntervals([]);
        setLabels([]);
        setBucketMap(null);
      }
    } catch (err) {
      console.error("Failed to open curation root:", err);
    }
  }, [loadActiveWorkspaceData]);

  const handleSelectWorkspace = useCallback(
    async (name: string) => {
      try {
        const info = await commands.setActiveWorkspace(name);
        setWsInfo(info);
        await loadActiveWorkspaceData();
      } catch (err) {
        console.error("Failed to switch workspace:", err);
      }
    },
    [loadActiveWorkspaceData],
  );

  const handleRunStage = useCallback(
    async (stage: string) => {
      setRunningStage(stage);
      setStageError(null);
      setStageProgress(null);
      try {
        await commands.runCurationJob(stage);
        await loadActiveWorkspaceData();
        await loadWorkspaceList();
      } catch (err) {
        setStageError(err instanceof Error ? err.message : String(err));
      } finally {
        setRunningStage(null);
        setStageProgress(null);
      }
    },
    [loadActiveWorkspaceData, loadWorkspaceList],
  );

  const selectedEpisode = useMemo(
    () => episodes.find((e) => e.episode_id === selectedEpisodeId),
    [episodes, selectedEpisodeId],
  );

  const episodeIntervals = useMemo(
    () =>
      selectedEpisode
        ? intervals.filter(
            (i) => i.source_key === selectedEpisode.source_key,
          )
        : [],
    [intervals, selectedEpisode],
  );

  const episodeLabels = useMemo(
    () =>
      selectedEpisode
        ? labels.filter((l) => l.source_key === selectedEpisode.source_key)
        : [],
    [labels, selectedEpisode],
  );

  useEffect(() => {
    if (!selectedEpisode) {
      setStreamUrl(null);
      return;
    }
    commands
      .getCurationStreamUrl(selectedEpisode.source_key)
      .then((url) => setStreamUrl(url))
      .catch((err) => {
        console.error("Failed to get curation stream URL:", err);
        setStreamUrl(null);
      });
  }, [selectedEpisode]);

  const summary = useMemo((): CurationSummary => {
    const keep = episodes.filter((e) => e.episode_status === "keep").length;
    const review = episodes.filter(
      (e) => e.episode_status === "review",
    ).length;
    const reject = episodes.filter(
      (e) => e.episode_status === "reject",
    ).length;
    const totalDurS = intervals.reduce((s, i) => s + i.duration_s, 0);
    const keptDurS = intervals
      .filter((i) => i.effective_interval_decision !== "reject")
      .reduce((s, i) => s + i.duration_s, 0);
    const manipulation = labels.filter((l) => l.is_manipulation).length;
    const avgConf =
      labels.length > 0
        ? labels.reduce((s, l) => s + l.confidence, 0) / labels.length
        : 0;

    return {
      episodes: { total: episodes.length, keep, review, reject },
      intervals: {
        total: intervals.length,
        kept: intervals.filter(
          (i) => i.effective_interval_decision !== "reject",
        ).length,
        totalDurationS: totalDurS,
        keptDurationS: keptDurS,
      },
      labels: { total: labels.length, manipulation, avgConfidence: avgConf },
      buckets: { total: bucketMap?.buckets.length ?? 0 },
    };
  }, [episodes, intervals, labels, bucketMap]);

  const handleSetEpisodeStatus = useCallback(
    async (episodeId: string, status: EpisodeStatus) => {
      try {
        await commands.writeCurationOverride("episodes", episodeId, {
          episode_status: status,
          updated_at: new Date().toISOString(),
        });
        setEpisodes((prev) =>
          prev.map((e) =>
            e.episode_id === episodeId
              ? { ...e, episode_status: status }
              : e,
          ),
        );
      } catch (err) {
        console.error("Failed to update episode status:", err);
      }
    },
    [],
  );

  const handleSetIntervalDecision = useCallback(
    async (intervalId: string, decision: IntervalDecision) => {
      try {
        await commands.writeCurationOverride("intervals", intervalId, {
          decision,
          updated_at: new Date().toISOString(),
        });
        setIntervals((prev) =>
          prev.map((i) =>
            i.interval_id === intervalId
              ? { ...i, effective_interval_decision: decision }
              : i,
          ),
        );
      } catch (err) {
        console.error("Failed to update interval decision:", err);
      }
    },
    [],
  );

  // ── No root set: show welcome screen ────────────────────────────────────────
  const hasRoot = wsInfo?.root != null;
  const hasActive = wsInfo?.hasWorkspace === true;

  if (!hasRoot && !hasActive && !loading) {
    return (
      <RecentWorkspacesWelcome
        onOpen={handleOpenRoot}
        onOpenRecent={async (path) => {
          try {
            const info = await commands.setCurationRoot(path);
            setWsInfo(info);
            const list = await commands.listWorkspaces();
            setWorkspaces(list);
            if (info.hasWorkspace) {
              await loadActiveWorkspaceData();
            }
          } catch (err) {
            console.error("Failed to open recent workspace:", err);
          }
        }}
      />
    );
  }

  if (loading && !hasActive && episodes.length === 0) {
    return (
      <div className="flex items-center justify-center h-full gap-2 text-muted-foreground">
        <Loader2 className="size-4 animate-spin" />
        <span className="text-sm">Loading workspace...</span>
      </div>
    );
  }

  return (
    <div className="flex gap-4 h-full">
      {/* Left sidebar: workspace picker + episode list */}
      <div className="w-72 shrink-0 flex flex-col border border-border rounded-lg overflow-hidden">
        {/* Workspace picker */}
        {workspaces.length > 1 && (
          <>
            <div className="px-3 py-2 border-b border-border flex items-center justify-between">
              <span className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
                Datasets ({workspaces.length})
              </span>
              <Button variant="ghost" size="icon-xs" onClick={handleOpenRoot}>
                <FolderOpen className="size-3" />
              </Button>
            </div>
            <ScrollArea className="max-h-48 shrink-0">
              <div className="flex flex-col gap-0.5 p-1">
                {workspaces.map((ws) => (
                  <button
                    key={ws.name}
                    onClick={() => handleSelectWorkspace(ws.name)}
                    className={cn(
                      "flex flex-col gap-0.5 px-3 py-1.5 rounded-lg text-left transition-colors w-full",
                      ws.name === wsInfo?.activeName
                        ? "bg-accent text-accent-foreground"
                        : "hover:bg-hover text-foreground",
                    )}
                  >
                    <div className="flex items-center gap-2">
                      <Database className="size-3 shrink-0 text-muted-foreground" />
                      <span className="text-[11px] font-medium truncate flex-1">
                        {ws.name}
                      </span>
                      {ws.name === wsInfo?.activeName && (
                        <ChevronRight className="size-3 text-muted-foreground" />
                      )}
                    </div>
                    <div className="flex items-center gap-2 text-[10px] text-muted-foreground pl-5">
                      <span>{ws.episodeCount} ep</span>
                      {ws.sourcePrefix && (
                        <span className="truncate">{ws.sourcePrefix}</span>
                      )}
                      <span>
                        {ws.completedStages.length}/{STAGE_ORDER.length}
                      </span>
                    </div>
                  </button>
                ))}
              </div>
            </ScrollArea>
            <Separator />
          </>
        )}

        {/* Episode list */}
        <div className="px-3 py-2 border-b border-border flex items-center justify-between">
          <span className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
            {wsInfo?.activeName ? (
              <>
                {wsInfo.activeName}{" "}
                <span className="font-normal">({episodes.length})</span>
              </>
            ) : (
              `Episodes (${episodes.length})`
            )}
          </span>
          <Button
            variant="ghost"
            size="icon-xs"
            onClick={loadActiveWorkspaceData}
          >
            <RefreshCw className="size-3" />
          </Button>
        </div>

        {!hasActive ? (
          <div className="flex-1 flex items-center justify-center p-4 text-[12px] text-muted-foreground text-center">
            Select a dataset above to view episodes
          </div>
        ) : (
          <ScrollArea className="flex-1 min-h-0">
            <div className="flex flex-col gap-0.5 p-1">
              {episodes.map((ep) => (
                <button
                  key={ep.episode_id}
                  onClick={() => setSelectedEpisodeId(ep.episode_id)}
                  className={cn(
                    "flex flex-col gap-1 px-3 py-2 rounded-lg text-left transition-colors w-full",
                    ep.episode_id === selectedEpisodeId
                      ? "bg-accent text-accent-foreground"
                      : "hover:bg-hover text-foreground",
                  )}
                >
                  <div className="flex items-center gap-2">
                    <span className="text-[11px] font-medium truncate flex-1">
                      {ep.session_name || ep.source_key.split("/").pop()}
                    </span>
                    <span
                      className={cn(
                        "text-[9px] font-semibold uppercase px-1.5 py-0.5 rounded",
                        STATUS_COLORS[ep.episode_status],
                      )}
                    >
                      {ep.episode_status}
                    </span>
                  </div>
                  <div className="flex items-center gap-2 text-[10px] text-muted-foreground">
                    <span>{formatDuration(ep.duration_s)}</span>
                    <span>{ep.frame_count} frames</span>
                    <span>{formatSize(ep.size_bytes)}</span>
                  </div>
                </button>
              ))}
              {episodes.length === 0 && (
                <div className="px-3 py-4 text-[12px] text-muted-foreground text-center">
                  No episodes. Run the pipeline to populate.
                </div>
              )}
            </div>
          </ScrollArea>
        )}
      </div>

      {/* Center: video + pipeline controls */}
      <div className="flex-1 min-w-0 flex flex-col gap-3">
        {/* Summary bar */}
        <div className="flex items-center gap-4 text-[11px] text-muted-foreground">
          <div className="flex items-center gap-1">
            <Film className="size-3" />
            <span>{summary.episodes.total} episodes</span>
          </div>
          <div className="flex items-center gap-1">
            <BarChart3 className="size-3" />
            <span>
              {summary.intervals.total} intervals (
              {formatDuration(summary.intervals.totalDurationS)})
            </span>
          </div>
          <div className="flex items-center gap-1">
            <Tags className="size-3" />
            <span>{summary.labels.total} labels</span>
          </div>
          <div className="flex items-center gap-1">
            <Layers className="size-3" />
            <span>{summary.buckets.total} buckets</span>
          </div>
        </div>

        {/* Pipeline controls */}
        {hasActive && (
          <div className="flex items-center gap-1.5 flex-wrap">
            {STAGE_ORDER.map((stage) => (
              <Button
                key={stage}
                variant="outline"
                size="xs"
                disabled={runningStage !== null}
                onClick={() => handleRunStage(stage)}
              >
                {runningStage === stage ? (
                  <Loader2 className="size-3 animate-spin" />
                ) : (
                  <Play className="size-3" />
                )}
                {stage}
              </Button>
            ))}
            {stageProgress && runningStage && (
              <span className="text-[10px] text-muted-foreground truncate max-w-xs">
                {stageProgress}
              </span>
            )}
            {stageError && (
              <span className="text-[10px] text-destructive">{stageError}</span>
            )}
          </div>
        )}

        <Separator />

        {/* Video */}
        {selectedEpisode ? (
          streamUrl ? (
            <VideoPlayer
              key={selectedEpisode.episode_id}
              src={streamUrl}
              className="flex-1 min-h-0"
            />
          ) : (
            <div className="flex-1 flex items-center justify-center text-muted-foreground gap-2 rounded-lg border border-border">
              <Film className="size-8 opacity-40" />
              <span className="text-sm">No stream available</span>
            </div>
          )
        ) : (
          <div className="flex-1 flex items-center justify-center text-muted-foreground gap-2 rounded-lg border border-border">
            <Film className="size-8 opacity-40" />
            <span className="text-sm">
              {hasActive ? "Select an episode" : "Select a dataset first"}
            </span>
          </div>
        )}
      </div>

      <Separator orientation="vertical" />

      {/* Right panel */}
      <div className="w-80 shrink-0 flex flex-col border border-border rounded-lg overflow-hidden">
        <div className="flex items-center border-b border-border">
          {(["qc", "intervals", "label", "bucket"] as RightTab[]).map(
            (tab) => (
              <button
                key={tab}
                onClick={() => setRightTab(tab)}
                className={cn(
                  "px-2.5 py-2 text-[10px] font-semibold uppercase tracking-wider transition-colors",
                  rightTab === tab
                    ? "text-foreground border-b-2 border-primary"
                    : "text-muted-foreground hover:text-foreground",
                )}
              >
                {tab}
              </button>
            ),
          )}
        </div>

        <ScrollArea className="flex-1 min-h-0">
          {!selectedEpisode ? (
            <div className="p-4 text-[12px] text-muted-foreground text-center">
              Select an episode to see details
            </div>
          ) : rightTab === "qc" ? (
            <QcPanel
              episode={selectedEpisode}
              onSetStatus={handleSetEpisodeStatus}
            />
          ) : rightTab === "intervals" ? (
            <IntervalsPanel
              intervals={episodeIntervals}
              onSetDecision={handleSetIntervalDecision}
            />
          ) : rightTab === "label" ? (
            <LabelsPanel labels={episodeLabels} />
          ) : rightTab === "bucket" ? (
            <BucketsPanel bucketMap={bucketMap} />
          ) : null}
        </ScrollArea>
      </div>
    </div>
  );
}

// ── Sub-panels ────────────────────────────────────────────────────────────────

function QcPanel({
  episode,
  onSetStatus,
}: {
  episode: EpisodeRow;
  onSetStatus: (id: string, status: EpisodeStatus) => void;
}) {
  return (
    <div className="p-3 space-y-4">
      <div className="space-y-2">
        <h3 className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
          Episode Info
        </h3>
        <div className="space-y-1 text-[11px]">
          <div className="flex justify-between">
            <span className="text-muted-foreground">Source</span>
            <span className="font-mono truncate max-w-[160px]">
              {episode.source_key}
            </span>
          </div>
          <div className="flex justify-between">
            <span className="text-muted-foreground">Duration</span>
            <span>{formatDuration(episode.duration_s)}</span>
          </div>
          <div className="flex justify-between">
            <span className="text-muted-foreground">Frames</span>
            <span>{episode.frame_count}</span>
          </div>
          <div className="flex justify-between">
            <span className="text-muted-foreground">Verdict</span>
            <span>{episode.analyze_verdict ?? "N/A"}</span>
          </div>
          {episode.activity_score != null && (
            <div className="flex justify-between">
              <span className="text-muted-foreground">Activity</span>
              <span>{(episode.activity_score * 100).toFixed(1)}%</span>
            </div>
          )}
        </div>
      </div>

      <Separator />

      <div className="space-y-2">
        <h3 className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
          Status
        </h3>
        <div className="flex gap-1.5">
          {(["keep", "review", "reject"] as EpisodeStatus[]).map((status) => (
            <Button
              key={status}
              variant={
                episode.episode_status === status ? "default" : "outline"
              }
              size="xs"
              onClick={() => onSetStatus(episode.episode_id, status)}
              className={cn(
                episode.episode_status === status && STATUS_COLORS[status],
              )}
            >
              {status === "keep" && <CheckCircle2 className="size-3" />}
              {status === "reject" && <XCircle className="size-3" />}
              {status}
            </Button>
          ))}
        </div>
      </div>
    </div>
  );
}

function IntervalsPanel({
  intervals,
  onSetDecision,
}: {
  intervals: IntervalRow[];
  onSetDecision: (id: string, decision: IntervalDecision) => void;
}) {
  if (intervals.length === 0) {
    return (
      <div className="p-4 text-[12px] text-muted-foreground text-center">
        No intervals for this episode
      </div>
    );
  }

  return (
    <div className="p-2 space-y-1">
      {intervals.map((intv) => {
        const activityPct = intv.active_fraction ?? intv.activity_fraction;
        return (
          <div
            key={intv.interval_id}
            className="rounded-lg border border-border p-2.5 space-y-1.5"
          >
            <div className="flex items-center justify-between text-[11px]">
              <span className="font-mono">
                {formatDuration(intv.start_s)} &rarr;{" "}
                {formatDuration(intv.end_s)}
              </span>
              <span className="text-muted-foreground">
                {formatDuration(intv.duration_s)}
              </span>
            </div>
            {activityPct != null && (
              <div className="flex items-center gap-2">
                <div className="flex-1 h-1 bg-surface rounded-full overflow-hidden">
                  <div
                    className="h-full bg-success rounded-full"
                    style={{
                      width: `${(activityPct * 100).toFixed(0)}%`,
                    }}
                  />
                </div>
                <span className="text-[10px] text-muted-foreground">
                  {(activityPct * 100).toFixed(0)}%
                </span>
              </div>
            )}
            <div className="flex gap-1">
              <Button
                variant={
                  intv.effective_interval_decision === "keep"
                    ? "default"
                    : "outline"
                }
                size="xs"
                onClick={() => onSetDecision(intv.interval_id, "keep")}
              >
                keep
              </Button>
              <Button
                variant={
                  intv.effective_interval_decision === "reject"
                    ? "default"
                    : "outline"
                }
                size="xs"
                onClick={() => onSetDecision(intv.interval_id, "reject")}
              >
                reject
              </Button>
            </div>
          </div>
        );
      })}
    </div>
  );
}

function LabelsPanel({ labels }: { labels: LabelRow[] }) {
  if (labels.length === 0) {
    return (
      <div className="p-4 text-[12px] text-muted-foreground text-center">
        No labels for this episode
      </div>
    );
  }

  return (
    <div className="p-2 space-y-1">
      {labels.map((lbl, i) => (
        <div
          key={`${lbl.interval_id}-${i}`}
          className="rounded-lg border border-border p-2.5 space-y-1"
        >
          <div className="flex items-center gap-2">
            <span
              className={cn(
                "text-[9px] font-semibold uppercase px-1.5 py-0.5 rounded",
                lbl.is_manipulation
                  ? "text-success bg-success-soft"
                  : "text-muted-foreground bg-muted",
              )}
            >
              {lbl.is_manipulation ? "manipulation" : "other"}
            </span>
            <span className="text-[11px] font-medium truncate">
              {lbl.proposed_task_name}
            </span>
          </div>
          <p className="text-[10px] text-muted-foreground">
            {lbl.short_caption}
          </p>
          <div className="flex items-center gap-2 text-[10px] text-muted-foreground">
            <span>confidence: {(lbl.confidence * 100).toFixed(0)}%</span>
          </div>
        </div>
      ))}
    </div>
  );
}

function BucketsPanel({ bucketMap }: { bucketMap: BucketMap | null }) {
  if (!bucketMap || bucketMap.buckets.length === 0) {
    return (
      <div className="p-4 text-[12px] text-muted-foreground text-center">
        No buckets available
      </div>
    );
  }

  return (
    <div className="p-2 space-y-1">
      {bucketMap.buckets.map((bucket) => (
        <div
          key={bucket.bucket_id}
          className="rounded-lg border border-border p-2.5 space-y-1"
        >
          <div className="flex items-center justify-between">
            <span className="text-[11px] font-medium">
              {bucket.canonical_task_name}
            </span>
            <Badge variant="inline">{bucket.member_count}</Badge>
          </div>
          <div className="text-[10px] text-muted-foreground">
            {bucket.interval_ids.length} intervals
          </div>
        </div>
      ))}
    </div>
  );
}

// ── Recent Workspaces Welcome ─────────────────────────────────────────────────

function formatRelativeTime(isoDate: string): string {
  const diff = Date.now() - new Date(isoDate).getTime();
  const minutes = Math.floor(diff / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d ago`;
  return new Date(isoDate).toLocaleDateString();
}

function RecentWorkspacesWelcome({
  onOpen,
  onOpenRecent,
}: {
  onOpen: () => void;
  onOpenRecent: (path: string) => void;
}) {
  const [recents, setRecents] = useState<RecentWorkspace[]>([]);
  const [editingPath, setEditingPath] = useState<string | null>(null);
  const [editValue, setEditValue] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    commands.getRecentWorkspaces().then(setRecents).catch(() => {});
  }, []);

  useEffect(() => {
    if (editingPath && inputRef.current) {
      inputRef.current.focus();
      inputRef.current.select();
    }
  }, [editingPath]);

  const handleRemove = async (path: string) => {
    try {
      await commands.removeRecentWorkspace(path);
      setRecents((prev) => prev.filter((w) => w.path !== path));
    } catch (err) {
      console.error("Failed to remove recent workspace:", err);
    }
  };

  const handleStartEdit = (ws: RecentWorkspace) => {
    setEditingPath(ws.path);
    setEditValue(ws.alias ?? ws.path.split("/").pop() ?? "");
  };

  const handleSaveEdit = async () => {
    if (!editingPath) return;
    const trimmed = editValue.trim();
    const alias = trimmed.length > 0 ? trimmed : null;
    try {
      await commands.updateRecentWorkspaceAlias(editingPath, alias);
      setRecents((prev) =>
        prev.map((w) => (w.path === editingPath ? { ...w, alias } : w)),
      );
    } catch (err) {
      console.error("Failed to update alias:", err);
    }
    setEditingPath(null);
  };

  const handleCancelEdit = () => {
    setEditingPath(null);
  };

  return (
    <div className="flex flex-col items-center justify-center h-full gap-6 text-muted-foreground">
      <Layers className="size-12 opacity-40" />
      <div className="text-center space-y-2">
        <h2 className="font-serif text-[20px] text-foreground">
          Open a Curation Workspace
        </h2>
        <p className="text-[13px] max-w-md">
          Select a curation workspace folder, or pick one you've opened before.
        </p>
      </div>
      <Button variant="highlight" size="sm" onClick={onOpen}>
        <FolderOpen className="size-4" />
        Open Workspace
      </Button>

      {recents.length > 0 && (
        <div className="w-full max-w-lg mt-2">
          <div className="flex items-center gap-2 mb-2">
            <Clock className="size-3 text-muted-foreground" />
            <span className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
              Recent Workspaces
            </span>
          </div>
          <div className="flex flex-col gap-1 border border-border rounded-lg overflow-hidden">
            {recents.map((ws) => (
              <div
                key={ws.path}
                className="group flex items-center gap-2 px-3 py-2 hover:bg-hover transition-colors"
              >
                <button
                  onClick={() => onOpenRecent(ws.path)}
                  className="flex-1 min-w-0 text-left"
                >
                  <div className="flex items-center gap-2">
                    <Database className="size-3 shrink-0 text-muted-foreground" />
                    {editingPath === ws.path ? (
                      <form
                        onSubmit={(e) => {
                          e.preventDefault();
                          handleSaveEdit();
                        }}
                        className="flex items-center gap-1 flex-1 min-w-0"
                        onClick={(e) => e.stopPropagation()}
                      >
                        <input
                          ref={inputRef}
                          type="text"
                          value={editValue}
                          onChange={(e) => setEditValue(e.target.value)}
                          onKeyDown={(e) => {
                            if (e.key === "Escape") handleCancelEdit();
                          }}
                          className="text-[12px] font-medium bg-surface border border-border rounded px-1.5 py-0.5 w-full min-w-0 outline-none focus:ring-1 focus:ring-primary"
                        />
                        <button
                          type="submit"
                          className="text-success hover:text-success/80 shrink-0"
                        >
                          <Check className="size-3" />
                        </button>
                        <button
                          type="button"
                          onClick={handleCancelEdit}
                          className="text-muted-foreground hover:text-foreground shrink-0"
                        >
                          <X className="size-3" />
                        </button>
                      </form>
                    ) : (
                      <span className="text-[12px] font-medium text-foreground truncate">
                        {ws.alias ?? ws.path.split("/").pop()}
                      </span>
                    )}
                  </div>
                  {editingPath !== ws.path && (
                    <div className="flex items-center gap-2 text-[10px] text-muted-foreground pl-5 mt-0.5">
                      <span className="truncate">{ws.path}</span>
                      <span className="shrink-0">
                        {formatRelativeTime(ws.lastOpenedAt)}
                      </span>
                    </div>
                  )}
                </button>

                {editingPath !== ws.path && (
                  <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity shrink-0">
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        handleStartEdit(ws);
                      }}
                      className="p-1 rounded hover:bg-accent text-muted-foreground hover:text-foreground transition-colors"
                      title="Rename"
                    >
                      <Pencil className="size-3" />
                    </button>
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        handleRemove(ws.path);
                      }}
                      className="p-1 rounded hover:bg-destructive/10 text-muted-foreground hover:text-destructive transition-colors"
                      title="Remove from recent"
                    >
                      <Trash2 className="size-3" />
                    </button>
                  </div>
                )}
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
