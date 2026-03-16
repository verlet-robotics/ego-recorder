import { useState, useCallback, useEffect } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { useRecorderStore } from "@/stores/recorder-store";
import { cn } from "@/lib/utils";
import { RefreshCw, AlertTriangle, Loader2, Usb } from "lucide-react";
import { commands } from "@/lib/tauri";
import { onUsbWarning } from "@/lib/tauri";

function formatElapsed(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`;
}

export function CameraPreview() {
  const previewState = useRecorderStore((s) => s.previewState);
  const cameraConnected = useRecorderStore((s) => s.cameraConnected);
  const cameraInfo = useRecorderStore((s) => s.cameraInfo);
  const rgbUrl = useRecorderStore((s) => s.previewRgbUrl);
  const depthUrl = useRecorderStore((s) => s.previewDepthUrl);
  const status = useRecorderStore((s) => s.status);

  const [restartCounter, setRestartCounter] = useState(0);
  const [retrying, setRetrying] = useState(false);
  const [streamError, setStreamError] = useState(false);
  const [usbWarning, setUsbWarning] = useState<string | null>(null);

  const isRecording = status.state === "recording";

  // Listen for USB 2.0 warning events from the backend
  useEffect(() => {
    const unlisten = onUsbWarning((msg) => setUsbWarning(msg));
    return () => { unlisten.then((f) => f()); };
  }, []);

  // Clear stale USB warning when preview restarts (new subprocess may be on USB 3.0)
  useEffect(() => {
    if (previewState === "retrying" || previewState === "starting") {
      setUsbWarning(null);
    }
  }, [previewState]);

  // Detect stream loss via img onerror (works with MJPEG unlike onLoad)
  const handleImgError = useCallback(() => {
    setStreamError(true);
  }, []);

  const handleImgLoad = useCallback(() => {
    setStreamError(false);
  }, []);

  const handleRetry = async () => {
    setRetrying(true);
    setStreamError(false);
    setUsbWarning(null);
    try {
      await commands.stopPreview().catch(() => {});
      await new Promise((r) => setTimeout(r, 500));
      await commands.startPreview();
      const rgbUrl = await commands.getPreviewUrl("rgb");
      const depthUrl = await commands.getPreviewUrl("depth");
      useRecorderStore.getState().setPreviewUrls(rgbUrl, depthUrl);
      useRecorderStore.getState().setPreviewState("previewing");
      setRestartCounter((c) => c + 1);
    } catch {
      // Error state will be set by events
    } finally {
      setRetrying(false);
    }
  };

  // No camera connected and not actively previewing/recording
  if (!cameraConnected && (previewState === "off" || previewState === "error")) {
    return (
      <div className="flex items-center justify-center h-[280px] rounded-lg border-2 border-dashed border-muted-foreground/25 bg-muted/5">
        <div className="flex flex-col items-center gap-3 text-center">
          <Usb className="size-8 text-muted-foreground/50" />
          <p className="text-sm font-medium text-muted-foreground">Connect a RealSense camera to start</p>
          <p className="text-xs text-muted-foreground/60">Preview will begin automatically</p>
        </div>
      </div>
    );
  }

  // Off or starting: skeleton loader
  if (previewState === "off" || previewState === "starting") {
    return (
      <div className="flex gap-3 h-[280px]">
        <Skeleton className="flex-[3] rounded-lg" />
        <Skeleton className="flex-[2] rounded-lg" />
      </div>
    );
  }

  // Retrying state: spinner with attempt info
  if (previewState === "retrying") {
    return (
      <div className="flex items-center justify-center h-[280px] rounded-lg border border-yellow-500/30 bg-yellow-900/5">
        <div className="flex flex-col items-center gap-3 text-center">
          <Loader2 className="size-8 text-yellow-500 animate-spin" />
          <p className="text-sm text-muted-foreground">Camera crashed, retrying...</p>
        </div>
      </div>
    );
  }

  // Error state
  if (previewState === "error") {
    return (
      <div className="flex items-center justify-center h-[280px] rounded-lg border border-destructive/30 bg-destructive/5">
        <div className="flex flex-col items-center gap-3 text-center">
          <AlertTriangle className="size-8 text-destructive" />
          <p className="text-sm text-muted-foreground">Camera disconnected</p>
          <Button
            variant="outline"
            size="sm"
            onClick={handleRetry}
            disabled={retrying}
            className="gap-1.5"
          >
            <RefreshCw className={cn("size-3.5", retrying && "animate-spin")} />
            {retrying ? "Reconnecting..." : "Retry"}
          </Button>
        </div>
      </div>
    );
  }

  // Previewing or recording: show live feeds
  return (
    <div className="relative">
      <div
        className={cn(
          "flex gap-3 h-[280px] rounded-lg overflow-hidden",
          isRecording && "ring-2 ring-destructive"
        )}
      >
        {/* RGB preview (60% width) */}
        <div className="flex-[3] relative bg-black rounded-lg overflow-hidden">
          {rgbUrl ? (
            <img
              key={`rgb-${restartCounter}`}
              src={rgbUrl}
              alt="RGB preview"
              className="w-full h-full object-contain"
              onLoad={handleImgLoad}
              onError={handleImgError}
            />
          ) : (
            <Skeleton className="w-full h-full" />
          )}
        </div>

        {/* Depth preview (40% width) */}
        <div className="flex-[2] relative bg-black rounded-lg overflow-hidden">
          {depthUrl ? (
            <img
              key={`depth-${restartCounter}`}
              src={depthUrl}
              alt="Depth preview"
              className="w-full h-full object-contain"
              onError={handleImgError}
            />
          ) : (
            <Skeleton className="w-full h-full" />
          )}
        </div>
      </div>

      {/* USB 2.0 warning banner */}
      {usbWarning && (
        <div className="absolute top-2 left-1/2 -translate-x-1/2 flex items-center gap-1.5 bg-amber-900/80 text-amber-200 px-3 py-1.5 rounded text-xs backdrop-blur-sm z-10">
          <AlertTriangle className="size-3 shrink-0" />
          {usbWarning}
        </div>
      )}

      {/* Stream error indicator */}
      {streamError && (
        <div className="absolute top-2 right-2 flex items-center gap-1.5 bg-yellow-900/80 text-yellow-200 px-2 py-1 rounded text-xs backdrop-blur-sm">
          <AlertTriangle className="size-3" />
          Stream lost
        </div>
      )}

      {/* Camera info badges */}
      {cameraInfo && (
        <div className="absolute bottom-2 left-2 flex gap-1.5">
          <Badge variant="chip" className="bg-black/60 text-white text-[10px] backdrop-blur-sm">
            {cameraInfo.serial}
          </Badge>
          <Badge variant="chip" className="bg-black/60 text-white text-[10px] backdrop-blur-sm">
            {cameraInfo.width}x{cameraInfo.height}
          </Badge>
          <Badge variant="chip" className="bg-black/60 text-white text-[10px] backdrop-blur-sm">
            USB {cameraInfo.usb}
          </Badge>
          {cameraInfo.hasImu && (
            <Badge variant="chip" className="bg-black/60 text-white text-[10px] backdrop-blur-sm">
              IMU
            </Badge>
          )}
        </div>
      )}

      {/* REC overlay during recording */}
      {isRecording && (
        <>
          <div className="absolute top-2 left-2 flex items-center gap-1.5">
            <span className="size-2.5 rounded-full bg-destructive animate-pulse" />
            <span className="text-xs font-bold text-white drop-shadow-md">REC</span>
          </div>
          <div className="absolute top-2 right-2 flex items-center gap-2 bg-black/70 px-3 py-1.5 rounded-md backdrop-blur-sm">
            <span className="text-xl font-mono font-bold text-white tabular-nums tracking-tight">
              {formatElapsed(status.elapsedSeconds)}
            </span>
            <span className="text-lg font-mono text-white/80 tabular-nums">
              {status.framesWritten.toLocaleString()}f
            </span>
          </div>
        </>
      )}
    </div>
  );
}
