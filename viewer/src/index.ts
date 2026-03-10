// Bun.serve server for the egorec viewer.
//
// Discovers .egorec files from a --dir CLI argument, parses header/footer
// metadata, provides QC operations (analyze, prune, splice, mp4 conversion)
// via ego-qc, and serves converted videos for browser playback.

import { serve } from "bun";
import { resolve, join, basename, dirname, relative } from "node:path";
import { readdir, stat, mkdir } from "node:fs/promises";
import { parseEgorecMetadata } from "./lib/egorec-parser";
import { createEgorecVideoStream } from "./lib/egorec-remux";
import type { ConversionStatus, EgorecMetadata } from "./lib/types";
import index from "./index.html";

// ── CLI argument parsing ────────────────────────────────────────────────────

function parseArgs(): { dir: string; qc: string } {
  const args = process.argv.slice(2);

  const dirIdx = args.indexOf("--dir");
  if (dirIdx === -1 || dirIdx + 1 >= args.length) {
    console.error("Usage: bun run dev -- --dir /path/to/recordings [--qc /path/to/ego-qc]");
    process.exit(1);
  }
  const dir = resolve(args[dirIdx + 1]!);

  const qcIdx = args.indexOf("--qc");
  const qc = qcIdx !== -1 && qcIdx + 1 < args.length
    ? args[qcIdx + 1]!
    : "ego-qc";

  return { dir, qc };
}

const { dir: RECORDINGS_DIR, qc: QC } = parseArgs();

// ── In-memory file index ────────────────────────────────────────────────────

interface FileEntry {
  name: string;
  path: string;
  sizeBytes: number;
  metadata: EgorecMetadata;
  conversionStatus: ConversionStatus;
  error?: string;
}

const fileIndex = new Map<string, FileEntry>();
let cacheDir: string;

/** Resolve cache path for a file entry name (which may include subdirectory). */
function getCachePath(name: string, suffix: string = ".mp4"): string {
  const dir = dirname(name);
  const stem = basename(name).replace(/\.egorec$/, "");
  return join(cacheDir, dir, `${stem}${suffix}`);
}

// ── Conversion queue ────────────────────────────────────────────────────────

let activeConversion: string | null = null;
const conversionQueue: string[] = [];

async function processConversionQueue() {
  if (activeConversion) return;

  const next = conversionQueue.shift();
  if (!next) return;

  const entry = fileIndex.get(next);
  if (!entry) return;

  activeConversion = next;
  entry.conversionStatus = "converting";

  try {
    const mp4Path = getCachePath(next);
    const outputDir = dirname(mp4Path);
    await mkdir(outputDir, { recursive: true });

    const proc = Bun.spawn([QC, "mp4", entry.path, "-o", outputDir, "-q"], {
      stdout: "pipe",
      stderr: "pipe",
    });

    const exitCode = await proc.exited;
    const stderr = await new Response(proc.stderr).text();

    if (exitCode !== 0) {
      entry.conversionStatus = "error";
      entry.error = stderr.trim() || `ego-qc mp4 exited with code ${exitCode}`;
      console.error(`Conversion failed for ${next}: ${entry.error}`);
    } else {
      // Verify output exists
      const mp4File = Bun.file(mp4Path);

      if (await mp4File.exists()) {
        entry.conversionStatus = "ready";
        console.log(`Converted: ${next} → ${mp4Path}`);
      } else {
        entry.conversionStatus = "error";
        entry.error = "MP4 file not found after conversion";
      }
    }
  } catch (err) {
    entry.conversionStatus = "error";
    entry.error = err instanceof Error ? err.message : String(err);
    console.error(`Conversion error for ${next}:`, entry.error);
  } finally {
    activeConversion = null;
    // Process next in queue
    processConversionQueue();
  }
}

// ── File discovery ──────────────────────────────────────────────────────────

async function discoverFiles(): Promise<void> {
  // Set up cache directory
  cacheDir = join(RECORDINGS_DIR, ".cache");
  await mkdir(cacheDir, { recursive: true });

  // Recursively scan for .egorec files
  async function scanDir(dirPath: string): Promise<void> {
    const entries = await readdir(dirPath, { withFileTypes: true }).catch(() => []);

    for (const entry of entries) {
      const fullPath = join(dirPath, entry.name);

      // Recurse into non-hidden subdirectories
      if (entry.isDirectory()) {
        if (entry.name.startsWith(".")) continue;
        await scanDir(fullPath);
        continue;
      }

      if (!entry.isFile() || !entry.name.endsWith(".egorec")) continue;

      // Use relative path from RECORDINGS_DIR as the key
      const relPath = relative(RECORDINGS_DIR, fullPath);

      try {
        const fileStat = await stat(fullPath);
        const metadata = await parseEgorecMetadata(fullPath);

        // Check if already converted
        const mp4Path = getCachePath(relPath);
        const isConverted = await Bun.file(mp4Path).exists();

        // Determine initial status:
        // - already converted → "ready"
        // - H.264 encoded (rgbCodec=2) → "streamable" (can remux without full conversion)
        // - otherwise → "idle" (needs full conversion)
        let conversionStatus: ConversionStatus = "idle";
        if (isConverted) {
          conversionStatus = "ready";
        } else if (metadata.rgbCodec === 2) {
          conversionStatus = "streamable";
        }

        fileIndex.set(relPath, {
          name: relPath,
          path: fullPath,
          sizeBytes: fileStat.size,
          metadata,
          conversionStatus,
        });
      } catch (err) {
        console.warn(`Skipping ${relPath}: ${err instanceof Error ? err.message : err}`);
      }
    }
  }

  await scanDir(RECORDINGS_DIR);
  console.log(`Discovered ${fileIndex.size} .egorec files in ${RECORDINGS_DIR}`);
}

// ── Analysis cache ──────────────────────────────────────────────────────────

let analysisCache: unknown[] | null = null;
let analysisRunning = false;

// ── Route handlers ──────────────────────────────────────────────────────────

function jsonResponse(data: unknown, status = 200): Response {
  return new Response(JSON.stringify(data), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function handleGetFiles(): Response {
  const files = Array.from(fileIndex.values()).map((f) => {
    const dir = dirname(f.name);
    return {
      name: f.name,
      dataset: dir === "." ? null : dir,
      sessionName: f.metadata.sessionName,
      rgbCodec: f.metadata.rgbCodec,
      colorWidth: f.metadata.colorWidth,
      colorHeight: f.metadata.colorHeight,
      fps: f.metadata.fps,
      totalFrames: f.metadata.totalFrames,
      durationS: f.metadata.durationS,
      sizeBytes: f.sizeBytes,
      conversionStatus: f.conversionStatus,
      hasImu: f.metadata.hasImu,
    };
  });

  // Sort by name
  files.sort((a, b) => a.name.localeCompare(b.name));

  return jsonResponse({
    dir: RECORDINGS_DIR,
    qc: QC,
    files,
  });
}

function handleGetFile(name: string): Response {
  const entry = fileIndex.get(name);
  if (!entry) {
    return jsonResponse({ error: "File not found" }, 404);
  }

  return jsonResponse({
    name: entry.name,
    metadata: entry.metadata,
    sizeBytes: entry.sizeBytes,
    conversionStatus: entry.conversionStatus,
    error: entry.error,
  });
}

function handleConvert(name: string): Response {
  const entry = fileIndex.get(name);
  if (!entry) {
    return jsonResponse({ error: "File not found" }, 404);
  }

  if (entry.conversionStatus === "ready") {
    return jsonResponse({ status: "ready", message: "Already converted" });
  }

  if (entry.conversionStatus === "converting" || entry.conversionStatus === "queued") {
    return jsonResponse({ status: entry.conversionStatus, message: "Conversion already in progress" });
  }

  // Reset error state if retrying
  entry.error = undefined;
  entry.conversionStatus = "queued";
  conversionQueue.push(name);

  // Start processing
  processConversionQueue();

  return jsonResponse({ status: "queued", message: "Conversion queued" });
}

function handleGetStatus(name: string): Response {
  const entry = fileIndex.get(name);
  if (!entry) {
    return jsonResponse({ error: "File not found" }, 404);
  }

  return jsonResponse({
    name: entry.name,
    status: entry.conversionStatus,
    error: entry.error,
  });
}

async function handleVideoProxy(name: string, stream: "color" | "depth"): Promise<Response> {
  const entry = fileIndex.get(name);
  if (!entry) {
    return new Response("File not found", { status: 404 });
  }

  if (entry.conversionStatus !== "ready") {
    return new Response("Video not yet converted", { status: 404 });
  }

  const suffix = stream === "depth" ? ".depth.mp4" : ".mp4";
  const mp4Path = getCachePath(name, suffix);
  const file = Bun.file(mp4Path);

  if (!(await file.exists())) {
    if (stream === "color") entry.conversionStatus = "idle";
    return new Response(`${stream} video file missing`, { status: 404 });
  }

  // Bun.file() handles Range headers automatically
  return new Response(file, {
    headers: {
      "Content-Type": "video/mp4",
    },
  });
}

async function handleStreamProxy(name: string, req: Request): Promise<Response> {
  const entry = fileIndex.get(name);
  if (!entry) {
    return new Response("Not found", { status: 404 });
  }

  if (entry.metadata.rgbCodec !== 2) {
    return new Response("Not H.264 — use full conversion", { status: 400 });
  }

  // Check cache first — Bun.file() handles Range headers automatically
  const cachedPath = getCachePath(name, ".stream.mp4");
  const cachedFile = Bun.file(cachedPath);
  if (await cachedFile.exists()) {
    return new Response(cachedFile, {
      headers: { "Content-Type": "video/mp4" },
    });
  }

  const stream = await createEgorecVideoStream(entry.path, {
    fps: entry.metadata.fps || 30,
    signal: req.signal,
  });

  return new Response(stream, {
    headers: { "Content-Type": "video/mp4" },
  });
}

// ── Analysis & curation handlers ────────────────────────────────────────────

async function handleAnalyze(): Promise<Response> {
  if (analysisRunning) {
    return jsonResponse({ status: "running" });
  }

  analysisRunning = true;
  analysisCache = null;

  try {
    // Collect all .egorec file paths
    const paths = Array.from(fileIndex.values()).map((f) => f.path);
    if (paths.length === 0) {
      analysisRunning = false;
      return jsonResponse({ status: "done", results: [] });
    }

    const proc = Bun.spawn(
      [QC, "analyze", ...paths, "--report", "/dev/stdout"],
      { stdout: "pipe", stderr: "pipe" },
    );

    const [stdout, stderr] = await Promise.all([
      new Response(proc.stdout).text(),
      new Response(proc.stderr).text(),
    ]);
    const exitCode = await proc.exited;

    if (exitCode !== 0) {
      analysisRunning = false;
      return jsonResponse(
        { status: "error", error: stderr.trim() || `exit code ${exitCode}` },
        500,
      );
    }

    // ego-qc analyze --report writes JSON to the report path
    const results = JSON.parse(stdout);
    analysisCache = results;
    analysisRunning = false;

    return jsonResponse({ status: "done", results });
  } catch (err) {
    analysisRunning = false;
    return jsonResponse(
      { status: "error", error: err instanceof Error ? err.message : String(err) },
      500,
    );
  }
}

function handleGetAnalysis(): Response {
  if (analysisRunning) {
    return jsonResponse({ status: "running" });
  }
  if (analysisCache) {
    return jsonResponse({ status: "done", results: analysisCache });
  }
  return jsonResponse({ status: "idle" });
}

async function handlePrune(name: string): Promise<Response> {
  const entry = fileIndex.get(name);
  if (!entry) {
    return jsonResponse({ error: "File not found" }, 404);
  }

  const proc = Bun.spawn(
    [QC, "prune", "--apply", entry.path],
    { stdout: "pipe", stderr: "pipe" },
  );

  const stderr = await new Response(proc.stderr).text();
  const exitCode = await proc.exited;

  if (exitCode !== 0) {
    return jsonResponse(
      { error: stderr.trim() || `exit code ${exitCode}` },
      500,
    );
  }

  // File was moved to .pruned/ — remove from index
  fileIndex.delete(name);

  // Also remove from analysis cache
  if (analysisCache) {
    analysisCache = (analysisCache as { filename: string }[]).filter(
      (r) => r.filename !== name,
    );
  }

  return jsonResponse({ status: "pruned", name });
}

async function handleSplice(name: string, req: Request): Promise<Response> {
  const entry = fileIndex.get(name);
  if (!entry) {
    return jsonResponse({ error: "File not found" }, 404);
  }

  // Parse optional params from body
  let minGap: number | undefined;
  let minDuration: number | undefined;
  let replaceOriginal = false;
  try {
    const body = await req.json();
    minGap = body.minGap;
    minDuration = body.minDuration;
    replaceOriginal = body.replaceOriginal ?? false;
  } catch {
    // no body is fine, use defaults
  }

  const args = [QC, "splice", entry.path];
  if (minGap !== undefined) args.push("--min-gap", String(minGap));
  if (minDuration !== undefined) args.push("--min-duration", String(minDuration));
  if (replaceOriginal) args.push("--replace-original");

  const proc = Bun.spawn(args, { stdout: "pipe", stderr: "pipe" });

  const [stdout, stderr] = await Promise.all([
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
  ]);
  const exitCode = await proc.exited;

  if (exitCode !== 0) {
    return jsonResponse(
      { error: stderr.trim() || `exit code ${exitCode}` },
      500,
    );
  }

  // Parse SPLICE lines from stdout to find created segments
  const segments: string[] = [];
  for (const line of stdout.split("\n")) {
    const match = line.match(/SPLICE\s+\S+\s+->\s+(\S+)/);
    if (match) segments.push(match[1]!);
  }

  // If original was replaced, remove from index
  if (replaceOriginal) {
    fileIndex.delete(name);
    if (analysisCache) {
      analysisCache = (analysisCache as { filename: string }[]).filter(
        (r) => r.filename !== name,
      );
    }
  }

  // Discover new segment files (segments are created next to the source file)
  const sourceRelDir = dirname(name);
  const newFiles: FileEntry[] = [];
  for (const segName of segments) {
    const segPath = join(dirname(entry.path), segName);
    const segRelName = sourceRelDir === "." ? segName : join(sourceRelDir, segName);
    try {
      const segStat = await stat(segPath);
      const metadata = await parseEgorecMetadata(segPath);
      let conversionStatus: ConversionStatus = "idle";
      if (metadata.rgbCodec === 2) conversionStatus = "streamable";

      const segEntry: FileEntry = {
        name: segRelName,
        path: segPath,
        sizeBytes: segStat.size,
        metadata,
        conversionStatus,
      };
      fileIndex.set(segRelName, segEntry);
      newFiles.push(segEntry);
    } catch {
      // segment file might not exist if splice skipped
    }
  }

  return jsonResponse({
    status: "spliced",
    name,
    segments,
    newFiles: newFiles.map((f) => {
      const dir = dirname(f.name);
      return {
      name: f.name,
      dataset: dir === "." ? null : dir,
      sessionName: f.metadata.sessionName,
      rgbCodec: f.metadata.rgbCodec,
      colorWidth: f.metadata.colorWidth,
      colorHeight: f.metadata.colorHeight,
      fps: f.metadata.fps,
      totalFrames: f.metadata.totalFrames,
      durationS: f.metadata.durationS,
      sizeBytes: f.sizeBytes,
      conversionStatus: f.conversionStatus,
      hasImu: f.metadata.hasImu,
    }}),
    originalRemoved: replaceOriginal,
  });
}

async function handleRestore(name: string): Promise<Response> {
  // name may include subdirectory (e.g., "dataset1/recording.egorec")
  const relDir = dirname(name);
  const fileName = basename(name);
  const sourceDir = relDir === "." ? RECORDINGS_DIR : join(RECORDINGS_DIR, relDir);

  const proc = Bun.spawn(
    [QC, "restore", sourceDir, fileName],
    { stdout: "pipe", stderr: "pipe" },
  );

  const stderr = await new Response(proc.stderr).text();
  const exitCode = await proc.exited;

  if (exitCode !== 0) {
    return jsonResponse(
      { error: stderr.trim() || `exit code ${exitCode}` },
      500,
    );
  }

  // Add restored file back to index
  const restoredPath = join(sourceDir, fileName);
  try {
    const fileStat = await stat(restoredPath);
    const metadata = await parseEgorecMetadata(restoredPath);
    let conversionStatus: ConversionStatus = "idle";
    if (metadata.rgbCodec === 2) conversionStatus = "streamable";

    fileIndex.set(name, {
      name,
      path: restoredPath,
      sizeBytes: fileStat.size,
      metadata,
      conversionStatus,
    });

    return jsonResponse({
      status: "restored",
      file: {
        name,
        dataset: relDir === "." ? null : relDir,
        sessionName: metadata.sessionName,
        rgbCodec: metadata.rgbCodec,
        colorWidth: metadata.colorWidth,
        colorHeight: metadata.colorHeight,
        fps: metadata.fps,
        totalFrames: metadata.totalFrames,
        durationS: metadata.durationS,
        sizeBytes: fileStat.size,
        conversionStatus,
        hasImu: metadata.hasImu,
      },
    });
  } catch (err) {
    return jsonResponse(
      { error: err instanceof Error ? err.message : String(err) },
      500,
    );
  }
}

async function handleListPruned(): Promise<Response> {
  const pruned: string[] = [];

  async function scanPruned(dirPath: string, prefix: string) {
    // Check for .pruned/ inside this directory
    const prunedDir = join(dirPath, ".pruned");
    try {
      const entries = await readdir(prunedDir, { withFileTypes: true });
      for (const e of entries) {
        if (e.isFile() && e.name.endsWith(".egorec")) {
          pruned.push(prefix ? `${prefix}/${e.name}` : e.name);
        }
      }
    } catch {}

    // Recurse into non-hidden subdirectories
    try {
      const entries = await readdir(dirPath, { withFileTypes: true });
      for (const e of entries) {
        if (e.isDirectory() && !e.name.startsWith(".")) {
          await scanPruned(
            join(dirPath, e.name),
            prefix ? `${prefix}/${e.name}` : e.name,
          );
        }
      }
    } catch {}
  }

  await scanPruned(RECORDINGS_DIR, "");
  pruned.sort();
  return jsonResponse({ files: pruned });
}

// ── Server ──────────────────────────────────────────────────────────────────

await discoverFiles();

const server = serve({
  hostname: "0.0.0.0",
  port: 4200,

  routes: {
    "/api/files": {
      GET: () => handleGetFiles(),
    },

    "/api/files/:name/convert": {
      POST: (req) => handleConvert(decodeURIComponent(req.params.name)),
    },

    "/api/files/:name/status": {
      GET: (req) => handleGetStatus(decodeURIComponent(req.params.name)),
    },

    "/api/files/:name/prune": {
      POST: (req) => handlePrune(decodeURIComponent(req.params.name)),
    },

    "/api/files/:name/splice": {
      POST: (req) => handleSplice(decodeURIComponent(req.params.name), req),
    },

    "/api/files/:name/restore": {
      POST: (req) => handleRestore(decodeURIComponent(req.params.name)),
    },

    "/api/files/:name": {
      GET: (req) => handleGetFile(decodeURIComponent(req.params.name)),
    },

    "/api/analyze": {
      POST: () => handleAnalyze(),
      GET: () => handleGetAnalysis(),
    },

    "/api/pruned": {
      GET: () => handleListPruned(),
    },

    "/stream/:name": {
      GET: (req) => handleStreamProxy(decodeURIComponent(req.params.name), req),
    },

    "/video/:name": {
      GET: (req) => handleVideoProxy(decodeURIComponent(req.params.name), "color"),
    },

    "/video/:name/depth": {
      GET: (req) => handleVideoProxy(decodeURIComponent(req.params.name), "depth"),
    },

    // Catch-all for SPA
    "/*": index,
  },

  development: process.env.NODE_ENV !== "production" && {
    hmr: true,
    console: true,
  },
});

console.log(`Egorec Viewer running at ${server.url}`);
console.log(`Recordings dir: ${RECORDINGS_DIR}`);
console.log(`QC: ${QC}`);
console.log(`Files: ${fileIndex.size}`);
