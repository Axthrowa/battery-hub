import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";

export interface BatteryReading {
  ok: boolean;
  percent: number | null;
  charging: boolean;
  transport: string;
  product: string;
  error: string | null;
  donglePresent?: boolean;
  /** Unix epoch ms of the measurement itself, not of its delivery. */
  updatedAtMs: number;
}

/** Brand slug from the backend (razer, logitech, steelseries, …). */
export type DeviceBrand = string;

export interface DeviceReading {
  brand: DeviceBrand;
  brandLabel: string;
  ok: boolean;
  percent: number | null;
  charging: boolean;
  transport: string;
  product: string;
  error: string | null;
  present: boolean;
  updatedAtMs: number;
}

export interface DeviceSnapshot {
  devices: DeviceReading[];
  online: DeviceReading[];
  updatedAtMs: number;
  primary: BatteryReading;
}

export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const EVENT_BATTERY = "battery://update";
export const EVENT_DEVICES = "devices://update";
export const EVENT_OPEN_SETTINGS = "ui://open-settings";

async function safeInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T | null> {
  if (!isTauri) return null;
  try {
    return await invoke<T>(cmd, args);
  } catch (err) {
    console.error(`invoke ${cmd} failed`, err);
    return null;
  }
}

export const readBattery = () => safeInvoke<BatteryReading>("get_battery");
export const readDevices = () => safeInvoke<DeviceSnapshot>("get_devices");
export const lastReading = () => safeInvoke<BatteryReading | null>("last_reading");
export const lastDevices = () => safeInvoke<DeviceSnapshot | null>("last_devices");
export const requestRefresh = () => safeInvoke<void>("refresh_now");
export const setPollSeconds = (seconds: number) => safeInvoke<void>("set_poll_seconds", { seconds });
export const hideToTray = () => safeInvoke<void>("close_to_tray");
export const closeToTray = () => safeInvoke<void>("close_to_tray");
export const quitApp = () => safeInvoke<void>("quit_app");

/** WinRT BLE GATT Battery Service (0x180F) scan. */
export interface BleBatteryInfo {
  ok: boolean;
  name: string;
  percent: number | null;
  deviceId: string;
  error: string | null;
}

export const readBluetoothBattery = () =>
  safeInvoke<BleBatteryInfo[]>("read_bluetooth_battery");

export const applyLocalization = (
  locale: string,
  labels: { show: string; settings: string; quit: string },
) => safeInvoke<void>("apply_localization", { locale, ...labels });

export async function onEvent<T>(
  name: string,
  handler: (payload: T) => void,
): Promise<UnlistenFn> {
  if (!isTauri) return () => {};
  return listen<T>(name, (event) => handler(event.payload));
}
