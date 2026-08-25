"""System-tray battery monitor. Polls slowly to stay light on RAM/CPU."""

from __future__ import annotations

import argparse
import sys
import threading
from typing import Any

import pystray

from . import __version__
from .bluetooth_source import read_bluetooth_battery
from .hid_source import HidUnavailable, read_hid_battery
from .icons import make_icon
from .probe import run_probe
from .settings import load_settings, save_settings, set_startup_enabled

RECOVER_BATTERY = 25


class Monitor:
    def __init__(self) -> None:
        self._stop = threading.Event()
        self._refresh = threading.Event()
        self._lock = threading.Lock()
        self._settings = load_settings()
        self._state: dict[str, Any] = {
            "ok": False,
            "percent": None,
            "charging": False,
            "transport": "",
            "product": "Razer BlackShark V2 HyperSpeed",
            "error": "Cihaz aranıyor…",
        }
        self._notified_low = False
        self._last_visual: tuple[int | None, bool, bool] | None = None
        self._last_title: str | None = None
        self.icon: pystray.Icon | None = None

    def _persist(self) -> None:
        save_settings(self._settings)

    def poll_once(self) -> dict[str, Any]:
        hid_error: str | None = None
        try:
            hid = read_hid_battery()
        except HidUnavailable as exc:
            hid = None
            hid_error = str(exc)
        except Exception as exc:
            hid = None
            hid_error = f"HID hata: {exc}"

        if hid and hid.get("ok"):
            return hid

        bt: dict[str, Any] | None
        try:
            bt = read_bluetooth_battery()
        except Exception as exc:
            bt = {
                "ok": False,
                "source": "bluetooth",
                "transport": "Bluetooth",
                "error": f"Bluetooth hata: {exc}",
            }

        if bt and bt.get("ok"):
            return bt

        if hid:
            return hid
        if bt:
            return bt
        return {
            "ok": False,
            "percent": None,
            "charging": False,
            "transport": "",
            "product": "Razer BlackShark V2 HyperSpeed",
            "error": hid_error
            or "Kulaklık bulunamadı. 2.4 GHz dongle takılı veya Bluetooth bağlı olsun.",
        }

    def snapshot(self) -> dict[str, Any]:
        with self._lock:
            return dict(self._state)

    def _apply(self, state: dict[str, Any]) -> None:
        with self._lock:
            self._state = state
        icon = self.icon
        if icon is None:
            return
        ok = bool(state.get("ok"))
        percent = state.get("percent") if ok else None
        charging = bool(state.get("charging"))

        # Assigning icon/title makes pystray rebuild a Win32 HICON and poke the
        # shell, so only do it when the value actually moved.
        visual = (percent, charging, not ok)
        if visual != self._last_visual:
            self._last_visual = visual
            icon.icon = make_icon(percent, charging=charging, missing=not ok)
        title = self._tooltip(state)
        if title != self._last_title:
            self._last_title = title
            icon.title = title

        low = int(self._settings.get("low_battery", 15))
        if (
            self._settings.get("notify_low", True)
            and ok
            and isinstance(percent, int)
            and percent <= low
            and not charging
        ):
            if not self._notified_low:
                self._notified_low = True
                try:
                    icon.notify(f"Pil düşük: %{percent}", "BlackShark Battery")
                except Exception:
                    pass
        elif ok and isinstance(percent, int) and percent >= max(low + 10, RECOVER_BATTERY):
            self._notified_low = False

    @staticmethod
    def _tooltip(state: dict[str, Any]) -> str:
        product = state.get("product") or "BlackShark V2 HyperSpeed"
        if state.get("ok"):
            charge = " (şarj oluyor)" if state.get("charging") else ""
            transport = state.get("transport") or "?"
            return f"{product}\n%{state.get('percent')}{charge} · {transport}"
        err = state.get("error") or "Bilinmiyor"
        return f"{product}\n{err}"

    def loop(self) -> None:
        while not self._stop.is_set():
            try:
                self._apply(self.poll_once())
            except Exception as exc:
                self._apply(
                    {
                        "ok": False,
                        "percent": None,
                        "charging": False,
                        "transport": "",
                        "product": "Razer BlackShark V2 HyperSpeed",
                        "error": str(exc),
                    }
                )
            wait = int(self._settings.get("poll_seconds", 45))
            self._refresh.wait(wait)
            self._refresh.clear()

    def request_refresh(self, _icon=None, _item=None) -> None:
        self._refresh.set()

    def quit(self, icon=None, _item=None) -> None:
        self._stop.set()
        self._refresh.set()
        if icon is not None:
            icon.stop()
        elif self.icon is not None:
            self.icon.stop()

    def _status_text(self, _=None) -> str:
        s = self.snapshot()
        if s.get("ok"):
            charge = " · şarj" if s.get("charging") else ""
            return f"%{s.get('percent')}{charge} · {s.get('transport')}"
        return s.get("error") or "Bağlı değil"

    def _startup_checked(self, _=None) -> bool:
        return bool(self._settings.get("run_at_startup"))

    def _toggle_startup(self, icon=None, item=None) -> None:
        enabled = not bool(self._settings.get("run_at_startup"))
        try:
            set_startup_enabled(enabled)
            self._settings["run_at_startup"] = enabled
            self._persist()
            if icon is not None:
                try:
                    icon.notify(
                        "Windows açılışında başlayacak." if enabled else "Başlangıçtan kaldırıldı.",
                        "Ayarlar",
                    )
                except Exception:
                    pass
        except OSError as exc:
            if icon is not None:
                try:
                    icon.notify(f"Başlangıç ayarı başarısız: {exc}", "Ayarlar")
                except Exception:
                    pass

    def _notify_checked(self, _=None) -> bool:
        return bool(self._settings.get("notify_low", True))

    def _toggle_notify(self, _icon=None, _item=None) -> None:
        self._settings["notify_low"] = not bool(self._settings.get("notify_low", True))
        self._persist()

    def _poll_label(self, seconds: int):
        def _checked(_=None) -> bool:
            return int(self._settings.get("poll_seconds", 45)) == seconds

        def _set(_icon=None, _item=None) -> None:
            self._settings["poll_seconds"] = seconds
            self._persist()
            self._refresh.set()

        return pystray.MenuItem(f"{seconds} sn", _set, checked=_checked, radio=True)

    def run_tray(self) -> None:
        settings_menu = pystray.Menu(
            pystray.MenuItem(
                "Başlangıçta çalıştır",
                self._toggle_startup,
                checked=self._startup_checked,
            ),
            pystray.MenuItem(
                "Düşük pil bildirimi",
                self._toggle_notify,
                checked=self._notify_checked,
            ),
            pystray.Menu.SEPARATOR,
            pystray.MenuItem(
                "Yenileme aralığı",
                pystray.Menu(
                    self._poll_label(15),
                    self._poll_label(30),
                    self._poll_label(45),
                    self._poll_label(60),
                    self._poll_label(120),
                ),
            ),
        )
        menu = pystray.Menu(
            pystray.MenuItem(self._status_text, None, enabled=False),
            pystray.Menu.SEPARATOR,
            pystray.MenuItem("Şimdi yenile", self.request_refresh, default=True),
            pystray.MenuItem("Ayarlar", settings_menu),
            pystray.Menu.SEPARATOR,
            pystray.MenuItem("Çıkış", self.quit),
        )
        self.icon = pystray.Icon(
            "BlackSharkBattery",
            make_icon(None, missing=True),
            "BlackShark V2 HyperSpeed",
            menu,
        )
        worker = threading.Thread(target=self.loop, name="battery-poll", daemon=True)
        worker.start()
        self.icon.run()
        self._stop.set()
        self._refresh.set()
        worker.join(timeout=2)


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Razer BlackShark V2 HyperSpeed pil göstergesi (sistem tepsisi)"
    )
    parser.add_argument("--probe", action="store_true", help="Cihazları listele ve çık")
    parser.add_argument("--once", action="store_true", help="Bir kez sorgula, tepsi açma")
    parser.add_argument("--check-protocol", action="store_true", help="Donanımsız protokol self-check")
    parser.add_argument("--version", action="store_true")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    if args.version:
        print(__version__)
        return 0
    if args.check_protocol:
        from .check_protocol import main as check_main

        return check_main()
    if args.probe:
        return run_probe()
    monitor = Monitor()
    if args.once:
        state = monitor.poll_once()
        print(state)
        return 0 if state.get("ok") else 1
    if sys.platform != "win32":
        print("Bu uygulama Windows 10/11 içindir.", file=sys.stderr)
        return 2
    monitor.run_tray()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
