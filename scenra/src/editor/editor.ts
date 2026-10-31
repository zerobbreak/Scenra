import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Scenario } from "../script";

// Mirrors `editor/model.rs` and `editor/library.rs`. Times are seconds from a take's first frame.

export type CutSource = { kind: "manual" } | { kind: "ai"; reason: string };
export type Cut = { from: number; to: number; source: CutSource; enabled: boolean };
export type SceneEdit = {
  scene: number;
  take: string | null;
  /** Why Assemble with AI chose this take; null when you chose it. */
  takeNote: string | null;
  trimIn: number;
  trimOut: number | null;
  /** Why Tidy up set the trims; null when you set them. */
  trimNote: string | null;
  cuts: Cut[];
  excluded: boolean;
};
export type Edit = { version: number; scenes: SceneEdit[] };

export type ActionRecord = { kind: "approach" | "press"; label: string; start: number; end: number; ok: boolean };
export type TakeInfo = {
  folder: string;
  scene: number;
  attempt: number;
  duration: number;
  hasActions: boolean;
  actions: ActionRecord[];
};
export type Segment = { scene: number; take: string; srcIn: number; srcOut: number; outAt: number };
export type Timeline = { segments: Segment[]; duration: number; warnings: string[] };

export type SessionSummary = { id: string; dir: string; title: string; scenes: number; takes: number; exported: boolean };
export type SessionData = {
  id: string;
  dir: string;
  title: string;
  script: Scenario | null;
  sceneCount: number;
  takes: TakeInfo[];
  edit: Edit;
  timeline: Timeline;
  exports: string[];
  /** Cached transcripts by take folder. */
  transcripts: Record<string, Word[]>;
};
export type Word = { text: string; start: number; end: number; filler: boolean };
export type Consent = "ask" | "allowed" | "off";
export type EditorAiStatus = { consent: Consent; configured: boolean };

export const openEditor = (session?: string) => invoke<void>("open_editor", { session: session ?? null });
export const listSessions = () => invoke<SessionSummary[]>("list_sessions");
export const loadSession = (session: string) => invoke<SessionData>("load_session", { session });
export const saveEdit = (dir: string, edit: Edit) => invoke<SessionData>("save_edit", { dir, edit });
export const tidySession = (dir: string) => invoke<SessionData>("tidy_session", { dir });
export const assembleSession = (dir: string) => invoke<SessionData>("assemble_session", { dir });
export const editorAiStatus = () => invoke<EditorAiStatus>("editor_ai_status");
export const setTranscription = (consent: Consent) => invoke<EditorAiStatus>("set_transcription", { consent });
export const onAssembleProgress = (handler: (p: { dir: string; done: number; total: number }) => void) =>
  listen<{ dir: string; done: number; total: number }>("assemble-progress", (e) => handler(e.payload));
export const exportSession = (dir: string) => invoke<string>("export_session", { dir });
export const cancelExport = () => invoke<void>("cancel_export");
export const retakeScene = (dir: string, scene: number) => invoke<void>("retake_scene", { dir, scene });
export type Description = { title: string; description: string; chapters: string; chapterNote: string | null };
export const describeSession = (dir: string) => invoke<Description>("describe_session", { dir });
export const revealInExplorer = (path: string) => invoke<void>("reveal_in_explorer", { path });

export const onOpenSession = (handler: (session: string) => void) =>
  listen<string>("editor-open-session", (e) => handler(e.payload));
export const onExportProgress = (handler: (p: { dir: string; percent: number }) => void) =>
  listen<{ dir: string; percent: number }>("export-progress", (e) => handler(e.payload));

/** A take's file, playable in the webview (asset protocol). */
export const mediaUrl = (dir: string, take: string, file: "video.mp4" | "microphone.wav") =>
  convertFileSrc(`${dir}\\${take}\\${file}`);

/** Lower-cased word without punctuation (as `assemble::bare`). */
export const bare = (w: string) => w.toLowerCase().replace(/[^\p{L}\p{N}']/gu, "");

/** For each said word, whether it lines up with the script (longest common subsequence). */
export function onScript(script: string[], said: string[]): boolean[] {
  const n = script.length, m = said.length;
  const t = Array.from({ length: n + 1 }, () => new Uint16Array(m + 1));
  for (let i = n - 1; i >= 0; i--)
    for (let j = m - 1; j >= 0; j--) t[i][j] = script[i] === said[j] ? t[i + 1][j + 1] + 1 : Math.max(t[i + 1][j], t[i][j + 1]);
  const out = new Array<boolean>(m).fill(false);
  let i = 0, j = 0;
  while (i < n && j < m) {
    if (script[i] === said[j]) { out[j] = true; i++; j++; }
    else if (t[i + 1][j] >= t[i][j + 1]) i++;
    else j++;
  }
  return out;
}

export const fmt = (seconds: number) => {
  const s = Math.max(0, seconds);
  const m = Math.floor(s / 60);
  return `${m}:${(s - m * 60).toFixed(1).padStart(4, "0")}`;
};

/** "20261031-190242" (UTC) → local date and time. */
export const sessionDate = (id: string) => {
  const m = /^(\d{4})(\d{2})(\d{2})-(\d{2})(\d{2})(\d{2})$/.exec(id);
  if (!m) return id;
  const d = new Date(Date.UTC(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +m[6]));
  return d.toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
};
