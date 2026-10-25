> Status: released v2 (partial) — reviewed 2026-10-24. Items marked UNKNOWN/INFERRED are not yet confirmed.

# 02 – Framing

**[observed]** = evident in the shipped tool; **[inferred]** =
reasoned, not yet confirmed on hardware; **UNKNOWN** = not established.

The serial byte stream carries **two protocols**, both started by the PC:

| Protocol            | Used for                                                                                       | Encoding |
|---------------------|------------------------------------------------------------------------------------------------|----------|
| A. Text commands    | session connect/disconnect, device info, **read/write/save configuration**, reset to defaults, reboot, keyword (password), SIM PIN | ASCII lines |
| B. Binary "FMBX"    | live status/diagnostics, single status parameters, records, file transfer, BT/BLE control, ICCID, etc. | binary frames with CRC-16 |

Configuration read/write needs **only protocol A**. Protocol B is needed for the status
pages and for some extra capabilities (02 §2, 03-commands §7).

---

## 1. Protocol A – text commands [observed]

### 1.1 Requests
- ASCII. Each command ends with a single carriage return `\r` (0x0D). There is no LF.
  (One exception is the firmware-update helper commands, which end in `\r\n`; see 06-firmware.)
- Command prefixes:
  - `:` for configuration commands (`:cfg_…`, `:sec_…`)
  - `.` for system commands (`.reset`, `.log:N`)
  - `@` for an auxiliary query (`@asset_send:…`)
- No checksum and no length field.

### 1.2 Replies
Replies are ASCII text. Most carry a tag in angle brackets, for example
`<SOME_TAG>` or `<SOME_TAG>:N`. The reader finishes in one of three ways (01-transport §3):

| Completion           | Detail                                                                                                     |
|----------------------|------------------------------------------------------------------------------------------------------------|
| Ends with `\r`       | the reply's last byte is CR                                                                                 |
| Ends with marker     | full configuration dump: reply ends with the 5 bytes `17 1C 14 00 0D` (ETB, FS, DC4, NUL, CR)               |
| Idle (read until idle) | info replies and log toggles. The idle period depends on the transport: **200 ms** of silence on USB; **about 1 s** without data on Bluetooth (01-transport §2) |

Idle timing per transport [observed]:

| Transport | Idle completion |
|---|---|
| USB (serial port) | reply complete after **200 ms** with no new bytes |
| Bluetooth (SPP) | the link is polled about every 100 ms; reply complete after **about 1000 ms** with no new bytes |

Use the Bluetooth value on Bluetooth: radio links deliver data in bursts with gaps that
can exceed 200 ms, and a 200 ms idle rule would cut replies short there [inferred reason].
Overall per-command timeouts are unchanged (03-commands).

Per-command reply formats are in 03-commands.

Note [inferred]: the tracker may also emit unsolicited debug log text. The tool sends
`.log:0\r` before info queries to quiet it. A robust parser should search the received
text for the expected tag rather than assume it starts at byte 0.

---

## 2. Protocol B – binary "FMBX" frames [observed]

### 2.1 Layout (request and response are identical in shape)

All multi-byte integers are **big-endian**.

| Offset | Size | Field        | Notes                                                                                       |
|-------:|-----:|--------------|---------------------------------------------------------------------------------------------|
| 0      | 4    | Protocol ID  | ASCII `F` `M` `B` `X` = `46 4D 42 58` (as a big-endian u32: 0x464D4258)                      |
| 4      | 4    | Request ID   | u32. Echoed unchanged in the response                                                        |
| 8      | 2    | Command / Response ID | u16. Requests carry a command ID, responses a response ID                       |
| 10     | 2    | Payload length `N` | u16, 0…65535                                                                           |
| 12     | N    | Payload      | command-specific                                                                             |
| 12+N   | 2    | CRC          | u16 big-endian, **CRC-16/ARC** over bytes 4 … 11+N (request ID through end of payload; **the protocol ID is not included**) |

Minimum frame size is 14 bytes.

### 2.2 Request ID [observed]
- A process-wide counter, incremented **before** each binary command is sent. The first
  command after start-up therefore uses 1.
- The response must echo the same request ID.

### 2.3 Receiving and validating a response [observed]
1. Read exactly 12 bytes (the header).
2. Check that bytes 0–3 equal `FMBX`. If not, treat the reply as invalid (bad protocol ID).
3. Check that bytes 4–7 equal the request ID that was sent. If not, fail.
4. Read `N + 2` more bytes (payload plus CRC), where `N` comes from bytes 10–11.
5. Recompute CRC-16/ARC over bytes 4…(11+N) and compare it with the trailing u16. If they
   differ, treat the reply as corrupt (checksum mismatch).
6. Check the response ID against the set of response IDs expected for that command
   (03-commands §7). On mismatch, fail. A generic error response has ID `0x0014`.
7. Timeout: 30 s by default, or a per-command override.

### 2.4 CRC-16/ARC parameters [observed]

| Parameter | Value |
|-----------|-------|
| Width     | 16    |
| Polynomial (normal) | 0x8005 |
| Polynomial (reflected, as used in a right-shifting implementation) | 0xA001 |
| Init      | 0x0000 |
| RefIn / RefOut | true / true |
| XorOut    | 0x0000 |
| Check value (`"123456789"`) | **0xBB3D** |
| Byte order on the wire | big-endian (high byte first) |

This is the standard catalogue algorithm "CRC-16/ARC" (also called CRC-16/LHA or
CRC-IBM).

### 2.5 Test vectors
These were computed with an independent implementation of the parameters above
[inferred correct, derived from the observed algorithm and layout. Not yet captured from a device. The reviewer independently recomputed all CRC values below and they match].

```
CRC-16/ARC("123456789")            = 0xBB3D
CRC-16/ARC("")                     = 0x0000

Request ID 1, command 0x0002 (get status data), empty payload:
46 4D 42 58 00 00 00 01 00 02 00 00 00 9C

Request ID 1, command 0x001C (request device info), empty payload:
46 4D 42 58 00 00 00 01 00 1C 00 00 06 FC

Request ID 2, command 0x0038 (get single status parameter), payload = 0x030D (781):
46 4D 42 58 00 00 00 02 00 38 00 02 03 0D F0 E2

Request ID 3, command 0x0038, payload = 0x0310 (784):
46 4D 42 58 00 00 00 03 00 38 00 02 03 10 39 32
```

### 2.6 Multi-part responses [observed, details partial]
Some commands (records, file download) are answered by a first response whose payload
holds a count of further responses. That count is 1, 2 or 4 bytes long; the payload
length tells which. The PC then reads that many more frames with the same request ID.
Details are in 03-commands §7 and are **partially UNKNOWN**.

### 2.7 Protocol version and feature gating [observed]
- The tool reads a **binary protocol version** from status parameter **781**. Byte 0 is
  the major version and byte 1 the minor.
- It reads a **feature bitmask** from status parameter **784**.
- Each binary command has a minimum protocol version (0.0 or 0.1) and a required feature
  bit (see 03-commands §7). The tool does not send a command the device does not support.
- If the version cannot be read, the tool assumes 0.0.
