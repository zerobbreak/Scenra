import { useEffect, useState } from "react";
import Button from "./Button";
import { Scenario } from "./script";
import {
  FLAG_TEXT, TeachSession, TeachStep, deleteTeachStep, draftFromTeach, onTeachStep, startTeach, stopTeach, teachState,
} from "./teach";

// Teach mode: click through the workflow once by hand, then let Gemini narrate exactly those clicks.
export default function TeachPanel(props: {
  brief: string;
  busy: boolean;
  aiConfigured: boolean;
  run: (label: string, action: () => Promise<void>) => Promise<void>;
  onScript: (script: Scenario) => void;
}) {
  const [active, setActive] = useState(false);
  const [live, setLive] = useState<TeachStep[]>([]);
  const [session, setSession] = useState<TeachSession | null>(null);

  useEffect(() => {
    teachState().then((status) => {
      setActive(status.active);
      setSession(status.last);
    });
    const unlisten = onTeachStep((step) => setLive((steps) => [...steps, step]));
    return () => {
      unlisten.then((stop) => stop());
    };
  }, []);

  const steps = active ? live : session?.steps ?? [];
  const usable = (session?.steps ?? []).filter((s) => s.flags.length === 0).length;

  return (
    <div className="space-y-3 rounded-lg bg-zinc-900 p-4 text-sm">
      <div>
        <p className="font-semibold">Teach a walkthrough</p>
        <p className="mt-1 text-zinc-400">
          Click through your workflow once in the app. Scenra records each click and the page just
          before it; Gemini then narrates exactly those clicks, in order, using the brief above.
          Clicks only for now: typing into fields isn't recorded.
        </p>
      </div>

      <div className="flex flex-wrap gap-2">
        {active ? (
          <Button
            disabled={props.busy}
            onClick={() => props.run("Stopping…", async () => {
              const finished = await stopTeach();
              setActive(false);
              setSession(finished);
            })}
          >
            Stop teaching
          </Button>
        ) : (
          <Button
            disabled={props.busy}
            onClick={() => props.run("Starting…", async () => {
              setLive([]);
              await startTeach();
              setActive(true);
            })}
          >
            Start teaching
          </Button>
        )}
        <Button
          disabled={props.busy || active || usable === 0 || !props.aiConfigured}
          onClick={() => props.run("Drafting from the walkthrough… (about 10–30 s)", async () => {
            props.onScript(await draftFromTeach(props.brief));
          })}
        >
          Draft from walkthrough
        </Button>
      </div>

      {active && <p className="text-emerald-300">Teaching: click through the app now, then press Stop teaching.</p>}

      {steps.length > 0 && (
        <ol className="space-y-1" aria-label="Taught steps">
          {steps.map((step) => (
            <li key={`${step.index}-${step.atMs}`} className="flex items-start gap-2">
              <span className="w-6 shrink-0 text-right text-zinc-500">{step.index + 1}.</span>
              <span className="min-w-0 flex-1">
                <span className={step.flags.length ? "text-zinc-400" : ""}>
                  {step.label ? `Click “${step.label}”` : step.flags.includes("field") ? "Click into a field" : "Click"}
                </span>
                {step.flags.length > 0 && (
                  <span className="block text-xs text-amber-300">
                    Skipped: {step.flags.map((f) => FLAG_TEXT[f]).join(", ")}
                  </span>
                )}
              </span>
              {!active && (
                <button
                  aria-label={`Delete step ${step.index + 1}`}
                  title="Delete this step"
                  disabled={props.busy}
                  className="rounded px-1.5 text-zinc-500 hover:bg-zinc-800 hover:text-zinc-200 disabled:opacity-40"
                  onClick={() => props.run("Deleting step…", async () => setSession(await deleteTeachStep(step.index)))}
                >
                  ×
                </button>
              )}
            </li>
          ))}
        </ol>
      )}

      {!active && session && session.steps.length === 0 && <p className="text-zinc-400">No clicks were recorded.</p>}
      {!active && session && session.warnings.length > 0 && (
        <ul className="list-disc pl-5 text-amber-300">
          {session.warnings.map((w) => <li key={w}>{w}</li>)}
        </ul>
      )}
    </div>
  );
}
