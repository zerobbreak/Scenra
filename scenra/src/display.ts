import { invoke } from "@tauri-apps/api/core";

export type MonitorInfo = {
  name: string | null;
  /** Physical px: x, y, width, height. */
  rect: [number, number, number, number];
  /** Windows display scaling, e.g. 1.5 for 150 %. */
  scale: number;
  primary: boolean;
};

export type DisplayInfo = {
  monitors: MonitorInfo[];
  overlayZoom: number;
  /** The monitor takes record (Windows device name); null = the primary monitor. */
  captureMonitor: string | null;
  minZoom: number;
  maxZoom: number;
};

export const displayInfo = () => invoke<DisplayInfo>("display_info");
export const setOverlayZoom = (zoom: number) => invoke<DisplayInfo>("set_overlay_zoom", { zoom });
export const setCaptureMonitor = (monitor: string | null) => invoke<DisplayInfo>("set_capture_monitor", { monitor });
