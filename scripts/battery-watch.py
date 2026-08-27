"""Live battery view that runs under Smart App Control.

Smart App Control in enforcement mode refuses any freshly built executable, so
on a machine where it has turned itself on there is no way to run a new
battery-hub.exe at all -- signed or not, however long you wait. What SAC gates
is the loading of new native images, not interpreted code, which is the same
opening scripts/hid-probe.py uses: Windows' own signed hid.dll, driven through
ctypes under a python.exe that is already trusted.

So this is the app's charge logic without the app. It speaks the same two
vendor frames the Rust readers speak, and it runs judge_charge() from
src-tauri/src/lib.rs -- transcribed here, and kept in step with it by the
constants below -- so the state it prints is the state the panel would show.

    python scripts/battery-watch.py                  # every 8 seconds, forever
    python scripts/battery-watch.py 30 --raw         # every 30, with the frames
    python scripts/battery-watch.py 8 --count 24     # a bounded run

Only the two 2.4 GHz devices with vendor frames are covered: Aula/Compx
keyboards and Ajazz mice. Logitech speaks HID++, and Razer and Soundcore have
protocols of their own; none of those are reimplemented here.
"""
import ctypes as C
import importlib.util
import os
import sys
import time
from ctypes import wintypes as W

_here = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location("hidprobe", os.path.join(_here, "hid-probe.py"))
hp = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(hp)

hp.hid.HidD_SetFeature.argtypes = [C.c_void_p, C.c_void_p, W.ULONG]
hp.hid.HidD_GetFeature.argtypes = [C.c_void_p, C.c_void_p, W.ULONG]

# --- judge_charge, from src-tauri/src/lib.rs ------------------------------
#
# A charging flag on its own is worth very little: a receiver keeps serving the
# last frame it heard from a device that has gone off the air, so it will claim
# the cable for hours after the mouse is back on the desk. The level is the
# evidence. A charge that is happening gains points; one that is losing them is
# not happening at all.
FULL_ENOUGH = 95
CHARGE_GRACE_MS = 3 * 60 * 1000
CHARGE_STALL_MS = 10 * 60 * 1000
FULL_STALL_MS = 15 * 60 * 1000
CHARGE_DROP_TOLERANCE = 2

FILLING, FULL, DISBELIEVED = "charging", "full", "not charging"


class Watch:
    def __init__(self, percent, now):
        self.peak = percent
        self.since = now
        self.climbed = False
        self.stale = False


def judge_charge(watch, percent, now):
    if percent > watch.peak:
        # It gained: the flag belongs to a charge that is really under way.
        watch.peak, watch.since = percent, now
        watch.climbed, watch.stale = True, False
    elif watch.peak - percent >= CHARGE_DROP_TOLERANCE:
        # It lost charge while claiming the cable, which nothing on a charger
        # does. The peak follows the level down so a charge starting later
        # reads as a gain on the very next poll.
        watch.peak, watch.since, watch.stale = percent, now, True

    waited = max(0, now - watch.since)
    near_done = watch.climbed and percent >= FULL_ENOUGH
    patience = CHARGE_STALL_MS if watch.climbed else CHARGE_GRACE_MS
    if not near_done and waited >= patience:
        watch.stale = True

    if percent >= 100:
        return FULL
    if watch.stale:
        return DISBELIEVED
    if near_done and waited >= FULL_STALL_MS:
        return FULL
    return FILLING
# -------------------------------------------------------------------------

AULA_VIDS = (0x3554, 0x258A, 0x372E)
AULA_USAGE_PAGE = 0xFF02
AULA_REPORT_ID = 0x13
AULA_CMD_BATTERY = 0x4A
AULA_STATUS_CHARGING = 0x10

AJAZZ_IDS = (0x3151, 0x5007)
AJAZZ_USAGE_PAGE, AJAZZ_USAGE = 0xFFFF, 0x0002


def collections():
    """Every HID collection, opened once and kept for the life of the run."""
    found = []
    for path in hp.interface_paths():
        for overlapped in (True, False):
            handle = hp.open_device(path, overlapped=overlapped)
            if handle is None:
                break
            info = hp.describe(handle)
            if not info:
                hp.k32.CloseHandle(C.c_void_p(handle))
                break
            aula = (info["vid"] in AULA_VIDS and info["usage_page"] == AULA_USAGE_PAGE
                    and overlapped)
            ajazz = ((info["vid"], info["pid"]) == AJAZZ_IDS and overlapped is False
                     and info["usage_page"] == AJAZZ_USAGE_PAGE
                     and info["usage"] == AJAZZ_USAGE and info["feat_len"] > 8)
            if aula:
                found.append(("aula", handle, info))
                break
            if ajazz:
                found.append(("ajazz", handle, info))
                break
            hp.k32.CloseHandle(C.c_void_p(handle))
            if overlapped is False:
                break
    return found


def read_aula(handle, info):
    """TX 4A -> RX `4A packets index len percent status`, status 0x10 on cable."""
    payload = bytearray(19)
    payload[0] = AULA_CMD_BATTERY
    payload[18] = hp.crc(AULA_REPORT_ID, payload)
    frame = bytes([AULA_REPORT_ID]) + bytes(payload)
    out = C.create_string_buffer(frame, info["out_len"])
    for _ in range(5):
        # The 2.4 GHz link stays quiet through the odd frame, so ask again
        # rather than reporting a keyboard that is switched on.
        if not hp.hid.HidD_SetOutputReport(handle, out, info["out_len"]):
            return None
        reply = hp.read_input(handle, info["in_len"], 400)
        if not reply:
            continue
        body = reply[1:] if reply[0] == AULA_REPORT_ID else reply
        if len(body) >= 6 and body[0] == AULA_CMD_BATTERY:
            percent, status = body[4], body[5]
            if 1 <= percent <= 100:
                return percent, bool(status & AULA_STATUS_CHARGING), reply[:8]
    return None


def read_ajazz(handle, info):
    """SET_FEATURE F7 wakes telemetry, GET_FEATURE 05 -> `00 00 pct 01 link`.

    The fifth byte is not a charging flag, whatever it looks like: it says
    whether the mouse is on the 2.4 GHz link. `00` while it is talking, `01`
    the moment it goes quiet -- and the receiver then repeats the last frame it
    heard for as long as the mouse stays away. Reading that byte as "charging"
    is what put a bolt on a mouse sitting switched off in a drawer.

    Returns (percent, on_air). There is no charging indicator here at all: a
    cable takes the mouse off the radio, so the one state in which it is
    charging is the one state in which it cannot say so.
    """
    n = info["feat_len"]
    for attempt in range(4):
        poll = C.create_string_buffer(bytes([0x00, 0xF7]) + b"\x00" * (n - 2), n)
        hp.hid.HidD_SetFeature(handle, poll, n)
        time.sleep(0.05 + attempt * 0.03)
        buf = C.create_string_buffer(n)
        buf[0] = b"\x05"
        if not hp.hid.HidD_GetFeature(handle, buf, n):
            continue
        raw = bytes(buf.raw)
        body = raw[1:] if raw[0] == 0x05 else raw
        if len(body) >= 5 and body[0] == 0 and body[1] == 0 and 1 <= body[2] <= 100:
            return body[2], body[4] == 0, body[:7]
    return None


READERS = {"aula": read_aula, "ajazz": read_ajazz}
LABELS = {"aula": "Aula / Compx keyboard", "ajazz": "Ajazz mouse"}


def main():
    argv = sys.argv[1:]
    count = 0
    if "--count" in argv:
        i = argv.index("--count")
        count = int(argv[i + 1])
        del argv[i:i + 2]
    args = [a for a in argv if not a.startswith("--")]
    gap = float(args[0]) if args else 8.0
    show_raw = "--raw" in argv

    devices = collections()
    if not devices:
        print("No Aula or Ajazz 2.4 GHz receiver found - is the device switched on?")
        return 1

    print("Battery Hub charge rule, without the app (Ctrl+C to stop)")
    for kind, _, info in devices:
        print(f"  {LABELS[kind]:<24} {info['vid']:04X}:{info['pid']:04X}  {info['product']}")
    print()

    watches = {}
    started = time.monotonic()
    rounds = 0
    try:
        while not count or rounds < count:
            rounds += 1
            now = int((time.monotonic() - started) * 1000)
            stamp = time.strftime("%H:%M:%S")
            for kind, handle, info in devices:
                got = READERS[kind](handle, info)
                if not got:
                    print(f"{stamp}  {LABELS[kind]:<24}    -   no answer")
                    continue
                percent, second, raw = got
                extra = f"   {hp.hexs(raw)}" if show_raw else ""
                if kind == "ajazz" and not second:
                    # Off the link: the level beside the flag belongs to
                    # whenever the mouse last spoke, so the app drops the card
                    # rather than standing an hours-old number on the panel.
                    watches.pop(kind, None)
                    print(f"{stamp}  {LABELS[kind]:<24}    -   off the link (card hidden){extra}")
                    continue
                charging = second and kind != "ajazz"
                if charging:
                    if kind not in watches:
                        watches[kind] = Watch(percent, now)
                    state = judge_charge(watches[kind], percent, now)
                else:
                    watches.pop(kind, None)
                    state = "on battery"
                print(f"{stamp}  {LABELS[kind]:<24} {percent:3d}%   {state}{extra}")
            if not count or rounds < count:
                time.sleep(gap)
    except KeyboardInterrupt:
        print("\nstopped")
    finally:
        for _, handle, _ in devices:
            hp.k32.CloseHandle(C.c_void_p(handle))
    return 0


if __name__ == "__main__":
    sys.exit(main())
