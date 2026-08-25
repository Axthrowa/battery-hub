import { useTranslation } from "react-i18next";
import { POLL_OPTIONS } from "../i18n/resources";
import type { Locale } from "../i18n/resources";
import { useSettings } from "../context/SettingsContext";

interface SettingsModalProps {
  open: boolean;
  onClose: () => void;
}

const LOCALES: { id: Locale; labelKey: "turkish" | "english"; flag: string }[] = [
  { id: "tr", labelKey: "turkish", flag: "TR" },
  { id: "en", labelKey: "english", flag: "EN" },
];

export function SettingsModal({ open, onClose }: SettingsModalProps) {
  const { t } = useTranslation();
  const { settings, update } = useSettings();

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-end justify-center bg-black/60 backdrop-blur-sm sm:items-center"
      onClick={onClose}
    >
      <div
        className="w-full max-w-sm rounded-2xl border border-white/10 bg-ink-900/95 p-5 shadow-2xl shadow-black/60"
        onClick={(event) => event.stopPropagation()}
      >
        <header className="mb-5 flex items-center justify-between">
          <h2 className="text-base font-semibold text-neutral-100">{t("settings")}</h2>
          <button
            type="button"
            onClick={onClose}
            className="grid h-8 w-8 place-items-center rounded-lg text-neutral-400 transition hover:bg-white/10 hover:text-neutral-100"
            aria-label={t("close")}
          >
            ✕
          </button>
        </header>

        <section className="mb-5">
          <p className="mb-2 text-xs font-medium tracking-wide text-neutral-400 uppercase">
            {t("language")}
          </p>
          <div className="grid grid-cols-2 gap-2">
            {LOCALES.map((item) => {
              const active = settings.locale === item.id;
              return (
                <button
                  key={item.id}
                  type="button"
                  onClick={() => void update({ locale: item.id })}
                  className={`rounded-xl border px-3 py-2.5 text-sm transition ${
                    active
                      ? "border-accent/60 bg-accent/15 text-accent"
                      : "border-white/10 bg-ink-800 text-neutral-300 hover:border-white/25"
                  }`}
                >
                  <span className="mr-2 text-xs font-semibold opacity-70">{item.flag}</span>
                  {t(item.labelKey)}
                </button>
              );
            })}
          </div>
        </section>

        <section className="mb-5">
          <p className="mb-1 text-xs font-medium tracking-wide text-neutral-400 uppercase">
            {t("pollInterval")}
          </p>
          <p className="mb-2 text-xs text-neutral-500">{t("pollIntervalHint")}</p>
          <div className="grid grid-cols-4 gap-2">
            {POLL_OPTIONS.map((seconds) => {
              const active = settings.pollSeconds === seconds;
              return (
                <button
                  key={seconds}
                  type="button"
                  onClick={() => void update({ pollSeconds: seconds })}
                  className={`rounded-xl border py-2.5 text-sm tabular-nums transition ${
                    active
                      ? "border-accent/60 bg-accent/15 text-accent"
                      : "border-white/10 bg-ink-800 text-neutral-300 hover:border-white/25"
                  }`}
                >
                  {seconds}
                  <span className="ml-0.5 text-[10px] text-neutral-500">s</span>
                </button>
              );
            })}
          </div>
        </section>

        <section className="rounded-xl border border-white/10 bg-ink-800 p-3.5">
          <label className="flex cursor-pointer items-center justify-between gap-3">
            <span>
              <span className="block text-sm text-neutral-200">{t("autostart")}</span>
              <span className="block text-xs text-neutral-500">{t("autostartHint")}</span>
            </span>
            <span className="relative inline-flex shrink-0">
              <input
                type="checkbox"
                className="peer sr-only"
                checked={settings.autostart}
                onChange={(event) => void update({ autostart: event.target.checked })}
              />
              <span className="h-6 w-11 rounded-full bg-ink-700 transition peer-checked:bg-accent/70" />
              <span className="absolute top-0.5 left-0.5 h-5 w-5 rounded-full bg-neutral-300 transition peer-checked:translate-x-5 peer-checked:bg-white" />
            </span>
          </label>
        </section>

        <footer className="mt-5">
          <button
            type="button"
            onClick={onClose}
            className="w-full rounded-xl bg-accent/90 py-2.5 text-sm font-semibold text-ink-950 transition hover:bg-accent"
          >
            {t("close")}
          </button>
        </footer>
      </div>
    </div>
  );
}
