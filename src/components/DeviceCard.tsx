import type { DeviceReading } from "../lib/bridge";
import { BrandLogo, brandAccent } from "./BrandLogo";
import { levelColor } from "./BatteryRing";

interface DeviceCardProps {
  device: DeviceReading;
  index: number;
  chargingLabel: string;
  offlineLabel: string;
  locale: string;
  logo?: string;
  onPickLogo?: () => void;
  onClearLogo?: () => void;
  pickLogoLabel?: string;
  clearLogoLabel?: string;
}

export function DeviceCard({
  device,
  index,
  chargingLabel,
  offlineLabel,
  locale,
  logo,
  onPickLogo,
  onClearLogo,
  pickLogoLabel,
  clearLogoLabel,
}: DeviceCardProps) {
  const online = device.ok;
  const percent = device.percent;
  const color = levelColor(percent, online);
  const accent = brandAccent(device.brand);
  const timeLabel = device.updatedAtMs
    ? new Date(device.updatedAtMs).toLocaleTimeString(locale === "tr" ? "tr-TR" : "en-US", {
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
      })
    : "--:--:--";

  const barWidth = online && percent !== null ? Math.max(0, Math.min(100, percent)) : 0;

  return (
    <article
      className="card-enter rounded-2xl border border-white/10 bg-ink-850/75 p-3.5 backdrop-blur-sm transition hover:border-white/18"
      style={{
        animationDelay: `${index * 55}ms`,
        boxShadow: online ? `inset 0 0 0 1px ${accent}18` : undefined,
      }}
    >
      <div className="mb-3 flex items-start gap-3">
        <BrandLogo
          brand={device.brand}
          size={20}
          logo={logo}
          onPick={onPickLogo}
          onClear={onClearLogo}
          pickLabel={pickLogoLabel}
          clearLabel={clearLogoLabel}
        />
        <div className="min-w-0 flex-1">
          <div className="flex items-center justify-between gap-2">
            <h3 className="truncate text-sm font-semibold text-neutral-100">
              {device.product || device.brandLabel}
            </h3>
            <span
              className={`inline-flex items-center gap-1.5 text-[11px] font-medium ${
                online ? "text-neutral-300" : device.present ? "text-amber-400/90" : "text-neutral-500"
              }`}
            >
              <span
                className={`h-1.5 w-1.5 rounded-full ${online || device.present ? "pulse-dot" : ""}`}
                style={{
                  backgroundColor: online ? accent : device.present ? "#f59e0b" : "#4b5563",
                }}
              />
              {online
                ? device.transport || "OK"
                : device.present
                  ? "algılandı"
                  : offlineLabel}
            </span>
          </div>
          <p className="mt-0.5 truncate text-[11px] text-neutral-500">{device.brandLabel}</p>
        </div>
      </div>

      <div className="mb-2 flex items-end justify-between">
        <div className="flex items-start">
          <span
            className="text-3xl leading-none font-semibold tabular-nums tracking-tight"
            style={{ color: online ? "#f3f5f7" : "#6b7280" }}
          >
            {online && percent !== null ? percent : "—"}
          </span>
          <span className="mt-1 ml-1 text-sm font-medium text-neutral-500">%</span>
        </div>
        {device.charging && online ? (
          <span className="text-xs font-medium" style={{ color }}>
            {chargingLabel}
          </span>
        ) : null}
      </div>

      <div className="h-1.5 overflow-hidden rounded-full bg-ink-700">
        <div
          className="h-full rounded-full transition-[width,background-color] duration-700 ease-out"
          style={{ width: `${barWidth}%`, backgroundColor: color }}
        />
      </div>

      <p className="mt-2 text-[10px] tabular-nums text-neutral-600">{timeLabel}</p>
    </article>
  );
}
