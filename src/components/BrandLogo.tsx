import type { DeviceBrand, DeviceKind } from "../lib/bridge";

// A device with no artwork and a brand we have no logo for used to fall back
// to one shared placeholder, so a keyboard, a mouse and a headset all looked
// alike. Drawing what the thing actually is costs nothing and reads instantly.
const KIND_PATHS: Partial<Record<DeviceKind, string>> = {
  keyboard:
    "M3 6.5h18a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1v-9a1 1 0 0 1 1-1Zm2.5 3v1.5H7V9.5H5.5Zm3 0v1.5H10V9.5H8.5Zm3 0v1.5H13V9.5h-1.5Zm3 0v1.5H16V9.5h-1.5Zm3 0v1.5H19V9.5h-1.5Zm-12 3v1.5H7v-1.5H5.5Zm3 0v1.5H16v-1.5H8.5Zm9 0v1.5H19v-1.5h-1.5Z",
  mouse:
    "M12 2a6 6 0 0 0-6 6v8a6 6 0 0 0 12 0V8a6 6 0 0 0-6-6Zm-.75 4.5h1.5v3.75h-1.5V6.5Z",
  headset:
    "M12 3a8 8 0 0 0-8 8v5.5A2.5 2.5 0 0 0 6.5 19H8a1 1 0 0 0 1-1v-4a1 1 0 0 0-1-1H6v-2a6 6 0 0 1 12 0v2h-2a1 1 0 0 0-1 1v4a1 1 0 0 0 1 1h1.5A2.5 2.5 0 0 0 20 16.5V11a8 8 0 0 0-8-8Z",
};

interface BrandLogoProps {
  brand: DeviceBrand;
  kind?: DeviceKind;
  size?: number;
  className?: string;
  /** Custom artwork the user picked for this device. */
  logo?: string;
  onPick?: () => void;
  onClear?: () => void;
  pickLabel?: string;
  clearLabel?: string;
}

const KNOWN: Record<string, { color: string; src: string }> = {
  razer: { color: "#44d62c", src: "/brands/razer.svg" },
  logitech: { color: "#00b8fc", src: "/brands/logitech.svg" },
  ajazz: { color: "#d81e06", src: "/brands/ajazz.png" },
  soundcore: { color: "#00c2a8", src: "/brands/soundcore.svg" },
  anker: { color: "#00c2a8", src: "/brands/soundcore.svg" },
};

const FALLBACK = { color: "#8b9bb4", src: "/brands/generic.svg" };

function resolve(brand: string) {
  return KNOWN[brand.toLowerCase()] ?? FALLBACK;
}

export function brandAccent(brand: DeviceBrand) {
  return resolve(brand).color;
}

export function BrandLogo({
  brand,
  kind,
  size = 22,
  className = "",
  logo,
  onPick,
  onClear,
  pickLabel,
  clearLabel,
}: BrandLogoProps) {
  const { color, src } = resolve(brand);
  const known = src !== FALLBACK.src;
  const shape = kind ? KIND_PATHS[kind] : undefined;
  const image =
    !logo && !known && shape ? (
      <svg
        viewBox="0 0 24 24"
        width={size}
        height={size}
        fill={color}
        style={{ display: "block" }}
        aria-hidden
      >
        <path d={shape} />
      </svg>
    ) : (
      <img
        src={logo ?? src}
        alt=""
        width={size}
        height={size}
        draggable={false}
        style={{ display: "block", objectFit: "contain" }}
      />
    );
  const shell = `inline-grid place-items-center rounded-xl border border-white/10 bg-black/35 p-2 ${className}`;

  if (!onPick) {
    return (
      <span className={shell} style={{ boxShadow: `0 0 18px ${color}22` }} aria-hidden>
        {image}
      </span>
    );
  }

  return (
    <span className="group relative shrink-0">
      <button
        type="button"
        onClick={onPick}
        title={pickLabel}
        aria-label={pickLabel}
        className={`${shell} cursor-pointer transition hover:border-accent/50`}
        style={{ boxShadow: `0 0 18px ${color}22` }}
      >
        {image}
      </button>
      {logo && onClear ? (
        <button
          type="button"
          onClick={onClear}
          title={clearLabel}
          aria-label={clearLabel}
          className="absolute -top-1 -right-1 hidden h-4 w-4 place-items-center rounded-full border border-white/15 bg-ink-900 text-[9px] leading-none text-neutral-400 transition group-hover:grid hover:text-red-300"
        >
          ✕
        </button>
      ) : null}
    </span>
  );
}
