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
