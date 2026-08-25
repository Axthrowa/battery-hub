"""CLI diagnostics: list HID and Bluetooth targets without starting the tray."""

from __future__ import annotations

from .bluetooth_source import list_bluetooth_pnp, read_bluetooth_battery
from .hid_source import HidUnavailable, list_candidates, ranked_candidates, read_hid_battery


def run_probe() -> int:
    print("Razer BlackShark V2 HyperSpeed — cihaz taraması")
    print("=" * 56)

    print("\n[USB HID]")
    try:
        candidates = list_candidates()
    except HidUnavailable as exc:
        print(f"  {exc}")
        candidates = []
    if not candidates:
        print("  Dongle/kablo bulunamadı (1532:0565 / 1532:056e).")
    else:
        ranked = ranked_candidates(candidates)
        best = ranked[0] if ranked else None
        for c in ranked:
            mark = " <-- tercih" if best and c.path == best.path else ""
            print(
                f"  PID={c.product_id:04X} if={c.interface} "
                f"usage_page=0x{c.usage_page:04X} usage=0x{c.usage:04X} "
                f"{c.product!r}{mark}"
            )
            print(f"    path={c.path}")
        result = read_hid_battery()
        print(f"  sorgu: {result}")

    print("\n[Bluetooth PnP]")
    try:
        devices = list_bluetooth_pnp()
    except Exception as exc:
        print(f"  PnP okunamadı: {exc}")
        devices = []
    if not devices:
        print("  Eşleşen Bluetooth cihazı yok.")
    else:
        for d in devices:
            print(f"  {d.name!r} connected={d.connected} battery={d.battery}")
        print(f"  sorgu: {read_bluetooth_battery()}")

    print("\nSynapse gerekmez. HID kilitliyse Razer yazılımını kapatıp tekrar deneyin.")
    return 0
