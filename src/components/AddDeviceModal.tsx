import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { CandidateValue, DeviceCandidate, LearnedDevice } from "../lib/bridge";
import {
  addLearnedDevice,
  learnedDevices,
  removeLearnedDevice,
  requestRefresh,
  scanDevices,
} from "../lib/bridge";

interface AddDeviceModalProps {
  open: boolean;
  onClose: () => void;
}

function deviceKey(candidate: DeviceCandidate, value: CandidateValue) {
  const hex = (n: number, width: number) => n.toString(16).toUpperCase().padStart(width, "0");
  return [
    hex(candidate.vendorId, 4),
    hex(candidate.productId, 4),
    hex(value.usagePage, 4),
    hex(value.reportId, 2),
    value.byteOffset,
  ].join(":");
}

export function AddDeviceModal({ open, onClose }: AddDeviceModalProps) {
  const { t } = useTranslation();
  const [scanning, setScanning] = useState(false);
  const [scanned, setScanned] = useState(false);
  const [candidates, setCandidates] = useState<DeviceCandidate[]>([]);
  const [added, setAdded] = useState<LearnedDevice[]>([]);
  const [picked, setPicked] = useState<Record<string, number>>({});
  const [names, setNames] = useState<Record<string, string>>({});

  useEffect(() => {
    if (!open) return;
    void learnedDevices().then((list) => setAdded(list ?? []));
  }, [open]);

  const scan = useCallback(async () => {
    setScanning(true);
    const found = await scanDevices();
    setCandidates(found ?? []);
    setScanned(true);
    setScanning(false);
  }, []);

  const add = useCallback(
    async (candidate: DeviceCandidate, value: CandidateValue) => {
      const list = await addLearnedDevice({
        id: deviceKey(candidate, value),
        name: (names[candidate.id] ?? candidate.name).trim() || candidate.name,
        vendorId: candidate.vendorId,
        productId: candidate.productId,
        usagePage: value.usagePage,
        reportId: value.reportId,
        byteOffset: value.byteOffset,
        maxValue: 100,
      });
      if (list) setAdded(list);
      setCandidates((current) =>
        current.map((item) => (item.id === candidate.id ? { ...item, added: true } : item)),
      );
      void requestRefresh();
    },
    [names],
  );

  const rename = useCallback(async (device: LearnedDevice, name: string) => {
    const trimmed = name.trim();
    if (!trimmed || trimmed === device.name) return;
    // The backend upserts by id, so re-adding with a new name renames it.
    const list = await addLearnedDevice({ ...device, name: trimmed });
    if (list) setAdded(list);
    void requestRefresh();
  }, []);

  const drop = useCallback(async (id: string) => {
    const list = await removeLearnedDevice(id);
    if (list) setAdded(list);
    void requestRefresh();
  }, []);

  if (!open) return null;

  const addable = candidates.filter((item) => !item.automatic && item.values.length > 0);

  return (
    <div
      className="fixed inset-0 z-50 flex items-end justify-center bg-black/60 backdrop-blur-sm sm:items-center"
      onClick={onClose}
    >
      <div
        className="flex max-h-[85vh] w-full max-w-sm flex-col rounded-2xl border border-white/10 bg-ink-900/95 p-5 shadow-2xl shadow-black/60"
        onClick={(event) => event.stopPropagation()}
      >
        <header className="mb-4 flex items-center justify-between">
          <h2 className="text-base font-semibold text-neutral-100">{t("addDeviceTitle")}</h2>
          <button
            type="button"
            onClick={onClose}
            className="grid h-8 w-8 place-items-center rounded-lg text-neutral-400 transition hover:bg-white/10 hover:text-neutral-100"
            aria-label={t("close")}
          >
            ✕
          </button>
        </header>

        <p className="mb-3 text-xs text-neutral-500">{t("addDeviceHint")}</p>

        <button
          type="button"
          onClick={() => void scan()}
          disabled={scanning}
          className="mb-4 w-full rounded-xl bg-accent py-2.5 text-sm font-semibold text-ink-950 transition hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {scanning ? t("scanning") : scanned ? t("scanAgain") : t("scan")}
        </button>

        <div className="min-h-0 flex-1 overflow-y-auto pr-0.5">
          {scanned && addable.length === 0 ? (
            <p className="rounded-xl border border-white/8 bg-ink-850/50 px-3 py-3 text-center text-xs text-neutral-500">
              {t("scanEmpty")}
            </p>
          ) : null}

          {addable.map((candidate) => {
            const chosen = picked[candidate.id] ?? 0;
            const value = candidate.values[chosen];
            return (
              <section
                key={candidate.id}
                className="mb-3 rounded-xl border border-white/10 bg-ink-850/60 p-3"
              >
                <input
                  value={names[candidate.id] ?? candidate.name}
                  onChange={(event) =>
                    setNames((current) => ({ ...current, [candidate.id]: event.target.value }))
                  }
                  aria-label={t("deviceName")}
                  className="mb-1 w-full rounded-lg border border-white/10 bg-black/30 px-2 py-1.5 text-sm text-neutral-100 outline-none focus:border-accent/50"
                />
                <p className="mb-1 text-[10px] text-neutral-500">{t("deviceNameHint")}</p>
                <p className="mb-2 text-[10px] tracking-wide text-neutral-600 uppercase">
                  {candidate.id}
                </p>

                <p className="mb-2 text-xs text-neutral-400">{t("pickValueHint")}</p>
                <div className="mb-3 flex flex-wrap gap-1.5">
                  {candidate.values.map((item, index) => {
                    const active = index === chosen;
                    return (
                      <button
                        key={`${item.reportId}-${item.byteOffset}-${index}`}
                        type="button"
                        onClick={() =>
                          setPicked((current) => ({ ...current, [candidate.id]: index }))
                        }
                        className={`rounded-lg border px-2.5 py-1 text-xs transition ${
                          active
                            ? "border-accent/60 bg-accent/15 text-accent"
                            : "border-white/10 text-neutral-400 hover:border-white/25"
                        }`}
                      >
                        %{item.percent}
                      </button>
                    );
                  })}
                </div>

                <button
                  type="button"
                  disabled={!value || candidate.added}
                  onClick={() => value && void add(candidate, value)}
                  className="w-full rounded-lg border border-accent/40 py-2 text-xs font-semibold text-accent transition hover:bg-accent/10 disabled:cursor-not-allowed disabled:opacity-40"
                >
                  {candidate.added ? t("alreadyAdded") : t("add")}
                </button>
              </section>
            );
          })}

          {added.length > 0 ? (
            <section className="mt-4 border-t border-white/8 pt-3">
              <p className="text-xs font-medium tracking-wide text-neutral-400 uppercase">
                {t("addedDevices")}
              </p>
              <p className="mb-2 text-[10px] text-neutral-600">{t("renameHint")}</p>
              {added.map((device) => (
                <div
                  key={device.id}
                  className="mb-1.5 flex items-center justify-between gap-2 rounded-lg border border-white/8 bg-ink-850/50 px-3 py-2"
                >
                  <input
                    defaultValue={device.name}
                    aria-label={t("deviceName")}
                    onBlur={(event) => void rename(device, event.target.value)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter") event.currentTarget.blur();
                    }}
                    className="min-w-0 flex-1 truncate rounded-md bg-transparent px-1 py-0.5 text-xs text-neutral-300 outline-none transition hover:bg-white/5 focus:bg-black/30 focus:text-neutral-100"
                  />
                  <button
                    type="button"
                    onClick={() => void drop(device.id)}
                    className="shrink-0 rounded-md px-2 py-1 text-[11px] text-neutral-500 transition hover:bg-white/10 hover:text-red-300"
                  >
                    {t("remove")}
                  </button>
                </div>
              ))}
            </section>
          ) : null}
        </div>
      </div>
    </div>
  );
}
