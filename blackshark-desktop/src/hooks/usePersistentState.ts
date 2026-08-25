import { useCallback, useEffect, useRef, useState } from "react";

/**
 * The tray build destroys the webview instead of hiding it, so every React
 * state tree is thrown away on close. This keeps a slice of state mirrored in
 * localStorage (which survives in the WebView2 user-data folder) and restores
 * it when the window is recreated.
 *
 * Writes are debounced, and a final flush is forced when the page is torn down
 * so nothing is lost between the last keystroke and the window closing.
 */
export function usePersistentState<T>(key: string, initialValue: T, debounceMs = 200) {
  const [value, setValue] = useState<T>(() => {
    try {
      const raw = localStorage.getItem(key);
      return raw === null ? initialValue : (JSON.parse(raw) as T);
    } catch {
      return initialValue;
    }
  });

  const valueRef = useRef(value);
  valueRef.current = value;

  const flush = useCallback(() => {
    try {
      localStorage.setItem(key, JSON.stringify(valueRef.current));
    } catch (err) {
      console.error(`failed to persist "${key}"`, err);
    }
  }, [key]);

  useEffect(() => {
    const id = window.setTimeout(flush, debounceMs);
    return () => window.clearTimeout(id);
  }, [value, flush, debounceMs]);

  useEffect(() => {
    const onHidden = () => {
      if (document.visibilityState === "hidden") flush();
    };

    // `pagehide` is the reliable teardown signal in WebView2; the others cover
    // the cases where the window is destroyed without a full unload.
    window.addEventListener("pagehide", flush);
    window.addEventListener("beforeunload", flush);
    window.addEventListener("blur", flush);
    document.addEventListener("visibilitychange", onHidden);

    return () => {
      window.removeEventListener("pagehide", flush);
      window.removeEventListener("beforeunload", flush);
      window.removeEventListener("blur", flush);
      document.removeEventListener("visibilitychange", onHidden);
      flush();
    };
  }, [flush]);

  return [value, setValue, flush] as const;
}

export default usePersistentState;
