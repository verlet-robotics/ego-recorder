import { useState } from "react";
import { useViewerStore } from "@/stores/viewer-store";
import { FileList } from "./file-list";
import { MetadataPanel } from "./metadata-panel";
import { AnalysisPanel, PrunedFilesPanel } from "./analysis-panel";
import { ConvertButton } from "./convert-button";
import { VideoPlayer } from "@/components/video/video-player";
import { Separator } from "@/components/ui/separator";
import { Button } from "@/components/ui/button";
import { Film, Camera, Layers, BarChart3, Info, Archive } from "lucide-react";
type VideoStream = "color" | "depth";
type RightTab = "metadata" | "analysis" | "pruned";

export function ViewerPage() {
  const currentFile = useViewerStore((s) => s.currentFile);
  const conversionStatus = useViewerStore((s) => s.conversionStatus);
  const analysisResults = useViewerStore((s) => s.analysisResults);
  const [activeStream, setActiveStream] = useState<VideoStream>("color");
  const [rightTab, setRightTab] = useState<RightTab>("metadata");

  const status = currentFile ? (conversionStatus[currentFile] ?? "idle") : "idle";
  const hasAnalysis = Object.keys(analysisResults).length > 0;

  // Auto-switch to analysis tab when results arrive
  if (hasAnalysis && rightTab === "metadata" && currentFile && analysisResults[currentFile]) {
    // don't auto-switch, let user choose
  }

  // Build video source based on status
  const videoSrc = (() => {
    if (!currentFile) return null;
    if (status === "streamable") {
      return `/stream/${encodeURIComponent(currentFile)}`;
    }
    if (status === "ready") {
      const base = `/video/${encodeURIComponent(currentFile)}`;
      return activeStream === "depth" ? `${base}/depth` : base;
    }
    return null;
  })();

  const canPlayVideo = status === "ready" || status === "streamable";
  const hasDepthToggle = status === "ready";

  return (
    <div className="flex gap-4 h-full">
      {/* Left panel: file list */}
      <div className="w-72 shrink-0 flex flex-col border border-border rounded-lg overflow-hidden">
        <div className="px-3 py-2 border-b border-border">
          <span className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
            Recordings
          </span>
        </div>
        <div className="flex-1 min-h-0">
          <FileList />
        </div>
      </div>

      {/* Center panel: video / conversion */}
      <div className="flex-1 min-w-0 flex flex-col gap-2">
        {currentFile ? (
          canPlayVideo && videoSrc ? (
            <>
              {/* Stream toggle — only show depth option for fully converted files */}
              {hasDepthToggle ? (
                <div className="flex items-center gap-1.5 flex-shrink-0">
                  <span className="text-[10px] uppercase tracking-wider text-muted-foreground ml-1">
                    Stream
                  </span>
                  <Button
                    variant={activeStream === "color" ? "default" : "outline"}
                    size="xs"
                    onClick={() => setActiveStream("color")}
                  >
                    <Camera className="size-3" />
                    Color
                  </Button>
                  <Button
                    variant={activeStream === "depth" ? "default" : "outline"}
                    size="xs"
                    onClick={() => setActiveStream("depth")}
                  >
                    <Layers className="size-3" />
                    Depth
                  </Button>
                </div>
              ) : (
                <div className="flex items-center gap-1.5 flex-shrink-0">
                  <span className="text-[10px] uppercase tracking-wider text-muted-foreground ml-1">
                    Streaming (color only)
                  </span>
                  <ConvertButton fileName={currentFile} />
                </div>
              )}
              <VideoPlayer
                key={`${currentFile}-${activeStream}`}
                src={videoSrc}
                className="flex-1 min-h-0"
              />
            </>
          ) : (
            <div className="flex-1 flex items-center justify-center rounded-lg border border-border">
              <ConvertButton fileName={currentFile} />
            </div>
          )
        ) : (
          <div className="flex-1 flex flex-col items-center justify-center text-muted-foreground gap-2 rounded-lg border border-border">
            <Film className="size-10 opacity-40" />
            <span className="text-sm">Select a recording</span>
          </div>
        )}
      </div>

      <Separator orientation="vertical" />

      {/* Right panel: metadata / analysis / pruned */}
      <div className="w-72 shrink-0 flex flex-col border border-border rounded-lg overflow-hidden">
        {/* Tab bar */}
        <div className="flex items-center border-b border-border">
          <button
            onClick={() => setRightTab("metadata")}
            className={`flex items-center gap-1 px-3 py-2 text-[10px] font-semibold uppercase tracking-wider transition-colors ${
              rightTab === "metadata"
                ? "text-foreground border-b-2 border-primary"
                : "text-muted-foreground hover:text-foreground"
            }`}
          >
            <Info className="size-3" />
            Info
          </button>
          <button
            onClick={() => setRightTab("analysis")}
            className={`flex items-center gap-1 px-3 py-2 text-[10px] font-semibold uppercase tracking-wider transition-colors ${
              rightTab === "analysis"
                ? "text-foreground border-b-2 border-primary"
                : "text-muted-foreground hover:text-foreground"
            }`}
          >
            <BarChart3 className="size-3" />
            Analysis
            {hasAnalysis && currentFile && analysisResults[currentFile] && (
              <span className="size-1.5 rounded-full bg-success ml-0.5" />
            )}
          </button>
          <button
            onClick={() => setRightTab("pruned")}
            className={`flex items-center gap-1 px-3 py-2 text-[10px] font-semibold uppercase tracking-wider transition-colors ${
              rightTab === "pruned"
                ? "text-foreground border-b-2 border-primary"
                : "text-muted-foreground hover:text-foreground"
            }`}
          >
            <Archive className="size-3" />
            Pruned
          </button>
        </div>

        {/* Tab content */}
        <div className="flex-1 min-h-0">
          {rightTab === "metadata" && (
            currentFile ? (
              <MetadataPanel fileName={currentFile} />
            ) : (
              <div className="flex items-center justify-center h-full text-muted-foreground text-[12px]">
                No file selected
              </div>
            )
          )}
          {rightTab === "analysis" && (
            currentFile ? (
              <AnalysisPanel fileName={currentFile} />
            ) : (
              <div className="flex items-center justify-center h-full text-muted-foreground text-[12px]">
                No file selected
              </div>
            )
          )}
          {rightTab === "pruned" && <PrunedFilesPanel />}
        </div>
      </div>
    </div>
  );
}
