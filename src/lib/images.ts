import { useImageMap } from "./imageMap";

/**
 * Bringing a picture in from disk.
 *
 * Whatever someone picks is almost never the shape of the frame it has to
 * fill, so it is cropped here rather than left to the browser: centred, scaled
 * until it covers, and the overflow cut away. The settings panel still names
 * the size each frame wants, so anyone who would rather crop it themselves
 * knows what to crop it to — but nothing has to be prepared for this to work.
 *
 * The result is re-encoded small on purpose. It is stored as a data URI in the
 * settings, which are read on every launch and mirrored into localStorage, and
 * a phone photo dropped in whole would be several megabytes of that.
 */
export interface ImageTarget {
  width: number;
  height: number;
}

/** The window at 1.5x its default size, so it stays sharp when resized. */
export const BACKGROUND_TARGET: ImageTarget = { width: 800, height: 1100 };

/** The device list is as wide as the panel and grows with the cards. */
export const DEVICES_TARGET: ImageTarget = { width: 900, height: 700 };

/** One card, at twice the size it takes in the two-column layout. */
export const CARD_TARGET: ImageTarget = { width: 480, height: 300 };

const QUALITY = 0.82;

/**
 * Backdrops kept per device, beside the logos and handled the same way — see
 * `imageMap`. A separate store because they are a different size of thing:
 * losing every card's picture should not be the price of clearing the logos.
 */
export function useCardImages() {
  const { imageFor, setImage, clearImage, moveImage } = useImageMap(
    "card-images.json",
    "cardImages",
  );
  return {
    cardImageFor: imageFor,
    setCardImage: setImage,
    clearCardImage: clearImage,
    moveCardImage: moveImage,
  };
}

/**
 * Where the picture sits inside the frame, in frame units.
 *
 * `zoom` is a multiple of the smallest size that still covers the frame, so 1
 * is "no empty corners" and anything above it is closer in. `x` and `y` are
 * the top-left of the drawn picture relative to the frame's top-left, which
 * are negative whenever the picture is larger than the frame — which, at zoom
 * 1 or more, it always is.
 */
export interface Crop {
  zoom: number;
  x: number;
  y: number;
}

/** The size at which the picture just covers the frame. */
export function coverScale(image: { width: number; height: number }, target: ImageTarget): number {
  return Math.max(target.width / image.width, target.height / image.height);
}

/** Centred, and no more zoomed in than it has to be. */
export function centredCrop(
  image: { width: number; height: number },
  target: ImageTarget,
): Crop {
  const base = coverScale(image, target);
  return {
    zoom: 1,
    x: (target.width - image.width * base) / 2,
    y: (target.height - image.height * base) / 2,
  };
}

/** Hold the picture against the frame's edges: never a gap, never a stray pan. */
export function clampCrop(
  crop: Crop,
  image: { width: number; height: number },
  target: ImageTarget,
): Crop {
  const scale = coverScale(image, target) * crop.zoom;
  const width = image.width * scale;
  const height = image.height * scale;
  return {
    zoom: crop.zoom,
    x: Math.min(0, Math.max(target.width - width, crop.x)),
    y: Math.min(0, Math.max(target.height - height, crop.y)),
  };
}

/** Draw the chosen part of the picture at the frame's real size. */
export async function renderCrop(
  file: File,
  target: ImageTarget,
  crop?: Crop,
): Promise<string> {
  const bitmap = await createImageBitmap(file);
  try {
    const canvas = document.createElement("canvas");
    canvas.width = target.width;
    canvas.height = target.height;
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("canvas unavailable");

    const placed = clampCrop(crop ?? centredCrop(bitmap, target), bitmap, target);
    const scale = coverScale(bitmap, target) * placed.zoom;
    ctx.drawImage(bitmap, placed.x, placed.y, bitmap.width * scale, bitmap.height * scale);
    return canvas.toDataURL("image/jpeg", QUALITY);
  } finally {
    bitmap.close();
  }
}

/** Open the OS picker and hand back the file, or `null` if nothing was chosen. */
export function pickImageFile(): Promise<File | null> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "image/*";
    input.style.display = "none";
    document.body.appendChild(input);

    const cleanup = () => input.remove();
    input.onchange = () => {
      const file = input.files?.[0] ?? null;
      cleanup();
      resolve(file);
    };
    // A cancelled picker fires no event; drop the element on the next focus.
    window.addEventListener("focus", () => window.setTimeout(cleanup, 500), { once: true });
    input.click();
  });
}
