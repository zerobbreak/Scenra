import { useEffect, useState } from "react";
import {
  CaptureStatus,
  ProbeResult,
  captureStatus,
  closeTeleprompter,
  openTeleprompter,
  probeCapture,
  setCaptureExcluded,
} from "./capture";

const AFFINITY_NAMES: Record<number, string> = {
  0x00: "WDA_NONE",
  0x01: "WDA_MONITOR",
  0x11: "WDA_EXCLUDEFROMCAPTURE",
};

function Button(props: { onClick: () => void; disabled?: boolean; children: React.ReactNode }) {
  return (
    <button
      className="rounded-md bg-indigo-600 px-3 py-1.5 text-sm font-medium hover:bg-indigo-500 disabled:opacity-40"
      {...props}
    />
  );
}

export default function App() {
  const [status, setStatus] = useState<CaptureStatus | null>(null);
  const [probe, setProbe] = useState<ProbeResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function run<T>(action: () => Promise<T>, apply: (value: T) => void) {
    setError(null);
    try {
      apply(await action());
    } catch (e) {
      setError(String(e));
    }
  }
  const update = (action: () => Promise<CaptureStatus>) =>
    run(action, (next) => {
      setStatus(next);
      setProbe(null);
    });

  useEffect(() => {
    update(captureStatus);
  }, []);

  const open = status?.open ?? false;

  return (
    <main className="mx-auto max-w-2xl space-y-6 p-6">
      <header>
        <p className="text-xs uppercase tracking-widest text-indigo-400">Phase 0 · Step 1</p>
        <h1 className="text-xl font-semibold">Teleprompter capture exclusion</h1>
        <p className="mt-1 text-sm text-zinc-400">
          Open the teleprompter over the demo target, record with OBS, and check the
          teleprompter is missing from the video. Procedure: docs/phase0/01-capture-exclusion.md
        </p>
      </header>

      <section className="flex flex-wrap gap-2">
        <Button onClick={() => update(() => openTeleprompter(true))}>Open (excluded)</Button>
        <Button onClick={() => update(() => openTeleprompter(false))}>Open (control, visible)</Button>
        <Button disabled={!open} onClick={() => update(() => setCaptureExcluded(!status?.excluded))}>
          Toggle exclusion
        </Button>
        <Button disabled={!open} onClick={() => run(probeCapture, setProbe)}>
          Run GDI probe
        </Button>
        <Button disabled={!open} onClick={() => update(closeTeleprompter)}>
          Close
        </Button>
      </section>

      {status && (
        <dl className="grid grid-cols-[10rem_1fr] gap-y-1 rounded-lg bg-zinc-900 p-4 text-sm">
          <dt className="text-zinc-400">Windows build</dt>
          <dd>
            {status.osBuild}{" "}
            {status.supported ? (
              <span className="text-emerald-400">supported</span>
            ) : (
              <span className="text-amber-400">too old (needs 19041+)</span>
            )}
          </dd>
          <dt className="text-zinc-400">Teleprompter</dt>
          <dd>{open ? "open" : "closed"}</dd>
          <dt className="text-zinc-400">Display affinity</dt>
          <dd>
            {status.affinity === null
              ? "—"
              : `${AFFINITY_NAMES[status.affinity] ?? "unknown"} (0x${status.affinity.toString(16)})`}
          </dd>
        </dl>
      )}

      {probe && (
        <div
          className={`rounded-lg p-4 text-sm ${probe.leaks ? "bg-red-950 text-red-200" : "bg-emerald-950 text-emerald-200"}`}
        >
          <p className="font-medium">
            {probe.leaks
              ? "Teleprompter is visible to GDI capture"
              : "Teleprompter is hidden from GDI capture"}
          </p>
          <p className="mt-1 text-xs opacity-80">
            {probe.markerPixels.toLocaleString()} of {probe.totalPixels.toLocaleString()} pixels
            matched the marker · rect {probe.rect.join(", ")} (physical px). GDI is a pre-check;
            OBS is the real gate.
          </p>
        </div>
      )}

      {error && <p className="rounded-lg bg-red-950 p-3 text-sm text-red-200">{error}</p>}
    </main>
  );
}
