import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from "react";
import { Segment, fmt, mediaUrl } from "./editor";

/// Where playback is: in the output, and in the source take it comes from.
export type PlayPosition = { out: number; segment: number; take: string | null; src: number };

export type PlayerHandle = { seek: (out: number) => void; pause: () => void };

/// Re-sync the mic to the picture when they drift further apart than this (seconds).
const MAX_DRIFT = 0.04;

/**
 * Plays a list of segments back to back: the take's silent `video.mp4` with its
 * `microphone.wav`, seeking between segments. Preview of the whole edit, or of one raw take.
 */
const Player = forwardRef<PlayerHandle, { dir: string; segments: Segment[]; onPosition?: (p: PlayPosition) => void }>(
  function Player({ dir, segments, onPosition }, ref) {
    const video = useRef<HTMLVideoElement>(null);
    const audio = useRef<HTMLAudioElement>(null);
    const current = useRef(0);
    const [playing, setPlaying] = useState(false);
    const [out, setOut] = useState(0);
    const duration = segments.length ? segments[segments.length - 1].outAt + segments[segments.length - 1].srcOut - segments[segments.length - 1].srcIn : 0;

    const report = (i: number, src: number) => {
      const s = segments[i];
      if (!s) return;
      const o = s.outAt + (src - s.srcIn);
      setOut(o);
      onPosition?.({ out: o, segment: i, take: s.take, src });
    };

    /// Shows segment `i` at `src` seconds of its take; resolves once both elements are there.
    const load = async (i: number, src: number) => {
      const s = segments[i];
      const v = video.current, a = audio.current;
      if (!s || !v || !a) return;
      current.current = i;
      const vUrl = mediaUrl(dir, s.take, "video.mp4"), aUrl = mediaUrl(dir, s.take, "microphone.wav");
      const ready = (el: HTMLMediaElement, url: string) =>
        new Promise<void>((resolve) => {
          if (el.src === url && el.readyState >= 1) return resolve();
          el.addEventListener("loadedmetadata", () => resolve(), { once: true });
          el.src = url;
        });
      await Promise.all([ready(v, vUrl), ready(a, aUrl)]);
      v.currentTime = src;
      a.currentTime = src;
      report(i, src);
    };

    const seek = async (o: number) => {
      const found = segments.findIndex((s) => o < s.outAt + (s.srcOut - s.srcIn));
      const i = found === -1 ? segments.length - 1 : found;
      const s = segments[i];
      if (!s) return;
      const wasPlaying = playing;
      await load(i, Math.min(s.srcOut, s.srcIn + Math.max(0, o - s.outAt)));
      if (wasPlaying) void Promise.all([video.current?.play(), audio.current?.play()]);
    };

    const pause = () => {
      video.current?.pause();
      audio.current?.pause();
      setPlaying(false);
    };

    useImperativeHandle(ref, () => ({ seek: (o) => void seek(o), pause }));

    // New segments (the edit changed): stay near the same output time.
    useEffect(() => {
      pause();
      if (segments.length) void seek(Math.min(out, Math.max(0, duration - 0.05)));
      // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [dir, segments]);

    // While playing: follow segment boundaries and keep the mic on the picture.
    useEffect(() => {
      if (!playing) return;
      let frame = 0;
      let stopped = false;
      const tick = async () => {
        const v = video.current, a = audio.current;
        const i = current.current, s = segments[i];
        if (!v || !a || !s) return;
        if (v.currentTime >= s.srcOut - 0.01 || v.ended) {
          if (i + 1 >= segments.length) {
            pause();
            report(i, s.srcOut);
            return;
          }
          v.pause();
          a.pause();
          await load(i + 1, segments[i + 1].srcIn);
          // Paused or edited while the next segment loaded: don't resume.
          if (stopped) return;
          await Promise.all([v.play(), a.play()]).catch(() => {});
          if (stopped) return;
        } else {
          if (Math.abs(a.currentTime - v.currentTime) > MAX_DRIFT) a.currentTime = v.currentTime;
          report(i, v.currentTime);
        }
        frame = requestAnimationFrame(() => void tick());
      };
      frame = requestAnimationFrame(() => void tick());
      return () => {
        stopped = true;
        cancelAnimationFrame(frame);
      };
      // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [playing, segments]);

    const toggle = async () => {
      if (playing) return pause();
      if (!segments.length) return;
      if (out >= duration - 0.05) await seek(0);
      await Promise.all([video.current?.play(), audio.current?.play()]).catch(() => {});
      setPlaying(true);
    };

    return (
      <div className="space-y-2">
        <div className="relative mx-auto aspect-video max-h-[48vh] max-w-full overflow-hidden rounded-lg bg-black">
          <video ref={video} className="h-full w-full object-contain" muted playsInline />
          <audio ref={audio} />
          {!segments.length && (
            <p className="absolute inset-0 m-0 flex items-center justify-center text-sm text-zinc-500">Nothing to play</p>
          )}
        </div>
        <div className="flex items-center gap-3">
          <button
            type="button"
            disabled={!segments.length}
            onClick={() => void toggle()}
            className="w-20 rounded-md bg-indigo-600 px-3 py-1.5 text-sm font-medium hover:bg-indigo-500 disabled:opacity-40"
          >
            {playing ? "Pause" : "Play"}
          </button>
          <input
            type="range"
            className="flex-1"
            min={0}
            max={duration || 0}
            step={0.01}
            value={out}
            onChange={(e) => void seek(Number(e.target.value))}
            aria-label="Position"
          />
          <span className="w-28 text-right text-xs tabular-nums text-zinc-400">
            {fmt(out)} / {fmt(duration)}
          </span>
        </div>
      </div>
    );
  },
);

export default Player;
