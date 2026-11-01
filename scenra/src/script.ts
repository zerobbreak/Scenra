import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// Mirrors `ai::Scenario` in Rust.
/** Where code is: a 1-based line hint, that line's text, and the enclosing symbol (see steps.rs). */
export type Anchor = { line: number; text: string; symbol?: string | null };
/** A code step in VS Code (docs/adr/0003-code-navigation.md). */
export type Step =
  | { kind: "open"; path: string; line?: number | null; instant?: boolean }
  | { kind: "scroll"; path: string; at: Anchor; lines: number }
  | { kind: "highlight"; path: string; at: Anchor; lines: number }
  | { kind: "palette"; command: string }
  | { kind: "search"; query: string };
/** Fires on `word` of its line: clicks the control labelled `target`, or runs `step`. */
export type Action = { word: number; target: string; label: string; step?: Step | null };
export type CodePosition = { path: string; at: Anchor; lines: number };
export type Line = { text: string; actions: Action[] };
export type Scene = { title: string; lines: Line[] };
export type Scenario = { title: string; scenes: Scene[]; warnings: string[] };

export type AiStatus = {
  configured: boolean;
  source: "credential-manager" | "environment" | null;
  model: string;
};

export const aiStatus = () => invoke<AiStatus>("ai_status");
export const draftScript = (brief: string) => invoke<Scenario>("draft_script", { brief });
export const currentScript = () => invoke<Scenario | null>("current_script");
export const approachAction = (action: Action) => invoke<void>("approach_action", { action });
export const pressAction = (action: Action, expiresAt: number) => invoke<void>("press_action", { action, expiresAt });
/** Installs a script written or edited by hand. */
export const setScript = (scenario: Scenario) => invoke<Scenario>("set_script", { scenario });
/** VS Code's open file and selected lines, as a step position. */
export const captureCodePosition = () => invoke<CodePosition>("capture_code_position");
export const onScriptChanged = (handler: (s: Scenario) => void) =>
  listen<Scenario>("script-changed", (e) => handler(e.payload));

// Mirrors `script::Session`: one pass through the script in the pill.
export type Mode = "rehearse" | "record";
/** `scene`: a retake from the editor (record just that scene). `attempts`: takes already on disk per scene. */
export type Session = { mode: Mode; id: string; scene?: number; attempts?: Record<number, number> };

export const startSession = (mode: Mode) => invoke<Session>("start_session", { mode });
export const currentSession = () => invoke<Session | null>("current_session");
export const onSessionChanged = (handler: (s: Session) => void) =>
  listen<Session>("session-changed", (e) => handler(e.payload));

export const words = (line: Line) => line.text.split(/\s+/).filter(Boolean);

export const checkSceneStart = (scene: number) => invoke<void>("check_scene_start", { scene });
export const resetScenePage = (scene: number) => invoke<void>("reset_scene_page", { scene });
export const approveSceneStart = (scene: number) => invoke<void>("approve_scene_start", { scene });

export const playbackGuard = (active: boolean) => invoke<void>("playback_guard", { active });
export const playbackStatus = () => invoke<{foreign: number; stopped: boolean; interference: boolean}>("guard_status");

export type SetupResult = { actionsRun: number };
export const prepareScene = (scene: number, scenario: Scenario) => invoke<SetupResult>("prepare_scene", { request: { scene, scenario, requestId: Date.now() } });
export const cancelSceneSetup = () => invoke<void>("cancel_scene_setup");
