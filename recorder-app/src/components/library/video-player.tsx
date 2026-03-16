/**
 * VideoPlayer — simplified single-video player for .egorec MP4 playback.
 *
 * Adapted from the annotation viewer's SyncVideoPlayer but stripped down to
 * a single video with play/pause, scrub, speed control, and frame stepping.
 *
 * Keyboard shortcuts:
 *   Space — play/pause
 *   Left/Right arrows — skip 5s
 *   , / . — frame step backward/forward
 *   [ / ] — decrease/increase speed
 *   Home/End — go to start/end
 */

import {
  useState,
  useRef,
  useEffect,
  useCallback,
} from "react";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import {
  Play,
  Pause,
  SkipBack,
  SkipForward,
  ChevronFirst,
  ChevronLast,
  Gauge,
} from "lucide-react";

export interface VideoPlayerDiagnostics {
  src: string;
  lastEvent: string;
  readyState: number;
  readyStateLabel: string;
  networkState: number;
  networkStateLabel: string;
  currentTime: number;
  duration: number;
  bufferedUntil: number;
  seekableUntil: number;
  paused: boolean;
  seeking: boolean;
  ended: boolean;
  mediaError: string | null;
}

// --- Speed presets ----

const SPEED_OPTIONS = [0.25, 0.5, 1, 1.5, 2, 4] as const;

function nextSpeed(current: number, direction: 1 | -1): number {
  const idx = SPEED_OPTIONS.indexOf(current as (typeof SPEED_OPTIONS)[number]);
  if (idx === -1) return 1;
  const len = SPEED_OPTIONS.length;
  const next = (idx + direction + len) % len;
  return SPEED_OPTIONS[next]!;
}

// --- Time formatting ----

function formatTime(seconds: number): string {
  if (!isFinite(seconds) || seconds < 0) return "0:00";
  const sign = seconds < 0 ? "-" : "";
  const total = Math.max(0, seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const sec = s.toFixed(1).padStart(4, "0");
  if (h > 0) {
    return `${sign}${h}:${m.toString().padStart(2, "0")}:${sec}`;
  }
  return `${sign}${m}:${sec}`;
}

function formatAbsoluteTime(startIso: string | null | undefined, offsetSeconds: number): string | null {
  if (!startIso) return null;
  const start = new Date(startIso);
  if (Number.isNaN(start.getTime())) return null;
  return new Date(start.getTime() + offsetSeconds * 1000).toLocaleString();
}

function rangeEnd(ranges: TimeRanges): number {
  if (!ranges.length) return 0;
  try {
    return ranges.end(ranges.length - 1);
  } catch {
    return 0;
  }
}

function readyStateLabel(value: number): string {
  switch (value) {
    case 0:
      return "HAVE_NOTHING";
    case 1:
      return "HAVE_METADATA";
    case 2:
      return "HAVE_CURRENT_DATA";
    case 3:
      return "HAVE_FUTURE_DATA";
    case 4:
      return "HAVE_ENOUGH_DATA";
    default:
      return `UNKNOWN_${value}`;
  }
}

function networkStateLabel(value: number): string {
  switch (value) {
    case 0:
      return "NETWORK_EMPTY";
    case 1:
      return "NETWORK_IDLE";
    case 2:
      return "NETWORK_LOADING";
    case 3:
      return "NETWORK_NO_SOURCE";
    default:
      return `UNKNOWN_${value}`;
  }
}

// --- Component ----

interface VideoPlayerProps {
  src: string;
  className?: string;
  timeOffsetS?: number;
  absoluteStartIso?: string | null;
  sourceLabel?: string;
  onTimeChange?: (timeS: number) => void;
  onDiagnosticsChange?: (diagnostics: VideoPlayerDiagnostics) => void;
}

export function VideoPlayer({
  src,
  className,
  timeOffsetS = 0,
  absoluteStartIso = null,
  sourceLabel = "source",
  onTimeChange,
  onDiagnosticsChange,
}: VideoPlayerProps) {
  const videoRef = useRef<HTMLVideoElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  const [isPlaying, setIsPlaying] = useState(false);
  const [speed, setSpeed] = useState(1);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [isScrubbing, setIsScrubbing] = useState(false);
  const [mediaError, setMediaError] = useState<string | null>(null);
  const [lastEvent, setLastEvent] = useState("idle");

  const isPlayingRef = useRef(false);
  const isScrubbingRef = useRef(false);
  const speedRef = useRef(1);
  const wasPlayingBeforeScrub = useRef(false);

  isPlayingRef.current = isPlaying;
  isScrubbingRef.current = isScrubbing;
  speedRef.current = speed;

  const publishDiagnostics = useCallback(
    (eventName: string) => {
      const vid = videoRef.current;
      if (!vid) return;
      onDiagnosticsChange?.({
        src,
        lastEvent: eventName,
        readyState: vid.readyState,
        readyStateLabel: readyStateLabel(vid.readyState),
        networkState: vid.networkState,
        networkStateLabel: networkStateLabel(vid.networkState),
        currentTime: vid.currentTime,
        duration: Number.isFinite(vid.duration) ? vid.duration : 0,
        bufferedUntil: rangeEnd(vid.buffered),
        seekableUntil: rangeEnd(vid.seekable),
        paused: vid.paused,
        seeking: vid.seeking,
        ended: vid.ended,
        mediaError: mediaError,
      });
    },
    [mediaError, onDiagnosticsChange, src],
  );

  // -- Reset on src change ----

  useEffect(() => {
    const vid = videoRef.current;
    setIsPlaying(false);
    isPlayingRef.current = false;
    setCurrentTime(0);
    setDuration(0);
    setIsScrubbing(false);
    isScrubbingRef.current = false;
    setMediaError(null);
    setLastEvent("reset");
    if (vid) {
      vid.pause();
      // React owns the src attribute; only force a reload here so the
      // element resets cleanly across Strict Mode's mount/cleanup/remount
      // cycle without manually mutating the DOM attribute.
      vid.load();
    }
    return () => {
      videoRef.current?.pause();
    };
  }, [src]);

  useEffect(() => {
    onTimeChange?.(currentTime);
  }, [currentTime, onTimeChange]);

  useEffect(() => {
    publishDiagnostics(lastEvent);
  }, [currentTime, duration, lastEvent, mediaError, publishDiagnostics]);

  // -- Playback controls ----

  const play = useCallback(() => {
    const vid = videoRef.current;
    if (!vid) return;
    vid.playbackRate = speedRef.current;
    vid.play().catch(() => {});
    setIsPlaying(true);
  }, []);

  const pause = useCallback(() => {
    videoRef.current?.pause();
    setIsPlaying(false);
  }, []);

  const togglePlay = useCallback(() => {
    if (isPlayingRef.current) pause();
    else play();
  }, [play, pause]);

  const seekTo = useCallback((time: number) => {
    const vid = videoRef.current;
    if (!vid) return;
    const clamped = Math.max(0, Math.min(time, vid.duration || 0));
    vid.currentTime = clamped;
    setCurrentTime(clamped);
  }, []);

  const stepFrame = useCallback(
    (direction: 1 | -1) => {
      pause();
      const vid = videoRef.current;
      if (!vid) return;
      seekTo(vid.currentTime + direction * (1 / 30));
    },
    [pause, seekTo],
  );

  const skip = useCallback(
    (seconds: number) => {
      const vid = videoRef.current;
      if (!vid) return;
      seekTo(vid.currentTime + seconds);
    },
    [seekTo],
  );

  const changeSpeed = useCallback((newSpeed: number) => {
    setSpeed(newSpeed);
    speedRef.current = newSpeed;
    const vid = videoRef.current;
    if (vid) vid.playbackRate = newSpeed;
  }, []);

  const goToStart = useCallback(() => seekTo(0), [seekTo]);
  const goToEnd = useCallback(() => seekTo(duration), [seekTo, duration]);

  // -- Video event handlers ----

  const onTimeUpdate = useCallback(() => {
    if (isScrubbingRef.current) return;
    const vid = videoRef.current;
    if (vid) {
      setCurrentTime(vid.currentTime);
      setLastEvent("timeupdate");
    }
  }, []);

  const onLoadedMetadata = useCallback(() => {
    const vid = videoRef.current;
    if (vid && isFinite(vid.duration)) {
      setDuration(vid.duration);
      setMediaError(null);
      setLastEvent("loadedmetadata");
    }
  }, []);

  const onEnded = useCallback(() => {
    setIsPlaying(false);
    setLastEvent("ended");
  }, []);

  const onError = useCallback(() => {
    const vid = videoRef.current;
    const code = vid?.error?.code;
    const suffix = typeof code === "number" ? ` (code ${code})` : "";
    setMediaError(`Video failed to load${suffix}`);
    setIsPlaying(false);
    setLastEvent("error");
  }, []);

  const onLoadStart = useCallback(() => {
    setLastEvent("loadstart");
  }, []);

  const onLoadedData = useCallback(() => {
    setLastEvent("loadeddata");
  }, []);

  const onCanPlay = useCallback(() => {
    setLastEvent("canplay");
  }, []);

  const onCanPlayThrough = useCallback(() => {
    setLastEvent("canplaythrough");
  }, []);

  const onWaiting = useCallback(() => {
    setLastEvent("waiting");
  }, []);

  const onStalled = useCallback(() => {
    setLastEvent("stalled");
  }, []);

  const onSuspend = useCallback(() => {
    setLastEvent("suspend");
  }, []);

  const onSeeking = useCallback(() => {
    setLastEvent("seeking");
  }, []);

  const onSeeked = useCallback(() => {
    setLastEvent("seeked");
  }, []);

  const onPlaying = useCallback(() => {
    setLastEvent("playing");
  }, []);

  const onProgress = useCallback(() => {
    setLastEvent("progress");
  }, []);

  // -- Scrub bar handlers ----

  const onScrubStart = useCallback(() => {
    wasPlayingBeforeScrub.current = isPlayingRef.current;
    setIsScrubbing(true);
    videoRef.current?.pause();
  }, []);

  const onScrubChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const time = parseFloat(e.target.value);
      setCurrentTime(time);
      const vid = videoRef.current;
      if (vid) vid.currentTime = time;
    },
    [],
  );

  const onScrubEnd = useCallback(() => {
    setIsScrubbing(false);
    if (wasPlayingBeforeScrub.current) {
      const vid = videoRef.current;
      if (vid) {
        vid.playbackRate = speedRef.current;
        vid.play().catch(() => {});
      }
      setIsPlaying(true);
    }
  }, []);

  // -- Keyboard shortcuts ----

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    function handleKey(e: KeyboardEvent) {
      if (
        e.target instanceof HTMLInputElement ||
        e.target instanceof HTMLTextAreaElement
      )
        return;

      switch (true) {
        case e.code === "Space": {
          e.preventDefault();
          e.stopPropagation();
          togglePlay();
          break;
        }
        case e.code === "ArrowLeft" && !e.shiftKey: {
          e.preventDefault();
          e.stopPropagation();
          skip(-5);
          break;
        }
        case e.code === "ArrowRight" && !e.shiftKey: {
          e.preventDefault();
          e.stopPropagation();
          skip(5);
          break;
        }
        case e.code === "Comma": {
          e.preventDefault();
          stepFrame(-1);
          break;
        }
        case e.code === "Period": {
          e.preventDefault();
          stepFrame(1);
          break;
        }
        case e.code === "BracketLeft": {
          e.preventDefault();
          changeSpeed(nextSpeed(speedRef.current, -1));
          break;
        }
        case e.code === "BracketRight": {
          e.preventDefault();
          changeSpeed(nextSpeed(speedRef.current, 1));
          break;
        }
        case e.code === "Home": {
          e.preventDefault();
          goToStart();
          break;
        }
        case e.code === "End": {
          e.preventDefault();
          goToEnd();
          break;
        }
      }
    }

    el.addEventListener("keydown", handleKey);
    return () => el.removeEventListener("keydown", handleKey);
  }, [togglePlay, skip, stepFrame, changeSpeed, goToStart, goToEnd]);

  // -- Render ----

  const progress = duration > 0 ? (currentTime / duration) * 100 : 0;
  const sourceCurrentTime = currentTime + timeOffsetS;
  const sourceEndTime = duration + timeOffsetS;
  const clipWindowLabel =
    timeOffsetS > 0
      ? `clip ${formatTime(currentTime)} / ${formatTime(duration)}`
      : `clip ${formatTime(currentTime)} / ${formatTime(duration)}`;
  const sourceWindowLabel = `${sourceLabel} ${formatTime(sourceCurrentTime)} / ${formatTime(sourceEndTime)}`;
  const absoluteCurrentLabel = formatAbsoluteTime(absoluteStartIso, sourceCurrentTime);

  return (
    <div
      ref={containerRef}
      tabIndex={0}
      className={cn(
        "flex flex-col gap-2 w-full outline-none focus:ring-0",
        className,
      )}
    >
      {/* Video element */}
      <div className="flex-1 min-h-0">
        <video
          ref={videoRef}
          src={src}
          preload="metadata"
          playsInline
          muted
          onLoadStart={onLoadStart}
          onTimeUpdate={onTimeUpdate}
          onLoadedMetadata={onLoadedMetadata}
          onLoadedData={onLoadedData}
          onCanPlay={onCanPlay}
          onCanPlayThrough={onCanPlayThrough}
          onEnded={onEnded}
          onWaiting={onWaiting}
          onStalled={onStalled}
          onSuspend={onSuspend}
          onSeeking={onSeeking}
          onSeeked={onSeeked}
          onPlaying={onPlaying}
          onProgress={onProgress}
          onError={onError}
          onClick={togglePlay}
          className="w-full h-full rounded-lg object-contain bg-black/5 cursor-pointer"
        />
      </div>

      {/* Transport controls */}
      <div className="flex flex-col gap-1.5">
        <div className="flex items-center justify-between gap-3 text-[10px] text-muted-foreground tabular-nums">
          <span>{clipWindowLabel}</span>
          <span className="text-right truncate">{absoluteCurrentLabel ?? sourceWindowLabel}</span>
        </div>
        {mediaError && <div className="text-[10px] text-destructive">{mediaError}</div>}

        {/* Scrub bar */}
        <div className="group relative flex items-center gap-2">
          <span className="text-[10px] tabular-nums text-muted-foreground w-16 text-right shrink-0">
            {formatTime(sourceCurrentTime)}
          </span>
          <div className="relative flex-1 h-5 flex items-center">
            <div className="absolute inset-x-0 h-1 rounded-full bg-muted" />
            <div
              className="absolute left-0 h-1 rounded-full bg-primary transition-[width] duration-75"
              style={{ width: `${progress}%` }}
            />
            <input
              type="range"
              min={0}
              max={duration || 1}
              step={0.01}
              value={currentTime}
              onMouseDown={onScrubStart}
              onTouchStart={onScrubStart}
              onChange={onScrubChange}
              onMouseUp={onScrubEnd}
              onTouchEnd={onScrubEnd}
              className="absolute inset-0 w-full h-full opacity-0 cursor-pointer z-10"
            />
            <div
              className="absolute top-1/2 -translate-y-1/2 size-3 rounded-full bg-primary shadow-sm border border-primary-foreground/20 pointer-events-none transition-opacity opacity-0 group-hover:opacity-100"
              style={{ left: `calc(${progress}% - 6px)` }}
            />
          </div>
          <span className="text-[10px] tabular-nums text-muted-foreground w-16 shrink-0">
            {formatTime(sourceEndTime)}
          </span>
        </div>

        {/* Buttons row */}
        <div className="flex items-center justify-center gap-1">
          {/* Left: speed */}
          <div className="flex items-center gap-1 mr-auto">
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="sm"
                  className={cn(
                    "h-7 text-xs px-2 tabular-nums gap-1",
                    speed !== 1 && "text-primary font-semibold",
                  )}
                  onClick={() => changeSpeed(nextSpeed(speed, 1))}
                >
                  <Gauge className="size-3" />
                  {speed}x
                </Button>
              </TooltipTrigger>
              <TooltipContent side="top">
                <p className="text-xs">
                  Playback speed ·{" "}
                  <kbd className="font-mono text-[10px]">[</kbd> slower{" "}
                  <kbd className="font-mono text-[10px]">]</kbd> faster
                </p>
              </TooltipContent>
            </Tooltip>
          </div>

          {/* Center: transport */}
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className="size-7"
                onClick={goToStart}
              >
                <ChevronFirst className="size-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent side="top">
              <span className="text-xs">
                Start · <kbd className="font-mono text-[10px]">Home</kbd>
              </span>
            </TooltipContent>
          </Tooltip>

          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className="size-7"
                onClick={() => stepFrame(-1)}
              >
                <SkipBack className="size-3.5" />
              </Button>
            </TooltipTrigger>
            <TooltipContent side="top">
              <span className="text-xs">
                Frame back ·{" "}
                <kbd className="font-mono text-[10px]">,</kbd>
              </span>
            </TooltipContent>
          </Tooltip>

          <Button
            variant="outline"
            size="icon"
            className="size-8"
            onClick={togglePlay}
          >
            {isPlaying ? (
              <Pause className="size-4" />
            ) : (
              <Play className="size-4 ml-0.5" />
            )}
          </Button>

          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className="size-7"
                onClick={() => stepFrame(1)}
              >
                <SkipForward className="size-3.5" />
              </Button>
            </TooltipTrigger>
            <TooltipContent side="top">
              <span className="text-xs">
                Frame forward ·{" "}
                <kbd className="font-mono text-[10px]">.</kbd>
              </span>
            </TooltipContent>
          </Tooltip>

          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className="size-7"
                onClick={goToEnd}
              >
                <ChevronLast className="size-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent side="top">
              <span className="text-xs">
                End · <kbd className="font-mono text-[10px]">End</kbd>
              </span>
            </TooltipContent>
          </Tooltip>

          {/* Right spacer for symmetry */}
          <div className="ml-auto w-16" />
        </div>
      </div>
    </div>
  );
}
