import type { DeviceBrand } from "../lib/bridge";

interface BrandLogoProps {
  brand: DeviceBrand;
  size?: number;
  className?: string;
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

export function BrandLogo({ brand, size = 22, className = "" }: BrandLogoProps) {
  const { color, src } = resolve(brand);

  return (
    <span
      className={`inline-grid place-items-center rounded-xl border border-white/10 bg-black/35 p-2 ${className}`}
      style={{ boxShadow: `0 0 18px ${color}22` }}
      aria-hidden
    >
      <img
        src={src}
        alt=""
        width={size}
        height={size}
        draggable={false}
        style={{ display: "block", objectFit: "contain" }}
      />
    </span>
  );
}
