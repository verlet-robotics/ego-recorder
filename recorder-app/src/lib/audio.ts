let audioCtx: AudioContext | null = null;

function getAudioContext(): AudioContext {
  if (!audioCtx) {
    audioCtx = new AudioContext();
  }
  // Resume if suspended (browsers require user gesture to start audio)
  if (audioCtx.state === "suspended") {
    audioCtx.resume();
  }
  return audioCtx;
}

/** Play a tone at the given frequency for the given duration. */
function playTone(frequency: number, durationMs: number, volume = 0.3): void {
  const ctx = getAudioContext();
  const osc = ctx.createOscillator();
  const gain = ctx.createGain();

  osc.type = "sine";
  osc.frequency.value = frequency;
  gain.gain.value = volume;

  // Quick fade out to avoid click
  gain.gain.setValueAtTime(volume, ctx.currentTime);
  gain.gain.exponentialRampToValueAtTime(0.001, ctx.currentTime + durationMs / 1000);

  osc.connect(gain);
  gain.connect(ctx.destination);

  osc.start();
  osc.stop(ctx.currentTime + durationMs / 1000);
}

/** Play the 3-second countdown: beep at 3, 2, 1, then GO.
 *  Pass an AbortSignal to cancel mid-countdown (e.g. on unmount). */
export async function playCountdown(
  onTick: (remaining: number) => void,
  signal?: AbortSignal,
): Promise<void> {
  for (const n of [3, 2, 1]) {
    if (signal?.aborted) throw new DOMException("Aborted", "AbortError");
    onTick(n);
    playTone(800, 150);
    await sleep(1000);
  }
  if (signal?.aborted) throw new DOMException("Aborted", "AbortError");
  onTick(0);
  playTone(1200, 300, 0.4);
}

/** Play a confirmation tone for service mode toggle. */
export function playConfirmTone(): void {
  playTone(600, 100, 0.2);
  setTimeout(() => playTone(900, 100, 0.2), 120);
}

/** Play an urgent warning tone (two descending beeps). */
export function playWarningTone(): void {
  playTone(1000, 200, 0.4);
  setTimeout(() => playTone(600, 300, 0.4), 250);
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
