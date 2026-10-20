import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import Teleprompter from "./Teleprompter";
import "./index.css";

// One bundle, two windows: the Rust side opens the teleprompter with the label "teleprompter".
const isTeleprompter = getCurrentWindow().label === "teleprompter";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{isTeleprompter ? <Teleprompter /> : <App />}</React.StrictMode>,
);
