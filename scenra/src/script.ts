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
export const runAction = (target: string) => invoke<void>("run_action", { target });
export const onScriptChanged = (handler: (s: Scenario) => void) =>
  listen<Scenario>("script-changed", (e) => handler(e.payload));

export const words = (line: Line) => line.text.split(/\s+/).filter(Boolean);
