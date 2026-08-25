"""Try every vendor HID collection / write path and dump raw replies."""

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


def open_dev(path):
    try:
        h = hid.device()
        raw = path if isinstance(path, bytes) else path.encode("utf-8")
        h.open_path(raw)
        h.set_nonblocking(False)
        return h, "cython"
    except Exception as exc:
        print("  open cython fail:", exc)
    try:
        h = hid.Device(path=path)
        return h, "trezor"
    except Exception as exc:
        print("  open trezor fail:", exc)
    return None, None


def drain(h, style: str) -> None:
    try:
        h.set_nonblocking(True)
        for _ in range(32):
            data = h.read(REPORT_LEN) if style == "cython" else h.read(REPORT_LEN, timeout=1)
            if not data:
                break
            print("  drain", bytes(data).hex())
        h.set_nonblocking(False)
    except Exception as exc:
        print("  drain skip", exc)


def read_loop(h, style: str, ms: int = 900) -> list[bytes]:
    chunks: list[bytes] = []
    end = time.monotonic() + ms / 1000.0
    while time.monotonic() < end:
        rem = max(1, int((end - time.monotonic()) * 1000))
        try:
            data = (
                h.read(REPORT_LEN, timeout=min(rem, 250))
                if style == "trezor"
                else h.read(REPORT_LEN, min(rem, 250))
            )
        except Exception as exc:
            print("  read err", exc)
            break
        if not data:
            continue
        b = bytes(data)
        chunks.append(b)
        print(f"  RX {b.hex()}")
        print(
            "     batt=",
            parse_reply(b, CMD_BATTERY),
            "chg=",
            parse_reply(b, CMD_CHARGING),
            "link=",
            parse_reply(b, CMD_LINK),
            "len=",
            len(b),
            "b0=",
            f"0x{b[0]:02x}",
            "b10=",
            f"0x{b[10]:02x}" if len(b) > 10 else "n/a",
            "b11=",
            f"0x{b[11]:02x}" if len(b) > 11 else "n/a",
            "b13=",
            f"0x{b[13]:02x}" if len(b) > 13 else "n/a",
        )
    return chunks


def main() -> int:
    devs = list(hid.enumerate(RAZER_VID, PID_DONGLE))
    print(f"found {len(devs)} interfaces")
    for i, d in enumerate(devs):
        print(
            f"[{i}] if={d.get('interface_number')} "
            f"up=0x{d.get('usage_page'):04X} u=0x{d.get('usage'):04X}"
        )

    targets = [d for d in devs if (d.get("usage_page") or 0) >= 0xFF00] or devs
    for d in targets:
        path = d["path"]
        print(f"\n=== TRY up=0x{d.get('usage_page'):04X} ===")
        h, style = open_dev(path)
        if not h:
            continue
        print("opened", style)
        drain(h, style)

        for wake in (bytes([0x05, 0x00]), bytes([0x05, 0x00] + [0] * 62)):
            for method in ("write", "send_output_report", "send_feature_report"):
                fn = getattr(h, method, None)
                if not fn:
                    continue
                try:
                    r = fn(wake)
                    print(f"  wake via {method} ok ({r}) len={len(wake)}")
                except Exception as exc:
                    print(f"  wake via {method} FAIL {exc}")
            time.sleep(0.04)

        for cls_name, dongle, cls in (
            ("dongle_headset", True, CLASS_HEADSET),
            ("dongle_recv", True, 0x00),
            ("wired_style", False, CLASS_HEADSET),
        ):
            for cmd, cname in ((CMD_LINK, "link"), (CMD_BATTERY, "batt"), (CMD_CHARGING, "chg")):
                q = build_query(cmd, dongle=dongle, cls=cls)
                print(f"  TX {cname}/{cls_name} {q[:14].hex()} crc={q[62]:02x}")
                answered = False
                for method in ("write", "send_output_report", "send_feature_report"):
                    fn = getattr(h, method, None)
                    if not fn:
                        continue
                    drain(h, style)
                    try:
                        r = fn(q)
                        print(f"    via {method} -> {r}")
                    except Exception as exc:
                        print(f"    via {method} FAIL {exc}")
                        continue
                    got = read_loop(h, style, 900)
                    if got:
                        answered = True
                        break
                if answered:
                    break

        try:
            h.close()
        except Exception:
            pass

    print("DONE")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
