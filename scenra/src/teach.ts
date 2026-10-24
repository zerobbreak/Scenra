import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Scenario } from "./script";

// Mirrors `teach.rs` in Rust.
export type StepFlag = "field" | "notControl" | "unlabelled" | "ambiguous" | "disabled";
export type PageState = { url: string; headings: string[]; dialogs: string[] };
export type TeachStep = {
  index: number;
  atMs: number;
  label: string;
  testId: string | null;
  role: string | null;
  before: PageState;
  flags: StepFlag[];
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
};

export const startTeach = () => invoke<void>("start_teach");
export const stopTeach = () => invoke<TeachSession>("stop_teach");
export const teachState = () => invoke<TeachStatus>("teach_state");
export const deleteTeachStep = (index: number) => invoke<TeachSession>("delete_teach_step", { index });
export const draftFromTeach = (brief: string) => invoke<Scenario>("draft_from_teach", { brief });
export const onTeachStep = (handler: (step: TeachStep) => void) =>
  listen<TeachStep>("teach-step", (e) => handler(e.payload));
