import { useEffect, useState } from "react";
import Button from "./Button";
import {
  GuardCounts,
  ObsStatus,
  Probe,
  TakeResult,
  obsStatus,
  startTake,
  stopTake,
  clearStop,
  clickElement,
  guardStatus,
  probeElement,
  simulateHang,
  slowMoveTest,
  slowTypeTest,
  typeInto,
} from "./engine";

// Phase 0 steps 4–7 against the demo target (launch it in Step 2 first).
const NEW_RECORDING = { testId: "create-recording" };
const TITLE = { testId: "title-input" };

export default function EnginePanel() {
  const [log, setLog] = useState<{ ok: boolean; text: string }[]>([]);
  const [probe, setProbe] = useState<Probe | null>(null);
  const [guard, setGuard] = useState<GuardCounts | null>(null);
  const [busy, setBusy] = useState(false);
  const [obs, setObs] = useState<ObsStatus | null>(null);
  const [take, setTake] = useState<TakeResult | null>(null);

  useEffect(() => {
    const id = setInterval(() => guardStatus().then(setGuard), 500);
    return () => clearInterval(id);
  }, []);

  async function run(name: string, action: () => Promise<unknown>) {
    setBusy(true);
    const started = performance.now();
    try {
      await action();
      setLog((l) => [{ ok: true, text: `${name}: ok (${Math.round(performance.now() - started)} ms)` }, ...l].slice(0, 8));
    } catch (e) {
      setLog((l) => [{ ok: false, text: `${name}: ${e}` }, ...l].slice(0, 8));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="space-y-4">
      <header>
        <p className="text-xs uppercase tracking-widest text-indigo-400">Phase 0 · Steps 4–7</p>
        <h2 className="text-lg font-semibold">Real cursor, emergency stop, input guard</h2>
        <p className="mt-1 text-sm text-zinc-400">
          These move your real mouse: hands off while they run. <kbd>Ctrl Alt Esc</kbd> stops
          Scenra at any time, even while the UI is frozen. Moving the mouse yourself also stops it.
        </p>
      </header>

      <div className="flex flex-wrap gap-2">
        <Button disabled={busy} onClick={() => run("Probe New Recording", async () => setProbe(await probeElement(NEW_RECORDING)))}>
          Probe New Recording
        </Button>
        <Button disabled={busy} onClick={() => run("Click New Recording", async () => setProbe(await clickElement(NEW_RECORDING)))}>
          Click New Recording
        </Button>
        <Button disabled={busy} onClick={() => run("Type title", () => typeInto(TITLE, "Product sync"))}>
          Type title
        </Button>
        <Button disabled={busy} onClick={() => run("Slow move", slowMoveTest)}>
          Slow move (8 s)
        </Button>
        <Button
          disabled={busy}
          onClick={() =>
            run("Slow type", async () => {
              await clickElement(TITLE);
              await slowTypeTest(" weekly product sync with the design team");
            })
          }
        >
          Slow type (10 s)
        </Button>
        {/* Not disabled while busy: the hang has to overlap a motion or typing test. */}
        <Button onClick={() => void simulateHang(10)}>
          Simulate UI hang (10 s)
        </Button>
        <Button disabled={busy} onClick={() => run("Clear stop", clearStop)}>
          Clear stop
        </Button>
      </div>

      <div className="space-y-2 rounded-lg bg-zinc-900 p-4 text-sm">
        <p className="font-medium">OBS take (step 5)</p>
        <p className="text-zinc-400">
          {obs
            ? `OBS ${obs.version} · profile ${obs.profile} · mic ${obs.mic ?? "none"}${obs.recording ? " · RECORDING" : ""}`
            : "Records with Scenra's own OBS profile (MKV, PCM, mic on track 2), then extracts microphone.wav."}
        </p>
        <div className="flex flex-wrap gap-2">
          <Button disabled={busy} onClick={() => run("OBS status", async () => setObs(await obsStatus()))}>
            Check OBS
          </Button>
          <Button disabled={busy} onClick={() => run("Start take", startTake)}>
            Start take
          </Button>
          <Button disabled={busy} onClick={() => run("Stop take", async () => setTake(await stopTake()))}>
            Stop take
          </Button>
          <Button
            disabled={busy}
            onClick={() =>
              run("Demo take", async () => {
                await startTake();
                await new Promise((r) => setTimeout(r, 1500));
                await clickElement(NEW_RECORDING);
                await new Promise((r) => setTimeout(r, 600));
                await typeInto(TITLE, "Product sync");
                await new Promise((r) => setTimeout(r, 1500));
                setTake(await stopTake());
              })
            }
          >
            Record demo take
          </Button>
        </div>
        {take && (
          <p className="text-xs break-all text-emerald-300">
            {take.video} · microphone.wav {take.microphoneSeconds.toFixed(1)} s @ {take.sampleRate} Hz × {take.channels}
          </p>
        )}
      </div>

      {guard && (
        <dl className="grid grid-cols-[10rem_1fr] gap-y-1 rounded-lg bg-zinc-900 p-4 text-sm">
          <dt className="text-zinc-400">Emergency stop</dt>
          <dd className={guard.stopped ? "font-semibold text-red-300" : ""}>{guard.stopped ? "STOPPED" : "armed"}</dd>
          <dt className="text-zinc-400">Scenra input events</dt>
          <dd>{guard.expected.toLocaleString()}</dd>
          <dt className="text-zinc-400">Your input during runs</dt>
          <dd>{guard.foreign.toLocaleString()}</dd>
        </dl>
      )}

      {probe && (
        <div className={`rounded-lg p-4 text-sm ${probe.passed ? "bg-emerald-950 text-emerald-200" : "bg-red-950 text-red-200"}`}>
          <p className="font-medium">
            Hover probe {probe.passed ? "passed" : "failed"}
            {probe.errorCss !== null && ` · off by ${probe.errorCss.toFixed(2)} CSS px`}
          </p>
          <p className="mt-1 text-xs opacity-80">
            dpr {probe.located.devicePixelRatio} · screen {Math.round(probe.located.screen.x)},{Math.round(probe.located.screen.y)}{" "}
            {Math.round(probe.located.screen.width)}×{Math.round(probe.located.screen.height)} px · content area at{" "}
            {probe.located.viewport.x},{probe.located.viewport.y} · page saw{" "}
            {probe.seenCss ? probe.seenCss.map((v) => v.toFixed(1)).join(", ") : "nothing"}
          </p>
        </div>
      )}

      {log.length > 0 && (
        <ul className="space-y-1 font-mono text-xs">
          {log.map((l, i) => (
            <li key={i} className={l.ok ? "text-emerald-300" : "text-amber-300"}>
              {l.text}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
