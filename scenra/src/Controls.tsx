import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { closeTeleprompter } from "./capture";
import { clearStop } from "./engine";
import { onPrompter, sendPrompter } from "./prompter";

// Minimised control bar, design C2: design/scenra-canvas/CompactAction.dc.html (bottom nav).
// Its own capture-excluded window, so it never appears in the video.

function ControlButton(props: {
  label: string;
  onClick: () => void;
  tone?: "default" | "active" | "stop";
  pressed?: boolean;
  children: React.ReactNode;
}) {
  const tone = {
    default: "bg-[#33363D] hover:bg-[#40444C]",
    active: "bg-[#4F46E5] hover:bg-[#5B53F0]",
    stop: "bg-[#DC2626] hover:bg-[#E23B3B]",
  }[props.tone ?? "default"];
  return (
    <button
      type="button"
      aria-label={props.label}
      title={props.label}
      aria-pressed={props.pressed}
      onClick={props.onClick}
      className={`flex size-11 cursor-pointer items-center justify-center rounded-[13px] border-none ${tone}`}
    >
      {props.children}
    </button>
  );
}

const stroke = {
  fill: "none",
  stroke: "#FFFFFF",
  strokeLinecap: "round",
  strokeLinejoin: "round",
} as const;

export default function Controls() {
  const [paused, setPaused] = useState(false);
  const [peek, setPeek] = useState(false);

  // The pill can collapse itself; keep the "Show full prompter" button in step.
  useEffect(() => {
    const unlisten = onPrompter((event) => {
      if (event.kind === "peek") setPeek(event.open);
    });
    // Ctrl+Alt+Esc: the engine has already let go of everything; pause the take until resumed.
    const unlistenStop = listen("emergency-stop", () => {
      setPaused(true);
      sendPrompter({ kind: "pause", paused: true });
    });
    return () => {
      unlisten.then((stop) => stop());
      unlistenStop.then((stop) => stop());
    };
  }, []);

  async function togglePause() {
    // Resuming also clears a latched emergency stop.
    if (paused) await clearStop();
    setPaused(!paused);
    sendPrompter({ kind: "pause", paused: !paused });
  }
  function togglePeek() {
    setPeek(!peek);
    sendPrompter({ kind: "peek", open: !peek });
  }

  return (
    <nav
      aria-label="Controls (minimised)"
      className="flex h-full items-center gap-1.5 border border-white/10 bg-[#0E1013] p-1.5"
    >
      <ControlButton label={paused ? "Resume" : "Pause"} tone={paused ? "active" : "default"} onClick={togglePause}>
        {paused ? (
          <svg width="16" height="16" viewBox="0 0 24 24" aria-hidden="true">
            <path d="M8 5l11 7-11 7z" fill="#FFFFFF" />
          </svg>
        ) : (
          <svg width="16" height="16" viewBox="0 0 24 24" aria-hidden="true">
            <path d="M9 5v14M15 5v14" {...stroke} strokeWidth="2.4" />
          </svg>
        )}
      </ControlButton>
      <ControlButton label="Retry scene" onClick={() => sendPrompter({ kind: "retry" })}>
        <svg width="16" height="16" viewBox="0 0 24 24" aria-hidden="true">
          <path d="M5 12a7 7 0 1 0 2.1-5M5 4v4h4" {...stroke} strokeWidth="2.2" />
        </svg>
      </ControlButton>
      <ControlButton label="Show full prompter" tone={peek ? "active" : "default"} pressed={peek} onClick={togglePeek}>
        <svg width="18" height="18" viewBox="0 0 24 24" aria-hidden="true">
          <path d="M5 6h14M5 10h14M5 14h10M5 18h7" {...stroke} strokeWidth="2.2" />
        </svg>
      </ControlButton>
      <ControlButton label="Stop recording" tone="stop" onClick={() => closeTeleprompter()}>
        <svg width="14" height="14" viewBox="0 0 24 24" aria-hidden="true">
          <path d="M7 7h10v10H7z" fill="#FFFFFF" />
        </svg>
      </ControlButton>
    </nav>
  );
}
