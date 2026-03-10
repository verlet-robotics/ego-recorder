import { useEffect, useState } from "react";
import { Separator } from "@/components/ui/separator";
import { ScrollArea } from "@/components/ui/scroll-area";
import type { EgorecMetadata } from "@/lib/types";

interface MetadataPanelProps {
  fileName: string;
}

function formatTimestamp(us: number): string {
  if (us <= 0) return "N/A";
  return new Date(us / 1000).toLocaleString();
}

function formatDuration(seconds: number): string {
  if (!isFinite(seconds) || seconds <= 0) return "0:00";
  const m = Math.floor(seconds / 60);
  const s = (seconds % 60).toFixed(1);
  return `${m}:${s.padStart(4, "0")}`;
}

function codecName(codec: number): string {
  switch (codec) {
    case 0: return "Raw";
    case 1: return "MJPEG";
    case 2: return "H.264";
    default: return `Unknown (${codec})`;
  }
}

function distortionModelName(model: number): string {
  switch (model) {
    case 0: return "None";
    case 1: return "Brown-Conrady";
    case 2: return "Inverse Brown-Conrady";
    case 3: return "Theta";
    case 4: return "KB4";
    default: return `Model ${model}`;
  }
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="space-y-2">
      <h3 className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
        {title}
      </h3>
      <div className="space-y-1">{children}</div>
    </div>
  );
}

function Field({ label, value }: { label: string; value: string | number }) {
  return (
    <div className="flex items-baseline justify-between gap-2">
      <span className="text-[11px] text-muted-foreground shrink-0">{label}</span>
      <span className="text-[11px] font-mono text-foreground text-right truncate">
        {value}
      </span>
    </div>
  );
}

function IntrinsicsBlock({ title, data }: {
  title: string;
  data: { width: number; height: number; fx: number; fy: number; ppx: number; ppy: number; distortionModel: number; distortionCoeffs: number[] };
}) {
  return (
    <Section title={title}>
      <Field label="Resolution" value={`${data.width} x ${data.height}`} />
      <Field label="fx" value={data.fx.toFixed(3)} />
      <Field label="fy" value={data.fy.toFixed(3)} />
      <Field label="ppx" value={data.ppx.toFixed(3)} />
      <Field label="ppy" value={data.ppy.toFixed(3)} />
      <Field label="Distortion" value={distortionModelName(data.distortionModel)} />
      {data.distortionCoeffs.some((c) => c !== 0) && (
        <div className="text-[10px] font-mono text-muted-foreground break-all">
          [{data.distortionCoeffs.map((c) => c.toFixed(4)).join(", ")}]
        </div>
      )}
    </Section>
  );
}

export function MetadataPanel({ fileName }: MetadataPanelProps) {
  const [metadata, setMetadata] = useState<EgorecMetadata | null>(null);
  const [sizeBytes, setSizeBytes] = useState(0);

  useEffect(() => {
    fetch(`/api/files/${encodeURIComponent(fileName)}`)
      .then((res) => res.json())
      .then((data) => {
        setMetadata(data.metadata);
        setSizeBytes(data.sizeBytes);
      })
      .catch(() => setMetadata(null));
  }, [fileName]);

  if (!metadata) {
    return (
      <div className="flex items-center justify-center h-full text-muted-foreground text-[12px]">
        Loading...
      </div>
    );
  }

  const fileSizeMB = (sizeBytes / (1024 * 1024)).toFixed(1);

  return (
    <ScrollArea className="h-full">
      <div className="space-y-4 p-3">
        <Section title="Recording">
          <Field label="Session" value={metadata.sessionName || "N/A"} />
          <Field label="Serial" value={metadata.serialNumber || "N/A"} />
          <Field label="USB" value={metadata.usbType || "N/A"} />
          <Field label="Recorded" value={formatTimestamp(metadata.startTimestampUs)} />
          <Field label="Duration" value={formatDuration(metadata.durationS)} />
          <Field label="Frames" value={metadata.totalFrames.toLocaleString()} />
          <Field label="FPS" value={metadata.fps.toFixed(1)} />
          <Field label="File Size" value={`${fileSizeMB} MB`} />
          <Field label="IMU" value={metadata.hasImu ? "Yes" : "No"} />
        </Section>

        <Separator />

        <Section title="Codec">
          <Field label="RGB Codec" value={codecName(metadata.rgbCodec)} />
          <Field label="Depth Codec" value={codecName(metadata.depthCodec)} />
          <Field label="RGB Quality" value={metadata.rgbQuality} />
          <Field label="Zstd Level" value={metadata.zstdLevel} />
        </Section>

        <Separator />

        <IntrinsicsBlock title="Color Camera" data={metadata.intrinsics.color} />

        <Separator />

        <IntrinsicsBlock title="Depth Camera" data={metadata.intrinsics.depth} />

        <Separator />

        <Section title="Extrinsics">
          <div className="space-y-1">
            <span className="text-[10px] text-muted-foreground">Rotation (3x3)</span>
            <div className="font-mono text-[10px] text-foreground leading-relaxed">
              {[0, 1, 2].map((row) => (
                <div key={row}>
                  [{metadata.extrinsics.rotation.slice(row * 3, row * 3 + 3).map((v) => v.toFixed(4)).join(", ")}]
                </div>
              ))}
            </div>
          </div>
          <div className="space-y-1">
            <span className="text-[10px] text-muted-foreground">Translation</span>
            <div className="font-mono text-[10px] text-foreground">
              [{metadata.extrinsics.translation.map((v) => v.toFixed(4)).join(", ")}]
            </div>
          </div>
        </Section>
      </div>
    </ScrollArea>
  );
}
