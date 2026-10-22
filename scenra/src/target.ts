import { invoke } from "@tauri-apps/api/core";

export type CdpCapability = {
  available: boolean;
  flavor: "webview2" | "edge" | "chrome" | "electron" | "unknown" | null;
  endpoint: string | null;
  browser: string | null;
  endpointProcess: string | null;
  ownedByTarget: boolean;
  loopbackOnly: boolean;
  pageUrl: string | null;
  testIds: number | null;
};

export type CapabilityProfile = {
  application: string;
  process: { pid: number; exe: string };
  capabilities: { cdp: CdpCapability };
  preferredLocator: "cdp" | null;
  warnings: string[];
};

export const launchTarget = () => invoke<CapabilityProfile>("launch_target");
export const targetProfile = () => invoke<CapabilityProfile | null>("target_profile");
export const stopTarget = () => invoke<void>("stop_target");
export const launchChrome = (url: string) => invoke<CapabilityProfile>("launch_chrome", { url });
