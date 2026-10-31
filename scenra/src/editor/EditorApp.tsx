import { useEffect, useMemo, useRef, useState } from "react";
import {
  Cut,
  Edit,
  EditorAiStatus,
  Word,
  assembleSession,
  editorAiStatus,
  onAssembleProgress,
  setTranscription,
  SceneEdit,
  Segment,
  SessionData,
  SessionSummary,
  TakeInfo,
  cancelExport,
  exportSession,
  fmt,
  listSessions,
  loadSession,
  onExportProgress,
  onOpenSession,
  retakeScene,
  revealInExplorer,
  describeSession,
  Description,
  saveEdit,
  sessionDate,
  tidySession,
} from "./editor";
import Player, { PlayPosition, PlayerHandle } from "./Player";
import TranscriptView from "./TranscriptView";

/** The Editor window: pick a session, then shape it into one finished video. */
export default function EditorApp() {
  const [sessions, setSessions] = useState<SessionSummary[] | null>(null);
  const [open, setOpen] = useState<SessionData | null>(null);
  const [version, setVersion] = useState(0);
  const [error, setError] = useState<string | null>(null);

  const refresh = () => listSessions().then(setSessions, (e) => setError(String(e)));
  const shown = useRef<SessionData | null>(null);
  useEffect(() => { shown.current = open; }, [open]);
  const openSession = (session: string) =>
    loadSession(session).then(async (next) => {
      // Back from a retake: select each scene's new take.
      const previous = shown.current;
      const updated = previous && previous.dir === next.dir ? await selectNewTakes(previous, next) : null;
      setOpen(updated ?? next);
      setVersion((v) => v + 1);
    }, (e) => setError(String(e)));

  useEffect(() => {
    void refresh();
    const asked = new URLSearchParams(window.location.search).get("session");
    if (asked) void openSession(asked);
    const unlisten = onOpenSession((id) => void openSession(id));
    return () => void unlisten.then((stop) => stop());
  }, []);

  return (
    <main className="flex h-full flex-col">
      {error && (
        <p className="m-0 bg-red-950 px-4 py-2 text-sm text-red-200" onClick={() => setError(null)}>
          {error}
        </p>
      )}
      {open ? (
        <SessionView
          key={`${open.dir}#${version}`}
          initial={open}
          onBack={() => {
            setOpen(null);
            void refresh();
          }}
          onError={setError}
        />
      ) : (
        <SessionList sessions={sessions} onOpen={(s) => void openSession(s.dir)} onRefresh={() => void refresh()} />
      )}
    </main>
  );
}

function SessionList({ sessions, onOpen, onRefresh }: { sessions: SessionSummary[] | null; onOpen: (s: SessionSummary) => void; onRefresh: () => void }) {
  return (
    <section className="mx-auto w-full max-w-3xl space-y-4 overflow-auto p-6">
      <header className="flex items-end justify-between">
        <div>
          <p className="m-0 text-xs uppercase tracking-widest text-indigo-400">Editor</p>
          <h1 className="m-0 text-xl font-semibold">Recorded sessions</h1>
        </div>
        <button type="button" onClick={onRefresh} className="rounded-md bg-zinc-800 px-3 py-1.5 text-sm hover:bg-zinc-700">
          Refresh
        </button>
      </header>
      {sessions === null && <p className="text-sm text-zinc-400">Loading…</p>}
      {sessions?.length === 0 && (
        <p className="text-sm text-zinc-400">No recorded sessions yet. Record a script from the prompter, then come back here.</p>
      )}
      <ul className="m-0 list-none space-y-2 p-0">
        {sessions?.map((s) => (
          <li key={s.dir}>
            <button
              type="button"
              onClick={() => onOpen(s)}
              className="flex w-full items-center gap-4 rounded-lg bg-zinc-900 p-4 text-left hover:bg-zinc-800"
            >
              <div className="min-w-0 flex-1">
                <p className="m-0 truncate font-medium">{s.title}</p>
                <p className="m-0 text-xs text-zinc-400">
                  {sessionDate(s.id)} · {s.scenes} {s.scenes === 1 ? "scene" : "scenes"} · {s.takes} {s.takes === 1 ? "take" : "takes"}
                </p>
              </div>
              {s.exported && <span className="rounded-full bg-emerald-950 px-2 py-0.5 text-xs text-emerald-300">Exported</span>}
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}

/** After a retake: point each scene that got a new take at it (fresh trims and cuts). */
async function selectNewTakes(before: SessionData, after: SessionData): Promise<SessionData | null> {
  const known = new Set(before.takes.map((t) => t.folder));
  const fresh = after.takes.filter((t) => !known.has(t.folder));
  if (!fresh.length) return null;
  const edit = {
    ...after.edit,
    scenes: after.edit.scenes.map((s) => {
      const take = fresh.filter((t) => t.scene === s.scene).sort((a, b) => b.attempt - a.attempt)[0];
      return take ? { ...s, take: take.folder, takeNote: null, trimIn: 0, trimOut: null, trimNote: null, cuts: [] } : s;
    }),
  };
  return saveEdit(after.dir, edit);
}

// ---- Edit operations (pure) -------------------------------------------------------------------

const withScene = (edit: Edit, scene: number, change: (s: SceneEdit) => SceneEdit): Edit => ({
  ...edit,
  scenes: edit.scenes.map((s) => (s.scene === scene ? change(s) : s)),
});
const round = (t: number) => Math.round(t * 1000) / 1000;
const cutCount = (s: SceneEdit) => s.cuts.filter((c) => c.enabled).length;

function SessionView({ initial, onBack, onError }: { initial: SessionData; onBack: () => void; onError: (e: string) => void }) {
  const [data, setData] = useState(initial);
  const [history, setHistory] = useState<Edit[]>([]);
  const [future, setFuture] = useState<Edit[]>([]);
  const [selected, setSelected] = useState(0);
  const [mode, setMode] = useState<"film" | "scene">("film");
  const [position, setPosition] = useState<PlayPosition | null>(null);
  const [cutStart, setCutStart] = useState<number | null>(null);
  const [exporting, setExporting] = useState<number | null>(null);
  const [exported, setExported] = useState<string | null>(null);
  const [tidying, setTidying] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const [description, setDescription] = useState<Description | null>(null);
  const [describing, setDescribing] = useState(false);
  const [ai, setAi] = useState<EditorAiStatus | null>(null);
  const [asking, setAsking] = useState(false);
  const [assembling, setAssembling] = useState<{ done: number; total: number } | null>(null);
  const player = useRef<PlayerHandle>(null);

  const { edit, timeline, takes } = data;
  const scene = edit.scenes.find((s) => s.scene === selected) ?? edit.scenes[0];
  const take = takes.find((t) => t.folder === scene?.take && t.scene === scene?.scene) ?? null;
  const sceneTakes = takes.filter((t) => t.scene === selected);

  const apply = async (next: Edit, record = true) => {
    if (record) {
      setHistory((h) => [...h.slice(-99), edit]);
      setFuture([]);
    }
    setData((d) => ({ ...d, edit: next }));
    try {
      setData(await saveEdit(data.dir, next));
    } catch (e) {
      onError(String(e));
    }
  };
  const undo = () => {
    const prev = history[history.length - 1];
    if (!prev) return;
    setHistory((h) => h.slice(0, -1));
    setFuture((f) => [edit, ...f]);
    void apply(prev, false);
  };
  const redo = () => {
    const next = future[0];
    if (!next) return;
    setFuture((f) => f.slice(1));
    setHistory((h) => [...h, edit]);
    void apply(next, false);
  };

  useEffect(() => {
    const keys = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || (e.target as HTMLElement).tagName === "INPUT") return;
      if (e.key === "z" && !e.shiftKey) { e.preventDefault(); undo(); }
      if (e.key === "y" || (e.key === "z" && e.shiftKey)) { e.preventDefault(); redo(); }
    };
    window.addEventListener("keydown", keys);
    return () => window.removeEventListener("keydown", keys);
  });

  useEffect(() => {
    const unlisten = onExportProgress((p) => p.dir === data.dir && setExporting(p.percent));
    const unlistenAi = onAssembleProgress((p) => p.dir === data.dir && setAssembling({ done: p.done, total: p.total }));
    void editorAiStatus().then(setAi);
    return () => {
      void unlisten.then((stop) => stop());
      void unlistenAi.then((stop) => stop());
    };
  }, [data.dir]);

  // Scene mode plays the selected take raw, start to end, so trims and cuts can be placed.
  const segments: Segment[] = useMemo(() => {
    if (mode === "film") return timeline.segments;
    return take ? [{ scene: take.scene, take: take.folder, srcIn: 0, srcOut: take.duration, outAt: 0 }] : [];
  }, [mode, timeline.segments, take]);

  const here = position && mode === "scene" ? round(position.src) : null;
  const sceneOf = (s: number) => data.script?.scenes[s];

  const runTidy = async () => {
    setTidying(true);
    setHistory((h) => [...h.slice(-99), edit]);
    setFuture([]);
    try {
      setData(await tidySession(data.dir));
    } catch (e) {
      onError(String(e));
    } finally {
      setTidying(false);
    }
  };

  const assemble = async () => {
    setAssembling({ done: 0, total: takes.length });
    setHistory((h) => [...h.slice(-99), edit]);
    setFuture([]);
    try {
      setData(await assembleSession(data.dir));
    } catch (e) {
      onError(String(e));
    } finally {
      setAssembling(null);
    }
  };
  const runAssemble = () => (ai?.consent === "allowed" ? void assemble() : setAsking(true));

  /** Removes everything Tidy up and Assemble with AI did; your own choices stay. */
  const removeAutomatic = () =>
    void apply({
      ...edit,
      scenes: edit.scenes.map((s) => ({
        ...s,
        takeNote: null,
        cuts: s.cuts.filter((c) => c.source.kind === "manual"),
        ...(s.trimNote ? { trimIn: 0, trimOut: null, trimNote: null } : {}),
      })),
    });
  const hasAutomatic = edit.scenes.some((s) => s.takeNote || s.trimNote || s.cuts.some((c) => c.source.kind === "ai"));

  const runExport = async () => {
    setExporting(0);
    setExported(null);
    try {
      setExported(await exportSession(data.dir));
      setData(await loadSession(data.dir));
    } catch (e) {
      onError(String(e));
    } finally {
      setExporting(null);
    }
  };

  return (
    <div className="flex min-h-0 flex-1">
      {/* Scene board */}
      <aside className="flex w-72 flex-none flex-col border-r border-zinc-800">
        <div className="space-y-1 border-b border-zinc-800 p-4">
          <button type="button" onClick={onBack} className="text-xs text-indigo-400 hover:underline">
            ← All sessions
          </button>
          <h1 className="m-0 truncate text-lg font-semibold" title={data.title}>{data.title}</h1>
          <p className="m-0 text-xs text-zinc-400">
            {sessionDate(data.id)} · {fmt(timeline.duration)} finished
          </p>
        </div>
        <ol className="m-0 flex-1 list-none space-y-1 overflow-auto p-2">
          {edit.scenes.map((s) => {
            const t = takes.find((x) => x.folder === s.take);
            const kept = timeline.segments.filter((x) => x.scene === s.scene).reduce((sum, x) => sum + x.srcOut - x.srcIn, 0);
            return (
              <li key={s.scene}>
                <button
                  type="button"
                  onClick={() => {
                    setSelected(s.scene);
                    setCutStart(null);
                    if (mode === "film") {
                      const first = timeline.segments.find((x) => x.scene === s.scene);
                      if (first) player.current?.seek(first.outAt);
                    }
                  }}
                  className={`w-full rounded-md p-3 text-left ${s.scene === selected ? "bg-indigo-950 ring-1 ring-indigo-500" : "bg-zinc-900 hover:bg-zinc-800"} ${s.excluded ? "opacity-50" : ""}`}
                >
                  <p className="m-0 text-xs font-bold tracking-wide text-zinc-400">SCENE {s.scene + 1}</p>
                  <p className="m-0 truncate text-sm">{sceneOf(s.scene)?.title ?? (t ? t.folder : "No take")}</p>
                  <p className="m-0 text-xs text-zinc-500">
                    {s.excluded ? "Left out" : t ? `Take ${t.attempt} · ${fmt(kept)}${cutCount(s) ? ` · ${cutCount(s)} ${cutCount(s) === 1 ? "cut" : "cuts"}` : ""}` : "Not recorded"}
                  </p>
                </button>
              </li>
            );
          })}
        </ol>
      </aside>

      {/* Preview + selected scene */}
      <section className="flex min-w-0 flex-1 flex-col overflow-auto">
        <header className="flex items-center gap-2 border-b border-zinc-800 px-4 py-2">
          <div className="flex rounded-md bg-zinc-900 p-0.5 text-sm">
            {(["film", "scene"] as const).map((m) => (
              <button
                key={m}
                type="button"
                onClick={() => setMode(m)}
                className={`rounded px-3 py-1 ${mode === m ? "bg-zinc-700" : "text-zinc-400 hover:text-zinc-200"}`}
              >
                {m === "film" ? "Whole video" : `Scene ${selected + 1} take`}
              </button>
            ))}
          </div>
          <div className="flex-1" />
          <button
            type="button"
            disabled={!ai?.configured || assembling !== null || !takes.length}
            onClick={runAssemble}
            title={ai?.configured ? "Pick the best take of each scene, trim silence, shorten pauses and cut stumbles" : "Add a Gemini key first (Step 3 in the main window)"}
            className="rounded-md bg-violet-700 px-3 py-1.5 text-sm font-medium hover:bg-violet-600 disabled:opacity-40"
          >
            {assembling ? `Listening… ${assembling.done}/${assembling.total}` : "Assemble with AI"}
          </button>
          {hasAutomatic && (
            <button type="button" onClick={removeAutomatic} className="rounded-md bg-zinc-800 px-3 py-1.5 text-sm hover:bg-zinc-700"
              title="Remove every automatic trim, cut and take note; your own edits stay">
              Remove automatic changes
            </button>
          )}
          <button
            type="button"
            disabled={tidying || !timeline.segments.length}
            onClick={() => void runTidy()}
            title="Trim silence at the start and end of each scene and shorten long pauses. Runs on this computer."
            className="rounded-md bg-zinc-800 px-3 py-1.5 text-sm hover:bg-zinc-700 disabled:opacity-40"
          >
            {tidying ? "Tidying…" : "Tidy up"}
          </button>
          <button type="button" disabled={!history.length} onClick={undo} className="rounded-md bg-zinc-800 px-3 py-1.5 text-sm hover:bg-zinc-700 disabled:opacity-40" title="Ctrl+Z">
            Undo
          </button>
          <button type="button" disabled={!future.length} onClick={redo} className="rounded-md bg-zinc-800 px-3 py-1.5 text-sm hover:bg-zinc-700 disabled:opacity-40" title="Ctrl+Y">
            Redo
          </button>
          <button
            type="button"
            disabled={!ai?.configured || describing || !timeline.segments.length}
            onClick={async () => {
              setDescribing(true);
              try { setDescription(await describeSession(data.dir)); } catch (e) { onError(String(e)); } finally { setDescribing(false); }
            }}
            title="Write a title, description and chapters for YouTube"
            className="rounded-md bg-zinc-800 px-3 py-1.5 text-sm hover:bg-zinc-700 disabled:opacity-40"
          >
            {describing ? "Writing…" : "Title & description"}
          </button>
          {exporting === null ? (
            <button
              type="button"
              disabled={!timeline.segments.length}
              onClick={() => void runExport()}
              className="rounded-md bg-indigo-600 px-4 py-1.5 text-sm font-medium hover:bg-indigo-500 disabled:opacity-40"
            >
              Export video
            </button>
          ) : (
            <div className="flex items-center gap-2">
              <div className="h-2 w-40 overflow-hidden rounded bg-zinc-800">
                <div className="h-full bg-indigo-500" style={{ width: `${exporting}%` }} />
              </div>
              <span className="w-10 text-xs tabular-nums">{Math.round(exporting)}%</span>
              <button type="button" onClick={() => void cancelExport()} className="rounded-md bg-zinc-800 px-3 py-1.5 text-sm hover:bg-zinc-700">
                Cancel
              </button>
            </div>
          )}
        </header>

        {asking && (
          <ConsentDialog
            onAnswer={async (allow, remember) => {
              setAsking(false);
              if (allow) {
                setAi(await setTranscription("allowed"));
                void assemble();
              } else if (remember) {
                setAi(await setTranscription("off"));
              }
            }}
          />
        )}

        {note && (
          <p className="m-0 flex items-center gap-3 bg-indigo-950 px-4 py-2 text-sm text-indigo-200">
            <span className="flex-1">{note}</span>
            <button type="button" onClick={() => setNote(null)} className="text-xs underline">Dismiss</button>
          </p>
        )}

        {description && <DescriptionPanel value={description} onClose={() => setDescription(null)} />}

        {exported && (
          <p className="m-0 flex items-center gap-3 bg-emerald-950 px-4 py-2 text-sm text-emerald-200">
            <span className="min-w-0 flex-1 truncate">Exported {exported}</span>
            <button type="button" onClick={() => void revealInExplorer(exported)} className="underline">
              Show in folder
            </button>
          </p>
        )}
        {timeline.warnings.length > 0 && (
          <ul className="m-0 list-none space-y-0.5 bg-amber-950/60 px-4 py-2 text-xs text-amber-200">
            {timeline.warnings.map((w) => <li key={w}>{w}</li>)}
          </ul>
        )}

        <div className="space-y-4 p-4">
          <Player ref={player} dir={data.dir} segments={segments} onPosition={setPosition} />

          {scene && (
            <SceneEditor
              scene={scene}
              take={take}
              takes={sceneTakes}
              lines={sceneOf(scene.scene)?.lines.map((l) => l.text) ?? []}
              words={take ? data.transcripts[take.folder] ?? null : null}
              here={here}
              cutStart={cutStart}
              sceneMode={mode === "scene"}
              onSeek={(src) => {
                if (mode !== "scene") setMode("scene");
                setTimeout(() => player.current?.seek(src), 0);
              }}
              onShowTake={() => setMode("scene")}
              onCutStart={setCutStart}
              onChange={(change) => void apply(withScene(edit, scene.scene, change))}
              canRetake={!!data.script}
              onRetake={() => retakeScene(data.dir, scene.scene).then(() => setNote(`Recording scene ${scene.scene + 1} in the prompter. Press “Back to editor” there when it's saved.`), (e) => onError(String(e)))}
            />
          )}
        </div>
      </section>
    </div>
  );
}

function SceneEditor(props: {
  scene: SceneEdit;
  take: TakeInfo | null;
  takes: TakeInfo[];
  lines: string[];
  words: Word[] | null;
  here: number | null;
  cutStart: number | null;
  sceneMode: boolean;
  onSeek: (src: number) => void;
  onShowTake: () => void;
  onCutStart: (t: number | null) => void;
  onChange: (change: (s: SceneEdit) => SceneEdit) => void;
  canRetake: boolean;
  onRetake: () => void;
}) {
  const { scene, take, takes, here, cutStart, onChange } = props;
  const end = scene.trimOut ?? take?.duration ?? 0;
  const btn = "rounded-md bg-zinc-800 px-3 py-1.5 text-sm hover:bg-zinc-700 disabled:opacity-40";

  return (
    <div className="space-y-3 rounded-lg bg-zinc-900 p-4">
      <div className="flex flex-wrap items-center gap-3">
        <p className="m-0 font-medium">Scene {scene.scene + 1}</p>
        <label className="flex items-center gap-2 text-sm text-zinc-400">
          Take
          <select
            className="rounded-md bg-zinc-800 p-1.5 text-zinc-100"
            value={scene.take ?? ""}
            onChange={(e) => onChange((s) => ({ ...s, take: e.target.value || null, takeNote: null, trimIn: 0, trimOut: null, trimNote: null, cuts: [] }))}
          >
            {takes.length === 0 && <option value="">Not recorded</option>}
            {takes.map((t) => (
              <option key={t.folder} value={t.folder}>
                Take {t.attempt} · {fmt(t.duration)}
                {t.attempt === Math.max(...takes.map((x) => x.attempt)) ? " · latest" : ""}
              </option>
            ))}
          </select>
        </label>
        <label className="flex items-center gap-2 text-sm text-zinc-400">
          <input type="checkbox" checked={scene.excluded} onChange={(e) => onChange((s) => ({ ...s, excluded: e.target.checked }))} />
          Leave out of the video
        </label>
        <div className="flex-1" />
        <button
          type="button"
          disabled={!props.canRetake}
          onClick={props.onRetake}
          title={props.canRetake ? "Record this scene again in the prompter" : "This session was recorded before scripts were saved, so it can't be retaken"}
          className="rounded-md bg-zinc-800 px-3 py-1.5 text-sm hover:bg-zinc-700 disabled:opacity-40"
        >
          Retake scene
        </button>
        {take && !take.hasActions && (
          <span className="text-xs text-amber-300" title="Recorded before Scenra saved click times">
            No click times for this take: cuts aren't checked against clicks
          </span>
        )}
      </div>

      {scene.takeNote && (
        <p className="m-0 rounded-md bg-violet-950/60 px-3 py-1.5 text-xs text-violet-200">AI picked {scene.takeNote}</p>
      )}
      {scene.trimNote && (
        <p className="m-0 flex items-center gap-3 rounded-md bg-zinc-950 px-3 py-1.5 text-sm">
          <span className="flex-1 text-xs text-zinc-400">Auto · trimmed {scene.trimNote}</span>
          <button type="button" className="text-xs text-indigo-400 hover:underline"
            onClick={() => onChange((s) => ({ ...s, trimIn: 0, trimOut: null, trimNote: null }))}>
            Undo trims
          </button>
        </p>
      )}

      {take && <Strip scene={scene} take={take} here={props.sceneMode ? here : null} cutStart={cutStart} onSeek={props.onSeek} />}

      {take && (
        <div className="flex flex-wrap items-center gap-2">
          {!props.sceneMode ? (
            <button type="button" className={btn} onClick={props.onShowTake}>
              Edit this take…
            </button>
          ) : (
            <>
              <span className="w-24 text-xs tabular-nums text-zinc-400">at {fmt(here ?? 0)}</span>
              <button type="button" className={btn} disabled={here === null || here >= end}
                onClick={() => onChange((s) => ({ ...s, trimIn: here!, trimNote: null }))}>
                Start here
              </button>
              <button type="button" className={btn} disabled={here === null || here <= scene.trimIn}
                onClick={() => onChange((s) => ({ ...s, trimOut: here!, trimNote: null }))}>
                End here
              </button>
              {cutStart === null ? (
                <button type="button" className={btn} disabled={here === null} onClick={() => props.onCutStart(here)}>
                  Mark cut start
                </button>
              ) : (
                <>
                  <button type="button" className="rounded-md bg-red-700 px-3 py-1.5 text-sm hover:bg-red-600 disabled:opacity-40"
                    disabled={here === null || here <= cutStart}
                    onClick={() => {
                      const cut: Cut = { from: cutStart, to: here!, source: { kind: "manual" }, enabled: true };
                      onChange((s) => ({ ...s, cuts: [...s.cuts, cut] }));
                      props.onCutStart(null);
                    }}>
                    Cut {fmt(cutStart)} → here
                  </button>
                  <button type="button" className={btn} onClick={() => props.onCutStart(null)}>Cancel cut</button>
                </>
              )}
              {(scene.trimIn > 0 || scene.trimOut !== null) && (
                <button type="button" className={btn} onClick={() => onChange((s) => ({ ...s, trimIn: 0, trimOut: null, trimNote: null }))}>
                  Reset trims
                </button>
              )}
            </>
          )}
        </div>
      )}

      {scene.cuts.length > 0 && (
        <ul className="m-0 list-none space-y-1 p-0 text-sm">
          {scene.cuts.map((c, i) => (
            <li key={`${c.from}-${c.to}-${i}`} className={`flex items-center gap-3 rounded-md bg-zinc-950 px-3 py-1.5 ${c.enabled ? "" : "opacity-50"}`}>
              <span className="tabular-nums">{fmt(c.from)} → {fmt(c.to)}</span>
              <span className="flex-1 truncate text-xs text-zinc-400">
                {c.source.kind === "ai" ? `Auto · ${c.source.reason}` : "Your cut"}
              </span>
              <button type="button" className="text-xs text-indigo-400 hover:underline" onClick={() => props.onSeek(Math.max(0, c.from - 1))}>
                Play around it
              </button>
              <button type="button" className="text-xs text-indigo-400 hover:underline"
                onClick={() => onChange((s) => ({ ...s, cuts: s.cuts.map((x, j) => (j === i ? { ...x, enabled: !x.enabled } : x)) }))}>
                {c.enabled ? "Restore" : "Cut again"}
              </button>
              {c.source.kind === "manual" && (
                <button type="button" className="text-xs text-zinc-400 hover:underline"
                  onClick={() => onChange((s) => ({ ...s, cuts: s.cuts.filter((_, j) => j !== i) }))}>
                  Delete
                </button>
              )}
            </li>
          ))}
        </ul>
      )}

      {props.words && take && (
        <TranscriptView
          words={props.words}
          scene={scene}
          script={props.lines.flatMap((l) => l.split(/\s+/))}
          here={props.sceneMode ? here : null}
          onSeek={props.onSeek}
          onCut={(from, to) => onChange((s) => ({ ...s, cuts: [...s.cuts, { from, to, source: { kind: "manual" }, enabled: true }] }))}
        />
      )}

      {!props.words && props.lines.length > 0 && (
        <div className="space-y-1 border-t border-zinc-800 pt-3 text-sm text-zinc-300">
          {props.lines.map((l, i) => <p key={i} className="m-0">{l}</p>)}
        </div>
      )}
    </div>
  );
}

/** The whole take as a bar: kept time, trimmed and cut time, clicks, and the playhead. */
function Strip({ scene, take, here, cutStart, onSeek }: { scene: SceneEdit; take: TakeInfo; here: number | null; cutStart: number | null; onSeek: (src: number) => void }) {
  const d = take.duration || 1;
  const pct = (t: number) => `${(Math.min(Math.max(t, 0), d) / d) * 100}%`;
  const span = (from: number, to: number) => ({ left: pct(from), width: `${((Math.min(to, d) - Math.max(from, 0)) / d) * 100}%` });
  const end = scene.trimOut ?? d;
  return (
    <div
      className="relative h-10 cursor-pointer overflow-hidden rounded-md bg-emerald-900/60"
      onClick={(e) => {
        const r = e.currentTarget.getBoundingClientRect();
        onSeek(((e.clientX - r.left) / r.width) * d);
      }}
      title="Click to play from here"
    >
      <div className="absolute inset-y-0 bg-zinc-950/85" style={span(0, scene.trimIn)} />
      <div className="absolute inset-y-0 bg-zinc-950/85" style={span(end, d)} />
      {scene.cuts.map((c, i) => (
        <div key={i} className={`absolute inset-y-0 ${c.enabled ? "bg-red-700/80" : "border border-dashed border-red-500/70"}`} style={span(c.from, c.to)} />
      ))}
      {take.actions.map((a, i) => (
        <div key={i} className={`absolute inset-y-0 ${a.kind === "press" ? "bg-sky-400/90" : "bg-sky-400/30"}`} style={{ ...span(a.start, a.end), minWidth: 2 }} title={`${a.kind === "press" ? "Click" : "Cursor to"} ${a.label}`} />
      ))}
      {cutStart !== null && <div className="absolute inset-y-0 w-0.5 bg-red-400" style={{ left: pct(cutStart) }} />}
      {here !== null && <div className="absolute inset-y-0 w-0.5 bg-white" style={{ left: pct(here) }} />}
    </div>
  );
}

/** Asked once, before any narration leaves this computer. */
function ConsentDialog({ onAnswer }: { onAnswer: (allow: boolean, remember: boolean) => void }) {
  return (
    <div className="fixed inset-0 z-10 flex items-center justify-center bg-black/60 p-6">
      <div className="max-w-md space-y-3 rounded-xl bg-zinc-900 p-6 shadow-xl ring-1 ring-zinc-700">
        <h2 className="m-0 text-lg font-semibold">Send your narration to Gemini?</h2>
        <p className="m-0 text-sm text-zinc-300">
          To pick the best takes, cut stumbles and write captions, Scenra sends the microphone
          recording of each take to Google Gemini to transcribe it. Only the audio is sent, once per
          take. The video stays on this computer.
        </p>
        <p className="m-0 text-xs text-zinc-400">Without this, Tidy up still trims silence and shortens pauses on this computer.</p>
        <div className="flex flex-wrap justify-end gap-2 pt-2">
          <button type="button" onClick={() => onAnswer(false, true)} className="rounded-md px-3 py-1.5 text-sm text-zinc-400 hover:bg-zinc-800">
            Never
          </button>
          <button type="button" onClick={() => onAnswer(false, false)} className="rounded-md bg-zinc-800 px-3 py-1.5 text-sm hover:bg-zinc-700">
            Not now
          </button>
          <button type="button" onClick={() => onAnswer(true, true)} className="rounded-md bg-violet-700 px-4 py-1.5 text-sm font-medium hover:bg-violet-600">
            Allow and assemble
          </button>
        </div>
      </div>
    </div>
  );
}

/** Title, description and chapters, each with a copy button. */
function DescriptionPanel({ value, onClose }: { value: Description; onClose: () => void }) {
  const [copied, setCopied] = useState<string | null>(null);
  const copy = (label: string, text: string) => void navigator.clipboard.writeText(text).then(() => setCopied(label));
  const field = (label: string, text: string, rows: number) => (
    <label className="block space-y-1">
      <span className="flex items-center gap-3 text-xs font-bold tracking-wide text-zinc-400">
        {label.toUpperCase()}
        <button type="button" onClick={() => copy(label, text)} className="font-normal text-indigo-400 hover:underline">
          {copied === label ? "Copied" : "Copy"}
        </button>
      </span>
      <textarea readOnly rows={rows} value={text} className="w-full resize-y rounded-md bg-zinc-950 p-2 text-sm text-zinc-100" />
    </label>
  );
  return (
    <div className="space-y-3 border-b border-zinc-800 bg-zinc-900 px-4 py-3">
      <div className="flex items-center">
        <p className="m-0 flex-1 font-medium">For YouTube</p>
        <button type="button" onClick={onClose} className="text-xs text-zinc-400 hover:underline">Close</button>
      </div>
      {field("Title", value.title, 1)}
      {field("Description", value.description, 5)}
      {field("Chapters", value.chapters, Math.min(8, value.chapters.split("\n").length + 1))}
      {value.chapterNote && <p className="m-0 text-xs text-amber-300">{value.chapterNote}</p>}
      <p className="m-0 text-xs text-zinc-500">Also saved as export\description.txt in the session folder.</p>
    </div>
  );
}
