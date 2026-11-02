import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Scenario, Step } from "./script";

// Mirrors `teach.rs` in Rust.
export type StepFlag = "field" | "notControl" | "unlabelled" | "ambiguous" | "disabled" | "shortcut" | "notInCodebase" | "offScreen";
export type PageState = { url: string; headings: string[]; dialogs: string[] };
export type TeachStep = {
  index: number;
  atMs: number;
  label: string;
  testId: string | null;
  role: string | null;
  before: PageState;
  flags: StepFlag[];
  /// A VS Code step; its `label` already reads "Open take.rs".
  step?: Step | null;
};
export type TeachSession = { start: PageState; end: PageState; steps: TeachStep[]; warnings: string[] };
export type TeachStatus = { active: boolean; last: TeachSession | null };

// Same wording as `StepFlag::describe`.
export const FLAG_TEXT: Record<StepFlag, string> = {
  field: "a click into a field; typing isn't supported yet",
  notControl: "not on a link or button",
  unlabelled: "the control has no label",
  ambiguous: "another control has the same label",
  disabled: "the control was disabled",
  shortcut: "keyboard shortcuts aren't replayed yet",
  notInCodebase: "the file isn't in the codebase folder",
  offScreen: "the selection ran past the edge of the editor",
};

/// How a taught step reads in the list: a click on a web page, or what happened in VS Code.
export function stepText(step: TeachStep): string {
  if (step.step) return step.label;
  if (step.flags.includes("shortcut")) return `Shortcut ${step.label}`;
  if (step.flags.includes("notInCodebase") || step.flags.includes("offScreen")) return step.label;
  if (step.label) return `Click “${step.label}”`;
  return step.flags.includes("field") ? "Click into a field" : "Click";
}

export const startTeach = () => invoke<void>("start_teach");
export const stopTeach = () => invoke<TeachSession>("stop_teach");
export const teachState = () => invoke<TeachStatus>("teach_state");
export const deleteTeachStep = (index: number) => invoke<TeachSession>("delete_teach_step", { index });
export const draftFromTeach = (brief: string) => invoke<Scenario>("draft_from_teach", { brief });
export const onTeachStep = (handler: (step: TeachStep) => void) =>
  listen<TeachStep>("teach-step", (e) => handler(e.payload));
