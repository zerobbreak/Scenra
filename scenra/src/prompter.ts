import { emit, listen } from "@tauri-apps/api/event";

// Events from the control bar window to the pill window.
export type PrompterEvent =
  | { kind: "pause"; paused: boolean }
  | { kind: "retry" }
  | { kind: "peek"; open: boolean };

const EVENT = "prompter";

export const sendPrompter = (event: PrompterEvent) => emit(EVENT, event);
export const onPrompter = (handler: (event: PrompterEvent) => void) =>
  listen<PrompterEvent>(EVENT, (e) => handler(e.payload));
