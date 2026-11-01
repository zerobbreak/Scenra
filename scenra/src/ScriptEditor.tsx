import { useState } from "react";
import Button from "./Button";
import { Action, Scenario, Step, captureCodePosition, setScript } from "./script";

const words = (text: string) => text.split(/\s+/).filter(Boolean);

/// Write or edit a script by hand: narration lines, and on each line the actions that fire on a
/// chosen word: clicks, or code steps in VS Code (docs/adr/0003-code-navigation.md).

type Kind = "click" | Step["kind"];
const KINDS: { kind: Kind; label: string }[] = [
  { kind: "click", label: "Click" },
  { kind: "open", label: "Open file" },
  { kind: "scroll", label: "Scroll to lines" },
  { kind: "highlight", label: "Highlight lines" },
  { kind: "palette", label: "Run command" },
  { kind: "search", label: "Search" },
];

const field = "min-w-0 rounded-md bg-zinc-950 px-2 py-1 text-sm ring-1 ring-zinc-700 focus:ring-indigo-500 focus:outline-none";
const small = "rounded px-2 py-1 text-xs text-zinc-400 hover:bg-zinc-800 hover:text-zinc-200";

const emptyScript = (): Scenario => ({ title: "", warnings: [], scenes: [{ title: "Scene 1", lines: [{ text: "", actions: [] }] }] });

function newStep(kind: Kind): Step | null {
  switch (kind) {
    case "click": return null;
    case "open": return { kind, path: "", line: null, instant: false };
    case "scroll":
    case "highlight": return { kind, path: "", at: { line: 1, text: "", symbol: null }, lines: kind === "scroll" ? 10 : 3 };
    case "palette": return { kind, command: "" };
    case "search": return { kind, query: "" };
  }
}

export default function ScriptEditor({ initial, onSaved }: { initial: Scenario | null; onSaved: (s: Scenario) => void }) {
  const [draft, setDraft] = useState<Scenario>(() => (initial ? structuredClone(initial) : emptyScript()));
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // Immutable update helper: edit a copy, then store it.
  const change = (edit: (s: Scenario) => void) =>
    setDraft((d) => {
      const copy = structuredClone(d);
      edit(copy);
      return copy;
    });

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      onSaved(await setScript(draft));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="space-y-4 rounded-lg bg-zinc-900 p-4">
      <input className={`${field} w-full text-base`} placeholder="Script title" value={draft.title}
        onChange={(e) => change((s) => { s.title = e.target.value; })} />

      {draft.scenes.map((scene, si) => (
        <div key={si} className="space-y-2 rounded-md border border-zinc-800 p-3">
          <div className="flex items-center gap-2">
            <span className="text-xs font-bold tracking-wide text-zinc-500">SCENE {si + 1}</span>
            <input className={`${field} flex-1`} value={scene.title} placeholder="Scene title"
              onChange={(e) => change((s) => { s.scenes[si].title = e.target.value; })} />
            {draft.scenes.length > 1 && (
              <button type="button" className={small} onClick={() => change((s) => { s.scenes.splice(si, 1); })}>Remove scene</button>
            )}
          </div>

          {scene.lines.map((line, li) => (
            <div key={li} className="space-y-1 pl-3">
              <div className="flex gap-2">
                <input className={`${field} flex-1`} placeholder="What you say…" value={line.text}
                  onChange={(e) => change((s) => {
                    const l = s.scenes[si].lines[li];
                    l.text = e.target.value;
                    const n = words(l.text).length;
                    for (const a of l.actions) a.word = Math.min(a.word, Math.max(0, n - 1));
                  })} />
                <button type="button" className={small} onClick={() => change((s) => { s.scenes[si].lines[li].actions.push({ word: 0, target: "", label: "", step: null }); })}>
                  + action
                </button>
                {scene.lines.length > 1 && (
                  <button type="button" className={small} onClick={() => change((s) => { s.scenes[si].lines.splice(li, 1); })}>✕</button>
                )}
              </div>
              {line.actions.map((action, ai) => (
                <ActionRow key={ai} action={action} words={words(line.text)}
                  onChange={(next) => change((s) => { s.scenes[si].lines[li].actions[ai] = next; })}
                  onRemove={() => change((s) => { s.scenes[si].lines[li].actions.splice(ai, 1); })}
                  onError={setError} />
              ))}
            </div>
          ))}
          <button type="button" className={small} onClick={() => change((s) => { s.scenes[si].lines.push({ text: "", actions: [] }); })}>+ line</button>
        </div>
      ))}

      <div className="flex flex-wrap items-center gap-2">
        <button type="button" className={small}
          onClick={() => change((s) => { s.scenes.push({ title: `Scene ${s.scenes.length + 1}`, lines: [{ text: "", actions: [] }] }); })}>
          + scene
        </button>
        <div className="flex-1" />
        <Button disabled={busy} onClick={() => void save()}>{busy ? "Saving…" : "Use this script"}</Button>
      </div>
      {error && <p className="m-0 rounded-md bg-red-950 p-2 text-sm text-red-200">{error}</p>}
      <p className="m-0 text-xs text-zinc-500">
        Every scene starts from the app as it is when you save. For a later scene that starts somewhere else, get there and
        press “Use current page” in the prompter.
      </p>
    </div>
  );
}

function ActionRow({ action, words, onChange, onRemove, onError }: {
  action: Action; words: string[]; onChange: (a: Action) => void; onRemove: () => void; onError: (e: string | null) => void;
}) {
  const kind: Kind = action.step?.kind ?? "click";
  const step = action.step;
  const set = (patch: Partial<Action>) => onChange({ ...action, ...patch });
  const setStep = (patch: Record<string, unknown>) => set({ step: { ...(step as Step), ...patch } as Step });

  const capture = async () => {
    onError(null);
    try {
      const p = await captureCodePosition();
      setStep({ path: p.path, at: p.at, lines: p.lines });
    } catch (e) {
      onError(String(e));
    }
  };

  return (
    <div className="flex flex-wrap items-center gap-2 rounded-md bg-zinc-950/60 px-2 py-1.5 text-sm">
      <span className="text-xs text-zinc-500">on</span>
      <select className={field} value={Math.min(action.word, Math.max(0, words.length - 1))} onChange={(e) => set({ word: Number(e.target.value) })}>
        {(words.length ? words : ["(type the line first)"]).map((w, i) => <option key={i} value={i}>{i + 1}. {w}</option>)}
      </select>
      <select className={field} value={kind} onChange={(e) => set({ step: newStep(e.target.value as Kind), target: "", label: "" })}>
        {KINDS.map((k) => <option key={k.kind} value={k.kind}>{k.label}</option>)}
      </select>

      {kind === "click" && (
        <input className={`${field} w-48`} placeholder="Button or item name" value={action.target}
          onChange={(e) => set({ target: e.target.value, label: `Click ${e.target.value}` })} />
      )}
      {step?.kind === "open" && (
        <>
          <input className={`${field} w-64`} placeholder="src/main.rs (inside the codebase)" value={step.path} onChange={(e) => setStep({ path: e.target.value })} />
          <input className={`${field} w-20`} type="number" min={1} placeholder="line" value={step.line ?? ""}
            onChange={(e) => setStep({ line: e.target.value ? Number(e.target.value) : null })} />
          <label className="flex items-center gap-1 text-xs text-zinc-400">
            <input type="checkbox" checked={!!step.instant} onChange={(e) => setStep({ instant: e.target.checked })} /> instant
          </label>
        </>
      )}
      {(step?.kind === "scroll" || step?.kind === "highlight") && (
        <>
          <button type="button" className="rounded-md bg-indigo-700 px-2 py-1 text-xs hover:bg-indigo-600" onClick={() => void capture()}
            title="Select the lines in VS Code (or put the cursor on one), then click">
            Use VS Code selection
          </button>
          <input className={`${field} w-56`} placeholder="path" value={step.path} onChange={(e) => setStep({ path: e.target.value })} />
          <span className="text-xs text-zinc-500">
            {step.at.text ? `line ${step.at.line} “${step.at.text.slice(0, 40)}”${step.at.symbol ? ` in ${step.at.symbol}` : ""}` : `line ${step.at.line}`}
          </span>
          <input className={`${field} w-16`} type="number" min={1} value={step.lines} title="How many lines"
            onChange={(e) => setStep({ lines: Math.max(1, Number(e.target.value) || 1) })} />
          <span className="text-xs text-zinc-500">lines</span>
        </>
      )}
      {step?.kind === "palette" && (
        <input className={`${field} w-64`} placeholder="View: Toggle Terminal" value={step.command} onChange={(e) => setStep({ command: e.target.value })} />
      )}
      {step?.kind === "search" && (
        <input className={`${field} w-64`} placeholder="what to search for" value={step.query} onChange={(e) => setStep({ query: e.target.value })} />
      )}
      <input className={`${field} w-40`} placeholder="Pill label (optional)" value={action.label} onChange={(e) => set({ label: e.target.value })} />
      <button type="button" className={small} onClick={onRemove}>✕</button>
    </div>
  );
}
