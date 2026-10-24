import { useEffect, useRef, useState } from "react";
import { TakeResult, startTake, stopTake, takePreflight } from "./engine";
import { Action, Scenario, Session, approachAction, pressAction, checkSceneStart, resetScenePage, approveSceneStart, prepareScene, playbackGuard, playbackStatus, words } from "./script";

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
  const [halted, setHalted] = useState(false);
  const generation = useRef(0);
  const guardReady = useRef<Promise<void>>(Promise.resolve());
  const starting = useRef(false);
  const [needsSetup, setNeedsSetup] = useState(false);
  const [memoryWarning, setMemoryWarning] = useState<string | null>(null);
  const [setupNote, setSetupNote] = useState<string | null>(null);
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
    setMemoryWarning(null);
    setSetupNote(null);
    setHalted(false);
    setNeedsSetup(false);
    fired.current.clear();
    approached.current.clear();
    attempts.current = {};
  }, [script, session, record]);

  const finished = !script || pos.scene >= script.scenes.length;
  const reading = phase === "reading" && !finished && !halted;

  useEffect(() => {
    const active = reading && !paused && countdown <= 1;
    generation.current++;
    guardReady.current = playbackGuard(active);
    guardReady.current.catch((e) => { setHalted(true); setLastError(`Input guard unavailable: ${e}`); });
    if (!active) return;
    let cancelled = false;
    const id = setInterval(async () => {
      try {
        const status = await playbackStatus();
        if (cancelled) return;
        if (status.stopped || status.interference) {
          setHalted(true);
          setLastError("Playback interrupted by mouse or keyboard input. Retry this scene.");
        }
      } catch (e) { if (!cancelled) { setHalted(true); setLastError(`Input guard failed: ${e}`); } }
    }, 100);
    return () => { cancelled = true; clearInterval(id); void playbackGuard(false); };
  }, [reading, paused, countdown > 1]);
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
    const due = performance.now() + delay;
    const id = setTimeout(() => {
      if (performance.now() - due > 1000) {
        setHalted(true); setLastError("Playback stalled. Retry the scene to keep clicks on their cues."); return;
      }
      if (record && sceneDone) void endScene(pos.scene); else setPos((p) => advance(script, p));
    }, delay);
    return () => clearTimeout(id);
  }, [script, pos, reading, paused, speed, countdown, record]);

  // Start gliding towards the next action once it's within the lead time.
  const leadWords = Math.ceil(LEAD_MS / (60000 / SPEEDS[speed]));
  useEffect(() => {
    if (paused || !reading || countdown > 1 || !next || next.wordsAway >= leadWords) return;
    approach(key(next.scene, next.line, next.index), next.action);
  }, [paused, reading, countdown, next?.scene, next?.line, next?.index, next?.wordsAway, leadWords]);

  // Press as the trigger word starts (the chip's "now"): the press re-locates and holds the
  // button for a few hundred ms, so the click lands while the word is being said.
  useEffect(() => {
    if (paused || !script || !reading || countdown > 0) return;
    script.scenes[pos.scene].lines[pos.line].actions.forEach((action, i) => {
      const k = key(pos.scene, pos.line, i);
      if (action.word <= pos.word && !fired.current.has(k)) fire(k, action);
    });
  }, [script, pos, reading, countdown, paused]);

  function enqueue(job: () => Promise<void>) {
    const epoch = generation.current;
    const run = queue.current.then(async () => {
      await guardReady.current;
      if (generation.current !== epoch) throw new Error("Playback was paused or restarted");
      await job();
    });
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
    const cue = performance.now();
    const expiresAt = Date.now() + 500;
    const ready = approach(k, action);
    enqueue(async () => {
      await ready;
      if (performance.now() - cue > 500) throw new Error("Action missed its cue. Retry the scene.");
      await pressAction(action.target, expiresAt);
    })
      .then(() => setActions((a) => ({ ...a, [k]: "done" })))
      .catch((e) => {
        setHalted(true);
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
  async function startScene(scene = pos.scene, allowLowMemory = false) {
    if (!session || !script || scene >= script.scenes.length || starting.current) return;
    starting.current = true;
    setHalted(false);
    setPhase("starting");
    setPos({ scene, line: 0, word: 0 });
    setLastError(null);
    setSetupNote(null);
    forgetScene(scene);
    let checked = false;
    try {
      await checkSceneStart(scene);
      checked = true;
      setNeedsSetup(false);
      const preflight = await takePreflight();
      setMemoryWarning(preflight.warning);
      if (preflight.warning && !allowLowMemory) { setPhase("ready"); return; }
      setMemoryWarning(null);
      const attempt = (attempts.current[scene] ?? 0) + 1;
      attempts.current[scene] = attempt;
      await startTake(`${session.id}/scene-${pad(scene + 1)}-take-${attempt}`);
      setPos({ scene, line: 0, word: 0 });
      setCountdown(COUNTDOWN);
      setPhase("reading");
    } catch (e) {
      setLastError(`Couldn’t start recording: ${e}`);
      setNeedsSetup(!checked);
      setPhase("ready");
    } finally {
      starting.current = false;
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
    if (!next || paused || halted) return;
    setCountdown(0);
    setPos({ scene: next.scene, line: next.line, word: next.action.word + 1 });
  }

  async function restartScene() {
    if (!script) return;
    const scene = Math.min(pos.scene, script.scenes.length - 1);
    if (scene < 0) return;
    if (record && (finished || phase === "ready")) { await startScene(scene); return; }
    setHalted(false);
    if (!record) {
      forgetScene(scene);
      setPos({ scene, line: 0, word: 0 });
      setCountdown(COUNTDOWN);
      return;
    }
    // Recording: the attempt so far stays on disk as its own take; the scene starts a new one.
    if (phase !== "reading") return;
    setPhase("saving");
    await queue.current;
    let take: TakeResult;
    try { take = await stopTake(true); }
    catch (e) { setLastError(`Could not stop the take: ${e}. Use Stop to try again.`); setPhase("ready"); return; }
    setTakes((t) => [...t, { scene: pos.scene, attempt: attempts.current[pos.scene], take }]);
    await startScene(pos.scene);
  }

  return {
    halted,
    memoryWarning,
    needsSetup,
    setupNote,
    setupPage: async (mode: "reset" | "approve" | "replay" = "reset") => {
      if (!script || starting.current || finished || phase !== "ready") return;
      starting.current = true;
      setPhase("starting");
      setSetupNote(null);
      setLastError(null);
      try {
        let note = "Starting page checked. Record when you are ready.";
        if (mode === "approve") await approveSceneStart(pos.scene);
        else if (mode === "replay") {
          const result = await prepareScene(pos.scene, script);
          note = result.actionsRun === 0 ? "Starting page restored; no earlier clicks were needed." : `Replayed ${result.actionsRun} approved click${result.actionsRun === 1 ? "" : "s"}. Ready to record.`;
        } else await resetScenePage(pos.scene);
        await checkSceneStart(pos.scene);
        setNeedsSetup(false);
        setMemoryWarning(null);
        setSetupNote(note);
      } catch (e) {
        setNeedsSetup(true);
        setLastError(`Setup stopped. Prepare the page manually or retry: ${e}`);
      } finally { starting.current = false; setPhase("ready"); }
    },
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
    /// Records `scene` (default: the current one). After it, reading carries on from the scene after it.
    startScene: (scene?: number, allowLowMemory = false) => startScene(scene, allowLowMemory),
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
