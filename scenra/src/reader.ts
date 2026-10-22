import { useEffect, useRef, useState } from "react";
import { TakeResult, startTake, stopTake } from "./engine";
import { Action, Scenario, Session, approachAction, pressAction, words } from "./script";

// Reads the script word by word at a speaking pace and fires each action as narration reaches
// its trigger word. Until the narration state machine listens to the mic (PROJECT_OUTLINE §4),
// a clock stands in for the presenter's voice.
//
// Rehearse reads the whole script back to back after one countdown. Record makes each scene its
// own OBS take: the presenter starts each scene, and its take stops after the scene's last word.

export const SPEEDS = { Slow: 120, Natural: 150, Fast: 185 } as const;
export type Speed = keyof typeof SPEEDS;

export type Position = { scene: number; line: number; word: number };
export type ActionState = "done" | "failed";
/// `ready`: waiting for the presenter to record the scene. `starting`/`saving`: OBS is
/// starting or finishing the scene's take. Rehearsals are always `reading`.
export type Phase = "ready" | "starting" | "reading" | "saving";
export type SceneTake = { scene: number; attempt: number; take: TakeResult };

const COUNTDOWN = 3;
/// The approach (locate, glide of up to 1.1 s, hover check) starts about this long before the
/// trigger word, so only the press is left for the word itself.
const LEAD_MS = 1500;
/// After a scene's last word, before its take stops: lets the last click and ripple land on video.
const TAIL_MS = 800;

const key = (scene: number, line: number, index: number) => `${scene}:${line}:${index}`;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const pad = (n: number) => String(n).padStart(2, "0");

/// Extra pause after a word, in ms: a beat for commas, a breath for full stops.
function pauseAfter(word: string) {
  if (/[.!?]["”’)]*$/.test(word)) return 450;
  if (/[,;:—]$/.test(word)) return 180;
  return 0;
}

export function useReader(script: Scenario | null, session: Session | null, paused: boolean, speed: Speed) {
  const record = session?.mode === "record";
  const [pos, setPos] = useState<Position>({ scene: 0, line: 0, word: 0 });
  const [phase, setPhase] = useState<Phase>("reading");
  const [countdown, setCountdown] = useState(COUNTDOWN);
  const [actions, setActions] = useState<Record<string, ActionState>>({});
  const [lastError, setLastError] = useState<string | null>(null);
  const [takes, setTakes] = useState<SceneTake[]>([]);
  const fired = useRef(new Set<string>());
  /// Takes started per scene, failed ones included, so a retake never reuses a folder.
  const attempts = useRef<Record<number, number>>({});
  const approached = useRef(new Map<string, Promise<void>>());
  // Approaches and presses run one at a time, in order, so a quick second action can't start
  // gliding before the first has clicked.
  const queue = useRef<Promise<unknown>>(Promise.resolve());

  // A new script or session starts over: a rehearsal with the countdown, a recording at scene 1.
  useEffect(() => {
    setPos({ scene: 0, line: 0, word: 0 });
    setPhase(record ? "ready" : "reading");
    setCountdown(record ? 0 : COUNTDOWN);
    setActions({});
    setLastError(null);
    setTakes([]);
    fired.current.clear();
    approached.current.clear();
    attempts.current = {};
  }, [script, session, record]);

  const finished = !script || pos.scene >= script.scenes.length;
  const reading = phase === "reading" && !finished;
  const next = nextAction(script, pos, fired.current);

  useEffect(() => {
    if (!script || !reading || paused) return;
    if (countdown > 0) {
      const id = setTimeout(() => setCountdown((c) => c - 1), 1000);
      return () => clearTimeout(id);
    }
    const lines = script.scenes[pos.scene].lines;
    const lineWords = words(lines[pos.line]);
    const spoken = lineWords[pos.word - 1] ?? "";
    const delay = 60000 / SPEEDS[speed] + pauseAfter(spoken) + (pos.word === 0 ? 400 : 0);
    const sceneDone = pos.line === lines.length - 1 && pos.word >= lineWords.length;
    const id = setTimeout(() => (record && sceneDone ? endScene(pos.scene) : setPos((p) => advance(script, p))), delay);
    return () => clearTimeout(id);
  }, [script, pos, reading, paused, speed, countdown, record]);

  // Start gliding towards the next action once it's within the lead time.
  const leadWords = Math.ceil(LEAD_MS / (60000 / SPEEDS[speed]));
  useEffect(() => {
    if (!reading || countdown > 1 || !next || next.wordsAway >= leadWords) return;
    approach(key(next.scene, next.line, next.index), next.action);
  }, [reading, countdown, next?.scene, next?.line, next?.index, next?.wordsAway, leadWords]);

  // Press as the trigger word starts (the chip's "now"): the press re-locates and holds the
  // button for a few hundred ms, so the click lands while the word is being said.
  useEffect(() => {
    if (!script || !reading || countdown > 0) return;
    script.scenes[pos.scene].lines[pos.line].actions.forEach((action, i) => {
      const k = key(pos.scene, pos.line, i);
      if (action.word <= pos.word && !fired.current.has(k)) fire(k, action);
    });
  }, [script, pos, reading, countdown]);

  function enqueue(job: () => Promise<void>) {
    const run = queue.current.then(job);
    queue.current = run.catch(() => {});
    return run;
  }

  function approach(k: string, action: Action) {
    let ready = approached.current.get(k);
    if (!ready) {
      ready = enqueue(() => approachAction(action.target));
      ready.catch(() => {}); // reported when the press awaits it
      approached.current.set(k, ready);
    }
    return ready;
  }

  function fire(k: string, action: Action) {
    fired.current.add(k);
    const ready = approach(k, action);
    enqueue(async () => {
      await ready;
      await pressAction(action.target);
    })
      .then(() => setActions((a) => ({ ...a, [k]: "done" })))
      .catch((e) => {
        setActions((a) => ({ ...a, [k]: "failed" }));
        setLastError(`Couldn’t run ${action.label}: ${e}`);
      });
  }

  function forgetScene(scene: number) {
    for (const k of [...fired.current]) if (k.startsWith(`${scene}:`)) fired.current.delete(k);
    for (const k of [...approached.current.keys()]) if (k.startsWith(`${scene}:`)) approached.current.delete(k);
    setActions((a) => Object.fromEntries(Object.entries(a).filter(([k]) => !k.startsWith(`${scene}:`))));
  }

  /// Record mode: starts the scene's take (preflight, then OBS), then the countdown.
  async function startScene(scene = pos.scene) {
    if (!session || !script || scene >= script.scenes.length) return;
    setPhase("starting");
    setLastError(null);
    forgetScene(scene);
    const attempt = (attempts.current[scene] ?? 0) + 1;
    attempts.current[scene] = attempt;
    try {
      await startTake(`${session.id}/scene-${pad(scene + 1)}-take-${attempt}`);
      setPos({ scene, line: 0, word: 0 });
      setCountdown(COUNTDOWN);
      setPhase("reading");
    } catch (e) {
      setLastError(`Couldn’t start recording: ${e}`);
      setPhase("ready");
    }
  }

  /// Record mode: waits for the scene's clicks to land, then stops its take.
  async function endScene(scene: number) {
    setPhase("saving");
    await queue.current;
    await sleep(TAIL_MS);
    try {
      // Stay on Scenra's OBS profile until the last scene: switching resets OBS's video.
      const take = await stopTake(scene + 1 < (script?.scenes.length ?? 0));
      setTakes((t) => [...t, { scene, attempt: attempts.current[scene], take }]);
      setPos({ scene: scene + 1, line: 0, word: 0 });
    } catch (e) {
      // OBS may still be recording; Stop on the control bar tries again.
      setLastError(`Couldn’t save scene ${scene + 1}: ${e}`);
    }
    setPhase("ready");
  }

  /// "Do it now": jump narration to just past the next action and run it.
  function doNext() {
    if (!next) return;
    setCountdown(0);
    setPos({ scene: next.scene, line: next.line, word: next.action.word + 1 });
  }

  async function restartScene() {
    if (!script || finished) return;
    if (!record) {
      forgetScene(pos.scene);
      setPos({ scene: pos.scene, line: 0, word: 0 });
      return;
    }
    // Recording: the attempt so far stays on disk as its own take; the scene starts a new one.
    if (phase !== "reading") return;
    setPhase("saving");
    await queue.current;
    const take = await stopTake(true).catch(() => null);
    if (take) setTakes((t) => [...t, { scene: pos.scene, attempt: attempts.current[pos.scene], take }]);
    await startScene(pos.scene);
  }

  return {
    pos,
    phase,
    record,
    countdown,
    finished,
    takes,
    actionState: (scene: number, line: number, i: number) => actions[key(scene, line, i)],
    next,
    lastError,
    doNext,
    restartScene,
    startScene: () => startScene(),
  };
}

function advance(script: Scenario, p: Position): Position {
  const lines = script.scenes[p.scene].lines;
  if (p.word < words(lines[p.line]).length) return { ...p, word: p.word + 1 };
  if (p.line + 1 < lines.length) return { scene: p.scene, line: p.line + 1, word: 0 };
  return { scene: p.scene + 1, line: 0, word: 0 };
}

export type NextAction = {
  scene: number;
  line: number;
  index: number;
  action: Action;
  /// Words still to speak before it fires (0 = on the next word).
  wordsAway: number;
};

/// The next unfired action at or after the current position, within the current scene.
function nextAction(script: Scenario | null, pos: Position, fired: Set<string>): NextAction | null {
  if (!script || pos.scene >= script.scenes.length) return null;
  const lines = script.scenes[pos.scene].lines;
  let away = 0;
  for (let l = pos.line; l < lines.length; l++) {
    const start = l === pos.line ? pos.word : 0;
    for (const [i, action] of lines[l].actions.entries()) {
      if (action.word >= start && !fired.has(key(pos.scene, l, i))) {
        return { scene: pos.scene, line: l, index: i, action, wordsAway: away + action.word - start };
      }
    }
    away += words(lines[l]).length - start;
  }
  return null;
}
