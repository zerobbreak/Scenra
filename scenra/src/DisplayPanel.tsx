import { useEffect, useState } from "react";
import Button from "./Button";
import { DisplayInfo, displayInfo, setOverlayZoom } from "./display";
import { Probe, probeElement } from "./engine";

const NEW_RECORDING = { testId: "create-recording" };
const percent = (value: number) => `${Math.round(value * 100)} %`;

/** What Scenra detected about the presenter's displays, the pill size, and a quick self-check. */
export default function DisplayPanel() {
  const [info, setInfo] = useState<DisplayInfo | null>(null);
  const [zoom, setZoom] = useState(1);
  const [probe, setProbe] = useState<Probe | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function run<T>(action: () => Promise<T>, apply: (value: T) => void) {
    setBusy(true);
    setError(null);
    try {
      apply(await action());
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  const show = (next: DisplayInfo) => {
    setInfo(next);
    setZoom(next.overlayZoom);
  };

  useEffect(() => {
    run(displayInfo, show);
  }, []);

  return (
    <section className="space-y-4">
      <header>
        <p className="text-xs uppercase tracking-widest text-indigo-400">Setup</p>
        <h2 className="text-lg font-semibold">Display</h2>
        <p className="mt-1 text-sm text-zinc-400">
          Scenra works at any Windows display scaling. Check what it detected, size the prompter
          for your screen, and run the self-check with the demo target open (Step 2).
        </p>
      </header>

      {info && (
        <ul className="space-y-1 rounded-lg bg-zinc-900 p-4 text-sm">
          {info.monitors.map((m) => (
            <li key={m.rect.join(",")}>
              {m.rect[2]}×{m.rect[3]} at {percent(m.scale)} scaling
              {m.primary && <span className="text-zinc-400"> · primary</span>}
              <span className="text-zinc-500"> · {m.name ?? "unnamed"} at ({m.rect[0]}, {m.rect[1]})</span>
            </li>
          ))}
        </ul>
      )}

      {info && (
        <div className="space-y-2 rounded-lg bg-zinc-900 p-4 text-sm">
          <label className="flex items-center gap-3">
            <span className="w-28 text-zinc-400">Prompter size</span>
            <input
              type="range"
              className="flex-1"
              min={info.minZoom}
              max={info.maxZoom}
              step={0.05}
              value={zoom}
              onChange={(e) => setZoom(Number(e.target.value))}
              onPointerUp={() => run(() => setOverlayZoom(zoom), show)}
              onKeyUp={() => run(() => setOverlayZoom(zoom), show)}
            />
            <span className="w-12 text-right">{percent(zoom)}</span>
          </label>
          <p className="text-xs text-zinc-500">
            Resizes the pill and control bar on top of Windows scaling. Open the prompter (Step 1) to
            see it change. If it wouldn't fit your screen, Scenra shrinks it to fit.
          </p>
        </div>
      )}

      <div className="flex flex-wrap gap-2">
        <Button disabled={busy} onClick={() => run(() => probeElement(NEW_RECORDING), setProbe)}>
          Check pointer accuracy
        </Button>
        <Button disabled={busy} onClick={() => run(displayInfo, show)}>
          Re-detect displays
        </Button>
      </div>

      {probe && (
        <p className={`rounded-lg p-3 text-sm ${probe.passed ? "bg-emerald-950 text-emerald-200" : "bg-red-950 text-red-200"}`}>
          {probe.passed ? "Pointer lands where the page expects" : "Pointer missed the target"}:{" "}
          {probe.errorCss === null ? "the page never saw the cursor" : `${probe.errorCss.toFixed(2)} CSS px off (limit 2)`}
          {" "}at devicePixelRatio {probe.located.devicePixelRatio}.
        </p>
      )}

      {error && <p className="rounded-lg bg-red-950 p-3 text-sm text-red-200">{error}</p>}
    </section>
  );
}
