import { useCallback, useEffect, useRef, useState } from "react";
import { isTauri } from "./bridge";

/**
 * Pictures kept against device names.
 *
 * Two things need this — the small brand logo and the card's own backdrop —
 * and they need exactly the same handling: keyed by name so the picture
 * survives a reconnect and follows a rename, mirrored into localStorage so the
 * first frame after launch already has it, and written to a JSON store that is
 * only there under Tauri. Only the file and the key differ, so that is all the
 * caller passes.
 */
export type ImageMap = Record<string, string>;

type StoreHandle = {
  get: <T>(key: string) => Promise<T | undefined>;
  set: (key: string, value: unknown) => Promise<void>;
  save: () => Promise<void>;
};

/** Names arrive spelled differently by different readers; this is the key. */
export function imageKey(name: string): string {
  return name.toLowerCase().replace(/[^a-z0-9]/g, "");
}

function readLocal(storeKey: string): ImageMap {
  try {
    const raw = localStorage.getItem(storeKey);
    return raw ? (JSON.parse(raw) as ImageMap) : {};
  } catch {
    return {};
  }
}

function writeLocal(storeKey: string, map: ImageMap) {
  try {
    localStorage.setItem(storeKey, JSON.stringify(map));
  } catch {
    /* storage unavailable */
  }
}

export function useImageMap(storeFile: string, storeKey: string) {
  const [map, setMap] = useState<ImageMap>({});
  const storeRef = useRef<StoreHandle | null>(null);

  useEffect(() => {
    let cancelled = false;

    (async () => {
      let loaded = readLocal(storeKey);
      if (isTauri) {
        try {
          const { load } = await import("@tauri-apps/plugin-store");
          const store = (await load(storeFile, { autoSave: true })) as unknown as StoreHandle;
          storeRef.current = store;
          const stored = await store.get<ImageMap>(storeKey);
          if (stored) loaded = stored;
        } catch (err) {
          console.error(`${storeFile} load failed`, err);
        }
      }
      if (!cancelled) setMap(loaded);
    })();

    return () => {
      cancelled = true;
    };
  }, [storeFile, storeKey]);

  const persist = useCallback(
    async (next: ImageMap) => {
      setMap(next);
      writeLocal(storeKey, next);
      const store = storeRef.current;
      if (!store) return;
      try {
        await store.set(storeKey, next);
        await store.save();
      } catch (err) {
        console.error(`${storeFile} save failed`, err);
      }
    },
    [storeFile, storeKey],
  );

  const setImage = useCallback(
    async (name: string, dataUri: string) => {
      await persist({ ...map, [imageKey(name)]: dataUri });
    },
    [map, persist],
  );

  const clearImage = useCallback(
    async (name: string) => {
      const next = { ...map };
      delete next[imageKey(name)];
      await persist(next);
    },
    [map, persist],
  );

  /** Keep the picture attached when a device is renamed. */
  const moveImage = useCallback(
    async (fromName: string, toName: string) => {
      const from = imageKey(fromName);
      const to = imageKey(toName);
      if (from === to || !map[from]) return;
      const next = { ...map, [to]: map[from] };
      delete next[from];
      await persist(next);
    },
    [map, persist],
  );

  const imageFor = useCallback((name: string) => map[imageKey(name)], [map]);

  return { map, imageFor, setImage, clearImage, moveImage };
}
