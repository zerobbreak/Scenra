import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import Button from "./Button";
import DisplayPanel from "./DisplayPanel";
import EnginePanel from "./EnginePanel";
import ScriptPanel from "./ScriptPanel";
import TargetPanel from "./TargetPanel";
import {
  CaptureStatus,
  MatrixRow,
  ProbeResult,
  captureStatus,
  closeTeleprompter,
  openTeleprompter,
  probeCapture,
  runCaptureMatrix,
  setCaptureExcluded,
} from "./capture";

const AFFINITY_NAMES: Record<number, string> = {
  0x00: "WDA_NONE",
  0x01: "WDA_MONITOR",
  0x11: "WDA_EXCLUDEFROMCAPTURE",
};

export default function App() {
  const [status, setStatus] = useState<CaptureStatus | null>(null);
  const [probe, setProbe] = useState<ProbeResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [matrix, setMatrix] = useState<MatrixRow[] | null>(null);
  const [matrixRunning, setMatrixRunning] = useState(false);

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
    // The control bar's Stop button closes the teleprompter from another window.
    const unlisten = listen("teleprompter-closed", () => update(captureStatus));
    return () => {
      unlisten.then((stop) => stop());
    };
  }, []);

  const open = status?.open ?? false;

  return (
    <main className="mx-auto max-w-2xl space-y-6 p-6">
      <DisplayPanel />
      <hr className="border-zinc-800" />

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
        <Button
          disabled={matrixRunning}
          onClick={async () => {
            setMatrixRunning(true);
            setMatrix(null);
            await run(runCaptureMatrix, setMatrix);
            setMatrixRunning(false);
            update(captureStatus);
          }}
        >
          {matrixRunning ? "Running matrix…" : "Run OBS matrix"}
        </Button>
      </section>

      {matrix && (
        <div className="space-y-1 rounded-lg bg-zinc-900 p-4 text-sm">
          <p className="font-medium">OBS matrix at {Math.round(matrix[0]?.scale * 100)} % scaling</p>
          {matrix.map((row) => (
            <p key={row.name} className={row.passed ? "text-emerald-300" : "text-red-300"}>
              {row.passed ? "PASS" : "FAIL"} · {row.name} ·{" "}
              {row.shots.map((s) => `${s.label}: ${s.pillSeen ? "pill seen" : "no pill"}`).join(", ")}
            </p>
          ))}
          <p className="text-xs break-all text-zinc-500">Frames and takes: {matrix[0]?.take}</p>
        </div>
      )}

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

      <hr className="border-zinc-800" />
      <TargetPanel />

      <hr className="border-zinc-800" />
      <EnginePanel />

      <hr className="border-zinc-800" />
      <ScriptPanel />
    </main>
  );
}
