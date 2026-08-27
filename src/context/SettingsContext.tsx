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
import { BACKGROUNDS, DEFAULT_ACCENT, DEFAULT_BACKGROUND } from "../i18n/resources";
import type { Background, Locale } from "../i18n/resources";
import {
  applyLocalization,
  isTauri,
  setNotificationSound,
  setPollSeconds,
} from "../lib/bridge";

/** `system` follows the Windows light/dark setting. */
export type Theme = "system" | "dark" | "light";

export interface Settings {
  locale: Locale;
  pollSeconds: number;
  autostart: boolean;
  theme: Theme;
  /** Hex from `ACCENTS`, or any colour previously stored. */
  accent: string;
  /** Which ground the panel is drawn on — see `styles.css`. */
  background: Background;
  notificationSound: boolean;
  /** Names of the chosen sound files, for showing which is which. The files
   *  themselves live beside the settings, written by Rust. */
  soundFileFull: string | null;
  soundFileLow: string | null;
  /** Cropped data URIs — see `lib/images.ts`. */
  backgroundImage: string | null;
  devicesImage: string | null;
}

const DEFAULTS: Settings = {
  locale: "tr",
  pollSeconds: 60,
  autostart: false,
  theme: "dark",
  accent: DEFAULT_ACCENT,
  background: DEFAULT_BACKGROUND,
  notificationSound: true,
  soundFileFull: null,
  soundFileLow: null,
  backgroundImage: null,
  devicesImage: null,
};

const DARK_QUERY = "(prefers-color-scheme: dark)";

function prefersDark() {
  return typeof window !== "undefined" && window.matchMedia(DARK_QUERY).matches;
}

/** Hand the theme to the document: CSS does the rest, keyed off `data-theme`. */
function paint(settings: Settings) {
  const root = document.documentElement;
  root.dataset.theme =
    settings.theme === "system" ? (prefersDark() ? "dark" : "light") : settings.theme;
  // A picture wins over the chosen ground; without one the ground stands.
  root.dataset.bg = settings.backgroundImage ? "image" : settings.background;
  root.style.setProperty("--bh-accent", settings.accent);
  root.style.setProperty(
    "--bh-image",
    settings.backgroundImage ? `url("${settings.backgroundImage}")` : "none",
  );
}

/**
 * Paint what was stored before React renders anything.
 *
 * Settings arrive asynchronously — localStorage first, then the Tauri store —
 * and an effect only runs after the first frame is on screen. Someone who chose
 * the light theme would watch the panel open dark and flip, every launch. The
 * stored value is enough to get the first frame right.
 */
export function applyStoredTheme() {
  paint(readLocal());
}
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

function fileName(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value.slice(0, 120) : null;
}

function dataUri(value: unknown): string | null {
  return typeof value === "string" && value.startsWith("data:image/") ? value : null;
}

function sanitize(raw: unknown): Settings {
  const value = (raw ?? {}) as Partial<Settings>;
  return {
    locale: value.locale === "en" ? "en" : "tr",
    pollSeconds:
      typeof value.pollSeconds === "number" && value.pollSeconds >= 5
        ? Math.min(3600, Math.round(value.pollSeconds))
        : DEFAULTS.pollSeconds,
    autostart: Boolean(value.autostart),
    theme:
      value.theme === "light" || value.theme === "system" || value.theme === "dark"
        ? value.theme
        : DEFAULTS.theme,
    accent:
      typeof value.accent === "string" && /^#[0-9a-fA-F]{6}$/.test(value.accent)
        ? value.accent
        : DEFAULTS.accent,
    background: BACKGROUNDS.includes(value.background as Background)
      ? (value.background as Background)
      : DEFAULTS.background,
    notificationSound: value.notificationSound !== false,
    soundFileFull: fileName(value.soundFileFull),
    soundFileLow: fileName(value.soundFileLow),
    backgroundImage: dataUri(value.backgroundImage),
    devicesImage: dataUri(value.devicesImage),
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
  const [settings, setSettings] = useState<Settings>(readLocal);
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

  // The toasts come from Rust, so the switch has to reach it: the store is only
  // read at launch, and a setting that needs a restart is not a setting.
  useEffect(() => {
    if (!ready) return;
    void setNotificationSound(settings.notificationSound);
  }, [ready, settings.notificationSound]);

  // Painted before `ready` too, so the first frame is not the wrong theme.
  useEffect(() => {
    paint(settings);
  }, [settings]);

  // Only `system` cares what Windows is doing.
  useEffect(() => {
    if (settings.theme !== "system") return;
    const media = window.matchMedia(DARK_QUERY);
    const onChange = () => paint(settings);
    media.addEventListener("change", onChange);
    return () => media.removeEventListener("change", onChange);
  }, [settings]);

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
