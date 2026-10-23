import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// Mirrors `ai::Scenario` in Rust.
export type Action = { word: number; target: string; label: string };
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
export const approachAction = (target: string) => invoke<void>("approach_action", { target });
export const pressAction = (target: string, expiresAt: number) => invoke<void>("press_action", { target, expiresAt });
export const onScriptChanged = (handler: (s: Scenario) => void) =>
  listen<Scenario>("script-changed", (e) => handler(e.payload));

// Mirrors `script::Session`: one pass through the script in the pill.
export type Mode = "rehearse" | "record";
export type Session = { mode: Mode; id: string };

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
