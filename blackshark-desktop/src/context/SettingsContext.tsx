import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { Locale } from "../i18n/resources";
import { applyLocalization, isTauri, setPollSeconds } from "../lib/bridge";

export interface Settings {
  locale: Locale;
  pollSeconds: number;
  autostart: boolean;
}

const DEFAULTS: Settings = { locale: "tr", pollSeconds: 60, autostart: false };
const STORE_FILE = "settings.json";
const STORE_KEY = "settings";

type StoreHandle = {
  get: <T>(key: string) => Promise<T | undefined>;
  set: (key: string, value: unknown) => Promise<void>;
  save: () => Promise<void>;
};

interface SettingsContextValue {
  settings: Settings;
  ready: boolean;
  update: (patch: Partial<Settings>) => Promise<void>;
}

const SettingsContext = createContext<SettingsContextValue | null>(null);

function sanitize(raw: unknown): Settings {
  const value = (raw ?? {}) as Partial<Settings>;
  return {
    locale: value.locale === "en" ? "en" : "tr",
    pollSeconds:
      typeof value.pollSeconds === "number" && value.pollSeconds >= 5
        ? Math.min(3600, Math.round(value.pollSeconds))
        : DEFAULTS.pollSeconds,
    autostart: Boolean(value.autostart),
  };
}

function readLocal(): Settings {
  try {
    const raw = localStorage.getItem(STORE_KEY);
    return sanitize(raw ? JSON.parse(raw) : null);
  } catch {
    return DEFAULTS;
  }
}

function writeLocal(settings: Settings) {
  try {
    localStorage.setItem(STORE_KEY, JSON.stringify(settings));
  } catch {
    /* storage unavailable */
  }
}

export function SettingsProvider({ children }: { children: ReactNode }) {
  const { i18n, t } = useTranslation();
  const [settings, setSettings] = useState<Settings>(DEFAULTS);
  const [ready, setReady] = useState(false);
  const storeRef = useRef<StoreHandle | null>(null);

  useEffect(() => {
    let cancelled = false;

    (async () => {
      let loaded = readLocal();

      if (isTauri) {
        try {
          const { load } = await import("@tauri-apps/plugin-store");
          const store = (await load(STORE_FILE, { autoSave: true })) as unknown as StoreHandle;
          storeRef.current = store;
          const stored = await store.get<Settings>(STORE_KEY);
          if (stored) loaded = sanitize(stored);
        } catch (err) {
          console.error("store load failed", err);
        }

        try {
          const { isEnabled } = await import("@tauri-apps/plugin-autostart");
          loaded = { ...loaded, autostart: await isEnabled() };
        } catch {
          /* plugin unavailable */
        }
      }

      if (cancelled) return;
      setSettings(loaded);
      setReady(true);
    })();

    return () => {
      cancelled = true;
    };
  }, []);

  // Keep i18next, the Rust poll loop and the tray menu in sync with settings.
  useEffect(() => {
    if (!ready) return;
    void i18n.changeLanguage(settings.locale);
    void setPollSeconds(settings.pollSeconds);
  }, [ready, settings.locale, settings.pollSeconds, i18n]);

  useEffect(() => {
    if (!ready) return;
    void applyLocalization(settings.locale, {
      show: t("trayShow"),
      settings: t("traySettings"),
      quit: t("trayQuit"),
    });
  }, [ready, settings.locale, t]);

  const update = useCallback(
    async (patch: Partial<Settings>) => {
      const next = sanitize({ ...settings, ...patch });
      setSettings(next);
      writeLocal(next);

      if (storeRef.current) {
        try {
          await storeRef.current.set(STORE_KEY, next);
          await storeRef.current.save();
        } catch (err) {
          console.error("store save failed", err);
        }
      }

      if (isTauri && patch.autostart !== undefined) {
        try {
          const { enable, disable } = await import("@tauri-apps/plugin-autostart");
          await (patch.autostart ? enable() : disable());
        } catch (err) {
          console.error("autostart toggle failed", err);
        }
      }
    },
    [settings],
  );

  const value = useMemo<SettingsContextValue>(
    () => ({ settings, ready, update }),
    [settings, ready, update],
  );

  return <SettingsContext.Provider value={value}>{children}</SettingsContext.Provider>;
}

export function useSettings() {
  const ctx = useContext(SettingsContext);
  if (!ctx) throw new Error("useSettings must be used inside SettingsProvider");
  return ctx;
}
