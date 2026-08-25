import { useEffect, useRef, useState } from "react";

function currentlyVisible() {
  if (typeof document === "undefined") return true;
  return document.visibilityState === "visible";
}

async function trimMemory() {
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) return;
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    await invoke("release_memory");
  } catch (err) {
    console.error("release_memory failed", err);
  }
}

export interface UseVisibilityOptions {
  /** Ask the Rust side to trim the process working set when going hidden. */
  releaseMemoryOnHide?: boolean;
}

/**
 * Tracks `document.visibilityState`. Returns `false` while the window is
 * hidden in the tray or minimized, so callers can skip renders and timers.
 */
export function useVisibility({ releaseMemoryOnHide = false }: UseVisibilityOptions = {}) {
  const [visible, setVisible] = useState(currentlyVisible);

  useEffect(() => {
    const onChange = () => {
      const next = currentlyVisible();
      setVisible(next);
      if (!next && releaseMemoryOnHide) void trimMemory();
    };

    document.addEventListener("visibilitychange", onChange);
    // WebView2 does not always fire visibilitychange when the native window is
    // hidden, so window focus is used as a secondary signal.
    window.addEventListener("blur", onChange);
    window.addEventListener("focus", onChange);
    onChange();

    return () => {
      document.removeEventListener("visibilitychange", onChange);
      window.removeEventListener("blur", onChange);
      window.removeEventListener("focus", onChange);
    };
  }, [releaseMemoryOnHide]);

  return visible;
}

/**
 * `setInterval` that only runs while the window is visible. The timer is torn
 * down on hide and restarted on show, so nothing ticks in the background.
 */
export function useVisibleInterval(callback: () => void, delayMs: number | null) {
  const visible = useVisibility();
  const callbackRef = useRef(callback);

  useEffect(() => {
    callbackRef.current = callback;
  }, [callback]);

  useEffect(() => {
    if (!visible || delayMs === null) return;
    const id = window.setInterval(() => callbackRef.current(), delayMs);
    return () => window.clearInterval(id);
  }, [visible, delayMs]);

  return visible;
}

export default useVisibility;
