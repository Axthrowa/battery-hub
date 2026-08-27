import { imageKey, useImageMap } from "./imageMap";
import type { ImageMap } from "./imageMap";

/** Device name (normalised) -> PNG data URI. */
export type LogoMap = ImageMap;

const STORE_FILE = "logos.json";
const STORE_KEY = "logos";
/** Logos live inside a JSON store, so they are downscaled before saving. */
const LOGO_SIZE = 128;

/** Keyed by name so a logo survives reconnects and follows a rename. */
export const logoKey = imageKey;

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
  const { map, imageFor, setImage, clearImage, moveImage } = useImageMap(STORE_FILE, STORE_KEY);
  return {
    logos: map,
    logoFor: imageFor,
    setLogo: setImage,
    clearLogo: clearImage,
    moveLogo: moveImage,
  };
}
