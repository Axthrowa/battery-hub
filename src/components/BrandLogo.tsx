import type { DeviceBrand } from "../lib/bridge";

interface BrandLogoProps {
  brand: DeviceBrand;
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
  size = 22,
  className = "",
  logo,
  onPick,
  onClear,
  pickLabel,
  clearLabel,
}: BrandLogoProps) {
  const { color, src } = resolve(brand);
  const image = (
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
