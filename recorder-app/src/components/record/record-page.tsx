import { useState, useEffect, useCallback, useRef } from "react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useAppStore } from "@/stores/app-store";
import { useRecorderStore } from "@/stores/recorder-store";
import {
  commands,
  onRecorderStats,
  onRecorderStopped,
  onPreviewStateChanged,
  onPreviewDisconnected,
} from "@/lib/tauri";
import { playCountdown, playConfirmTone } from "@/lib/audio";
import { cn } from "@/lib/utils";
import { StatsPanel } from "./stats-panel";
import { DiskBar } from "./disk-bar";
import { CountdownOverlay } from "./countdown-overlay";
import { CameraPreview } from "./camera-preview";
import {
  Circle,
  Square,
  Lock,
  LockOpen,
  FolderOpen,
  Database,
} from "lucide-react";

function generateSessionName(): string {
  const now = new Date();
  const pad = (n: number) => n.toString().padStart(2, "0");
  return `rec_${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}_${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}`;
}

export function RecordPage() {
  const config = useAppStore((s) => s.config);
  const setConfig = useAppStore((s) => s.setConfig);
  const lidSafe = useAppStore((s) => s.lidSafe);
  const setLidSafe = useAppStore((s) => s.setLidSafe);

  const status = useRecorderStore((s) => s.status);
  const countdown = useRecorderStore((s) => s.countdown);
  const setCountdown = useRecorderStore((s) => s.setCountdown);
  const setStatus = useRecorderStore((s) => s.setStatus);
  const diskInfo = useRecorderStore((s) => s.diskInfo);
  const setDiskInfo = useRecorderStore((s) => s.setDiskInfo);
  const setPreviewState = useRecorderStore((s) => s.setPreviewState);
  const setPreviewUrls = useRecorderStore((s) => s.setPreviewUrls);
  const setCameraInfo = useRecorderStore((s) => s.setCameraInfo);
  const previewState = useRecorderStore((s) => s.previewState);

  const selectedDataset = useRecorderStore((s) => s.selectedDataset);
  const setSelectedDataset = useRecorderStore((s) => s.setSelectedDataset);
  const availableDatasets = useRecorderStore((s) => s.availableDatasets);
  const setAvailableDatasets = useRecorderStore((s) => s.setAvailableDatasets);

  const [sessionName, setSessionName] = useState(generateSessionName);
  const [outputDir, setOutputDir] = useState(config?.storage.output_dir ?? "");
  const [crf, setCrf] = useState(config?.recorder.default_crf ?? 23);
  const [error, setError] = useState<string | null>(null);
  const [recommendedPreset, setRecommendedPreset] = useState<string | null>(null);
  const [cpuLabel, setCpuLabel] = useState<string | null>(null);

  // AbortController for cancelling countdown on unmount (Fix 11)
  const countdownAbortRef = useRef<AbortController | null>(null);
  // Guard against double-start (countdown is async, state updates are batched)
  const recordingInFlight = useRef(false);

  // Start preview on mount, stop on unmount (Fix 10: check state before start)
  useEffect(() => {
    let mounted = true;

    async function initPreview() {
      try {
        // Check if preview is already running (Fix 10)
        const currentState = await commands.getPreviewState();
        if (!mounted) return;

        if (currentState === "previewing" || currentState === "recording") {
          // Reuse existing session — just fetch URLs
          setPreviewState(currentState);
          try {
            const info = await commands.getCameraInfo();
            if (!mounted) return;
            if (info) setCameraInfo(info);
          } catch {
            // Camera info not critical for reuse
          }
          try {
            const rgbUrl = await commands.getPreviewUrl("rgb");
            const depthUrl = await commands.getPreviewUrl("depth");
            if (!mounted) return;
            setPreviewUrls(rgbUrl, depthUrl);
          } catch {
            // URLs not available yet
          }
          return;
        }

        // Not running — start fresh
        setPreviewState("starting");
        const info = await commands.startPreview();
        if (!mounted) return;
        setCameraInfo(info);

        const rgbUrl = await commands.getPreviewUrl("rgb");
        const depthUrl = await commands.getPreviewUrl("depth");
        if (!mounted) return;
        setPreviewUrls(rgbUrl, depthUrl);
        setPreviewState("previewing");
      } catch (err) {
        if (!mounted) return;
        setPreviewState("error");
        console.error("Failed to start preview:", err);
      }
    }

    initPreview();

    return () => {
      mounted = false;
      // Abort any in-progress countdown (Fix 11)
      countdownAbortRef.current?.abort();
      // Only stop preview if not recording
      const state = useRecorderStore.getState();
      if (state.status.state !== "recording") {
        commands.stopPreview().catch(() => {});
        setPreviewState("off");
        setPreviewUrls(null, null);
        setCameraInfo(null);
      }
    };
  }, [setPreviewState, setPreviewUrls, setCameraInfo]);

  // Subscribe to events (Fix 12: proper promise-based cleanup)
  useEffect(() => {
    const subscriptions = [
      onRecorderStats((stats) => setStatus(stats)),
      onRecorderStopped((reason) => {
        recordingInFlight.current = false;
        if (reason !== "clean") setError(`Recording stopped: ${reason}`);
        // Refresh dataset list so episode count updates
        commands.listDatasets().then(setAvailableDatasets).catch(() => {});
      }),
      onPreviewStateChanged((state) => {
        setPreviewState(state);
      }),
      onPreviewDisconnected(() => {
        setPreviewState("off");
        setPreviewUrls(null, null);
      }),
    ];
    return () => {
      subscriptions.forEach((p) => p.then((unsub) => unsub()));
    };
  }, [setStatus, setPreviewState, setPreviewUrls]);

  // Periodic disk info polling
  useEffect(() => {
    if (!outputDir) return;
    const poll = () => {
      commands.getDiskInfo(outputDir).then(setDiskInfo).catch(() => {});
    };
    poll();
    const interval = setInterval(poll, 5000);
    return () => clearInterval(interval);
  }, [outputDir, setDiskInfo]);

  // Sync output dir from config
  useEffect(() => {
    if (config?.storage.output_dir) {
      setOutputDir(config.storage.output_dir);
    }
  }, [config]);

  // Fetch available datasets
  useEffect(() => {
    commands.listDatasets().then(setAvailableDatasets).catch(() => {});
  }, [setAvailableDatasets]);

  // Fetch system info for preset recommendation
  useEffect(() => {
    commands.getSystemInfo().then((info) => {
      setRecommendedPreset(info.recommendedPreset);
      // Shorten CPU model for display
      const short = info.cpuModel
        .replace(/\(R\)|\(TM\)/g, "")
        .replace(/\s+/g, " ")
        .trim();
      setCpuLabel(`${short} (${info.cpuCores} cores, ${info.arch})`);
    }).catch(() => {});
  }, []);

  const handleStartRecording = useCallback(async () => {
    if (recordingInFlight.current) return;
    setError(null);
    if (!outputDir) {
      setError("No output directory set. Configure it in Settings.");
      return;
    }

    if (!selectedDataset) {
      setError("Select a dataset before recording.");
      return;
    }

    if (previewState !== "previewing") {
      setError("Camera preview not ready. Wait for the preview to connect.");
      return;
    }

    recordingInFlight.current = true;

    // 3-second countdown with AbortSignal (Fix 11)
    const abortController = new AbortController();
    countdownAbortRef.current = abortController;

    setCountdown(3);
    try {
      await playCountdown((remaining) => {
        setCountdown(remaining);
      }, abortController.signal);
    } catch (err) {
      if (err instanceof DOMException && err.name === "AbortError") {
        setCountdown(null);
        recordingInFlight.current = false;
        return;
      }
      recordingInFlight.current = false;
      throw err;
    } finally {
      countdownAbortRef.current = null;
    }
    setCountdown(null);

    try {
      await commands.startRecording(`${outputDir}/${selectedDataset}`, sessionName, crf);
      setStatus({ ...useRecorderStore.getState().status, state: "recording", framesWritten: 0, framesDropped: 0, captureFps: 0, writeFps: 0, fileSizeMb: 0, elapsedSeconds: 0 });
      setPreviewState("recording");
      setSessionName(generateSessionName());
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      recordingInFlight.current = false;
    }
  }, [outputDir, sessionName, crf, setCountdown, previewState, selectedDataset]);

  const handleStopRecording = useCallback(async () => {
    try {
      await commands.stopRecording();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      recordingInFlight.current = false;
    }
  }, []);

  const handleToggleLidSafe = useCallback(async () => {
    try {
      const result = await commands.toggleLidSafe(!lidSafe);
      setLidSafe(result);
      playConfirmTone();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [lidSafe, setLidSafe]);

  const handlePickOutputDir = useCallback(async () => {
    const dir = await commands.openDirectory();
    if (dir) setOutputDir(dir);
  }, []);

  const isRecording = status.state === "recording";
  const isStopping = status.state === "stopping";
  const isCountingDown = countdown !== null;
  const canStart = !isRecording && !isStopping && !isCountingDown && previewState === "previewing" && !!selectedDataset;
  const selectedDatasetInfo = selectedDataset
    ? availableDatasets.find((d) => d.dirName === selectedDataset) ?? null
    : null;

  // Keyboard shortcut: Space to start/stop
  useEffect(() => {
    function handleKey(e: KeyboardEvent) {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
      if (e.code === "Space") {
        e.preventDefault();
        if (isRecording) handleStopRecording();
        else if (canStart) handleStartRecording();
      }
    }
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [isRecording, canStart, handleStartRecording, handleStopRecording]);

  return (
    <div className="flex flex-col h-full p-6 gap-4 relative">
      {/* Countdown overlay */}
      {isCountingDown && <CountdownOverlay count={countdown!} />}

      {/* Header */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-4 min-w-0 flex-1">
          <h1 className="text-2xl font-bold tracking-tight shrink-0">Record</h1>
          {selectedDatasetInfo ? (
            <div className="flex items-center gap-3 min-w-0 flex-1">
              <span className="text-lg font-medium text-foreground/80 flex items-center gap-1.5 shrink-0">
                <Database className="size-4" />
                {selectedDatasetInfo.name}
              </span>
              {selectedDatasetInfo.targetEpisodes ? (
                <div className="flex items-center gap-2.5 flex-1 min-w-0 max-w-xs">
                  <span className="text-xl font-mono font-bold tabular-nums shrink-0">
                    {selectedDatasetInfo.fileCount}
                    <span className="text-muted-foreground font-normal text-sm">/{selectedDatasetInfo.targetEpisodes}</span>
                  </span>
                  <div className="flex-1 h-2.5 rounded-full bg-muted overflow-hidden min-w-16">
                    <div
                      className={cn(
                        "h-full rounded-full transition-all duration-500",
                        selectedDatasetInfo.fileCount >= selectedDatasetInfo.targetEpisodes
                          ? "bg-highlight"
                          : "bg-highlight/70",
                      )}
                      style={{ width: `${Math.min((selectedDatasetInfo.fileCount / selectedDatasetInfo.targetEpisodes) * 100, 100)}%` }}
                    />
                  </div>
                  {selectedDatasetInfo.fileCount >= selectedDatasetInfo.targetEpisodes && (
                    <Badge variant="inline" className="bg-highlight/20 text-highlight-foreground shrink-0">
                      Complete
                    </Badge>
                  )}
                </div>
              ) : (
                <span className="text-xl font-mono font-bold tabular-nums">
                  {selectedDatasetInfo.fileCount}
                  <span className="text-sm font-normal text-muted-foreground ml-1">ep{selectedDatasetInfo.fileCount !== 1 ? "s" : ""}</span>
                </span>
              )}
            </div>
          ) : (
            <span className="text-sm text-muted-foreground">No dataset selected</span>
          )}
        </div>
        <Button
          variant={lidSafe ? "highlight" : "outline"}
          size="sm"
          onClick={handleToggleLidSafe}
          className="gap-1.5"
        >
          {lidSafe ? <Lock className="size-3.5" /> : <LockOpen className="size-3.5" />}
          {lidSafe ? "Lid-Close Safe" : "Lid-Close Safe"}
        </Button>
      </div>

      {/* Camera preview */}
      <CameraPreview />

      {/* Session config */}
      <div className="grid grid-cols-2 gap-4">
        <div className="space-y-1.5">
          <label className="text-[11px] font-medium text-muted-foreground uppercase tracking-wider">
            Output Directory
          </label>
          <div className="flex gap-2">
            <input
              type="text"
              value={outputDir}
              onChange={(e) => setOutputDir(e.target.value)}
              placeholder="/path/to/recordings"
              className="flex-1 h-8 px-3 rounded-md border border-input bg-background text-foreground placeholder:text-muted-foreground text-sm"
            />
            <Button variant="outline" size="sm" onClick={handlePickOutputDir}>
              <FolderOpen className="size-3.5" />
            </Button>
          </div>
        </div>

        <div className="space-y-1.5">
          <label className="text-[11px] font-medium text-muted-foreground uppercase tracking-wider">
            Dataset
          </label>
          {availableDatasets.length > 0 ? (
            <Select
              value={selectedDataset ?? "none"}
              onValueChange={(v) => setSelectedDataset(v === "none" ? null : v)}
            >
              <SelectTrigger
                className={cn(
                  !selectedDataset && "border-destructive/50 text-destructive",
                )}
              >
                <SelectValue placeholder="Select a dataset" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="none">Select a dataset</SelectItem>
                {availableDatasets.map((d) => (
                  <SelectItem key={d.dirName} value={d.dirName}>
                    <span className="flex items-center gap-2">
                      <Database className="size-3 shrink-0" />
                      {d.name}
                    </span>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          ) : (
            <div className="h-8 px-3 flex items-center rounded-md border border-destructive/50 bg-destructive-soft text-destructive text-sm">
              No datasets — create one first
            </div>
          )}
        </div>

        <div className="space-y-1.5">
          <label className="text-[11px] font-medium text-muted-foreground uppercase tracking-wider">
            Session Name
          </label>
          <input
            type="text"
            value={sessionName}
            onChange={(e) => setSessionName(e.target.value)}
            className="w-full h-8 px-3 rounded-md border border-input bg-background text-foreground text-sm font-mono"
          />
        </div>

        <div className="space-y-1.5">
          <label className="text-[11px] font-medium text-muted-foreground uppercase tracking-wider">
            CRF Quality ({crf})
          </label>
          <input
            type="range"
            min={0}
            max={51}
            value={crf}
            onChange={(e) => setCrf(parseInt(e.target.value))}
            className="w-full"
          />
          <div className="flex justify-between text-[9px] text-muted-foreground">
            <span>Lossless</span>
            <span>Lossy</span>
          </div>
        </div>

        <div className="space-y-1.5">
          <label className="text-[11px] font-medium text-muted-foreground uppercase tracking-wider">
            Encoder Speed
          </label>
          <Select
            value={config?.recorder.h264_preset ?? "ultrafast"}
            onValueChange={(v) => {
              if (config) {
                const updated = { ...config, recorder: { ...config.recorder, h264_preset: v } };
                commands.saveConfig(updated);
                setConfig(updated);
              }
            }}
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {(["ultrafast", "superfast", "veryfast", "fast"] as const).map((p) => (
                <SelectItem key={p} value={p}>
                  <span className="flex items-center gap-2">
                    {p === "ultrafast" ? "Ultrafast" : p === "superfast" ? "Superfast" : p === "veryfast" ? "Very Fast" : "Fast"}
                    {p === recommendedPreset && (
                      <span className="text-[10px] text-success font-medium">recommended</span>
                    )}
                    {p === "fast" && (
                      <span className="text-[10px] text-muted-foreground">smaller files</span>
                    )}
                  </span>
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <div className="text-[9px] text-muted-foreground space-y-0.5">
            <div>Faster = fewer dropped frames, larger files. Restart preview to apply.</div>
            {cpuLabel && (
              <div className="text-muted-foreground/70">{cpuLabel}</div>
            )}
          </div>
        </div>
      </div>

      {/* Record button */}
      <div className="flex items-center justify-center py-4">
        {isRecording || isStopping ? (
          <Button
            size="lg"
            variant="destructive"
            className="h-14 px-10 text-base gap-3"
            onClick={handleStopRecording}
            disabled={isStopping}
          >
            <Square className="size-5" />
            {isStopping ? "Stopping..." : "Stop Recording"}
          </Button>
        ) : (
          <Button
            size="lg"
            variant="highlight"
            className="h-14 px-10 text-base gap-3"
            onClick={handleStartRecording}
            disabled={!canStart}
          >
            <Circle className="size-5 fill-current" />
            Start Recording
          </Button>
        )}
      </div>

      {/* Error display */}
      {error && (
        <div className="rounded-lg bg-destructive-soft px-4 py-2 text-sm text-destructive">
          {error}
        </div>
      )}

      {/* Live stats panel */}
      {(isRecording || status.framesWritten > 0) && <StatsPanel />}

      {/* Disk space bar */}
      {diskInfo && (
        <DiskBar
          diskInfo={diskInfo}
          threshold={config?.storage.disk_threshold_mb ?? 500}
        />
      )}

      {/* Keyboard hint */}
      <div className="text-center text-[10px] text-muted-foreground mt-auto">
        Press <kbd className="font-mono bg-muted px-1.5 py-0.5 rounded text-[9px]">Space</kbd> to {isRecording ? "stop" : "start"} recording
      </div>
    </div>
  );
}

function formatElapsed(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`;
}
