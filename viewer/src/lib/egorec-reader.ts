/**
 * Binary frame reader for .egorec files.
 *
 * Reads the index table to locate frame offsets, provides both random-access
 * indexing and a sequential frame data iterator. Reuses parsers from egorec-parser.ts.
 *
 * Uses node:fs/promises FileHandle for persistent fd with positioned reads.
 */

import { open } from "node:fs/promises";
import { parseHeader, parseFooter, HEADER_SIZE, FOOTER_SIZE } from "./egorec-parser";
import type { ParsedHeader, ParsedFooter } from "./egorec-parser";

export interface FrameLocation {
  fileOffset: number;
  blockSize: number;
  rgbOffset: number;
  rgbSize: number;
  timestampUs: number;
  frameNumber: number;
}

export interface EgorecIndex {
  filePath: string;
  header: ParsedHeader;
  footer: ParsedFooter;
  frames: FrameLocation[];
  trailingDataOffset: number;
  trailingDataSize: number;
}

/**
 * Read header + footer from an .egorec file using a single file handle.
 */
async function readHeaderAndFooter(filePath: string) {
  const fh = await open(filePath, "r");
  try {
    const fileStat = await fh.stat();
    const fileSize = fileStat.size;

    // Read header
    const headerBuf = Buffer.alloc(HEADER_SIZE);
    await fh.read(headerBuf, 0, HEADER_SIZE, 0);
    const header = parseHeader(headerBuf);

    // Read footer (last 36 bytes)
    const footerBuf = Buffer.alloc(FOOTER_SIZE);
    await fh.read(footerBuf, 0, FOOTER_SIZE, fileSize - FOOTER_SIZE);
    const footer = parseFooter(footerBuf);

    return { header, footer, fileSize, fh };
  } catch (err) {
    await fh.close();
    throw err;
  }
}

/**
 * Build a full random-access index of all frames.
 *
 * 1. Reads header + footer
 * 2. Bulk-reads the index table to get the first frame offset
 * 3. Sequential pass through frame block headers to collect per-frame info
 */
export async function readEgorecIndex(filePath: string): Promise<EgorecIndex> {
  const { header, footer, fh } = await readHeaderAndFooter(filePath);

  try {
    const totalFrames = Number(footer.totalFrames);
    const indexOffset = Number(footer.indexOffset);

    // Read first index entry to get first frame offset
    const indexEntryBuf = Buffer.alloc(24);
    await fh.read(indexEntryBuf, 0, 24, indexOffset);
    const firstFrameOffset = Number(indexEntryBuf.readBigUInt64LE(8)); // file_offset field

    // Sequential pass through frame block headers
    const frames: FrameLocation[] = [];
    const blockHeaderBuf = Buffer.alloc(36);
    let offset = firstFrameOffset;

    for (let i = 0; i < totalFrames; i++) {
      await fh.read(blockHeaderBuf, 0, 36, offset);

      const magic = blockHeaderBuf.readUInt32LE(0);
      if (magic !== 0x46524d45) {
        throw new Error(
          `Invalid frame magic at offset ${offset}: 0x${magic.toString(16).padStart(8, "0")}`,
        );
      }

      const blockSize = blockHeaderBuf.readUInt32LE(4);
      const timestampUs = Number(blockHeaderBuf.readBigUInt64LE(8));
      const frameNumber = Number(blockHeaderBuf.readBigUInt64LE(16));
      const rgbSize = blockHeaderBuf.readUInt32LE(24);

      frames.push({
        fileOffset: offset,
        blockSize,
        rgbOffset: offset + 36,
        rgbSize,
        timestampUs,
        frameNumber,
      });

      offset += blockSize;
    }

    const trailingDataOffset = offset;
    const trailingDataSize = indexOffset - offset;

    return {
      filePath,
      header,
      footer,
      frames,
      trailingDataOffset,
      trailingDataSize,
    };
  } finally {
    await fh.close();
  }
}

/**
 * Async generator that yields RGB byte chunks sequentially using a single fd.
 * Preferred for streaming remux — no random seeks, minimal memory.
 */
export async function* iterateRgbChunks(
  filePath: string,
): AsyncGenerator<{ data: Uint8Array; frameNumber: number; totalFrames: number }> {
  const { header, footer, fh } = await readHeaderAndFooter(filePath);

  try {
    const totalFrames = Number(footer.totalFrames);
    const indexOffset = Number(footer.indexOffset);

    if (totalFrames === 0) return;

    // Read first index entry to get starting offset
    const indexEntryBuf = Buffer.alloc(24);
    await fh.read(indexEntryBuf, 0, 24, indexOffset);
    const firstFrameOffset = Number(indexEntryBuf.readBigUInt64LE(8));

    const blockHeaderBuf = Buffer.alloc(36);
    let offset = firstFrameOffset;

    for (let i = 0; i < totalFrames; i++) {
      await fh.read(blockHeaderBuf, 0, 36, offset);

      const blockSize = blockHeaderBuf.readUInt32LE(4);
      const rgbSize = blockHeaderBuf.readUInt32LE(24);

      if (rgbSize > 0) {
        const rgbBuf = Buffer.alloc(rgbSize);
        await fh.read(rgbBuf, 0, rgbSize, offset + 36);
        yield { data: rgbBuf, frameNumber: i, totalFrames };
      }

      offset += blockSize;
    }

    // Trailing flush NALs (between last frame block and index table)
    const trailingSize = indexOffset - offset;
    if (trailingSize > 0) {
      const trailing = Buffer.alloc(trailingSize);
      await fh.read(trailing, 0, trailingSize, offset);
      yield { data: trailing, frameNumber: totalFrames, totalFrames };
    }
  } finally {
    await fh.close();
  }
}
