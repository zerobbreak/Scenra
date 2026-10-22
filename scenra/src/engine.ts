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
};
export const obsStatus = () => invoke<ObsStatus>("obs_status");
export const startTake = () => invoke<string>("start_take");
export const stopTake = () => invoke<TakeResult>("stop_take");
