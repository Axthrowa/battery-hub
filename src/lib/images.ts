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

/** Centre-crop to the frame's shape, scale to its size, re-encode as JPEG. */
export async function toCroppedDataUri(file: File, target: ImageTarget): Promise<string> {
  const bitmap = await createImageBitmap(file);
  try {
    const canvas = document.createElement("canvas");
    canvas.width = target.width;
    canvas.height = target.height;
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("canvas unavailable");

    // Cover: the larger of the two ratios leaves no gap on either axis.
    const scale = Math.max(target.width / bitmap.width, target.height / bitmap.height);
    const width = bitmap.width * scale;
    const height = bitmap.height * scale;
    ctx.drawImage(bitmap, (target.width - width) / 2, (target.height - height) / 2, width, height);
    return canvas.toDataURL("image/jpeg", QUALITY);
  } finally {
    bitmap.close();
  }
}

/** Open a picker and return the cropped image, or `null` if nothing was chosen. */
export function pickImage(target: ImageTarget): Promise<string | null> {
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
      toCroppedDataUri(file, target)
        .then(resolve)
        .catch((err) => {
          console.error("image import failed", err);
          resolve(null);
        })
        .finally(cleanup);
    };
    // A cancelled picker fires no event; drop the element on the next focus.
    window.addEventListener("focus", () => window.setTimeout(cleanup, 500), { once: true });
    input.click();
  });
}
