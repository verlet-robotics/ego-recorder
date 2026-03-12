import { useState, useEffect } from "react";
import { useViewerStore } from "@/stores/viewer-store";
import { FileList } from "./file-list";
import { MetadataPanel } from "./metadata-panel";
import { AnalysisPanel, PrunedFilesPanel } from "./analysis-panel";
import { VideoPlayer } from "@/components/video/video-player";
import { Separator } from "@/components/ui/separator";
import { commands } from "@/lib/tauri";
import { Film, BarChart3, Info, Archive } from "lucide-react";

type RightTab = "metadata" | "analysis" | "pruned";

export function ViewerPage() {
  const currentFile = useViewerStore((s) => s.currentFile);
  const conversionStatus = useViewerStore((s) => s.conversionStatus);
  const analysisResults = useViewerStore((s) => s.analysisResults);
  const [rightTab, setRightTab] = useState<RightTab>("metadata");
  const [streamUrl, setStreamUrl] = useState<string | null>(null);

  const status = currentFile ? (conversionStatus[currentFile] ?? "idle") : "idle";
  const hasAnalysis = Object.keys(analysisResults).length > 0;

  useEffect(() => {
    if (!currentFile || status !== "streamable") {
      setStreamUrl(null);
      return;
    }
    commands
      .getStreamUrl(currentFile)
      .then((url) => setStreamUrl(url))
      .catch(() => setStreamUrl(null));
  }, [currentFile, status]);

  return (
    <div className="flex gap-4 h-full">
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

      <div className="flex-1 min-w-0 flex flex-col gap-2">
        {currentFile ? (
          streamUrl ? (
            <>
              <div className="flex items-center gap-1.5 flex-shrink-0">
                <span className="text-[10px] uppercase tracking-wider text-muted-foreground ml-1">
                  Streaming (color)
                </span>
              </div>
              <VideoPlayer
                key={currentFile}
                src={streamUrl}
                className="flex-1 min-h-0"
              />
            </>
          ) : (
            <div className="flex-1 flex flex-col items-center justify-center text-muted-foreground gap-2 rounded-lg border border-border">
              <Film className="size-10 opacity-40" />
              <span className="text-sm">
                {status !== "streamable"
                  ? "Not streamable (non-H.264 codec)"
                  : "Loading stream..."}
              </span>
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

      <div className="w-72 shrink-0 flex flex-col border border-border rounded-lg overflow-hidden">
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
