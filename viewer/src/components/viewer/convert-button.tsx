import { useCallback, useEffect, useRef } from "react";
import { Button } from "@/components/ui/button";
import { useViewerStore } from "@/stores/viewer-store";
import { Loader2, Play, RefreshCw, AlertCircle, Layers } from "lucide-react";
import type { ConversionStatus } from "@/lib/types";

interface ConvertButtonProps {
  fileName: string;
}

export function ConvertButton({ fileName }: ConvertButtonProps) {
  const conversionStatus = useViewerStore((s) => s.conversionStatus);
  const setConversionStatus = useViewerStore((s) => s.setConversionStatus);
  const status = conversionStatus[fileName] ?? "idle";
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const startConversion = useCallback(async () => {
    try {
      const res = await fetch(`/api/files/${encodeURIComponent(fileName)}/convert`, {
        method: "POST",
      });
      const data = await res.json();
      setConversionStatus(fileName, data.status as ConversionStatus);
    } catch {
      setConversionStatus(fileName, "error");
    }
  }, [fileName, setConversionStatus]);

  // Poll for conversion status when converting/queued
  useEffect(() => {
    if (status !== "converting" && status !== "queued") {
      if (pollRef.current) {
        clearInterval(pollRef.current);
        pollRef.current = null;
      }
      return;
    }

    pollRef.current = setInterval(async () => {
      try {
        const res = await fetch(`/api/files/${encodeURIComponent(fileName)}/status`);
        const data = await res.json();
        setConversionStatus(fileName, data.status as ConversionStatus);
      } catch {
        // Ignore poll errors
      }
    }, 1000);

    return () => {
      if (pollRef.current) {
        clearInterval(pollRef.current);
        pollRef.current = null;
      }
    };
  }, [status, fileName, setConversionStatus]);

  if (status === "ready") return null;

  // Streamable files are already viewable (color) — offer optional conversion for depth
  if (status === "streamable") {
    return (
      <Button variant="outline" size="xs" onClick={startConversion}>
        <Layers className="size-3" />
        Convert for depth
      </Button>
    );
  }

  if (status === "converting" || status === "queued") {
    return (
      <div className="flex flex-col items-center justify-center gap-3 p-8">
        <Loader2 className="size-8 text-primary animate-spin" />
        <span className="text-sm text-muted-foreground">
          {status === "queued" ? "Queued for conversion..." : "Converting to MP4..."}
        </span>
      </div>
    );
  }

  if (status === "error") {
    return (
      <div className="flex flex-col items-center justify-center gap-3 p-8">
        <AlertCircle className="size-8 text-destructive" />
        <span className="text-sm text-destructive">Conversion failed</span>
        <Button variant="outline" size="sm" onClick={startConversion}>
          <RefreshCw className="size-3.5" />
          Retry
        </Button>
      </div>
    );
  }

  return (
    <div className="flex flex-col items-center justify-center gap-3 p-8">
      <p className="text-sm text-muted-foreground text-center max-w-xs">
        This recording needs to be converted to MP4 before it can be played in the browser.
      </p>
      <Button onClick={startConversion}>
        <Play className="size-4" />
        Convert to MP4
      </Button>
    </div>
  );
}
