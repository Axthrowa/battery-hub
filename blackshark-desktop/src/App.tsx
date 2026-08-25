import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { BatteryRing, levelColor } from "./components/BatteryRing";
import { SettingsModal } from "./components/SettingsModal";
import { useSettings } from "./context/SettingsContext";
import type { BatteryReading } from "./lib/bridge";
import {
  EVENT_BATTERY,
  EVENT_OPEN_SETTINGS,
  hideToTray,
  isTauri,
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

export default function App() {
  const { t } = useTranslation();
  const { settings, ready } = useSettings();
  const [reading, setReading] = useState<BatteryReading | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [focused, setFocused] = useState(true);
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
      const cached = await lastReading();
      if (!cancelled) applyReading(cached ?? null);

      unlisteners.push(await onEvent<BatteryReading>(EVENT_BATTERY, applyReading));
      unlisteners.push(await onEvent<void>(EVENT_OPEN_SETTINGS, () => setSettingsOpen(true)));

      if (!cancelled && !cached) void refresh();
    })();

    return () => {
      cancelled = true;
      unlisteners.forEach((fn) => fn());
      if (timeoutRef.current) window.clearTimeout(timeoutRef.current);
    };
  }, [applyReading, refresh]);

  // While hidden in the tray the backend stops emitting, so pull the cached
  // reading (and pause CSS animations) around focus changes.
  useEffect(() => {
    const onFocus = () => {
      setFocused(true);
      void lastReading().then(applyReading);
    };
    const onBlur = () => setFocused(false);

    window.addEventListener("focus", onFocus);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("blur", onBlur);
    };
  }, [applyReading]);

  const online = Boolean(reading?.ok);
  const percent = reading?.percent ?? null;
  const color = levelColor(percent, online);

  const statusLabel = useMemo(() => {
    if (!online) return t("offline");
    if (reading?.charging) return t("charging");
    if (percent === null) return t("unknown");
    if (percent >= 50) return t("levelHigh");
    if (percent >= 20) return t("levelMedium");
    return t("levelLow");
  }, [online, percent, reading?.charging, t]);

  // Timestamped by the backend: a cached reading restored after the window was
  // destroyed must show when it was measured, not when the UI came back.
  const timeLabel = reading?.updatedAtMs
    ? new Date(reading.updatedAtMs).toLocaleTimeString(
        settings.locale === "tr" ? "tr-TR" : "en-US",
        { hour: "2-digit", minute: "2-digit", second: "2-digit" },
      )
    : "--:--:--";

  return (
    <div className="flex h-full flex-col px-6 pt-5 pb-6">
      <header className="mb-2 flex items-start justify-between">
        <div>
          <h1 className="text-lg leading-tight font-semibold text-neutral-100">{t("appName")}</h1>
          <p className="mt-0.5 text-xs text-neutral-500">{t("subtitle")}</p>
        </div>
        <button
          type="button"
          onClick={() => setSettingsOpen(true)}
          className="grid h-9 w-9 place-items-center rounded-xl border border-white/10 bg-ink-850 text-neutral-400 transition hover:border-razer/40 hover:text-razer"
          aria-label={t("settings")}
          title={t("settings")}
        >
          <GearIcon />
        </button>
      </header>

      <div className="my-auto grid place-items-center py-3">
        <BatteryRing
          percent={percent}
          charging={Boolean(reading?.charging)}
          online={online}
          animated={focused}
        />
        <p className="mt-4 text-xs tracking-[0.18em] text-neutral-500 uppercase">
          {t("batteryStatus")}
        </p>
        <p className="mt-1 text-sm font-medium" style={{ color }}>
          {statusLabel}
        </p>
      </div>

      <dl className="mb-4 grid grid-cols-2 gap-2 text-xs">
        <div className="rounded-xl border border-white/10 bg-ink-850/70 px-3 py-2.5">
          <dt className="text-neutral-500">{t("connection")}</dt>
          <dd className="mt-0.5 font-medium text-neutral-200">
            {online ? reading?.transport || "2.4 GHz" : t("offline")}
          </dd>
        </div>
        <div className="rounded-xl border border-white/10 bg-ink-850/70 px-3 py-2.5">
          <dt className="text-neutral-500">{t("lastUpdated")}</dt>
          <dd className="mt-0.5 font-medium tabular-nums text-neutral-200">{timeLabel}</dd>
        </div>
      </dl>

      {reading && !reading.ok && reading.error ? (
        <p className="mb-3 rounded-xl border border-red-500/20 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {reading.error}
        </p>
      ) : null}

      <button
        type="button"
        onClick={() => void refresh()}
        disabled={refreshing || !ready}
        className="w-full rounded-xl bg-razer py-3 text-sm font-semibold text-black transition hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-50"
      >
        {refreshing ? t("refreshing") : t("refreshNow")}
      </button>

      <p className="mt-3 text-center text-[11px] text-neutral-600">
        {t("minimizeHint")}
        {isTauri ? (
          <>
            {" · "}
            <button
              type="button"
              onClick={() => void hideToTray()}
              className="underline decoration-dotted transition hover:text-neutral-400"
            >
              {t("close")}
            </button>
          </>
        ) : null}
      </p>

      <SettingsModal open={settingsOpen} onClose={() => setSettingsOpen(false)} />
    </div>
  );
}
