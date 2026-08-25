import { useCallback, useEffect, useRef, useState } from "react";
import { isTauri } from "./bridge";

/** Device name (normalised) -> PNG data URI. */
export type LogoMap = Record<string, string>;

const STORE_FILE = "logos.json";
const STORE_KEY = "logos";
/** Logos live inside a JSON store, so they are downscaled before saving. */
const LOGO_SIZE = 128;

type StoreHandle = {
  get: <T>(key: string) => Promise<T | undefined>;
  set: (key: string, value: unknown) => Promise<void>;
  save: () => Promise<void>;
};

/** Keyed by name so a logo survives reconnects and follows a rename. */
export function logoKey(name: string): string {
  return name.toLowerCase().replace(/[^a-z0-9]/g, "");
}

function readLocal(): LogoMap {
  try {
    const raw = localStorage.getItem(STORE_KEY);
    return raw ? (JSON.parse(raw) as LogoMap) : {};
  } catch {
    return {};
  }
}

function writeLocal(map: LogoMap) {
  try {
    localStorage.setItem(STORE_KEY, JSON.stringify(map));
  } catch {
    /* storage unavailable */
  }
}

/** Scale the chosen image into a square PNG small enough to store as text. */
function toLogoDataUri(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error("read failed"));
    reader.onload = () => {
      const image = new Image();
      image.onerror = () => reject(new Error("decode failed"));
      image.onload = () => {
        const canvas = document.createElement("canvas");
        canvas.width = LOGO_SIZE;
        canvas.height = LOGO_SIZE;
        const ctx = canvas.getContext("2d");
        if (!ctx) {
          reject(new Error("no canvas"));
          return;
        }
        // Contain, centred, transparent padding — never distort the artwork.
        const scale = Math.min(LOGO_SIZE / image.width, LOGO_SIZE / image.height);
        const width = image.width * scale;
        const height = image.height * scale;
        ctx.drawImage(image, (LOGO_SIZE - width) / 2, (LOGO_SIZE - height) / 2, width, height);
        resolve(canvas.toDataURL("image/png"));
      };
      image.src = String(reader.result);
    };
    reader.readAsDataURL(file);
  });
}

/** Open the OS file picker and return the processed logo, or null if cancelled. */
export function pickLogoFile(): Promise<string | null> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "image/*";
    input.style.display = "none";
    document.body.appendChild(input);

    const cleanup = () => input.remove();
    input.onchange = () => {
      const file = input.files?.[0];
      if (!file) {
        cleanup();
        resolve(null);
        return;
      }
      toLogoDataUri(file)
        .then((uri) => resolve(uri))
        .catch((err) => {
          console.error("logo import failed", err);
          resolve(null);
        })
        .finally(cleanup);
    };
    // A cancelled picker fires no event; drop the element on the next focus.
    window.addEventListener("focus", () => window.setTimeout(cleanup, 500), { once: true });
    input.click();
  });
}

export function useLogos() {
  const [logos, setLogos] = useState<LogoMap>({});
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
          const stored = await store.get<LogoMap>(STORE_KEY);
          if (stored) loaded = stored;
        } catch (err) {
          console.error("logo store load failed", err);
        }
      }
      if (!cancelled) setLogos(loaded);
    })();

    return () => {
      cancelled = true;
    };
  }, []);

  const persist = useCallback(async (next: LogoMap) => {
    setLogos(next);
    writeLocal(next);
    const store = storeRef.current;
    if (!store) return;
    try {
      await store.set(STORE_KEY, next);
      await store.save();
    } catch (err) {
      console.error("logo store save failed", err);
    }
  }, []);

  const setLogo = useCallback(
    async (name: string, dataUri: string) => {
      await persist({ ...logos, [logoKey(name)]: dataUri });
    },
    [logos, persist],
  );

  const clearLogo = useCallback(
    async (name: string) => {
      const next = { ...logos };
      delete next[logoKey(name)];
      await persist(next);
    },
    [logos, persist],
  );

  /** Keep the logo attached when a device is renamed. */
  const moveLogo = useCallback(
    async (fromName: string, toName: string) => {
      const from = logoKey(fromName);
      const to = logoKey(toName);
      if (from === to || !logos[from]) return;
      const next = { ...logos, [to]: logos[from] };
      delete next[from];
      await persist(next);
    },
    [logos, persist],
  );

  const logoFor = useCallback((name: string) => logos[logoKey(name)], [logos]);

  return { logos, logoFor, setLogo, clearLogo, moveLogo };
}
