import { useState, useEffect, useCallback } from "react";
import { Button } from "@/components/ui/button";
import { useAppStore } from "@/stores/app-store";
import { useRecorderStore } from "@/stores/recorder-store";
import { commands, onFileAdded, onFileRemoved } from "@/lib/tauri";
import { FileList } from "./file-list";
import { VideoPlayer } from "./video-player";
import { MetadataPanel } from "./metadata-panel";
import { FolderOpen } from "lucide-react";

export function LibraryPage() {
  const config = useAppStore((s) => s.config);
  const libraryDir = useRecorderStore((s) => s.libraryDir);
  const setLibraryDir = useRecorderStore((s) => s.setLibraryDir);
  const files = useRecorderStore((s) => s.files);
  const setFiles = useRecorderStore((s) => s.setFiles);
  const addOrUpdateFile = useRecorderStore((s) => s.addOrUpdateFile);
  const removeFile = useRecorderStore((s) => s.removeFile);
  const currentFile = useRecorderStore((s) => s.currentFile);
  const selectFile = useRecorderStore((s) => s.selectFile);
  const videoServerPort = useRecorderStore((s) => s.videoServerPort);
  const setVideoServerPort = useRecorderStore((s) => s.setVideoServerPort);

  const [streamUrl, setStreamUrl] = useState<string | null>(null);

  // Get video server port on mount
  useEffect(() => {
    commands.getVideoServerPort().then((port) => {
      if (port) setVideoServerPort(port);
    });
  }, [setVideoServerPort]);

  // Auto-load from output_dir on mount (if no dir is set yet)
  useEffect(() => {
    if (libraryDir) return; // already have a dir
    const dir = config?.storage.output_dir;
    if (!dir) return;

    setLibraryDir(dir);
    commands.discoverFiles(dir).then((result) => {
      setFiles(result.files);
      if (result.files.length > 0) {
        selectFile(result.files[0]!.name);
      }
    }).catch(() => {});

    // Tell the watcher to watch this dir (may already be watching from startup)
    commands.watchDirectory(dir).catch(() => {});
  }, [config?.storage.output_dir, libraryDir, setLibraryDir, setFiles, selectFile]);

  // Subscribe to file watcher events
  useEffect(() => {
    const subscriptions = [
      onFileAdded((item) => addOrUpdateFile(item)),
      onFileRemoved((name) => removeFile(name)),
    ];
    return () => {
      subscriptions.forEach((p) => p.then((unsub) => unsub()));
    };
  }, [addOrUpdateFile, removeFile]);

  // Load stream URL when file changes
  useEffect(() => {
    if (!currentFile) {
      setStreamUrl(null);
      return;
    }
    commands.getStreamUrl(currentFile).then(setStreamUrl).catch(() => setStreamUrl(null));
  }, [currentFile]);

  const handleOpenDir = useCallback(async () => {
    const dir = await commands.openDirectory();
    if (!dir) return;
    setLibraryDir(dir);
    const result = await commands.discoverFiles(dir);
    setFiles(result.files);
    if (result.files.length > 0) {
      selectFile(result.files[0]!.name);
    }
    // Switch watcher to the new directory
    commands.watchDirectory(dir).catch(() => {});
  }, [setLibraryDir, setFiles, selectFile]);

  if (!libraryDir || files.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center h-full gap-4 text-muted-foreground">
        <FolderOpen className="size-12 opacity-30" />
        <p className="text-sm">
          {libraryDir
            ? "No recordings found in this directory"
            : "Open a directory to browse recordings"}
        </p>
        <Button variant="outline" onClick={handleOpenDir}>
          <FolderOpen className="size-4" />
          Open Directory
        </Button>
      </div>
    );
  }

  const selectedFileData = files.find((f) => f.name === currentFile);
  const isStreamable = selectedFileData?.conversionStatus === "streamable";

  return (
    <div className="flex h-full">
      {/* File list - left panel */}
      <div className="w-72 border-r border-border shrink-0">
        <div className="p-2 border-b border-border">
          <Button variant="ghost" size="sm" className="w-full justify-start gap-2 text-xs" onClick={handleOpenDir}>
            <FolderOpen className="size-3" />
            {libraryDir.split("/").pop()}
          </Button>
        </div>
        <FileList />
      </div>

      {/* Center - video player */}
      <div className="flex-1 min-w-0 p-4">
        {currentFile && isStreamable && streamUrl ? (
          <VideoPlayer src={streamUrl} className="h-full" />
        ) : currentFile ? (
          <div className="flex items-center justify-center h-full text-muted-foreground text-sm">
            Not streamable (not H.264 encoded)
          </div>
        ) : (
          <div className="flex items-center justify-center h-full text-muted-foreground text-sm">
            Select a file to preview
          </div>
        )}
      </div>

      {/* Right panel - metadata */}
      <div className="w-72 border-l border-border shrink-0">
        {currentFile ? (
          <MetadataPanel fileName={currentFile} />
        ) : (
          <div className="flex items-center justify-center h-full text-muted-foreground text-[12px]">
            Select a file to view metadata
          </div>
        )}
      </div>
    </div>
  );
}
