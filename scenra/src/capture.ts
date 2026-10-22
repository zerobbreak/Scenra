import { invoke } from "@tauri-apps/api/core";

export type CaptureStatus = {
  open: boolean;
  affinity: number | null;
  excluded: boolean;
  osBuild: number;
  supported: boolean;
};

export type ProbeResult = {
  rect: [number, number, number, number];
  markerPixels: number;
  totalPixels: number;
  leaks: boolean;
};

export const openTeleprompter = (excluded: boolean) =>
  invoke<CaptureStatus>("open_teleprompter", { excluded });
export const closeTeleprompter = () => invoke<CaptureStatus>("close_teleprompter");
export const setCaptureExcluded = (excluded: boolean) =>
  invoke<CaptureStatus>("set_capture_excluded", { excluded });
export const captureStatus = () => invoke<CaptureStatus>("capture_status");
export const probeCapture = () => invoke<ProbeResult>("probe_capture");

export type MatrixShot = {
  label: string;
  expectPill: boolean;
  markerPixels: number;
  pillPixels: number;
  pillSeen: boolean;
  png: string;
};

export type MatrixRow = {
  name: string;
  method: "WGC" | "DXGI";
  scale: number;
  take: string;
  shots: MatrixShot[];
  passed: boolean;
};

/** Runs the step 1 matrix for the current display scaling. Needs OBS and the demo target. */
export const runCaptureMatrix = () => invoke<MatrixRow[]>("run_capture_matrix");
