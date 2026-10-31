import { invoke } from "@tauri-apps/api/core";

export type Selector = { testId: string } | { label: string };
export type Rect = { x: number; y: number; width: number; height: number };
export type Probe = {
  located: { css: Rect; screen: Rect; devicePixelRatio: number; viewport: Rect };
  seenCss: [number, number] | null;
  errorCss: number | null;
  passed: boolean;
};
export type GuardCounts = { expected: number; foreign: number; keyEvents: number; stopped: boolean };

export const probeElement = (selector: Selector) => invoke<Probe>("probe_element", { selector });
export const clickElement = (selector: Selector) => invoke<Probe>("click_element", { selector });
export const typeInto = (selector: Selector, text: string) => invoke<void>("type_into", { selector, text });
export const slowMoveTest = () => invoke<void>("slow_move_test");
export const slowTypeTest = (text: string) => invoke<void>("slow_type_test", { text });
export const simulateHang = (seconds: number) => invoke<void>("simulate_hang", { seconds });
export const guardStatus = () => invoke<GuardCounts>("guard_status");
export const clearStop = () => invoke<void>("clear_stop");

/** What a take records: the monitor, the default mic, and the encoder. */
export type RecorderStatus = { mic: string | null; monitor: string; width: number; height: number; encoder: string };
export type TakeResult = {
  dir: string;
  video: string;
  microphone: string;
  microphoneSeconds: number;
  sampleRate: number;
  channels: number;
  /** Frames that reached the recorder more than 100 ms late (the machine fell behind), in percent. */
  lateFramesPercent: number;
  /** Physical memory free when the take stopped, in MB. */
  freeMemoryMb: number;
  /** Scenra's own committed memory when the take stopped, in MB. */
  scenraMemoryMb: number;
};
export const recorderStatus = () => invoke<RecorderStatus>("recorder_status");
/** `folder` is relative to the takes folder; without it the take gets a timestamped one. */
export const startTake = (folder?: string) => invoke<string>("start_take", { folder: folder ?? null });
export const stopTake = () => invoke<TakeResult>("stop_take");

export type TakePreflight = { freeMemoryMb: number; warning: string | null };
export const takePreflight = () => invoke<TakePreflight>("take_preflight");
