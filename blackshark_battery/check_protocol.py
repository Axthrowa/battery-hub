"""Protocol self-check (no hardware required)."""

from __future__ import annotations

from .protocol import CMD_BATTERY, build_query, parse_reply, xor_checksum


def main() -> int:
    report = build_query(CMD_BATTERY, dongle=True)
    assert report[0] == 0x02
    assert report[2] == 0x60
    assert report[6] == 0x04
    assert report[9] == 0x80
    assert report[10] == CMD_BATTERY
    assert report[62] == xor_checksum(report)
    # Known CRC from community captures for the same battery query frame.
    assert report[62] == 0xC7

    reply = bytearray(64)
    reply[0] = 0x02
    reply[10] = CMD_BATTERY
    reply[11] = 0x01
    reply[12] = 0x01
    reply[13] = 73
    assert parse_reply(bytes(reply), CMD_BATTERY) == 73
    print("protocol ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
