"""Query battery on the working 0xFF14 collection."""

from __future__ import annotations

import time

import hid

from blackshark_battery.protocol import (
    CLASS_HEADSET,
    CMD_BATTERY,
    CMD_CHARGING,
    CMD_LINK,
    PID_DONGLE,
    RAZER_VID,
    REPORT_LEN,
    build_query,
    parse_reply,
)


def main() -> int:
    target = None
    for d in hid.enumerate(RAZER_VID, PID_DONGLE):
        if d.get("usage_page") == 0xFF14:
            target = d
            break
    if not target:
        print("FF14 not found")
        return 1

    h = hid.device()
    path = target["path"]
    h.open_path(path if isinstance(path, bytes) else path.encode("utf-8"))
    h.set_nonblocking(False)
    print("opened FF14")

    def drain():
        h.set_nonblocking(True)
        while True:
            data = h.read(REPORT_LEN)
            if not data:
                break
        h.set_nonblocking(False)

    def query(cmd: int, label: str) -> int | None:
        q = build_query(cmd, dongle=True, cls=CLASS_HEADSET)
        drain()
        n = h.write(q)
        print(f"TX {label} write={n} {q[:14].hex()} crc={q[62]:02x}")
        end = time.monotonic() + 1.2
        while time.monotonic() < end:
            data = h.read(REPORT_LEN, 200)
            if not data:
                continue
            b = bytes(data)
            print(f"RX {b[:16].hex()} ...")
            val = parse_reply(b, cmd)
            print(f"  parsed={val}")
            if val is not None:
                return val
        return None

    # optional wake
    try:
        h.write(bytes([0x05, 0x00] + [0] * 62))
        time.sleep(0.05)
    except Exception as exc:
        print("wake", exc)

    link = query(CMD_LINK, "link")
    batt = query(CMD_BATTERY, "batt")
    chg = query(CMD_CHARGING, "chg")
    print(f"RESULT link={link} battery={batt} charging={chg}")
    h.close()
    return 0 if batt is not None else 2


if __name__ == "__main__":
    raise SystemExit(main())
