"""USB HID battery reader (2.4 GHz dongle and wired USB)."""

from __future__ import annotations

import time
from dataclasses import dataclass
from typing import Any

from .protocol import (
    CLASS_HEADSET,
    CMD_BATTERY,
    CMD_CHARGING,
    CMD_LINK,
    PID_DONGLE,
    PID_WIRED,
    RAZER_VID,
    REPLY_TIMEOUT_MS,
    REPORT_LEN,
    RF_WAKE_REPORT_ID,
    build_query,
    parse_reply,
)

# On Windows the dongle exposes several top-level HID collections on MI_03.
# Battery / link replies arrive on usage page 0xFF14 (Col04), not 0xFF13 (Col01).
PREFERRED_USAGE_PAGE = 0xFF14


@dataclass(frozen=True)
class HidCandidate:
    path: str
    product_id: int
    product: str
    usage_page: int
    usage: int
    interface: int


class HidUnavailable(RuntimeError):
    pass


def _import_hid() -> Any:
    try:
        import hid  # type: ignore
    except ImportError as exc:
        raise HidUnavailable(
            "hidapi yüklü değil. Kurulum: pip install hidapi"
        ) from exc
    return hid


def _as_str(value: Any) -> str:
    if value is None:
        return ""
    if isinstance(value, bytes):
        return value.decode("utf-8", errors="ignore")
    return str(value)


def list_candidates() -> list[HidCandidate]:
    hid = _import_hid()
    found: list[HidCandidate] = []
    for pid in (PID_DONGLE, PID_WIRED):
        for info in hid.enumerate(RAZER_VID, pid):
            found.append(
                HidCandidate(
                    path=_as_str(info.get("path")),
                    product_id=int(info.get("product_id") or pid),
                    product=_as_str(info.get("product_string")),
                    usage_page=int(info.get("usage_page") or 0),
                    usage=int(info.get("usage") or 0),
                    interface=int(info.get("interface_number") if info.get("interface_number") is not None else -1),
                )
            )
    return found


def _score(c: HidCandidate) -> tuple[int, int, int, int]:
    # Prefer FF14 (working vendor channel), then other vendor pages, then iface 3.
    preferred = 1 if c.usage_page == PREFERRED_USAGE_PAGE else 0
    vendor = 1 if c.usage_page >= 0xFF00 else 0
    iface3 = 1 if c.interface == 3 else 0
    dongle = 1 if c.product_id == PID_DONGLE else 0
    return (preferred, vendor, iface3, dongle)


def ranked_candidates(candidates: list[HidCandidate] | None = None) -> list[HidCandidate]:
    candidates = candidates if candidates is not None else list_candidates()
    return sorted(candidates, key=_score, reverse=True)


def pick_device(candidates: list[HidCandidate] | None = None) -> HidCandidate | None:
    ranked = ranked_candidates(candidates)
    return ranked[0] if ranked else None


class _HidSession:
    """Thin wrapper over both cython-hidapi (`hid.device`) and hid.Device."""

    def __init__(self, hid_mod: Any, path: str) -> None:
        self._hid = hid_mod
        self._path = path
        self._dev: Any = None
        self._style = ""

    def open(self) -> None:
        path = self._path
        path_bytes = path.encode("utf-8", "ignore") if isinstance(path, str) else path

        if hasattr(self._hid, "Device"):
            last_err: Exception | None = None
            for candidate in (path, path_bytes):
                try:
                    self._dev = self._hid.Device(path=candidate)
                    self._style = "trezor"
                    return
                except Exception as exc:  # noqa: PERF203 - try both path encodings
                    last_err = exc
            if last_err is not None and not hasattr(self._hid, "device"):
                raise OSError(f"HID açılamadı: {last_err}") from last_err

        device_cls = getattr(self._hid, "device", None)
        if device_cls is None:
            raise HidUnavailable("hidapi Device/device API bulunamadı")
        self._dev = device_cls()
        last_err = None
        for candidate in (path_bytes, path):
            try:
                self._dev.open_path(candidate)
                last_err = None
                break
            except Exception as exc:  # noqa: PERF203
                last_err = exc
        if last_err is not None:
            raise OSError(f"HID açılamadı: {last_err}") from last_err
        self._style = "cython"
        try:
            self._dev.set_nonblocking(False)
        except Exception:
            pass

    def close(self) -> None:
        dev = self._dev
        self._dev = None
        if dev is None:
            return
        try:
            dev.close()
        except Exception:
            pass

    def drain(self) -> None:
        """Drop any pending input reports so the next reply matches our query."""
        try:
            if self._style == "cython":
                self._dev.set_nonblocking(True)
                for _ in range(32):
                    data = self._dev.read(REPORT_LEN)
                    if not data:
                        break
                self._dev.set_nonblocking(False)
            else:
                for _ in range(32):
                    data = self._dev.read(REPORT_LEN, timeout=1)
                    if not data:
                        break
        except Exception:
            try:
                if self._style == "cython":
                    self._dev.set_nonblocking(False)
            except Exception:
                pass

    def send(self, report: bytes) -> None:
        # On this dongle `write()` (interrupt/control OUT path) works; feature
        # reports often return success without producing a usable reply channel.
        errors: list[Exception] = []
        for method in ("write", "send_output_report", "send_feature_report"):
            fn = getattr(self._dev, method, None)
            if fn is None:
                continue
            try:
                result = fn(report)
                # cython-hidapi returns -1 on failure for some paths.
                if isinstance(result, int) and result < 0:
                    errors.append(OSError(f"{method} returned {result}"))
                    continue
                return
            except Exception as exc:
                errors.append(exc)
        raise OSError(f"HID yazılamadı: {errors[-1] if errors else 'no method'}")

    def read(self, timeout_ms: int) -> bytes:
        dev = self._dev
        if self._style == "trezor":
            data = dev.read(REPORT_LEN, timeout=timeout_ms)
        else:
            data = dev.read(REPORT_LEN, timeout_ms)
        if not data:
            return b""
        return bytes(data)


def _query_byte(session: _HidSession, report: bytes, cmd: int, timeout_ms: int = REPLY_TIMEOUT_MS) -> int | None:
    session.drain()
    session.send(report)
    deadline = time.monotonic() + (timeout_ms / 1000.0)
    while time.monotonic() < deadline:
        remaining = max(1, int((deadline - time.monotonic()) * 1000))
        data = session.read(min(remaining, 250))
        if not data:
            continue
        value = parse_reply(data, cmd)
        if value is not None:
            return value
    return None


def _wake(session: _HidSession) -> None:
    for payload in (bytes([RF_WAKE_REPORT_ID, 0x00] + [0] * 62), bytes([RF_WAKE_REPORT_ID, 0x00])):
        try:
            session.send(payload)
            time.sleep(0.04)
            return
        except Exception:
            continue


def _read_on_candidate(hid_mod: Any, candidate: HidCandidate) -> dict[str, Any] | None:
    session = _HidSession(hid_mod, candidate.path)
    dongle = candidate.product_id == PID_DONGLE
    try:
        session.open()
        _wake(session)

        # Link status is a cheap health check; ignore failures.
        try:
            _query_byte(session, build_query(CMD_LINK, dongle=dongle), CMD_LINK, timeout_ms=500)
        except Exception:
            pass

        percent = _query_byte(
            session,
            build_query(CMD_BATTERY, dongle=dongle, cls=CLASS_HEADSET),
            CMD_BATTERY,
            timeout_ms=1200,
        )
        if percent is None and dongle:
            # Wired-style frame (no class/CRC) as a last resort on the same path.
            percent = _query_byte(
                session,
                build_query(CMD_BATTERY, dongle=False),
                CMD_BATTERY,
                timeout_ms=800,
            )

        charging_raw = None
        if percent is not None:
            charging_raw = _query_byte(
                session,
                build_query(CMD_CHARGING, dongle=dongle, cls=CLASS_HEADSET),
                CMD_CHARGING,
                timeout_ms=800,
            )
    finally:
        session.close()

    if percent is None:
        return None

    percent = max(0, min(100, int(percent)))
    charging = bool(charging_raw) if charging_raw is not None else False
    return {
        "ok": True,
        "source": "hid",
        "transport": "2.4 GHz" if dongle else "USB kablo",
        "product": candidate.product or "Razer BlackShark V2 HyperSpeed",
        "percent": percent,
        "charging": charging,
        "usage_page": f"0x{candidate.usage_page:04X}",
    }


def read_hid_battery() -> dict[str, Any] | None:
    """Return battery dict or None if the dongle/wired device is absent."""
    candidates = ranked_candidates()
    if not candidates:
        return None

    hid_mod = _import_hid()
    last_dongle = candidates[0]
    # Try preferred collections first; skip consumer-control pages unless nothing else works.
    ordered = [c for c in candidates if c.usage_page >= 0xFF00] or candidates
    for candidate in ordered:
        last_dongle = candidate
        try:
            result = _read_on_candidate(hid_mod, candidate)
        except Exception:
            continue
        if result is not None:
            return result

    dongle = last_dongle.product_id == PID_DONGLE
    return {
        "ok": False,
        "source": "hid",
        "transport": "2.4 GHz" if dongle else "USB kablo",
        "product": last_dongle.product or "Razer BlackShark V2 HyperSpeed",
        "error": "Dongle bulundu ama kulaklık yanıt vermedi. Açık ve eşli olduğundan emin olun.",
    }
