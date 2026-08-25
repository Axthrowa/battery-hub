"""Vendor HID framing for the Razer BlackShark V2 HyperSpeed.

The 2.4 GHz dongle (USB 1532:0565) and the wired/charging adapter (1532:056e)
use a 64-byte vendor report (ID 0x02) instead of the classic 90-byte Razer
protocol. Community reverse-engineering of this layout (OpenRazer / MediaTek
headset channel) is used here so Synapse is not required.

Frame:
  [0]  = 0x02 report id
  [2]  = 0x60 channel
  [6]  = 0x04 + payload length
  [9]  = command class (dongle only): 0x80 = relay to headset, 0x00 = receiver
  [10] = command
  [12] = payload length
  [13] = payload
  [62] = XOR of bytes [0..61] (dongle only)

Reply (interrupt IN, same report id):
  [10] echoes command, [11] == 0x01 ACK, [12] length, [13] value.
"""

from __future__ import annotations

RAZER_VID = 0x1532
PID_DONGLE = 0x0565  # Razer BlackShark V2 HS 2.4
PID_WIRED = 0x056E  # wired / charging USB

REPORT_LEN = 64
REPORT_ID = 0x02
RF_WAKE_REPORT_ID = 0x05
CHANNEL = 0x60
CRC_INDEX = 62
CLASS_HEADSET = 0x80
CLASS_RECEIVER = 0x00

CMD_BATTERY = 0x21
CMD_CHARGING = 0x2A
CMD_LINK = 0x20  # 0 none, 1 2.4 GHz, 2 BT-to-PC, 3 BT-to-phone

REPLY_TIMEOUT_MS = 1200


def xor_checksum(buf: bytes | bytearray) -> int:
    crc = 0
    for i in range(CRC_INDEX):
        crc ^= buf[i]
    return crc


def build_query(cmd: int, *, dongle: bool, cls: int = CLASS_HEADSET) -> bytes:
    buf = bytearray(REPORT_LEN)
    buf[0] = REPORT_ID
    buf[2] = CHANNEL
    buf[6] = 0x04
    buf[10] = cmd
    buf[12] = 0x00
    if dongle:
        buf[9] = cls
        buf[CRC_INDEX] = xor_checksum(buf)
    return bytes(buf)


def parse_reply(data: bytes, expected_cmd: int) -> int | None:
    if len(data) <= 13:
        return None
    # hidapi may or may not prefix the report id depending on the collection.
    if data[0] == REPORT_ID:
        payload = data
    elif len(data) > 14 and data[1] == REPORT_ID:
        payload = data[1:]
    else:
        payload = data
        if payload[0] != REPORT_ID:
            return None
    if payload[10] != expected_cmd:
        return None
    if payload[11] != 0x01:
        return None
    return int(payload[13])
