/**
 * TypeScript port of python/egorec_header.py
 *
 * Parses .egorec v2 binary headers (368 bytes) and footers (36 bytes)
 * using only Node/Bun Buffer APIs. Matches the format defined in
 * ego-recorder/rust/egorec/src/format.rs.
 *
 * All multi-byte values are little-endian.
 */

import type { EgorecMetadata } from "./types";

// Magic bytes: ASCII "EGOREC" + version 2.0
const FILE_MAGIC = Buffer.from([0x45, 0x47, 0x4f, 0x52, 0x45, 0x43, 0x02, 0x00]);

// Footer magic: 'DONE' (0x454E4F44)
const FOOTER_MAGIC = 0x454e4f44;

// Sizes
const HEADER_SIZE = 368;
const FOOTER_SIZE = 36;

/** Read a null-terminated C string from a buffer slice. */
function readCString(buf: Buffer, offset: number, maxLen: number): string {
  const slice = buf.subarray(offset, offset + maxLen);
  const end = slice.indexOf(0);
  const len = end === -1 ? maxLen : end;
  return slice.subarray(0, len).toString("utf-8");
}

/** Read an array of N float32 values at offset. */
function readFloatArray(buf: Buffer, offset: number, count: number): number[] {
  const result: number[] = [];
  for (let i = 0; i < count; i++) {
    result.push(buf.readFloatLE(offset + i * 4));
  }
  return result;
}

export interface ParsedHeader {
  sessionName: string;
  serialNumber: string;
  usbType: string;
  flags: number;
  depthScale: number;
  depthWidth: number;
  depthHeight: number;
  depthFx: number;
  depthFy: number;
  depthPpx: number;
  depthPpy: number;
  depthDistortionModel: number;
  depthDistortionCoeffs: number[];
  colorWidth: number;
  colorHeight: number;
  colorFx: number;
  colorFy: number;
  colorPpx: number;
  colorPpy: number;
  colorDistortionModel: number;
  colorDistortionCoeffs: number[];
  extrinsicRotation: number[];
  extrinsicTranslation: number[];
  startTimestampUs: number;
  rgbCodec: number;
  depthCodec: number;
  rgbQuality: number;
  zstdLevel: number;
}

export interface ParsedFooter {
  indexMagic: number;
  indexOffset: bigint;
  indexEntryCount: number;
  totalFrames: bigint;
  totalDurationUs: bigint;
  footerMagic: number;
}

export function parseHeader(buf: Buffer): ParsedHeader {
  if (buf.length < HEADER_SIZE) {
    throw new Error(`Need at least ${HEADER_SIZE} bytes, got ${buf.length}`);
  }

  // Validate magic
  const magic = buf.subarray(0, 8);
  if (!magic.equals(FILE_MAGIC)) {
    throw new Error(`Invalid .egorec magic: ${magic.toString("hex")}`);
  }

  // header_size(u32) + flags(u32) at offset 8
  const flags = buf.readUInt32LE(12);

  // serial_number: 32 bytes at offset 16
  const serialNumber = readCString(buf, 16, 32);

  // Depth intrinsics starting at offset 48
  let off = 48;
  const depthScale = buf.readFloatLE(off); off += 4;
  const depthWidth = buf.readUInt32LE(off); off += 4;
  const depthHeight = buf.readUInt32LE(off); off += 4;
  const depthFx = buf.readFloatLE(off); off += 4;
  const depthFy = buf.readFloatLE(off); off += 4;
  const depthPpx = buf.readFloatLE(off); off += 4;
  const depthPpy = buf.readFloatLE(off); off += 4;
  const depthDistortionModel = buf.readUInt32LE(off); off += 4;
  const depthDistortionCoeffs = readFloatArray(buf, off, 5); off += 20;

  // Color intrinsics
  const colorWidth = buf.readUInt32LE(off); off += 4;
  const colorHeight = buf.readUInt32LE(off); off += 4;
  const colorFx = buf.readFloatLE(off); off += 4;
  const colorFy = buf.readFloatLE(off); off += 4;
  const colorPpx = buf.readFloatLE(off); off += 4;
  const colorPpy = buf.readFloatLE(off); off += 4;
  const colorDistortionModel = buf.readUInt32LE(off); off += 4;
  const colorDistortionCoeffs = readFloatArray(buf, off, 5); off += 20;

  // Extrinsics
  const extrinsicRotation = readFloatArray(buf, off, 9); off += 36;
  const extrinsicTranslation = readFloatArray(buf, off, 3); off += 12;

  // Session name: 128 bytes
  const sessionName = readCString(buf, off, 128); off += 128;

  // Timestamp, USB type, codec settings
  // startTimestampUs is u64 - use Number for reasonable values
  const startTimestampUs = Number(buf.readBigUInt64LE(off)); off += 8;
  const usbType = readCString(buf, off, 8); off += 8;
  const rgbCodec = buf.readUInt8(off); off += 1;
  const depthCodec = buf.readUInt8(off); off += 1;
  const rgbQuality = buf.readUInt8(off); off += 1;
  const zstdLevel = buf.readUInt8(off);

  return {
    sessionName,
    serialNumber,
    usbType,
    flags,
    depthScale,
    depthWidth,
    depthHeight,
    depthFx,
    depthFy,
    depthPpx,
    depthPpy,
    depthDistortionModel,
    depthDistortionCoeffs,
    colorWidth,
    colorHeight,
    colorFx,
    colorFy,
    colorPpx,
    colorPpy,
    colorDistortionModel,
    colorDistortionCoeffs,
    extrinsicRotation,
    extrinsicTranslation,
    startTimestampUs,
    rgbCodec,
    depthCodec,
    rgbQuality,
    zstdLevel,
  };
}

export function parseFooter(buf: Buffer): ParsedFooter {
  if (buf.length < FOOTER_SIZE) {
    throw new Error(`Need at least ${FOOTER_SIZE} bytes for footer, got ${buf.length}`);
  }

  // Layout: index_magic(u32) index_offset(u64) index_entry_count(u32) total_frames(u64) total_duration_us(u64) footer_magic(u32)
  let off = 0;
  const indexMagic = buf.readUInt32LE(off); off += 4;
  const indexOffset = buf.readBigUInt64LE(off); off += 8;
  const indexEntryCount = buf.readUInt32LE(off); off += 4;
  const totalFrames = buf.readBigUInt64LE(off); off += 8;
  const totalDurationUs = buf.readBigUInt64LE(off); off += 8;
  const footerMagic = buf.readUInt32LE(off);

  if (footerMagic !== FOOTER_MAGIC) {
    throw new Error(`Invalid footer magic: 0x${footerMagic.toString(16).padStart(8, "0")}`);
  }

  return {
    indexMagic,
    indexOffset,
    indexEntryCount,
    totalFrames,
    totalDurationUs,
    footerMagic,
  };
}

/**
 * Parse .egorec metadata from a file path.
 * Only reads first HEADER_SIZE bytes + last FOOTER_SIZE bytes, so it's fast
 * even for multi-GB files.
 */
export async function parseEgorecMetadata(filePath: string): Promise<EgorecMetadata> {
  const file = Bun.file(filePath);
  const fileSize = file.size;

  // Read header (first 368 bytes)
  const headerSlice = file.slice(0, HEADER_SIZE);
  const headerBuf = Buffer.from(await headerSlice.arrayBuffer());
  const header = parseHeader(headerBuf);

  // Read footer (last 36 bytes)
  let totalFrames = 0;
  let totalDurationUs = 0;

  if (fileSize >= HEADER_SIZE + FOOTER_SIZE) {
    try {
      const footerSlice = file.slice(fileSize - FOOTER_SIZE, fileSize);
      const footerBuf = Buffer.from(await footerSlice.arrayBuffer());
      const footer = parseFooter(footerBuf);
      totalFrames = Number(footer.totalFrames);
      totalDurationUs = Number(footer.totalDurationUs);
    } catch {
      // Incomplete file, no valid footer
    }
  }

  const durationS = totalDurationUs / 1_000_000;
  const fps = durationS > 0 && totalFrames > 0
    ? totalFrames / durationS
    : 0;

  return {
    sessionName: header.sessionName,
    serialNumber: header.serialNumber,
    usbType: header.usbType,
    colorWidth: header.colorWidth,
    colorHeight: header.colorHeight,
    depthWidth: header.depthWidth,
    depthHeight: header.depthHeight,
    depthScale: header.depthScale,
    fps: Math.round(fps * 100) / 100,
    totalFrames,
    durationS,
    startTimestampUs: header.startTimestampUs,
    hasImu: (header.flags & 0x01) !== 0,
    rgbCodec: header.rgbCodec,
    depthCodec: header.depthCodec,
    rgbQuality: header.rgbQuality,
    zstdLevel: header.zstdLevel,
    intrinsics: {
      color: {
        width: header.colorWidth,
        height: header.colorHeight,
        fx: header.colorFx,
        fy: header.colorFy,
        ppx: header.colorPpx,
        ppy: header.colorPpy,
        distortionModel: header.colorDistortionModel,
        distortionCoeffs: header.colorDistortionCoeffs,
      },
      depth: {
        width: header.depthWidth,
        height: header.depthHeight,
        fx: header.depthFx,
        fy: header.depthFy,
        ppx: header.depthPpx,
        ppy: header.depthPpy,
        distortionModel: header.depthDistortionModel,
        distortionCoeffs: header.depthDistortionCoeffs,
        scale: header.depthScale,
      },
    },
    extrinsics: {
      rotation: header.extrinsicRotation,
      translation: header.extrinsicTranslation,
    },
  };
}

export { HEADER_SIZE, FOOTER_SIZE };
