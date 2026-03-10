/**
 * Streaming remux for .egorec H.264 recordings.
 *
 * Pipes raw H.264 NAL data from .egorec frame blocks into ffmpeg -c copy,
 * producing fragmented MP4 output. Zero decode/encode — just container repackaging.
 *
 * Reusable library: import from any Bun server.
 */

import { spawn } from "bun";
import { parseEgorecMetadata } from "./egorec-parser";
import { iterateRgbChunks } from "./egorec-reader";
import type { EgorecMetadata } from "./types";

export interface RemuxOptions {
  fps?: number;
  signal?: AbortSignal;
  onProgress?: (frame: number, total: number) => void;
}

/**
 * Returns true if the .egorec file can be streamed without full conversion.
 * rgbCodec === 2 means H.264 Annex B, which is browser-playable after remux.
 */
export function isStreamable(metadata: EgorecMetadata): boolean {
  return metadata.rgbCodec === 2;
}

/**
 * Create a ReadableStream of fMP4 data by remuxing H.264 NALs from an .egorec file.
 *
 * Spawns ffmpeg with -c copy (no transcode). The returned stream is ffmpeg's stdout,
 * which provides automatic backpressure.
 */
export async function createEgorecVideoStream(
  filePath: string,
  options?: RemuxOptions,
): Promise<ReadableStream<Uint8Array>> {
  const metadata = await parseEgorecMetadata(filePath);

  if (metadata.rgbCodec !== 2) {
    throw new Error(
      `Cannot stream remux: rgbCodec=${metadata.rgbCodec} (need 2 for H.264). Use full conversion instead.`,
    );
  }

  const fps = options?.fps || metadata.fps || 30;

  const proc = spawn({
    cmd: [
      "ffmpeg",
      "-f", "h264",
      "-r", String(fps),
      "-i", "pipe:0",
      "-c:v", "copy",
      "-movflags", "frag_keyframe+empty_moov+default_base_moof",
      "-f", "mp4",
      "pipe:1",
    ],
    stdin: "pipe",
    stdout: "pipe",
    stderr: "pipe",
  });

  // Background task: feed H.264 data to ffmpeg stdin
  const feedTask = (async () => {
    const writer = proc.stdin;
    try {
      for await (const chunk of iterateRgbChunks(filePath)) {
        if (options?.signal?.aborted) break;
        writer.write(chunk.data);
        options?.onProgress?.(chunk.frameNumber, chunk.totalFrames);
      }
    } catch (err) {
      // If signal was aborted, this is expected
      if (!options?.signal?.aborted) {
        console.error("Error feeding ffmpeg stdin:", err);
      }
    } finally {
      writer.flush();
      writer.end();
    }
  })();

  // Handle abort signal
  if (options?.signal) {
    const onAbort = () => {
      proc.kill();
    };
    options.signal.addEventListener("abort", onAbort, { once: true });

    // Clean up listener when process exits
    proc.exited.then(() => {
      options.signal!.removeEventListener("abort", onAbort);
    });
  }

  // Monitor for ffmpeg errors in background
  proc.exited.then(async (exitCode) => {
    if (exitCode !== 0 && !options?.signal?.aborted) {
      const stderr = await new Response(proc.stderr).text();
      console.error(`ffmpeg remux exited with code ${exitCode}: ${stderr}`);
    }
  });

  return proc.stdout as ReadableStream<Uint8Array>;
}
