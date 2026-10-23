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

export type ObsStatus = { version: string; profile: string; recording: boolean; mic: string | null };
export type TakeResult = {
  dir: string;
  video: string;
  microphone: string;
  microphoneSeconds: number;
  sampleRate: number;
  channels: number;
  /** Frames OBS dropped to rendering or encoding lag, in percent. */
  droppedFramesPercent: number;
  /** Physical memory free when the take stopped, in MB. */
  freeMemoryMb: number;
};
export const obsStatus = () => invoke<ObsStatus>("obs_status");
/** `folder` is relative to the takes folder; without it the take gets a timestamped one. */
export const startTake = (folder?: string) => invoke<string>("start_take", { folder: folder ?? null });
/** `keepProfile` leaves OBS on the Scenra profile for the next scene of a recorded session. */
export const stopTake = (keepProfile = false) => invoke<TakeResult>("stop_take", { keepProfile });
export const restoreObsProfile = () => invoke<void>("restore_obs_profile");

export type TakePreflight = { freeMemoryMb: number; warning: string | null };
export const takePreflight = () => invoke<TakePreflight>("take_preflight");
