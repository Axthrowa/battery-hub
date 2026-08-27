import type { CSSProperties, ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { ACCENTS, POLL_OPTIONS } from "../i18n/resources";
import type { Locale } from "../i18n/resources";
import { useSettings } from "../context/SettingsContext";
import type { Theme } from "../context/SettingsContext";

interface SettingsModalProps {
  open: boolean;
  onClose: () => void;
}

const LOCALES: { id: Locale; labelKey: "turkish" | "english"; flag: string }[] = [
  { id: "tr", labelKey: "turkish", flag: "TR" },
  { id: "en", labelKey: "english", flag: "EN" },
];

const THEMES: { id: Theme; labelKey: "themeSystem" | "themeDark" | "themeLight" }[] = [
  { id: "system", labelKey: "themeSystem" },
  { id: "dark", labelKey: "themeDark" },
  { id: "light", labelKey: "themeLight" },
];

function Section({ title, hint, children }: { title: string; hint?: string; children: ReactNode }) {
  return (
    <section className="mb-5">
      <p className="mb-1 text-xs font-medium tracking-wide text-neutral-400 uppercase">{title}</p>
      {hint ? <p className="mb-2 text-xs text-neutral-500">{hint}</p> : <div className="mb-2" />}
      {children}
    </section>
  );
}

function Choice({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`rounded-xl border px-3 py-2.5 text-sm transition ${
        active
          ? "border-accent/60 bg-accent/15 text-accent"
          : "border-white/10 bg-ink-800 text-neutral-300 hover:border-white/25"
      }`}
    >
      {children}
    </button>
  );
}

function Toggle({
  label,
  hint,
  checked,
  onChange,
}: {
  label: string;
  hint: string;
  checked: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <label className="flex cursor-pointer items-center justify-between gap-3 rounded-xl border border-white/10 bg-ink-800 p-3.5">
      <span>
        <span className="block text-sm text-neutral-200">{label}</span>
        <span className="block text-xs text-neutral-500">{hint}</span>
      </span>
      <span className="relative inline-flex shrink-0">
        <input
          type="checkbox"
          className="peer sr-only"
          checked={checked}
          onChange={(event) => onChange(event.target.checked)}
        />
        <span className="h-6 w-11 rounded-full bg-ink-700 transition peer-checked:bg-accent/70" />
        <span className="knob absolute top-0.5 left-0.5 h-5 w-5 rounded-full bg-neutral-300 transition peer-checked:translate-x-5" />
      </span>
    </label>
  );
}

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
        className="flex max-h-[88vh] w-full max-w-sm flex-col rounded-2xl border border-white/10 bg-ink-900/95 p-5 shadow-2xl shadow-black/60"
        onClick={(event) => event.stopPropagation()}
      >
        <header className="mb-5 flex shrink-0 items-center justify-between">
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

        <div className="-mr-1 min-h-0 flex-1 overflow-y-auto pr-1">
          <Section title={t("language")}>
            <div className="grid grid-cols-2 gap-2">
              {LOCALES.map((item) => (
                <Choice
                  key={item.id}
                  active={settings.locale === item.id}
                  onClick={() => void update({ locale: item.id })}
                >
                  <span className="mr-2 text-xs font-semibold opacity-70">{item.flag}</span>
                  {t(item.labelKey)}
                </Choice>
              ))}
            </div>
          </Section>

          <Section title={t("theme")} hint={t("themeHint")}>
            <div className="grid grid-cols-3 gap-2">
              {THEMES.map((item) => (
                <Choice
                  key={item.id}
                  active={settings.theme === item.id}
                  onClick={() => void update({ theme: item.id })}
                >
                  {t(item.labelKey)}
                </Choice>
              ))}
            </div>
          </Section>

          <Section title={t("accentColor")}>
            <div className="flex flex-wrap gap-2.5">
              {ACCENTS.map((accent) => {
                const active = settings.accent === accent.value;
                return (
                  <button
                    key={accent.id}
                    type="button"
                    onClick={() => void update({ accent: accent.value })}
                    aria-label={accent.id}
                    aria-pressed={active}
                    className={`accent-swatch h-9 w-9 rounded-full transition ${
                      active
                        ? "ring-2 ring-neutral-100 ring-offset-2 ring-offset-ink-900"
                        : "ring-1 ring-white/15 hover:ring-white/40"
                    }`}
                    style={{ "--swatch": accent.value } as CSSProperties}
                  />
                );
              })}
            </div>
          </Section>

          <Section title={t("pollInterval")} hint={t("pollIntervalHint")}>
            <div className="grid grid-cols-4 gap-2">
              {POLL_OPTIONS.map((seconds) => (
                <Choice
                  key={seconds}
                  active={settings.pollSeconds === seconds}
                  onClick={() => void update({ pollSeconds: seconds })}
                >
                  <span className="tabular-nums">{seconds}</span>
                  <span className="ml-0.5 text-[10px] text-neutral-500">s</span>
                </Choice>
              ))}
            </div>
          </Section>

          <Section title={t("notifications")}>
            <Toggle
              label={t("notificationSound")}
              hint={t("notificationSoundHint")}
              checked={settings.notificationSound}
              onChange={(value) => void update({ notificationSound: value })}
            />
          </Section>

          <Toggle
            label={t("autostart")}
            hint={t("autostartHint")}
            checked={settings.autostart}
            onChange={(value) => void update({ autostart: value })}
          />
        </div>

        <footer className="mt-5 shrink-0">
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
