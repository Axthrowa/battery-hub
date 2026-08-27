import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { BACKDROP_BLUR, centredCrop, clampCrop, coverScale, fitZoom, renderCrop } from "../lib/images";
import type { Crop, ImageTarget } from "../lib/images";

interface ImageCropModalProps {
  file: File;
  target: ImageTarget;
  /** The cropped data URI, or `null` if the choice was abandoned. */
  onDone: (uri: string | null) => void;
}

/** How wide the frame is drawn; its height follows the target's shape. */
const FRAME_WIDTH = 268;
const MAX_ZOOM = 3;

/**
 * Placing a picture in a frame it was not cut for.
 *
 * Importing centre-crops by default, which is right often enough to be the
 * default and wrong often enough to be worth arguing with: the thing worth
 * seeing is rarely dead centre. So the frame is shown at the shape it will
 * actually be, with the picture behind it to drag and a slider to come closer,
 * and the size it will be written at is printed underneath — for anyone who
 * would rather crop it properly somewhere else and come back.
 */
export function ImageCropModal({ file, target, onDone }: ImageCropModalProps) {
  const { t } = useTranslation();
  const [url, setUrl] = useState<string | null>(null);
  const [size, setSize] = useState<{ width: number; height: number } | null>(null);
  const [crop, setCrop] = useState<Crop | null>(null);
  const [busy, setBusy] = useState(false);
  const drag = useRef<{ x: number; y: number; crop: Crop } | null>(null);

  const frameHeight = Math.round((FRAME_WIDTH * target.height) / target.width);
  // One number turns frame units into the pixels on screen, and back.
  const display = FRAME_WIDTH / target.width;

  useEffect(() => {
    const objectUrl = URL.createObjectURL(file);
    setUrl(objectUrl);
    const image = new Image();
    image.onload = () => {
      const measured = { width: image.naturalWidth, height: image.naturalHeight };
      setSize(measured);
      setCrop(centredCrop(measured, target));
    };
    image.src = objectUrl;
    return () => URL.revokeObjectURL(objectUrl);
  }, [file, target]);

  const move = useCallback(
    (clientX: number, clientY: number) => {
      const start = drag.current;
      if (!start || !size) return;
      setCrop(
        clampCrop(
          {
            zoom: start.crop.zoom,
            x: start.crop.x + (clientX - start.x) / display,
            y: start.crop.y + (clientY - start.y) / display,
          },
          size,
          target,
        ),
      );
    },
    [display, size, target],
  );

  const zoomTo = (zoom: number) => {
    if (!size || !crop) return;
    // Zoom about the middle of the frame, so what is being looked at stays put.
    const before = coverScale(size, target) * crop.zoom;
    const after = coverScale(size, target) * zoom;
    const ratio = after / before;
    setCrop(
      clampCrop(
        {
          zoom,
          x: target.width / 2 - (target.width / 2 - crop.x) * ratio,
          y: target.height / 2 - (target.height / 2 - crop.y) * ratio,
        },
        size,
        target,
      ),
    );
  };

  const apply = async () => {
    if (!crop) return;
    setBusy(true);
    try {
      onDone(await renderCrop(file, target, crop));
    } catch (err) {
      console.error("crop failed", err);
      onDone(null);
    }
  };

  const scale = size && crop ? coverScale(size, target) * crop.zoom : 1;
  // Below this nothing new comes into view, only more empty room around it.
  const minZoom = size ? fitZoom(size, target) : 1;
  const zoomedOut = (crop?.zoom ?? 1) < 1;

  return (
    <div
      className="fixed inset-0 z-60 flex items-center justify-center bg-black/70 backdrop-blur-sm"
      onClick={() => onDone(null)}
    >
      <div
        className="w-full max-w-sm rounded-2xl border border-white/10 bg-ink-900/95 p-5 shadow-2xl shadow-black/60"
        onClick={(event) => event.stopPropagation()}
      >
        <h2 className="mb-3 text-base font-semibold text-neutral-100">{t("adjustImage")}</h2>

        <div
          className="relative mx-auto touch-none overflow-hidden rounded-xl border border-white/15 bg-ink-950"
          style={{ width: FRAME_WIDTH, height: frameHeight, cursor: drag.current ? "grabbing" : "grab" }}
          onPointerDown={(event) => {
            if (!crop) return;
            drag.current = { x: event.clientX, y: event.clientY, crop };
            event.currentTarget.setPointerCapture(event.pointerId);
          }}
          onPointerMove={(event) => move(event.clientX, event.clientY)}
          onPointerUp={() => {
            drag.current = null;
          }}
          onPointerCancel={() => {
            drag.current = null;
          }}
        >
          {url && size && crop ? (
            <>
              {zoomedOut ? (
                // What the render does with the gap, shown here too, so the
                // preview is not a promise the saved picture will break.
                <img
                  src={url}
                  alt=""
                  aria-hidden
                  draggable={false}
                  className="absolute inset-0 h-full w-full scale-125 object-cover select-none"
                  style={{ filter: `blur(${BACKDROP_BLUR * display}px)` }}
                />
              ) : null}
              <img
                src={url}
                alt=""
                draggable={false}
                className="relative max-w-none select-none"
                style={{
                  width: size.width * scale * display,
                  height: size.height * scale * display,
                  transform: `translate(${crop.x * display}px, ${crop.y * display}px)`,
                }}
              />
            </>
          ) : null}
        </div>

        <p className="mt-2 text-center text-[11px] text-neutral-500">
          {target.width} × {target.height} · {t("cropHint")}
        </p>

        <label className="mt-3 flex items-center gap-3">
          <span className="text-xs text-neutral-400">{t("zoom")}</span>
          <input
            type="range"
            min={minZoom}
            max={MAX_ZOOM}
            step={0.02}
            value={crop?.zoom ?? 1}
            onChange={(event) => zoomTo(Number(event.target.value))}
            disabled={!size}
            className="h-1 flex-1 cursor-pointer appearance-none rounded-full bg-ink-700 accent-[var(--color-accent)]"
          />
        </label>

        <div className="mt-4 flex gap-2">
          <button
            type="button"
            onClick={() => onDone(null)}
            className="flex-1 rounded-xl border border-white/10 py-2 text-sm text-neutral-300 transition hover:border-white/25"
          >
            {t("close")}
          </button>
          <button
            type="button"
            onClick={() => void apply()}
            disabled={!crop || busy}
            className="flex-1 rounded-xl bg-accent/90 py-2 text-sm font-semibold text-ink-950 transition hover:bg-accent disabled:opacity-50"
          >
            {t("apply")}
          </button>
        </div>
      </div>
    </div>
  );
}
