import { useEffect, useRef, useState } from "react";
import { Action, Scenario, runAction, words } from "./script";

// Reads the script word by word at a speaking pace and fires each action as narration reaches
// its trigger word. Until the narration state machine listens to the mic (PROJECT_OUTLINE §4),
// a clock stands in for the presenter's voice.

export const SPEEDS = { Slow: 120, Natural: 150, Fast: 185 } as const;
export type Speed = keyof typeof SPEEDS;

export type Position = { scene: number; line: number; word: number };
export type ActionState = "done" | "failed";

const COUNTDOWN = 3;
const key = (scene: number, line: number, index: number) => `${scene}:${line}:${index}`;

/// Extra pause after a word, in ms: a beat for commas, a breath for full stops.
function pauseAfter(word: string) {
  if (/[.!?]["”’)]*$/.test(word)) return 450;
  if (/[,;:—]$/.test(word)) return 180;
  return 0;
}

export function useReader(script: Scenario | null, paused: boolean, speed: Speed) {
  const [pos, setPos] = useState<Position>({ scene: 0, line: 0, word: 0 });
  const [countdown, setCountdown] = useState(COUNTDOWN);
  const [actions, setActions] = useState<Record<string, ActionState>>({});
  const [lastError, setLastError] = useState<string | null>(null);
  const fired = useRef(new Set<string>());

  // A new script starts over, with the countdown.
  useEffect(() => {
    setPos({ scene: 0, line: 0, word: 0 });
    setCountdown(COUNTDOWN);
    setActions({});
    setLastError(null);
    fired.current.clear();
  }, [script]);

  const finished = !script || pos.scene >= script.scenes.length;

  useEffect(() => {
    if (!script || paused || finished) return;
    if (countdown > 0) {
      const id = setTimeout(() => setCountdown((c) => c - 1), 1000);
      return () => clearTimeout(id);
    }
    const line = script.scenes[pos.scene].lines[pos.line];
    const lineWords = words(line);
    const spoken = lineWords[pos.word - 1] ?? "";
    const delay = 60000 / SPEEDS[speed] + pauseAfter(spoken) + (pos.word === 0 ? 400 : 0);
    const id = setTimeout(() => setPos((p) => advance(script, p)), delay);
    return () => clearTimeout(id);
  }, [script, pos, paused, speed, countdown, finished]);

  // Fire actions whose trigger word has just been spoken.
  useEffect(() => {
    if (!script || finished || countdown > 0) return;
    script.scenes[pos.scene].lines[pos.line].actions.forEach((action, i) => {
      const k = key(pos.scene, pos.line, i);
      if (action.word < pos.word && !fired.current.has(k)) fire(k, action);
    });
  }, [script, pos, finished, countdown]);

  function fire(k: string, action: Action) {
    fired.current.add(k);
    runAction(action.target)
      .then(() => setActions((a) => ({ ...a, [k]: "done" })))
      .catch((e) => {
        setActions((a) => ({ ...a, [k]: "failed" }));
        setLastError(`${action.label}: ${e}`);
      });
  }

  /// "Do it now": jump narration to just past the next action and run it.
  function doNext() {
    const next = nextAction(script, pos, fired.current);
    if (!next) return;
    setCountdown(0);
    setPos({ scene: next.scene, line: next.line, word: next.action.word + 1 });
  }

  function restartScene() {
    if (!script || finished) return;
    script.scenes[pos.scene].lines.forEach((line, l) =>
      line.actions.forEach((_, i) => fired.current.delete(key(pos.scene, l, i))),
    );
    setActions((a) => Object.fromEntries(Object.entries(a).filter(([k]) => !k.startsWith(`${pos.scene}:`))));
    setPos({ scene: pos.scene, line: 0, word: 0 });
  }

  return {
    pos,
    countdown,
    finished,
    actionState: (scene: number, line: number, i: number) => actions[key(scene, line, i)],
    next: nextAction(script, pos, fired.current),
    lastError,
    doNext,
    restartScene,
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
