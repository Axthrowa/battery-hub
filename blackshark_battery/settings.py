"""Persisted settings + Windows startup registration."""

from __future__ import annotations

import json
import os
import sys
import winreg
from pathlib import Path
from typing import Any

APP_NAME = "BlackSharkBattery"
RUN_VALUE = "BlackSharkBattery"
SETTINGS_DIR = Path(os.environ.get("APPDATA", Path.home())) / APP_NAME
SETTINGS_PATH = SETTINGS_DIR / "settings.json"

DEFAULTS: dict[str, Any] = {
    "run_at_startup": False,
    "poll_seconds": 45,
    "notify_low": True,
    "low_battery": 15,
}


def _clamp_poll(seconds: Any) -> int:
    try:
        value = int(seconds)
    except (TypeError, ValueError):
        value = DEFAULTS["poll_seconds"]
    return max(15, min(300, value))


def load_settings() -> dict[str, Any]:
    data = dict(DEFAULTS)
    try:
        if SETTINGS_PATH.is_file():
            raw = json.loads(SETTINGS_PATH.read_text(encoding="utf-8"))
            if isinstance(raw, dict):
                data.update(raw)
    except Exception:
        pass
    data["poll_seconds"] = _clamp_poll(data.get("poll_seconds"))
    data["run_at_startup"] = bool(data.get("run_at_startup"))
    data["notify_low"] = bool(data.get("notify_low", True))
    try:
        data["low_battery"] = max(5, min(50, int(data.get("low_battery", 15))))
    except (TypeError, ValueError):
        data["low_battery"] = 15
    # Keep JSON and registry in sync if the user toggled startup externally.
    data["run_at_startup"] = is_startup_enabled()
    return data


def save_settings(settings: dict[str, Any]) -> None:
    SETTINGS_DIR.mkdir(parents=True, exist_ok=True)
    payload = {
        "run_at_startup": bool(settings.get("run_at_startup")),
        "poll_seconds": _clamp_poll(settings.get("poll_seconds")),
        "notify_low": bool(settings.get("notify_low", True)),
        "low_battery": int(settings.get("low_battery", 15)),
    }
    SETTINGS_PATH.write_text(json.dumps(payload, indent=2, ensure_ascii=False), encoding="utf-8")


def launch_command() -> str:
    """Command written to HKCU\\...\\Run for login startup."""
    if getattr(sys, "frozen", False):
        return f'"{sys.executable}"'
    # Dev / venv: start the package without a console window when possible.
    pythonw = Path(sys.executable).with_name("pythonw.exe")
    exe = str(pythonw if pythonw.is_file() else sys.executable)
    return f'"{exe}" -m blackshark_battery'


def _run_key(access: int):
    return winreg.OpenKey(
        winreg.HKEY_CURRENT_USER,
        r"Software\Microsoft\Windows\CurrentVersion\Run",
        0,
        access,
    )


def is_startup_enabled() -> bool:
    try:
        with _run_key(winreg.KEY_READ) as key:
            value, _ = winreg.QueryValueEx(key, RUN_VALUE)
            return bool(value)
    except FileNotFoundError:
        return False
    except OSError:
        return False


def set_startup_enabled(enabled: bool) -> None:
    with _run_key(winreg.KEY_SET_VALUE) as key:
        if enabled:
            winreg.SetValueEx(key, RUN_VALUE, 0, winreg.REG_SZ, launch_command())
        else:
            try:
                winreg.DeleteValue(key, RUN_VALUE)
            except FileNotFoundError:
                pass
