import type { CSSProperties, ReactNode } from "react";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ACCENTS, BACKGROUNDS, POLL_OPTIONS } from "../i18n/resources";
import type { Background, Locale } from "../i18n/resources";
import { useSettings } from "../context/SettingsContext";
import { setNotificationSoundFile, testNotificationSound } from "../lib/bridge";
import type { Theme } from "../context/SettingsContext";

interface SettingsModalProps {
  open: boolean;
  onClose: () => void;
}

const LOCALES: { id: Locale; labelKey: "turkish" | "english"; flag: string }[] = [
  { id: "tr", labelKey: "turkish", flag: "TR" },
  { id: "en", labelKey: "english", flag: "EN" },
];

// The tile shows the ground itself, drawn by the same CSS the window uses, so
// what is on the button is what the panel becomes.
const BACKGROUND_LABEL: Record<Background, "bgAurora" | "bgMesh" | "bgGrid" | "bgPlain"> = {
  aurora: "bgAurora",
  mesh: "bgMesh",
  grid: "bgGrid",
  plain: "bgPlain",
};

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
  const soundInput = useRef<HTMLInputElement>(null);
  const [soundError, setSoundError] = useState<string | null>(null);

  // The file goes to Rust, which is what has to play it; only the name is kept
  // here, so the panel can say which one is in force.
  const pickSound = async (file: File | null) => {
    if (!file) return;
    setSoundError(null);
    const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
    const refused = await setNotificationSoundFile(bytes);
    if (refused) {
      setSoundError(refused);
      return;
    }
    await update({ soundFileName: file.name });
  };

  const clearSound = async () => {
    setSoundError(null);
    await setNotificationSoundFile(null);
    await update({ soundFileName: null });
  };

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
              <label
                className="relative grid h-9 w-9 cursor-pointer place-items-center rounded-full ring-1 ring-white/15 transition hover:ring-white/40"
                title={t("customColor")}
                style={{ backgroundColor: "var(--color-ink-800)" }}
              >
                <span className="text-[15px] leading-none text-neutral-400">+</span>
                <input
                  type="color"
                  className="absolute inset-0 cursor-pointer opacity-0"
                  value={settings.accent}
                  onChange={(event) => void update({ accent: event.target.value })}
                  aria-label={t("customColor")}
                />
              </label>
            </div>
          </Section>

          <Section title={t("background")}>
            <div className="grid grid-cols-4 gap-2">
              {BACKGROUNDS.map((id) => {
                const active = settings.background === id;
                return (
                  <button
                    key={id}
                    type="button"
                    onClick={() => void update({ background: id })}
                    aria-pressed={active}
                    className={`overflow-hidden rounded-xl border text-center transition ${
                      active
                        ? "border-accent/60 bg-accent/10"
                        : "border-white/10 bg-ink-800 hover:border-white/25"
                    }`}
                  >
                    <span className={`ground-preview-${id} block h-11 w-full`} />
                    <span
                      className={`block py-1.5 text-[11px] ${
                        active ? "text-accent" : "text-neutral-400"
                      }`}
                    >
                      {t(BACKGROUND_LABEL[id])}
                    </span>
                  </button>
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

            <div className="mt-2 rounded-xl border border-white/10 bg-ink-800 p-3.5">
              <p className="text-sm text-neutral-200">{t("soundFile")}</p>
              <p className="mt-0.5 text-xs text-neutral-500">{t("soundFileHint")}</p>
              <p className="mt-2 truncate text-xs text-neutral-400">
                {settings.soundFileName ?? t("soundFileDefault")}
              </p>
              {soundError ? (
                <p className="mt-1 text-xs text-red-300">{soundError}</p>
              ) : null}
              <div className="mt-2.5 flex gap-2">
                <button
                  type="button"
                  onClick={() => soundInput.current?.click()}
                  className="flex-1 rounded-lg border border-white/10 py-1.5 text-xs text-neutral-300 transition hover:border-white/25"
                >
                  {t("choose")}
                </button>
                {settings.soundFileName ? (
                  <button
                    type="button"
                    onClick={() => void clearSound()}
                    className="flex-1 rounded-lg border border-white/10 py-1.5 text-xs text-neutral-300 transition hover:border-white/25"
                  >
                    {t("remove")}
                  </button>
                ) : null}
                <button
                  type="button"
                  onClick={() => void testNotificationSound()}
                  disabled={!settings.notificationSound}
                  className="flex-1 rounded-lg border border-accent/40 py-1.5 text-xs text-accent transition hover:border-accent/70 disabled:cursor-not-allowed disabled:opacity-40"
                >
                  {t("test")}
                </button>
              </div>
              <input
                ref={soundInput}
                type="file"
                accept="audio/wav,.wav"
                className="hidden"
                onChange={(event) => void pickSound(event.target.files?.[0] ?? null)}
              />
            </div>
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
