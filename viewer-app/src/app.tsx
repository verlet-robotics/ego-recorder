import { useEffect, useCallback, useState } from "react";
import { TooltipProvider } from "@/components/ui/tooltip";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { ViewerPage } from "@/components/viewer/viewer-page";
import { CurationPage } from "@/components/curation/curation-page";
import { useViewerStore } from "@/stores/viewer-store";
import { commands, onAnalysisProgress, onMenuOpenDirectory } from "@/lib/tauri";
import {
  FolderOpen,
  ScanSearch,
  Loader2,
  CheckCircle2,
  Layers,
} from "lucide-react";
import type { AnalysisResult } from "@/lib/types";

type AppMode = "viewer" | "curation";

export function App() {
  const files = useViewerStore((s) => s.files);
  const dir = useViewerStore((s) => s.dir);
  const currentFile = useViewerStore((s) => s.currentFile);
  const setFiles = useViewerStore((s) => s.setFiles);
  const setDir = useViewerStore((s) => s.setDir);
  const selectFile = useViewerStore((s) => s.selectFile);
  const setVideoServerPort = useViewerStore((s) => s.setVideoServerPort);

  const analysisStatus = useViewerStore((s) => s.analysisStatus);
  const setAnalysisStatus = useViewerStore((s) => s.setAnalysisStatus);
  const setAnalysisResults = useViewerStore((s) => s.setAnalysisResults);
  const setAnalysisError = useViewerStore((s) => s.setAnalysisError);
  const analysisResults = useViewerStore((s) => s.analysisResults);

  const [mode, setMode] = useState<AppMode>("viewer");
  const [analysisProgressText, setAnalysisProgressText] = useState<string | null>(null);

  // On mount: check for CLI --dir or --workspace arg, discover files, fetch video port
  useEffect(() => {
    (async () => {
      const port = await commands.getVideoServerPort().catch(() => null);
      if (port) setVideoServerPort(port);

      // Check if a curation workspace was set via CLI
      const wsInfo = await commands.getCurationWorkspace().catch(() => null);
      if (wsInfo?.hasWorkspace) {
        setMode("curation");
        return;
      }

      const existingDir = await commands.getRecordingsDir().catch(() => null);
      if (existingDir) {
        setDir(existingDir);
        try {
          const resp = await commands.discoverFiles();
          setFiles(resp.files);
          if (resp.files.length > 0 && !currentFile) {
            selectFile(resp.files[0]!.name);
          }
        } catch (err) {
          console.error("Failed to discover files:", err);
        }
      }

      // Check for cached analysis
      try {
        const data = await commands.getAnalysis();
        if (data.status === "done" && data.results && data.results.length > 0) {
          setAnalysisResults(data.results);
          setAnalysisStatus("done");
        }
      } catch {}
    })();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const handleOpenDirectory = useCallback(async () => {
    const selectedDir = await commands.openDirectory();
    if (!selectedDir) return;

    setDir(selectedDir);
    try {
      const resp = await commands.discoverFiles();
      setFiles(resp.files);
      if (resp.files.length > 0) {
        selectFile(resp.files[0]!.name);
      }
    } catch (err) {
      console.error("Failed to discover files:", err);
    }
  }, [setDir, setFiles, selectFile]);

  useEffect(() => {
    const unlistenProgress = onAnalysisProgress((payload) => {
      setAnalysisProgressText(`${payload.current}/${payload.total}: ${payload.file}`);
    });
    const unlistenMenu = onMenuOpenDirectory(() => {
      handleOpenDirectory();
    });
    return () => {
      unlistenProgress.then((u) => u());
      unlistenMenu.then((u) => u());
    };
  }, [handleOpenDirectory]);

  const runAnalysis = useCallback(async () => {
    setAnalysisStatus("running");
    setAnalysisError(null);
    setAnalysisProgressText(null);
    try {
      const data = await commands.runAnalysis();
      if (data.status === "done" && data.results) {
        setAnalysisResults(data.results as AnalysisResult[]);
        setAnalysisStatus("done");
      } else if (data.error) {
        setAnalysisError(data.error);
        setAnalysisStatus("error");
      }
    } catch (err) {
      setAnalysisError(err instanceof Error ? err.message : "Analysis failed");
      setAnalysisStatus("error");
    } finally {
      setAnalysisProgressText(null);
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

          {dir ? (
            <div className="flex items-center gap-1.5 text-[11px] text-muted-foreground min-w-0">
              <FolderOpen className="size-3 shrink-0" />
              <span className="truncate">{dir}</span>
            </div>
          ) : (
            <Button variant="outline" size="xs" onClick={handleOpenDirectory}>
              <FolderOpen className="size-3" />
              Open Directory
            </Button>
          )}

          <div className="ml-auto flex items-center gap-2">
            <Button
              variant={mode === "viewer" ? "default" : "ghost"}
              size="xs"
              onClick={() => setMode("viewer")}
            >
              <FolderOpen className="size-3" />
              Viewer
            </Button>
            <Button
              variant={mode === "curation" ? "default" : "ghost"}
              size="xs"
              onClick={() => setMode("curation")}
            >
              <Layers className="size-3" />
              Curation
            </Button>
            <Separator orientation="vertical" className="h-4 mx-1" />
            {mode === "viewer" && dir && (
              <Button
                variant="outline"
                size="xs"
                onClick={handleOpenDirectory}
              >
                <FolderOpen className="size-3" />
                Change
              </Button>
            )}
            {mode === "viewer" && files.length > 0 && (
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
                  ? analysisProgressText ?? "Analyzing..."
                  : hasAnalysis
                    ? "Re-analyze"
                    : "Analyze"}
              </Button>
            )}
            {mode === "viewer" && (
              <span className="text-[11px] text-muted-foreground tabular-nums">
                {files.length} file{files.length !== 1 ? "s" : ""}
                {hasAnalysis && ` · ${analysisCount} analyzed`}
              </span>
            )}
          </div>
        </header>

        {/* Main area */}
        <main className="flex-1 min-h-0 p-4">
          {mode === "curation" ? (
            <CurationPage />
          ) : files.length === 0 ? (
            <EmptyState onOpen={handleOpenDirectory} hasDir={!!dir} />
          ) : (
            <ViewerPage />
          )}
        </main>
      </div>
    </TooltipProvider>
  );
}

function EmptyState({
  onOpen,
  hasDir,
}: {
  onOpen: () => void;
  hasDir: boolean;
}) {
  return (
    <div className="flex flex-col items-center justify-center h-full gap-4 text-muted-foreground">
      <FolderOpen className="size-12 opacity-40" />
      <div className="text-center space-y-2">
        <h2 className="font-serif text-[20px] text-foreground">
          {hasDir ? "No Recordings Found" : "Open a Recordings Directory"}
        </h2>
        <p className="text-[13px] max-w-md">
          {hasDir
            ? "The selected directory doesn't contain any .egorec files."
            : "Select a directory containing .egorec recording files to get started."}
        </p>
      </div>
      <Button variant="highlight" size="sm" onClick={onOpen}>
        <FolderOpen className="size-4" />
        {hasDir ? "Choose Another Directory" : "Open Directory"}
      </Button>
    </div>
  );
}
