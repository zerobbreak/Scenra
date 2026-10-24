import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { onPrompter, sendPrompter } from "./prompter";
import { NextAction, Position, SPEEDS, Speed, useReader } from "./reader";
import { TakeResult } from "./engine";
import { resizeTeleprompter } from "./capture";
import { Line, Scenario, Session, currentScript, currentSession, onScriptChanged, onSessionChanged, words } from "./script";

// Compact pill. One line while recording (design C2, CompactAction.dc.html); the control bar's
// "Show full prompter" expands it to peek ahead (design C3, CompactPeek.dc.html).
// The script comes from `draft_script` (Gemini); the reader moves through it word by word.
// In a recording session each scene is its own take, started from the pill (`ReadyToRecord`).

// The magenta strip is the Phase 0 marker `probe_capture` looks for. Keep it strongly magenta
// and above 1% of the window's area (see capture.rs). Remove it once capture exclusion is proven.
const MARKER = "#FF00FF";

// Logical px. The collapsed size must match PILL_SIZE in capture.rs.

const UI_FONT = { fontFamily: "'Plus Jakarta Sans', system-ui, sans-serif" };
const SCRIPT_FONT = { fontFamily: "Literata, Georgia, serif" };

type Reader = ReturnType<typeof useReader>;

function format(seconds: number) {
  return `${String(Math.floor(seconds / 60)).padStart(2, "0")}:${String(seconds % 60).padStart(2, "0")}`;
}

function CursorIcon({ color, size = 9 }: { color: string; size?: number }) {
  return (
    <svg width={size} height={(size * 22) / 18} viewBox="0 0 18 22" aria-hidden="true">
      <path d="M2 2l13 9-6 1.5L6.5 19z" fill={color} />
    </svg>
  );
}

function Timer({ elapsed, paused, countdown, rec }: { elapsed: number; paused: boolean; countdown: number; rec?: boolean }) {
  if (countdown > 0) {
    return (
      <span className="flex flex-none items-center gap-1.5 font-mono text-xs font-bold text-[#A5B4FC]">
        <span className="size-2 rounded-full bg-[#4F46E5]" />
        STARTS IN {countdown}
      </span>
    );
  }
  return paused ? (
    <span className="flex flex-none items-center gap-1.5 font-mono text-xs font-bold text-[#FDE68A]">
      <span className="size-2 rounded-full bg-[#F59E0B]" />
      PAUSED {format(elapsed)}
    </span>
  ) : (
    <span className="flex flex-none items-center gap-1.5 font-mono text-xs text-[#FCA5A5]">
      <span className="size-2 rounded-full bg-[#F04438]" />
      {rec && <span className="font-bold">REC</span>}
      {format(elapsed)}
    </span>
  );
}

function distance(next: NextAction) {
  if (next.wordsAway <= 0) return "now";
  return next.wordsAway === 1 ? "next word" : `in ${next.wordsAway} words`;
}

/// One line of script. Words before `spoken` are greyed; trigger words are underlined.
function LineWords({
  line,
  spoken,
  scene,
  lineIndex,
  reader,
  follow,
}: {
  line: Line;
  spoken: number;
  scene: number;
  lineIndex: number;
  reader: Reader;
  follow?: boolean;
}) {
  const current = useRef<HTMLSpanElement>(null);
  useLayoutEffect(() => {
    if (follow) current.current?.scrollIntoView({ inline: "center", block: "nearest", behavior: "smooth" });
  }, [follow, spoken]);

  return (
    <>
      {words(line).map((word, i) => {
        const a = line.actions.findIndex((act) => act.word === i);
        const state = a >= 0 ? reader.actionState(scene, lineIndex, a) : undefined;
        const underline =
          a < 0 ? "" : state === "done" ? "border-b-[3px] border-[#34D399]" : state === "failed" ? "border-b-[3px] border-[#F59E0B]" : "border-b-[3px] border-[#818CF8]";
        return (
          <span key={i}>
            <span
              ref={i === Math.max(0, spoken - 1) ? current : undefined}
              className={`${i < spoken ? "text-[#8D8F96]" : "text-white"} ${underline} pb-0.5 transition-colors duration-150`}
            >
              {word}
            </span>{" "}
          </span>
        );
      })}
    </>
  );
}

function ActionChip({ next }: { next: NextAction | null }) {
  if (!next) return null;
  return (
    <span className="inline-flex flex-none items-center gap-2 rounded-full bg-[#4F46E5] py-[5px] pr-3 pl-[5px] text-xs font-bold text-white shadow-[0_0_0_5px_rgba(79,70,229,0.25)]">
      <span className="inline-flex size-5 items-center justify-center rounded-full bg-white">
        <CursorIcon color="#4F46E5" />
      </span>
      {next.action.label}
      <span className="rounded-md bg-white/20 px-1.5 font-semibold">{distance(next)}</span>
    </span>
  );
}

function SceneProgress({ script, pos, reader }: { script: Scenario; pos: Position; reader: Reader }) {
  const scene = script.scenes[pos.scene];
  const nextLabel = reader.next ? reader.next.action.label : script.scenes[pos.scene + 1]?.title;
  return (
    <div role="img" aria-label={`Scene ${pos.scene + 1} of ${script.scenes.length}, line ${pos.line + 1} of ${scene.lines.length}`} className="mr-[54px] flex items-center gap-2.5">
      <span className="flex-none text-[11px] font-bold text-[#8D8F96]">
        SCENE {pos.scene + 1}/{script.scenes.length}
      </span>
      <div className="flex h-1 flex-1 gap-[3px]">
        {scene.lines.map((line, l) => {
          const n = words(line).length;
          const done = l < pos.line ? 1 : l > pos.line ? 0 : pos.word / n;
          return (
            <span key={l} style={{ flex: n }} className="relative rounded-sm bg-[#23262C]">
              <span style={{ width: `${done * 100}%` }} className="absolute inset-y-0 left-0 rounded-sm bg-[#34D399] transition-[width] duration-300" />
              {line.actions.map((action, i) => (
                <span
                  key={i}
                  style={{ left: `calc(${((action.word + 0.5) / n) * 100}% - 5px)` }}
                  className={`absolute -top-[3px] size-2.5 rounded-full border-2 border-[#0E1013] ${reader.actionState(pos.scene, l, i) ? "bg-[#818CF8]" : "bg-[#3B3A6B]"}`}
                />
              ))}
            </span>
          );
        })}
      </div>
      {nextLabel && <span className="max-w-[220px] flex-none truncate text-[11px] text-[#8D8F96]">Next: {nextLabel}</span>}
    </div>
  );
}

function DoItNow({ reader }: { reader: Reader }) {
  return (
    <button
      type="button"
      aria-label="Do it now (Ctrl Alt Space)"
      title="Do it now (Ctrl Alt Space)"
      disabled={!reader.next}
      onClick={reader.doNext}
      className="flex size-10 flex-none cursor-pointer items-center justify-center rounded-full border-none bg-[#33363D] hover:bg-[#40444C] disabled:cursor-default disabled:opacity-40"
    >
      <svg width="16" height="16" viewBox="0 0 24 24" aria-hidden="true">
        <path d="M6 5l9 7-9 7zM18 5v14" fill="none" stroke="#FFFFFF" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" />
      </svg>
    </button>
  );
}

function Message({ children }: { children: React.ReactNode }) {
  return <p className="m-0 flex flex-1 items-center px-6 text-sm text-[#C9C7C1]">{children}</p>;
}

function seconds(take: { microphoneSeconds: number }) {
  return format(Math.round(take.microphoneSeconds));
}

/// Above this, a take looks visibly choppy; worth a retake once the machine has room.
const DROPPED_FRAMES_LIMIT = 5;

function choppy(take: TakeResult) {
  if (take.droppedFramesPercent <= DROPPED_FRAMES_LIMIT) return null;
  const memory = take.freeMemoryMb < 1024 ? ` with ${take.freeMemoryMb} MB of RAM free; close other apps and` : ";";
  return `OBS dropped ${Math.round(take.droppedFramesPercent)}% of frames${memory} retake it`;
}

/// Recording, between scenes: what was just saved, and the button that records the next scene.
function ReadyToRecord({ script, reader }: { script: Scenario; reader: Reader }) {
  const { pos } = reader;
  const last = reader.takes[reader.takes.length - 1];
  return (
    <div className="flex flex-1 items-center gap-4 py-3 pr-3 pl-6">
      <div className="min-w-0 flex-1">
        <p className="m-0 truncate text-[11px] font-bold tracking-wide text-[#8D8F96]">
          SCENE {pos.scene + 1}/{script.scenes.length} · READY TO RECORD
          {last && ` · scene ${last.scene + 1} saved (${seconds(last.take)})`}
        </p>
        <p style={SCRIPT_FONT} className="m-0 truncate text-xl font-semibold">{script.scenes[pos.scene].title}</p>
        {(reader.lastError || reader.memoryWarning || reader.setupNote || (last && choppy(last.take))) && (
          <p title={reader.lastError ?? reader.memoryWarning ?? reader.setupNote ?? (last ? choppy(last.take) ?? undefined : undefined)} className="m-0 truncate text-[11px] font-semibold text-[#FDE68A]">
            {reader.lastError ?? reader.memoryWarning ?? reader.setupNote ?? `Scene ${last!.scene + 1}: ${choppy(last!.take)}`}
          </p>
        )}
      </div>
      {reader.needsSetup && (
        <>
        <button type="button" title="Reload the starting page. Unsaved page changes may be lost."
          onClick={() => reader.setupPage()}
          className="flex-none cursor-pointer rounded-full border-none bg-[#33363D] px-4 py-2.5 text-sm font-bold text-white hover:bg-[#40444C]">
          Reset page
        </button>
        {pos.scene > 0 && <button type="button" title="Restore the first scene's page and replay earlier approved clicks without recording. This changes the page."
          onClick={() => reader.setupPage("replay")}
          className="flex-none cursor-pointer rounded-full border-none bg-[#33363D] px-3 py-2.5 text-xs font-bold text-white hover:bg-[#40444C]">
          Replay setup
        </button>}
        <button type="button" title="Save the page you prepared as this scene's starting state."
          onClick={() => reader.setupPage("approve")}
          className="flex-none cursor-pointer rounded-full border-none bg-[#33363D] px-3 py-2.5 text-xs font-bold text-white hover:bg-[#40444C]">
          Use current page
        </button>
        </>
      )}
      {reader.memoryWarning && (
        <button type="button" onClick={() => reader.startScene()}
          className="flex-none cursor-pointer rounded-full border-none bg-[#33363D] px-3 py-2.5 text-sm font-bold text-white">Check again</button>
      )}
      {last && choppy(last.take) && (
        <button
          type="button"
          onClick={() => reader.startScene(last.scene)}
          className="flex-none cursor-pointer rounded-full border-none bg-[#33363D] px-4 py-2.5 text-sm font-bold text-white hover:bg-[#40444C]"
        >
          Retake scene {last.scene + 1}
        </button>
      )}
      <button
        type="button"
        onClick={() => reader.startScene(undefined, !!reader.memoryWarning)}
        className="flex flex-none cursor-pointer items-center gap-2 rounded-full border-none bg-[#DC2626] px-4 py-2.5 text-sm font-bold text-white hover:bg-[#E23B3B]"
      >
        <span className="size-2.5 rounded-full bg-white" />
        {reader.memoryWarning ? "Record anyway" : `Record scene ${pos.scene + 1}`}
      </button>
    </div>
  );
}

function OneLine({ script, reader, elapsed, paused }: { script: Scenario; reader: Reader; elapsed: number; paused: boolean }) {
  const { pos } = reader;
  const line = script.scenes[pos.scene].lines[pos.line];
  return (
    <div className="flex flex-1 flex-col justify-center gap-2.5 py-3 pr-3 pl-[18px]">
      <div className="flex items-center gap-3.5">
        <Timer elapsed={elapsed} paused={paused || reader.halted} countdown={reader.countdown} rec={reader.record} />
        <p style={SCRIPT_FONT} className="m-0 min-w-0 flex-1 overflow-hidden py-1 text-xl leading-snug font-semibold whitespace-nowrap">
          <LineWords line={line} spoken={pos.word} scene={pos.scene} lineIndex={pos.line} reader={reader} follow />
        </p>
        <ActionChip next={reader.next} />
        <DoItNow reader={reader} />
      </div>
      {reader.lastError ? (
        <p title={reader.lastError} className="m-0 mr-[54px] truncate text-[11px] font-semibold text-[#FDE68A]">{reader.lastError}</p>
      ) : (
        <SceneProgress script={script} pos={pos} reader={reader} />
      )}
    </div>
  );
}

function Peek({
  script,
  reader,
  elapsed,
  paused,
  speed,
  setSpeed,
  onCollapse,
}: {
  script: Scenario;
  reader: Reader;
  elapsed: number;
  paused: boolean;
  speed: Speed;
  setSpeed: (s: Speed) => void;
  onCollapse: () => void;
}) {
  const { pos } = reader;
  const scene = script.scenes[pos.scene];
  const shown = [pos.line - 1, pos.line, pos.line + 1].filter((l) => l >= 0 && l < scene.lines.length);
  const left = scene.lines
    .flatMap((line, l) => line.actions.map((_, i) => reader.actionState(pos.scene, l, i)))
    .filter((s) => !s).length;
  const sceneWords = script.scenes.map((s) => s.lines.reduce((n, line) => n + words(line).length, 0));
  const doneInScene =
    scene.lines.slice(0, pos.line).reduce((n, line) => n + words(line).length, 0) + pos.word;

  return (
    <div className="flex flex-1 flex-col gap-3.5 pt-4 pr-4 pb-3.5 pl-5">
      <div className="flex items-center gap-3">
        <Timer elapsed={elapsed} paused={paused || reader.halted} countdown={reader.countdown} rec={reader.record} />
        <span className="truncate text-xs font-bold text-[#C9C7C1]">
          Scene {pos.scene + 1} of {script.scenes.length} · {scene.title}
        </span>
        <button
          type="button"
          aria-label="Collapse to one line"
          title="Collapse to one line"
          onClick={onCollapse}
          className="ml-auto flex size-9 flex-none cursor-pointer items-center justify-center rounded-full border-none bg-[#23262C] hover:bg-[#33363D]"
        >
          <svg width="14" height="14" viewBox="0 0 16 16" aria-hidden="true">
            <path d="M3.5 10L8 5.5l4.5 4.5" fill="none" stroke="#FFFFFF" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        </button>
      </div>

      <ol style={SCRIPT_FONT} className="m-0 flex min-h-0 flex-1 list-none flex-col gap-3 overflow-hidden p-0">
        {shown.map((l) => {
          const line = scene.lines[l];
          const current = l === pos.line;
          return (
            <li
              key={l}
              className={`flex items-baseline gap-3.5 ${current ? "-ml-1.5 rounded-[14px] bg-[#17191D] py-3 pr-3.5 pl-1.5" : ""}`}
            >
              <span className={`w-[22px] flex-none font-mono text-[11px] ${current ? "text-[#A5B4FC]" : "text-[#8D8F96]"}`}>{l + 1}</span>
              <span className={current ? "text-[21px] leading-[1.45] font-semibold" : `text-base leading-normal ${l < pos.line ? "text-[#8D8F96]" : "text-[#C9C7C1]"}`}>
                <LineWords line={line} spoken={l < pos.line ? Infinity : current ? pos.word : 0} scene={pos.scene} lineIndex={l} reader={reader} />
                {line.actions.map((action, i) => {
                  const state = reader.actionState(pos.scene, l, i);
                  return state ? (
                    <span key={i} style={UI_FONT} className={`mr-2 text-[11px] font-bold ${state === "done" ? "text-[#6EE7B7]" : "text-[#FDE68A]"}`}>
                      {state === "done" ? "✓" : "!"} {action.label.replace(/^Click /, state === "done" ? "clicked " : "couldn't click ")}
                    </span>
                  ) : (
                    <span
                      key={i}
                      style={UI_FONT}
                      className="mr-2 inline-flex items-center gap-1.5 rounded-full border-[1.5px] border-[#6366F1] py-0.5 pr-2.5 pl-1.5 align-middle text-[11px] font-bold text-[#C7D2FE]"
                    >
                      <CursorIcon color="#A5B4FC" size={8} />
                      {action.label} · on “{words(line)[action.word]}”
                    </span>
                  );
                })}
              </span>
            </li>
          );
        })}
      </ol>

      <div className="mt-auto flex items-center gap-2.5 border-t border-[#1F2227] pt-2.5">
        <div role="img" aria-label={`Scene ${pos.scene + 1} of ${script.scenes.length} in progress`} className="flex h-1.5 flex-1 gap-1">
          {sceneWords.map((n, s) => {
            const done = s < pos.scene ? 1 : s > pos.scene ? 0 : doneInScene / n;
            return (
              <span key={s} style={{ flex: n }} className="relative overflow-hidden rounded-[3px] bg-[#23262C]">
                <span style={{ width: `${done * 100}%` }} className="absolute inset-y-0 left-0 bg-[#34D399]" />
              </span>
            );
          })}
        </div>
        <span className="text-xs text-[#8D8F96]">
          {left === 0 ? "No actions left in this scene" : `${left} action${left === 1 ? "" : "s"} left in this scene`}
        </span>
        <span className="flex gap-0.5 rounded-[9px] bg-[#1A1C21] p-[3px]">
          {(Object.keys(SPEEDS) as Speed[]).map((s) => (
            <button
              key={s}
              type="button"
              aria-pressed={speed === s}
              onClick={() => setSpeed(s)}
              className={`cursor-pointer rounded-[7px] border-none px-[9px] py-1 text-[11px] ${
                speed === s ? "bg-[#F2EFE8] font-bold text-[#0E1013]" : "bg-transparent font-semibold text-[#C9C7C1]"
              }`}
            >
              {s}
            </button>
          ))}
        </span>
      </div>
    </div>
  );
}

export default function Teleprompter() {
  const [script, setScript] = useState<Scenario | null>(null);
  const [session, setSession] = useState<Session | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const [paused, setPaused] = useState(false);
  const [peek, setPeek] = useState(false);
  const [speed, setSpeed] = useState<Speed>("Natural");
  const reader = useReader(script, session, paused, speed);

  useEffect(() => {
    Promise.all([currentScript(), currentSession()]).then(([s, started]) => {
      setScript(s);
      setSession(started);
      setLoaded(true);
    });
    const unlisten = onScriptChanged((s) => {
      setScript(s);
      setElapsed(0);
    });
    const unlistenSession = onSessionChanged((started) => {
      setSession(started);
      setElapsed(0);
    });
    return () => {
      unlisten.then((stop) => stop());
      unlistenSession.then((stop) => stop());
    };
  }, []);

  // In a recording the timer shows the current scene's take.
  useEffect(() => {
    if (reader.phase === "starting") setElapsed(0);
  }, [reader.phase]);

  const running = !!script && !reader.finished && reader.phase === "reading" && reader.countdown === 0 && !paused && !reader.halted;
  useEffect(() => {
    if (!running) return;
    const id = setInterval(() => setElapsed((s) => s + 1), 1000);
    return () => clearInterval(id);
  }, [running]);

  // Subscribe once; the ref always points at the latest reader.
  const restartScene = useRef(reader.restartScene);
  restartScene.current = reader.restartScene;
  useEffect(() => {
    const unlisten = onPrompter((event) => {
      if (event.kind === "pause") setPaused(event.paused);
      if (event.kind === "retry") restartScene.current();
      if (event.kind === "peek") setPeek(event.open);
    });
    return () => {
      unlisten.then((stop) => stop());
    };
  }, []);

  useEffect(() => {
    const tall = peek && script && !reader.finished && reader.phase === "reading";
    resizeTeleprompter(!!tall).catch(console.error);
  }, [peek, script, reader.finished, reader.phase]);

  // The window is frameless, so dragging anywhere except a button moves it.
  function drag(e: React.MouseEvent) {
    if (e.button === 0 && !(e.target as HTMLElement).closest("button")) {
      getCurrentWindow().startDragging();
    }
  }

  let body: React.ReactNode;
  if (!loaded) body = <Message>Loading script…</Message>;
  else if (!script) body = <Message>No script yet. In Scenra, launch the app and click “Draft script with Gemini”.</Message>;
  else if (reader.finished && reader.record)
    body = (
      <div className="flex flex-1 items-center gap-4 px-6">
        <div className="min-w-0 flex-1">
          <p className="m-0 text-sm">All {script.scenes.length} scenes recorded - {reader.takes.length} takes</p>
          {reader.takes.length > 0 && choppy(reader.takes[reader.takes.length - 1].take) && (
            <p title={choppy(reader.takes[reader.takes.length - 1].take)!} className="m-0 truncate text-xs text-[#FDE68A]">
              {choppy(reader.takes[reader.takes.length - 1].take)}
            </p>
          )}
          <p className="m-0 truncate text-xs text-[#8D8F96]">recordings/takes/{session?.id}</p>
        </div>
        <button type="button" onClick={() => reader.restartScene()}
          className="flex-none cursor-pointer rounded-full border-none bg-[#33363D] px-4 py-2.5 text-sm font-bold text-white hover:bg-[#40444C]">
          Retake scene {script.scenes.length}
        </button>
      </div>
    );
  else if (reader.finished) body = <Message>That’s the whole script · {format(elapsed)} · press Retry to read the last scene again.</Message>;
  else if (reader.phase === "ready") body = <ReadyToRecord script={script} reader={reader} />;
  else if (reader.phase === "starting")
    body = <Message>Scene {reader.pos.scene + 1}: checking or restoring the starting page…</Message>;
  else if (reader.phase === "saving") body = <Message>Saving scene {reader.pos.scene + 1}…</Message>;
  else if (peek)
    body = (
      <Peek
        script={script}
        reader={reader}
        elapsed={elapsed}
        paused={paused}
        speed={speed}
        setSpeed={setSpeed}
        onCollapse={() => {
          setPeek(false);
          sendPrompter({ kind: "peek", open: false });
        }}
      />
    );
  else body = <OneLine script={script} reader={reader} elapsed={elapsed} paused={paused || reader.halted} />;

  return (
    <section
      aria-label={peek ? "Script, peeking ahead (only you see this)" : "Script line (only you see this)"}
      onMouseDown={drag}
      style={UI_FONT}
      className="flex h-full cursor-grab select-none flex-col bg-[#0E1013] text-[#F2EFE8] active:cursor-grabbing"
    >
      {body}
      <div aria-hidden="true" title="Phase 0 capture marker" style={{ background: MARKER }} className="h-1 flex-none" />
    </section>
  );
}
