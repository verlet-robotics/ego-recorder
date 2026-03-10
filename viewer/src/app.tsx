import { useEffect, useCallback } from "react";
import { TooltipProvider } from "@/components/ui/tooltip";
import { Button } from "@/components/ui/button";
import { ViewerPage } from "@/components/viewer/viewer-page";
import { useViewerStore } from "@/stores/viewer-store";
import { FolderOpen, ScanSearch, Loader2, CheckCircle2 } from "lucide-react";
import type { AnalysisResult } from "@/lib/types";

export function App() {
  const files = useViewerStore((s) => s.files);
  const dir = useViewerStore((s) => s.dir);
  const currentFile = useViewerStore((s) => s.currentFile);
  const setFiles = useViewerStore((s) => s.setFiles);
  const setDir = useViewerStore((s) => s.setDir);
  const selectFile = useViewerStore((s) => s.selectFile);

  const analysisStatus = useViewerStore((s) => s.analysisStatus);
  const setAnalysisStatus = useViewerStore((s) => s.setAnalysisStatus);
  const setAnalysisResults = useViewerStore((s) => s.setAnalysisResults);
  const setAnalysisError = useViewerStore((s) => s.setAnalysisError);
  const analysisResults = useViewerStore((s) => s.analysisResults);

  // Fetch file list on mount
  useEffect(() => {
    fetch("/api/files")
      .then((res) => {
        if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
        return res.json();
      })
      .then((data) => {
        setDir(data.dir ?? "");
        setFiles(data.files ?? []);
        // Auto-select first file
        if (data.files?.length > 0 && !currentFile) {
          selectFile(data.files[0].name);
        }
      })
      .catch((err) => {
        console.error("Failed to load files:", err);
      });
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Check for cached analysis on mount
  useEffect(() => {
    fetch("/api/analyze")
      .then((res) => res.json())
      .then((data) => {
        if (data.status === "done" && data.results?.length > 0) {
          setAnalysisResults(data.results as AnalysisResult[]);
          setAnalysisStatus("done");
        }
      })
      .catch(() => {});
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const runAnalysis = useCallback(async () => {
    setAnalysisStatus("running");
    setAnalysisError(null);
    try {
      const res = await fetch("/api/analyze", { method: "POST" });
      const data = await res.json();
      if (data.status === "done" && data.results) {
        setAnalysisResults(data.results as AnalysisResult[]);
        setAnalysisStatus("done");
      } else if (data.status === "error") {
        setAnalysisError(data.error || "Analysis failed");
        setAnalysisStatus("error");
      }
    } catch (err) {
      setAnalysisError(err instanceof Error ? err.message : "Analysis failed");
      setAnalysisStatus("error");
    }
  }, [setAnalysisStatus, setAnalysisResults, setAnalysisError]);

  const hasAnalysis = Object.keys(analysisResults).length > 0;
  const analysisCount = Object.keys(analysisResults).length;

  return (
    <TooltipProvider>
      <div className="h-screen flex flex-col bg-background text-foreground font-sans">
        {/* Header */}
        <header className="flex items-center gap-3 px-4 py-2 border-b border-border flex-shrink-0">
          <h1 className="font-serif text-[16px] font-bold">
            Egorec Viewer
          </h1>
          {dir && (
            <div className="flex items-center gap-1.5 text-[11px] text-muted-foreground min-w-0">
              <FolderOpen className="size-3 shrink-0" />
              <span className="truncate">{dir}</span>
            </div>
          )}

          <div className="ml-auto flex items-center gap-2">
            {files.length > 0 && (
              <Button
                variant={hasAnalysis ? "outline" : "highlight"}
                size="xs"
                onClick={runAnalysis}
                disabled={analysisStatus === "running"}
              >
                {analysisStatus === "running" ? (
                  <Loader2 className="size-3 animate-spin" />
                ) : hasAnalysis ? (
                  <CheckCircle2 className="size-3 text-success" />
                ) : (
                  <ScanSearch className="size-3" />
                )}
                {analysisStatus === "running"
                  ? "Analyzing..."
                  : hasAnalysis
                    ? "Re-analyze"
                    : "Analyze"}
              </Button>
            )}
            <span className="text-[11px] text-muted-foreground tabular-nums">
              {files.length} file{files.length !== 1 ? "s" : ""}
              {hasAnalysis && ` \u00b7 ${analysisCount} analyzed`}
            </span>
          </div>
        </header>

        {/* Main area */}
        <main className="flex-1 min-h-0 p-4">
          {files.length === 0 ? (
            <EmptyState />
          ) : (
            <ViewerPage />
          )}
        </main>
      </div>
    </TooltipProvider>
  );
}

function EmptyState() {
  return (
    <div className="flex flex-col items-center justify-center h-full gap-4 text-muted-foreground">
      <FolderOpen className="size-12 opacity-40" />
      <div className="text-center space-y-2">
        <h2 className="font-serif text-[20px] text-foreground">
          No Recordings Found
        </h2>
        <p className="text-[13px] max-w-md">
          Start the viewer with a <code className="font-mono text-[12px] bg-muted px-1.5 py-0.5 rounded">--dir</code> argument
          pointing to a directory containing <code className="font-mono text-[12px] bg-muted px-1.5 py-0.5 rounded">.egorec</code> files.
        </p>
        <pre className="text-[11px] font-mono bg-muted/50 rounded-lg p-3 mt-3 text-left inline-block">
          bun run dev -- --dir /path/to/recordings
        </pre>
      </div>
    </div>
  );
}
