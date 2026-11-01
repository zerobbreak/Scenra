import React from "react";
import ReactDOM from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import Controls from "./Controls";
import EditorApp from "./editor/EditorApp";
import Teleprompter from "./Teleprompter";
import "./index.css";

// One bundle, four windows, picked by the label the Rust side gives each one.
const views: Record<string, () => React.JSX.Element> = { teleprompter: Teleprompter, controls: Controls, editor: EditorApp };
const View = views[getCurrentWindow().label] ?? App;

// Windows doesn't run Scenra's keyboard hook for keys typed into Scenra's own windows, so each
// window tells the InputGuard itself (B-5). Modifiers and Esc don't count, as in the hook.
const IGNORED_KEYS = new Set(["Control", "Alt", "Shift", "Meta", "AltGraph", "Escape"]);
window.addEventListener(
  "keydown",
  (event) => {
    if (!event.repeat && !IGNORED_KEYS.has(event.key)) void invoke("guard_key");
  },
  { capture: true },
);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <View />
  </React.StrictMode>,
);
