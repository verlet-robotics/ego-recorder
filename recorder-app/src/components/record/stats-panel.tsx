import { useEffect, useRef } from "react";
import { useRecorderStore } from "@/stores/recorder-store";
import { playWarningTone } from "@/lib/audio";
import type { RecorderStatus } from "@/lib/types";
import { AlertTriangle } from "lucide-react";

// --- Thresholds ---
// Write FPS is the ground truth: frames actually encoded per second of
// recording time. Capture FPS divides by total subprocess uptime (including
// preview), so it reads low if the user previewed before recording — ignore it
// for health purposes unless frames are actually being dropped.
const WRITE_FPS_WARN = 25; // yellow — encoder struggling
const WRITE_FPS_CRITICAL = 20; // red — significant frame loss
const DROP_RATE_WARN = 0.01; // > 1% drop rate
const DROP_RATE_CRITICAL = 0.05; // > 5% drop rate

// Don't evaluate health until the FPS average has had time to stabilize.
const GRACE_PERIOD_S = 5;

type HealthLevel = "ok" | "warn" | "critical";

function getRecordingHealth(status: RecorderStatus): {
  level: HealthLevel;
  reasons: string[];
} {
  const reasons: string[] = [];
  let level: HealthLevel = "ok";

  // Too early — averages haven't converged yet
  if (status.elapsedSeconds < GRACE_PERIOD_S) {
    return { level, reasons };
  }

  function escalate(to: HealthLevel) {
    if (to === "critical" || level === "critical") level = "critical";
    else level = to;
  }

  // Primary signal: write FPS (frames encoded per second of recording time)
  if (status.writeFps > 0 && status.writeFps < WRITE_FPS_CRITICAL) {
    escalate("critical");
    reasons.push(
      `Write FPS dropped to ${status.writeFps.toFixed(1)} — encoder cannot keep up`,
    );
  } else if (status.writeFps > 0 && status.writeFps < WRITE_FPS_WARN) {
    escalate("warn");
    reasons.push(
      `Write FPS is low: ${status.writeFps.toFixed(1)} — encoder may be struggling`,
    );
  }

  // Secondary signal: frame drops (queue overflow — camera producing faster
  // than encoder can consume)
  const totalFrames = status.framesWritten + status.framesDropped;
  if (totalFrames > 60) {
    const dropRate = status.framesDropped / totalFrames;
    if (dropRate >= DROP_RATE_CRITICAL) {
      escalate("critical");
      reasons.push(
        `${(dropRate * 100).toFixed(1)}% of frames dropped (${status.framesDropped} of ${totalFrames})`,
      );
    } else if (dropRate >= DROP_RATE_WARN) {
      escalate("warn");
      reasons.push(
        `${(dropRate * 100).toFixed(1)}% of frames dropped (${status.framesDropped} of ${totalFrames})`,
      );
    }
  }

  return { level, reasons };
}

function writeFpsColor(fps: number, elapsedS: number): string {
  if (elapsedS < GRACE_PERIOD_S) return "text-foreground";
  if (fps <= 0) return "text-muted-foreground";
  if (fps < WRITE_FPS_CRITICAL) return "text-destructive";
  if (fps < WRITE_FPS_WARN) return "text-warning";
  return "text-foreground";
}

export function StatsPanel() {
  const status = useRecorderStore((s) => s.status);
  const isRecording = status.state === "recording";

  const health = isRecording
    ? getRecordingHealth(status)
    : { level: "ok" as HealthLevel, reasons: [] };

  // Play warning tone once when health degrades, not on every stats update.
  const prevHealthRef = useRef<HealthLevel>("ok");
  useEffect(() => {
    if (!isRecording) {
      prevHealthRef.current = "ok";
      return;
    }

    const prev = prevHealthRef.current;
    const curr = health.level;

    if (
      (prev === "ok" && curr !== "ok") ||
      (prev === "warn" && curr === "critical")
    ) {
      playWarningTone();
    }

    prevHealthRef.current = curr;
  }, [health.level, isRecording]);

  return (
    <div className="space-y-3">
      {/* Health warning banner */}
      {isRecording && health.level !== "ok" && (
        <div
          className={`flex items-start gap-3 rounded-lg px-4 py-3 text-sm ${
            health.level === "critical"
              ? "bg-destructive/15 border border-destructive/30 text-destructive"
              : "bg-warning/15 border border-warning/30 text-warning"
          }`}
        >
          <AlertTriangle className="size-5 shrink-0 mt-0.5" />
          <div className="space-y-1">
            <div className="font-semibold">
              {health.level === "critical"
                ? "Recording quality degraded"
                : "Recording quality warning"}
            </div>
            {health.reasons.map((reason, i) => (
              <div key={i} className="text-[12px] opacity-90">
                {reason}
              </div>
            ))}
            <div className="text-[11px] opacity-70 mt-1">
              {health.level === "critical"
                ? "Check USB connection, close other applications, or reduce resolution."
                : "Monitor — if this persists, the recording may have gaps."}
            </div>
          </div>
        </div>
      )}

      {/* Stats grid */}
      <div className="grid grid-cols-3 gap-3">
        <StatCard label="Frames Written" value={status.framesWritten.toLocaleString()} />
        <StatCard
          label="Frames Dropped"
          value={status.framesDropped.toLocaleString()}
          warn={status.framesDropped > 0}
        />
        <StatCard
          label="Write FPS"
          value={status.writeFps.toFixed(1)}
          colorClass={isRecording ? writeFpsColor(status.writeFps, status.elapsedSeconds) : undefined}
        />
        <StatCard label="Capture FPS" value={status.captureFps.toFixed(1)} />
        <StatCard label="File Size" value={`${status.fileSizeMb.toFixed(1)} MB`} />
        <StatCard
          label="Elapsed"
          value={formatElapsed(status.elapsedSeconds)}
        />
      </div>
    </div>
  );
}

function formatElapsed(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`;
}

function StatCard({
  label,
  value,
  warn,
  colorClass,
}: {
  label: string;
  value: string;
  warn?: boolean;
  colorClass?: string;
}) {
  const textClass = colorClass ?? (warn ? "text-destructive" : "text-foreground");
  return (
    <div className="rounded-lg border bg-card p-3 space-y-1">
      <div className="text-[10px] font-medium text-muted-foreground uppercase tracking-wider">
        {label}
      </div>
      <div className={`text-lg font-mono font-semibold tabular-nums ${textClass}`}>
        {value}
      </div>
    </div>
  );
}
