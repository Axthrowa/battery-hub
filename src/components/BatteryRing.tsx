interface BatteryRingProps {
  percent: number | null;
  charging: boolean;
  online: boolean;
  animated: boolean;
}

const SIZE = 208;
const STROKE = 14;
const RADIUS = (SIZE - STROKE) / 2;
const CIRCUMFERENCE = 2 * Math.PI * RADIUS;

export function levelColor(percent: number | null, online: boolean) {
  if (!online || percent === null) return "#4b5563";
  if (percent >= 50) return "#00e676";
  if (percent >= 20) return "#ffb300";
  return "#ff1744";
}

export function BatteryRing({ percent, charging, online, animated }: BatteryRingProps) {
  const value = online && percent !== null ? Math.max(0, Math.min(100, percent)) : 0;
  const color = levelColor(percent, online);
  const offset = CIRCUMFERENCE * (1 - value / 100);

  return (
    <div className="relative grid place-items-center">
      <div
        className="absolute h-44 w-44 rounded-full blur-2xl transition-opacity duration-700"
        style={{ backgroundColor: color, opacity: online ? 0.16 : 0.05 }}
      />
      <svg width={SIZE} height={SIZE} viewBox={`0 0 ${SIZE} ${SIZE}`} className="-rotate-90">
        <circle
          cx={SIZE / 2}
          cy={SIZE / 2}
          r={RADIUS}
          fill="none"
          stroke="#1b1f24"
          strokeWidth={STROKE}
        />
        <circle
          cx={SIZE / 2}
          cy={SIZE / 2}
          r={RADIUS}
          fill="none"
          stroke={color}
          strokeWidth={STROKE}
          strokeLinecap="round"
          strokeDasharray={CIRCUMFERENCE}
          strokeDashoffset={offset}
          style={{
            transition: "stroke-dashoffset 700ms ease, stroke 400ms ease",
            filter: `drop-shadow(0 0 8px ${color}66)`,
          }}
        />
      </svg>

      <div className="absolute flex flex-col items-center">
        <div className="flex items-start">
          <span
            className="text-6xl leading-none font-semibold tabular-nums tracking-tight"
            style={{ color: online ? "#f3f5f7" : "#6b7280" }}
          >
            {online && percent !== null ? value : "--"}
          </span>
          <span className="mt-2 ml-1 text-2xl font-medium text-neutral-500">%</span>
        </div>
        {charging && online ? (
          <span
            className={`mt-2 text-xl ${animated ? "animate-pulse" : ""}`}
            style={{ color }}
            aria-hidden
          >
            ⚡
          </span>
        ) : null}
      </div>
    </div>
  );
}
