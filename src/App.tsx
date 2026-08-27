import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { AddDeviceModal } from "./components/AddDeviceModal";
import { DeviceCard } from "./components/DeviceCard";
import { SettingsModal } from "./components/SettingsModal";
import { useSettings } from "./context/SettingsContext";
import type { BatteryReading, DeviceReading, DeviceSnapshot } from "./lib/bridge";
import { pickLogoFile, useLogos } from "./lib/logos";
import {
  EVENT_BATTERY,
  EVENT_DEVICES,
  EVENT_OPEN_SETTINGS,
  closeToTray,
  isTauri,
  lastDevices,
  lastReading,
  onEvent,
  requestRefresh,
} from "./lib/bridge";

const REFRESH_TIMEOUT_MS = 9000;

function GearIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
      <circle cx="12" cy="12" r="3.2" />
      <path
        strokeLinecap="round"
        d="M19.4 15a1.7 1.7 0 0 0 .3 1.9l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.9-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.9.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.9 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.9l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.9.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.9-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.9V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"
      />
    </svg>
  );
}

// Only devices that answered this poll get a card, matching the tray menu.
// A receiver left plugged in with the device switched off is not a device the
// panel should keep showing.
function visibleDevices(snapshot: DeviceSnapshot | null): DeviceReading[] {
  return snapshot?.online ?? [];
}

export default function App() {
  const { t } = useTranslation();
  const { settings, ready } = useSettings();
  const { logoFor, setLogo, clearLogo, moveLogo } = useLogos();
  const [reading, setReading] = useState<BatteryReading | null>(null);
  const [devices, setDevices] = useState<DeviceSnapshot | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [addOpen, setAddOpen] = useState(false);
  const timeoutRef = useRef<number | null>(null);

  const applyReading = useCallback((value: BatteryReading | null) => {
    if (!value) return;
    setReading(value);
    setRefreshing(false);
    if (timeoutRef.current) {
      window.clearTimeout(timeoutRef.current);
      timeoutRef.current = null;
    }
  }, []);

  const applyDevices = useCallback(
    (value: DeviceSnapshot | null) => {
      if (!value) return;
      setDevices(value);
      applyReading(value.primary);
    },
    [applyReading],
  );

  const refresh = useCallback(async () => {
    setRefreshing(true);
    if (timeoutRef.current) window.clearTimeout(timeoutRef.current);
    timeoutRef.current = window.setTimeout(() => setRefreshing(false), REFRESH_TIMEOUT_MS);
    await requestRefresh();
  }, []);

  useEffect(() => {
    const unlisteners: Array<() => void> = [];
    let cancelled = false;

    (async () => {
      const [cached, cachedDevices] = await Promise.all([lastReading(), lastDevices()]);
      if (!cancelled) {
        if (cachedDevices) applyDevices(cachedDevices);
        else applyReading(cached ?? null);
      }

      unlisteners.push(await onEvent<BatteryReading>(EVENT_BATTERY, applyReading));
      unlisteners.push(await onEvent<DeviceSnapshot>(EVENT_DEVICES, applyDevices));
      unlisteners.push(await onEvent<void>(EVENT_OPEN_SETTINGS, () => setSettingsOpen(true)));

      if (!cancelled && !cached && !cachedDevices) void refresh();
    })();

    return () => {
      cancelled = true;
      unlisteners.forEach((fn) => fn());
      if (timeoutRef.current) window.clearTimeout(timeoutRef.current);
    };
  }, [applyDevices, applyReading, refresh]);

  useEffect(() => {
    const onFocus = () => {
      void lastDevices().then((snap) => {
        if (snap) applyDevices(snap);
        else void lastReading().then(applyReading);
      });
    };

    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [applyDevices, applyReading]);

  const pickLogo = useCallback(
    async (name: string) => {
      const logo = await pickLogoFile();
      if (logo) await setLogo(name, logo);
    },
    [setLogo],
  );

  const cards = useMemo(() => visibleDevices(devices), [devices]);
  const onlineCount = cards.length;

  return (
    <div className="flex h-full flex-col px-5 pt-5 pb-5">
      <header className="mb-4 flex items-start justify-between gap-3">
        <div>
          <p className="text-[10px] font-semibold tracking-[0.22em] text-accent/80 uppercase">
            Hardware Center
          </p>
          <h1 className="mt-0.5 text-xl leading-tight font-semibold text-neutral-50">{t("appName")}</h1>
          <p className="mt-0.5 text-xs text-neutral-500">
            {t("subtitle")} · {onlineCount} {t("online").toLowerCase()}
          </p>
        </div>
        <button
          type="button"
          onClick={() => setSettingsOpen(true)}
          className="grid h-9 w-9 place-items-center rounded-xl border border-white/10 bg-ink-850 text-neutral-400 transition hover:border-accent/40 hover:text-accent"
          aria-label={t("settings")}
          title={t("settings")}
        >
          <GearIcon />
        </button>
      </header>

      <section className="min-h-0 flex-1 overflow-y-auto pr-0.5">
        <div className="mb-2 flex items-center justify-between">
          <h2 className="text-xs font-medium tracking-wide text-neutral-400 uppercase">
            {t("dashboard")}
          </h2>
        </div>

        <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-2">
          {cards.map((device, index) => (
            <DeviceCard
              key={`${device.brand}-${device.product}-${index}`}
              device={device}
              index={index}
              chargingLabel={t("charging")}
              offlineLabel={t("offline")}
              locale={settings.locale}
              logo={logoFor(device.product)}
              onPickLogo={() => void pickLogo(device.product)}
              onClearLogo={() => void clearLogo(device.product)}
              pickLogoLabel={t("pickLogo")}
              clearLogoLabel={t("removeLogo")}
              unverifiedLabel={t("unverified")}
              unverifiedHint={t("unverifiedHint")}
            />
          ))}
        </div>

        {onlineCount === 0 ? (
          <p className="mt-4 rounded-xl border border-white/8 bg-ink-850/50 px-3 py-3 text-center text-xs text-neutral-500">
            {t("noDevices")}
            <span className="mt-1 block text-neutral-600">{t("noDevicesHint")}</span>
          </p>
        ) : null}

        <button
          type="button"
          onClick={() => setAddOpen(true)}
          className="mt-3 w-full rounded-xl border border-dashed border-white/15 py-2.5 text-xs font-medium text-neutral-400 transition hover:border-accent/40 hover:text-accent"
        >
          + {t("addDevice")}
        </button>

        {reading && !reading.ok && reading.error && onlineCount === 0 ? (
          <p className="mt-3 rounded-xl border border-red-500/20 bg-red-500/10 px-3 py-2 text-xs text-red-300">
            {reading.error}
          </p>
        ) : null}
      </section>

      <div className="mt-4 shrink-0">
        <button
          type="button"
          onClick={() => void refresh()}
          disabled={refreshing || !ready}
          className="w-full rounded-xl bg-accent py-3 text-sm font-semibold text-ink-950 transition hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {refreshing ? t("refreshing") : t("refreshNow")}
        </button>

        <p className="mt-2.5 text-center text-[11px] text-neutral-600">
          {t("minimizeHint")}
          {isTauri ? (
            <>
              {" · "}
              <button
                type="button"
                onClick={() => void closeToTray()}
                className="underline decoration-dotted transition hover:text-neutral-400"
              >
                {t("close")}
              </button>
            </>
          ) : null}
        </p>
      </div>

      <SettingsModal open={settingsOpen} onClose={() => setSettingsOpen(false)} />
      <AddDeviceModal
        open={addOpen}
        onClose={() => setAddOpen(false)}
        logoFor={logoFor}
        onPickLogo={pickLogo}
        onClearLogo={clearLogo}
        onRenameLogo={moveLogo}
      />
    </div>
  );
}
