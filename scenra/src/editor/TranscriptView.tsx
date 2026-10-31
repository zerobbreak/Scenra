import { useEffect, useMemo, useState } from "react";
import { SceneEdit, Word, bare, onScript } from "./editor";

/**
 * What was said in the take, word by word. Click a word to play from it; click another with
 * Shift to select a range, then Cut it. Words already cut or trimmed off are struck through,
 * words that aren't in the script are highlighted.
 */
export default function TranscriptView(props: {
  words: Word[];
  scene: SceneEdit;
  script: string[];
  here: number | null;
  onSeek: (src: number) => void;
  onCut: (from: number, to: number) => void;
}) {
  const { words, scene, here } = props;
  const [anchor, setAnchor] = useState<number | null>(null);
  const [focus, setFocus] = useState<number | null>(null);
  useEffect(() => { setAnchor(null); setFocus(null); }, [scene.take]);

  const matched = useMemo(() => {
    if (!props.script.length) return null;
    const said = words.map((w) => (w.filler ? "\u0000" : bare(w.text)));
    return onScript(props.script.map(bare).filter(Boolean), said);
  }, [words, props.script]);

  const end = scene.trimOut ?? Infinity;
  const removed = (w: Word) =>
    w.end <= scene.trimIn + 0.02 || w.start >= end - 0.02 || scene.cuts.some((c) => c.enabled && w.start >= c.from - 0.02 && w.end <= c.to + 0.02);
  const [lo, hi] = anchor === null ? [null, null] : [Math.min(anchor, focus ?? anchor), Math.max(anchor, focus ?? anchor)];

  const cutSelection = () => {
    if (lo === null || hi === null) return;
    props.onCut(Math.max(0, words[lo].start - 0.02), words[hi].end + 0.02);
    setAnchor(null);
    setFocus(null);
  };

  return (
    <div className="space-y-2 border-t border-zinc-800 pt-3">
      <div className="flex items-center gap-3">
        <p className="m-0 text-xs font-bold tracking-wide text-zinc-400">WHAT YOU SAID</p>
        <span className="flex-1 text-xs text-zinc-500">
          Click a word to play from it · Shift+click to select · <span className="text-amber-300">highlighted</span> = not in the script
        </span>
        {lo !== null && (
          <>
            <button type="button" onClick={cutSelection} className="rounded-md bg-red-700 px-3 py-1 text-sm hover:bg-red-600">
              Cut {hi! - lo + 1} {hi === lo ? "word" : "words"}
            </button>
            <button type="button" onClick={() => { setAnchor(null); setFocus(null); }} className="text-xs text-zinc-400 hover:underline">
              Clear
            </button>
          </>
        )}
      </div>
      <p className="m-0 flex flex-wrap gap-x-1 gap-y-0.5 text-[15px] leading-relaxed">
        {words.map((w, i) => {
          const playing = here !== null && here >= w.start && here < w.end;
          const selected = lo !== null && i >= lo && i <= hi!;
          const offScript = matched && !matched[i] && !w.filler;
          return (
            <span
              key={i}
              role="button"
              tabIndex={0}
              title={`${w.start.toFixed(2)} s${w.filler ? " · stumble" : ""}`}
              onClick={(e) => {
                if (e.shiftKey && anchor !== null) setFocus(i);
                else { setAnchor(i); setFocus(i); props.onSeek(Math.max(0, w.start - 0.05)); }
              }}
              onKeyDown={(e) => { if (e.key === "Delete" || e.key === "Backspace") cutSelection(); }}
              className={[
                "cursor-pointer rounded px-0.5",
                removed(w) ? "text-zinc-600 line-through" : w.filler ? "text-red-300" : offScript ? "text-amber-300" : "",
                selected ? "bg-indigo-700/70" : playing ? "bg-zinc-700" : "hover:bg-zinc-800",
              ].join(" ")}
            >
              {w.text}
            </span>
          );
        })}
        {!words.length && <span className="text-sm text-zinc-500">No speech in this take.</span>}
      </p>
    </div>
  );
}
