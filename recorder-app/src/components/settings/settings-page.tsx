import { useState, useEffect, useCallback } from "react";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useAppStore } from "@/stores/app-store";
import { commands } from "@/lib/tauri";
import type { AppConfig } from "@/lib/types";
import {
  FolderOpen,
  Check,
  Loader2,
  ChevronRight,
  Camera,
  HardDrive,
  Cloud,
  Terminal,
  ClipboardPaste,
  CircleCheck,
  CircleX,
  AlertTriangle,
} from "lucide-react";

export function SettingsPage() {
  const appConfig = useAppStore((s) => s.config);
  const setAppConfig = useAppStore((s) => s.setConfig);
  const firstRun = useAppStore((s) => s.firstRun);
  const setFirstRun = useAppStore((s) => s.setFirstRun);
  const setPage = useAppStore((s) => s.setPage);

  const [config, setConfig] = useState<AppConfig | null>(appConfig);
  const [saving, setSaving] = useState(false);
  const [saveStatus, setSaveStatus] = useState<string | null>(null);
  const [wizardStep, setWizardStep] = useState(0);
  const [binaryTestResult, setBinaryTestResult] = useState<string | null>(null);
  const [binaryTestPassed, setBinaryTestPassed] = useState(false);
  const [binaryTesting, setBinaryTesting] = useState(false);
  const [showEnvPaste, setShowEnvPaste] = useState(false);
  const [envPasteText, setEnvPasteText] = useState("");
  const [connectionTestResult, setConnectionTestResult] = useState<string | null>(null);
  const [connectionTestOk, setConnectionTestOk] = useState(false);
  const [connectionTesting, setConnectionTesting] = useState(false);

  useEffect(() => {
    if (!config) {
      commands.getConfig().then(setConfig);
    }
  }, [config]);

  // Auto-detect binary on first run
  useEffect(() => {
    if (firstRun && config && !config.recorder.binary_path) {
      commands.locateBinary().then((path) => {
        if (path) {
          setConfig((prev) => prev ? { ...prev, recorder: { ...prev.recorder, binary_path: path } } : prev);
        }
      });
    }
  }, [firstRun, config]);

  const handleSave = useCallback(async () => {
    if (!config) return;
    setSaving(true);
    setSaveStatus(null);
    try {
      await commands.saveConfig(config);
      setAppConfig(config);
      setSaveStatus("Saved");
      if (firstRun) {
        await commands.completeFirstRun();
        setFirstRun(false);
        setPage("record");
      }
    } catch (err) {
      setSaveStatus(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  }, [config, setAppConfig, firstRun, setFirstRun, setPage]);

  const handleTestBinary = useCallback(async (pathOverride?: string) => {
    const path = pathOverride ?? config?.recorder.binary_path;
    if (!path) return;
    setBinaryTesting(true);
    setBinaryTestResult(null);
    setBinaryTestPassed(false);
    try {
      const result = await commands.testCamera(path);
      setBinaryTestResult(result);
      setBinaryTestPassed(true);
    } catch (err) {
      setBinaryTestResult(err instanceof Error ? err.message : String(err));
      setBinaryTestPassed(false);
    } finally {
      setBinaryTesting(false);
    }
  }, [config]);

  // Auto-test binary when path is set (including auto-detect)
  useEffect(() => {
    if (config?.recorder.binary_path && !binaryTestPassed && !binaryTesting) {
      handleTestBinary(config.recorder.binary_path);
    }
  }, [config?.recorder.binary_path]); // eslint-disable-line react-hooks/exhaustive-deps

  const handlePickBinary = useCallback(async () => {
    const file = await commands.selectFile("Select ego-recorder binary");
    if (file) {
      setConfig((prev) => prev ? { ...prev, recorder: { ...prev.recorder, binary_path: file } } : prev);
    }
  }, []);

  const handlePickOutputDir = useCallback(async () => {
    const dir = await commands.openDirectory();
    if (dir) {
      setConfig((prev) => prev ? { ...prev, storage: { ...prev.storage, output_dir: dir } } : prev);
    }
  }, []);

  const handleTestConnection = useCallback(async () => {
    setConnectionTesting(true);
    setConnectionTestResult(null);
    setConnectionTestOk(false);
    try {
      const msg = await commands.testUploadConnection();
      setConnectionTestResult(msg);
      setConnectionTestOk(true);
    } catch (err) {
      setConnectionTestResult(err instanceof Error ? err.message : String(err));
      setConnectionTestOk(false);
    } finally {
      setConnectionTesting(false);
    }
  }, []);

  const handleEnvPaste = useCallback((text: string) => {
    setEnvPasteText(text);
    const keyMap: Record<string, keyof AppConfig["upload"]> = {};
    for (const k of ["S3_ENDPOINT", "AWS_ENDPOINT_URL", "ENDPOINT_URL", "R2_ENDPOINT", "ENDPOINT"])
      keyMap[k] = "endpoint";
    for (const k of ["S3_BUCKET", "AWS_BUCKET", "BUCKET", "BUCKET_NAME", "R2_BUCKET", "TELEOP_R2_EGO_BUCKET_NAME"])
      keyMap[k] = "bucket";
    for (const k of ["S3_REGION", "AWS_REGION", "AWS_DEFAULT_REGION", "REGION", "R2_REGION"])
      keyMap[k] = "region";
    for (const k of ["AWS_ACCESS_KEY_ID", "S3_ACCESS_KEY", "ACCESS_KEY", "R2_ACCESS_KEY_ID", "ACCESS_KEY_ID"])
      keyMap[k] = "access_key";
    for (const k of ["AWS_SECRET_ACCESS_KEY", "S3_SECRET_KEY", "SECRET_KEY", "R2_SECRET_ACCESS_KEY", "SECRET_ACCESS_KEY"])
      keyMap[k] = "secret_key";

    const parsed: Partial<Record<keyof AppConfig["upload"], string>> = {};
    for (const line of text.split("\n")) {
      const trimmed = line.replace(/^export\s+/, "").trim();
      if (!trimmed || trimmed.startsWith("#")) continue;
      const eqIdx = trimmed.indexOf("=");
      if (eqIdx < 1) continue;
      const key = trimmed.slice(0, eqIdx).trim();
      const val = trimmed.slice(eqIdx + 1).trim().replace(/^["']|["']$/g, "");
      const field = keyMap[key];
      if (field && val) parsed[field] = val;
    }

    if (Object.keys(parsed).length > 0) {
      setConfig((prev) => prev ? {
        ...prev,
        upload: {
          ...prev.upload,
          ...Object.fromEntries(Object.entries(parsed).map(([k, v]) => [k, v ?? prev.upload[k as keyof AppConfig["upload"]]])),
        },
      } : prev);
      setShowEnvPaste(false);
      setEnvPasteText("");
    }
  }, []);

  if (!config) {
    return <div className="flex items-center justify-center h-full text-muted-foreground">Loading...</div>;
  }

  const isWizard = firstRun;
  const steps = ["Recorder", "Storage", "Upload", "Done"];

  return (
    <ScrollArea className="h-full">
      <div className="max-w-2xl mx-auto p-6 space-y-6">
        <div className="flex items-center justify-between">
          <h1 className="text-xl font-semibold">
            {isWizard ? "Setup Wizard" : "Settings"}
          </h1>
          {isWizard && (
            <div className="flex items-center gap-1 text-[11px] text-muted-foreground">
              {steps.map((step, i) => (
                <span key={step} className="flex items-center gap-1">
                  <span className={i <= wizardStep ? "text-highlight font-medium" : ""}>{step}</span>
                  {i < steps.length - 1 && <ChevronRight className="size-3" />}
                </span>
              ))}
            </div>
          )}
        </div>

        {/* Recorder section */}
        {(!isWizard || wizardStep === 0) && (
          <section className="space-y-4">
            <div className="flex items-center gap-2">
              <Terminal className="size-4 text-muted-foreground" />
              <h2 className="text-sm font-semibold">Recorder Binary</h2>
            </div>
            <div className="space-y-3">
              <div className="space-y-1.5">
                <label className="text-[11px] font-medium text-muted-foreground uppercase tracking-wider">
                  Path to ego-recorder
                </label>
                <div className="flex gap-2">
                  <input
                    type="text"
                    value={config.recorder.binary_path ?? ""}
                    onChange={(e) => {
                      setBinaryTestPassed(false);
                      setBinaryTestResult(null);
                      setConfig({ ...config, recorder: { ...config.recorder, binary_path: e.target.value || null } });
                    }}
                    placeholder="Auto-detected or browse..."
                    className="flex-1 h-8 px-3 rounded-md border border-input bg-background text-foreground placeholder:text-muted-foreground text-sm font-mono"
                  />
                  <Button variant="outline" size="sm" onClick={handlePickBinary}>Browse</Button>
                  <Button variant="outline" size="sm" onClick={() => handleTestBinary()} disabled={binaryTesting || !config.recorder.binary_path}>
                    {binaryTesting ? <Loader2 className="size-3 animate-spin" /> : "Test"}
                  </Button>
                </div>

                {/* Test result feedback */}
                {binaryTesting && (
                  <p className="text-[10px] text-muted-foreground flex items-center gap-1.5">
                    <Loader2 className="size-3 animate-spin" />
                    Testing binary...
                  </p>
                )}
                {!binaryTesting && binaryTestPassed && (
                  <p className="text-[10px] text-success flex items-center gap-1.5">
                    <CircleCheck className="size-3" />
                    {binaryTestResult}
                  </p>
                )}
                {!binaryTesting && binaryTestResult && !binaryTestPassed && (
                  <div className="rounded-lg bg-destructive-soft px-3 py-2.5 space-y-2">
                    <p className="text-[11px] text-destructive font-medium flex items-center gap-1.5">
                      <CircleX className="size-3.5 shrink-0" />
                      {binaryTestResult}
                    </p>
                    <div className="text-[10px] text-destructive/80 space-y-1.5 pl-5">
                      <p className="font-medium">How to fix:</p>
                      <ol className="list-decimal pl-3.5 space-y-1">
                        <li>Build the recorder from the <span className="font-mono">ego-recorder/</span> directory:
                          <code className="block mt-0.5 bg-black/20 rounded px-1.5 py-0.5 font-mono">mkdir -p build && cd build && cmake .. && make -j$(nproc)</code>
                        </li>
                        <li>Verify it runs: <code className="bg-black/20 rounded px-1.5 py-0.5 font-mono">./build/ego-recorder --help</code></li>
                        <li>If you see <span className="font-mono">librealsense</span> errors, install the RealSense SDK:
                          <code className="block mt-0.5 bg-black/20 rounded px-1.5 py-0.5 font-mono">sudo apt install librealsense2-dev</code>
                        </li>
                        <li>Use <strong>Browse</strong> to select the binary manually if auto-detect failed.</li>
                      </ol>
                    </div>
                  </div>
                )}
                {!binaryTesting && !binaryTestResult && !config.recorder.binary_path && (
                  <p className="text-[10px] text-warning flex items-center gap-1.5">
                    <AlertTriangle className="size-3" />
                    No binary path set. Use Browse or enter the path manually.
                  </p>
                )}
              </div>
              <div className="grid grid-cols-2 gap-4">
                <div className="space-y-1.5">
                  <label className="text-[11px] font-medium text-muted-foreground uppercase tracking-wider">Default CRF</label>
                  <input
                    type="number"
                    min={0}
                    max={51}
                    value={config.recorder.default_crf}
                    onChange={(e) => setConfig({ ...config, recorder: { ...config.recorder, default_crf: parseInt(e.target.value) || 23 } })}
                    className="w-full h-8 px-3 rounded-md border border-input bg-background text-foreground placeholder:text-muted-foreground text-sm"
                  />
                </div>
                <div className="space-y-1.5">
                  <label className="text-[11px] font-medium text-muted-foreground uppercase tracking-wider">Warmup Frames</label>
                  <input
                    type="number"
                    min={0}
                    max={120}
                    value={config.recorder.warmup_frames}
                    onChange={(e) => setConfig({ ...config, recorder: { ...config.recorder, warmup_frames: parseInt(e.target.value) || 30 } })}
                    className="w-full h-8 px-3 rounded-md border border-input bg-background text-foreground placeholder:text-muted-foreground text-sm"
                  />
                </div>
              </div>
            </div>
          </section>
        )}

        {(!isWizard || wizardStep >= 0) && <Separator />}

        {/* Storage section */}
        {(!isWizard || wizardStep === 1) && (
          <section className="space-y-4">
            <div className="flex items-center gap-2">
              <HardDrive className="size-4 text-muted-foreground" />
              <h2 className="text-sm font-semibold">Storage</h2>
            </div>
            <div className="space-y-3">
              <div className="space-y-1.5">
                <label className="text-[11px] font-medium text-muted-foreground uppercase tracking-wider">
                  Default Recording Directory
                </label>
                <div className="flex gap-2">
                  <input
                    type="text"
                    value={config.storage.output_dir ?? ""}
                    onChange={(e) => setConfig({ ...config, storage: { ...config.storage, output_dir: e.target.value || null } })}
                    placeholder="/path/to/recordings"
                    className="flex-1 h-8 px-3 rounded-md border border-input bg-background text-foreground placeholder:text-muted-foreground text-sm font-mono"
                  />
                  <Button variant="outline" size="sm" onClick={handlePickOutputDir}>
                    <FolderOpen className="size-3.5" />
                  </Button>
                </div>
              </div>
              <div className="space-y-1.5">
                <label className="text-[11px] font-medium text-muted-foreground uppercase tracking-wider">
                  Disk Space Threshold (MB)
                </label>
                <input
                  type="number"
                  min={100}
                  value={config.storage.disk_threshold_mb}
                  onChange={(e) => setConfig({ ...config, storage: { ...config.storage, disk_threshold_mb: parseInt(e.target.value) || 500 } })}
                  className="w-48 h-8 px-3 rounded-md border border-input bg-background text-foreground text-sm"
                />
              </div>
            </div>
          </section>
        )}

        {(!isWizard || wizardStep >= 1) && <Separator />}

        {/* Upload section */}
        {(!isWizard || wizardStep === 2) && (
          <section className="space-y-4">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <Cloud className="size-4 text-muted-foreground" />
                <h2 className="text-sm font-semibold">Upload (R2/S3)</h2>
              </div>
              <Button
                variant="ghost"
                size="sm"
                className="text-[11px] gap-1.5 text-muted-foreground"
                onClick={() => setShowEnvPaste(!showEnvPaste)}
              >
                <ClipboardPaste className="size-3" />
                Paste .env
              </Button>
            </div>
            {showEnvPaste && (
              <div className="space-y-2">
                <textarea
                  value={envPasteText}
                  onChange={(e) => handleEnvPaste(e.target.value)}
                  placeholder={"AWS_ENDPOINT_URL=https://...\nAWS_ACCESS_KEY_ID=...\nAWS_SECRET_ACCESS_KEY=...\nS3_BUCKET=my-bucket\nAWS_REGION=auto"}
                  rows={5}
                  className="w-full px-3 py-2 rounded-md border border-input bg-background text-foreground placeholder:text-muted-foreground text-xs font-mono resize-none"
                />
                <p className="text-[10px] text-muted-foreground">
                  Paste env-style credentials — fields will auto-populate.
                </p>
              </div>
            )}
            <div className="space-y-3">
              <div className="space-y-1.5">
                <label className="text-[11px] font-medium text-muted-foreground uppercase tracking-wider">Endpoint</label>
                <input
                  type="text"
                  value={config.upload.endpoint ?? ""}
                  onChange={(e) => setConfig({ ...config, upload: { ...config.upload, endpoint: e.target.value || null } })}
                  placeholder="https://....r2.cloudflarestorage.com"
                  className="w-full h-8 px-3 rounded-md border border-input bg-background text-foreground placeholder:text-muted-foreground text-sm font-mono"
                />
              </div>
              <div className="grid grid-cols-2 gap-4">
                <div className="space-y-1.5">
                  <label className="text-[11px] font-medium text-muted-foreground uppercase tracking-wider">Bucket</label>
                  <input
                    type="text"
                    value={config.upload.bucket ?? ""}
                    onChange={(e) => setConfig({ ...config, upload: { ...config.upload, bucket: e.target.value || null } })}
                    className="w-full h-8 px-3 rounded-md border border-input bg-background text-foreground placeholder:text-muted-foreground text-sm"
                  />
                </div>
                <div className="space-y-1.5">
                  <label className="text-[11px] font-medium text-muted-foreground uppercase tracking-wider">Region</label>
                  <input
                    type="text"
                    value={config.upload.region ?? ""}
                    onChange={(e) => setConfig({ ...config, upload: { ...config.upload, region: e.target.value || null } })}
                    placeholder="auto"
                    className="w-full h-8 px-3 rounded-md border border-input bg-background text-foreground placeholder:text-muted-foreground text-sm"
                  />
                </div>
              </div>
              <div className="grid grid-cols-2 gap-4">
                <div className="space-y-1.5">
                  <label className="text-[11px] font-medium text-muted-foreground uppercase tracking-wider">Access Key</label>
                  <input
                    type="text"
                    value={config.upload.access_key ?? ""}
                    onChange={(e) => setConfig({ ...config, upload: { ...config.upload, access_key: e.target.value || null } })}
                    className="w-full h-8 px-3 rounded-md border border-input bg-background text-foreground placeholder:text-muted-foreground text-sm font-mono"
                  />
                </div>
                <div className="space-y-1.5">
                  <label className="text-[11px] font-medium text-muted-foreground uppercase tracking-wider">Secret Key</label>
                  <input
                    type="password"
                    value={config.upload.secret_key ?? ""}
                    onChange={(e) => setConfig({ ...config, upload: { ...config.upload, secret_key: e.target.value || null } })}
                    className="w-full h-8 px-3 rounded-md border border-input bg-background text-foreground placeholder:text-muted-foreground text-sm font-mono"
                  />
                </div>
              </div>
              {/* Test connection */}
              <div className="flex items-center gap-2 pt-1">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={handleTestConnection}
                  disabled={connectionTesting || !config.upload.endpoint || !config.upload.bucket || !config.upload.access_key || !config.upload.secret_key}
                >
                  {connectionTesting ? <Loader2 className="size-3 animate-spin" /> : "Test Connection"}
                </Button>
                {connectionTesting && (
                  <span className="text-[10px] text-muted-foreground">Connecting...</span>
                )}
                {!connectionTesting && connectionTestOk && connectionTestResult && (
                  <span className="text-[10px] text-success flex items-center gap-1">
                    <CircleCheck className="size-3" />
                    {connectionTestResult}
                  </span>
                )}
                {!connectionTesting && !connectionTestOk && connectionTestResult && (
                  <span className="text-[10px] text-destructive flex items-center gap-1">
                    <CircleX className="size-3" />
                    {connectionTestResult}
                  </span>
                )}
              </div>
              {/* Advanced upload settings */}
              {!isWizard && (
                <div className="space-y-3 pt-2">
                  <p className="text-[10px] font-medium text-muted-foreground uppercase tracking-wider">Advanced</p>
                  <div className="space-y-1.5">
                    <label className="text-[11px] font-medium text-muted-foreground uppercase tracking-wider">Key Prefix</label>
                    <input
                      type="text"
                      value={config.upload.prefix ?? ""}
                      onChange={(e) => setConfig({ ...config, upload: { ...config.upload, prefix: e.target.value || null } })}
                      placeholder="device-01/"
                      className="w-full h-8 px-3 rounded-md border border-input bg-background text-foreground placeholder:text-muted-foreground text-sm font-mono"
                    />
                  </div>
                  <div className="grid grid-cols-3 gap-4">
                    <div className="space-y-1.5">
                      <label className="text-[11px] font-medium text-muted-foreground uppercase tracking-wider">Chunk Size (MB)</label>
                      <input
                        type="number"
                        min={5}
                        max={100}
                        value={config.upload.multipart_chunk_mb}
                        onChange={(e) => setConfig({ ...config, upload: { ...config.upload, multipart_chunk_mb: parseInt(e.target.value) || 32 } })}
                        className="w-full h-8 px-3 rounded-md border border-input bg-background text-foreground text-sm"
                      />
                    </div>
                    <div className="space-y-1.5">
                      <label className="text-[11px] font-medium text-muted-foreground uppercase tracking-wider">Poll Interval (s)</label>
                      <input
                        type="number"
                        min={5}
                        max={300}
                        value={config.upload.poll_interval_s}
                        onChange={(e) => setConfig({ ...config, upload: { ...config.upload, poll_interval_s: parseInt(e.target.value) || 30 } })}
                        className="w-full h-8 px-3 rounded-md border border-input bg-background text-foreground text-sm"
                      />
                    </div>
                    <div className="space-y-1.5">
                      <label className="text-[11px] font-medium text-muted-foreground uppercase tracking-wider">Settle Time (s)</label>
                      <input
                        type="number"
                        min={0}
                        max={120}
                        value={config.upload.file_settle_s}
                        onChange={(e) => setConfig({ ...config, upload: { ...config.upload, file_settle_s: parseInt(e.target.value) || 10 } })}
                        className="w-full h-8 px-3 rounded-md border border-input bg-background text-foreground text-sm"
                      />
                    </div>
                  </div>
                </div>
              )}
            </div>
          </section>
        )}

        {/* Wizard navigation / Save */}
        <div className="flex items-center justify-between pt-4">
          {isWizard && wizardStep > 0 && (
            <Button variant="ghost" onClick={() => setWizardStep(wizardStep - 1)}>Back</Button>
          )}
          <div className="ml-auto flex items-center gap-2">
            {saveStatus && <span className="text-[11px] text-muted-foreground">{saveStatus}</span>}
            {isWizard && wizardStep < 2 ? (
              <Button
                variant="highlight"
                onClick={() => setWizardStep(wizardStep + 1)}
                disabled={wizardStep === 0 && !binaryTestPassed}
              >
                Next <ChevronRight className="size-4" />
              </Button>
            ) : (
              <Button variant="highlight" onClick={handleSave} disabled={saving}>
                {saving ? <Loader2 className="size-4 animate-spin" /> : <Check className="size-4" />}
                {isWizard ? "Finish Setup" : "Save"}
              </Button>
            )}
          </div>
        </div>
      </div>
    </ScrollArea>
  );
}
