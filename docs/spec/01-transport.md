> Status: released v2 (partial) — reviewed 2026-10-24. Items marked UNKNOWN/INFERRED are not yet confirmed.

# 01 – Transport

Facts are labelled **[observed]** (directly evident in the
shipped tool's static content) or **[inferred]** (a reasonable conclusion that has not
been confirmed against a live device). Anything not established is marked **UNKNOWN**.

Scope: FM-series trackers (FMB/FMC/FMM/FMT, plus FMB640-family) as handled by the
vendor configurator v1.8.21. The tool supports two physical transports: USB virtual
COM port and Bluetooth Classic SPP (RFCOMM). HID and BLE GATT are **not** used for
configuration [observed: neither HID nor GATT is used for configuration; see §2A for BLE].

## 1. USB (virtual COM / CDC-ACM)

### 1.1 Device discovery

The tool builds a candidate port list from two sources and merges them
(case-insensitive, de-duplicated) [observed]:

1. **Filtered USB serial ports.** OS serial devices of the "Ports (COM & LPT)" class whose
   USB hardware ID begins with one of these VID/PID prefixes:

   | VID  | PID  | Interface | Notes [inferred]                    |
   |------|------|-----------|-------------------------------------|
   | 0E8D | 0023 | MI_00     | MediaTek-based units (composite)     |
   | 0483 | 5740 | –         | STMicroelectronics virtual COM port  |
   | 1FC9 | 0094 | –         | NXP-based units                      |
   | 0483 | 5702 | MI_00     | STMicroelectronics (composite)       |
   | 1D12 | 0200 | –         | other module vendor                  |

   On Windows the port name is taken from the device caption's trailing `(COMn)`.
2. **All OS serial ports.** Every serial port name the OS reports is added as well.
   So the VID/PID list is a hint, not a hard filter.

Each candidate port is then **probed** (§1.4). A port counts as a tracker only if the
probe returns a valid info reply.

Cross-platform guidance [inferred]: on macOS/Linux, list `/dev/cu.usbmodem*` /
`/dev/ttyACM*` (and `/dev/ttyUSB*`). Prefer ports whose USB VID:PID matches the table,
but still allow probing any port the user selects.

A Windows driver installer for these virtual COM ports ships inside the tool
[observed]. macOS and Linux CDC-ACM class drivers are expected to work without it
[inferred, UNKNOWN for MediaTek composite devices].

### 1.2 Serial port settings [observed]

| Setting                | Value                                           |
|------------------------|-------------------------------------------------|
| Baud rate              | 115200                                          |
| Data bits              | 8 (platform default; not set explicitly)        |
| Parity                 | None                                            |
| Stop bits              | 1                                               |
| Flow control/handshake | None (not set explicitly)                       |
| DTR                    | asserted (on)                                   |
| RTS                    | asserted (on)                                   |
| Read timeout           | 500 ms per low-level read                       |
| Write timeout          | 500 ms                                          |
| Receive buffer         | 1 MiB                                           |

For a real USB CDC device the baud rate is likely irrelevant [inferred], but use
115200 8N1 anyway. Asserting DTR is commonly required for CDC devices to start
talking [inferred]; keep it asserted.

### 1.3 Opening and closing [observed]

- Opening is retried up to **4 attempts**, with a 300 ms delay between attempts.
- If the port is already open by someone else, opening fails. Report this as "port in use".
- Close: just close the port. Before closing a configuration session, the tool sends
  the disconnect command (see 03-commands) without waiting for a reply.

### 1.4 Probe sequence (per candidate port) [observed]

1. Open the port (§1.3).
2. Send `.log:0\r` (turn the debug log stream off) and read until the line is idle
   (§3). The content is ignored.
3. Send `:cfg_info:?\r` and read until idle. Parse it as the *info reply*
   (03-commands §2). Stop here if it is not valid.
4. Optional extra reads, depending on feature flags from the info reply: some
   binary status parameters (02-framing) and an asset/BLE-chip version query
   (03-commands §8).
5. Send `.log:1\r` (turn the debug log back on). No reply is awaited.
6. Close the port.

Configuration then happens in a separate session: open, then `:cfg_connect\r`
(03-commands §1).

## 2. Bluetooth Classic (SPP)

### 2.1 Profile and connection [observed]
- Bluetooth **Classic (BR/EDR)** only. The tool opens an **RFCOMM** socket to the
  tracker's **Serial Port Profile** service (service class UUID
  `00001101-0000-1000-8000-00805F9B34FB`) at the tracker's 48-bit BT address. The RFCOMM
  channel is resolved through the SDP lookup of that UUID. No fixed channel number is used.
- It does **not** use an OS virtual COM port ("Standard Serial over Bluetooth link"). It
  connects with a socket directly to the address.
- Connect sequence: wait 100 ms, then try to connect. If the OS reports the socket as
  in use or "invalid argument" (Windows WSA error 10022), wait about 100 ms and retry, up
  to **3 attempts** in total. Any other error fails at once.
- If the link has dropped when a read is attempted, the tool tries **once** to reconnect
  to the same address before failing.
- Once connected, the byte stream carries exactly the same text and binary commands
  and replies as USB (02-framing, 03-commands). **No baud/DTR/RTS settings apply.**

### 2.2 Discovery [observed]
The tool runs its **own Classic inquiry** continuously in the background. It does not
just list OS-paired devices, but it only *accepts* paired ones:

1. Run a device inquiry (maximum 255 results) that returns **authenticated (paired)**
   devices which are **currently in range**. Remembered-but-absent and unknown (unpaired)
   devices are not requested.
2. Keep only devices where:
   - **paired/authenticated** = true, and
   - the **Class-of-Device service-class field** (CoD bits 13–23) **equals exactly**
     `0x308` = Positioning | Audio | Telephony. In raw 24-bit CoD terms this means bits
     16, 21 and 22 are set and no other service bit (including limited-discoverable, bit 13)
     is set. Equivalently, the CoD value masked with 0xFFE000 equals 0x610000.
     The major/minor device class bits are not checked.
3. De-duplicate by BT address and keep the most recent "last seen" time.
4. A device is listed while it was seen within the **last 1 minute**.
5. As soon as one inquiry finishes, start the next.
6. **No name filtering** is applied. The device name is shown to the user as-is. The
   tracker's advertised name is configurable on the device (parameter 801 "Local name",
   up to 16 or 30 characters depending on model). An empty value means the firmware
   default [inferred: the default is model and IMEI based; UNKNOWN exact form].
7. Each listed device is then **probed** exactly like a USB port (§1.4): connect,
   `.log:0`, `:cfg_info:?`, and so on.
8. If the OS has no Bluetooth radio, BT discovery is disabled silently.

Cross-platform guidance [inferred]: macOS (IOBluetooth) and Linux (BlueZ) can both open
an RFCOMM channel to SPP after SDP lookup. Treat the CoD check as a hint. Some stacks
report CoD differently or cache it, so allow manual address entry as a fallback.

### 2.3 Pairing and PIN [observed]
- The PC side performs **no pairing**. It never asks for a PIN, never requests pairing from
  the OS and never sets a passkey. The user must pair the tracker in the OS Bluetooth
  settings first. (The tracker-side Bluetooth control commands in 03-commands §8 act on the
  tracker's own radio and its own paired-device list, not on the PC's pairing.)
- The PIN is a **tracker configuration parameter**, not something the tool supplies:

  | Param ID | Meaning | Values / default |
  |---:|---|---|
  | 800 | BT radio | 0 = disabled, 1 = enabled (hidden), 2 = enabled (visible). Default 2 |
  | 801 | Local (advertised) name | string, empty by default |
  | 802 | **Local PIN** | exactly 4 digits. **Default `5555`** |
  | 803 | Security mode | 0 = PIN only, 1 = PIN + MAC list, 2 = MAC list only, 3 = none (model dependent). Default 0 |
  | 806 | External PIN | 0–8 digits (used when the *tracker* connects to other devices) |
  | 830–834 | Authorized MAC list | 5 entries, 12 hex characters each |

  So on a factory-default tracker, pairing uses legacy PIN **5555** [observed as the default
  value in the tool's parameter definitions; inferred to be the factory value]. Whether the firmware uses legacy PIN or
  Secure Simple Pairing is **UNKNOWN**.
- The tracker must have BT radio enabled (800 = 1 or 2). It must be visible (800 = 2) to be
  found by an inquiry before pairing [inferred].

### 2.4 Differences from USB [observed]

| Aspect | USB | Bluetooth |
|---|---|---|
| Open attempts | 4 tries, 300 ms apart | 3 connect tries (only on "in use" errors), about 100 ms apart |
| Idle-read completion ("read until idle") | 200 ms of silence | polls every 100 ms; finishes after **about 1000 ms** without data (02-framing and 03-commands quote the 200 ms USB figure; use the longer value on Bluetooth) |
| Read full configuration timeout | 30 s | **90 s** |
| Save timeout (FMB640 family) | 20 s | 30 s |
| Other command timeouts | 30 s | 30 s |
| Write path | pending input and output discarded before each write | pending input drained and discarded before each write |
| Probe sequence | §1.4 | identical |

## 2A. Bluetooth Low Energy (BLE)

**The Configurator does not use BLE on the PC side at all** [observed]:
- The PC side performs no BLE scanning, advertising parsing or GATT access. Its only
  Bluetooth functions are the Classic inquiry and RFCOMM connection described in §2.
- So **configuration over BLE is not supported by this tool**. No GATT service or
  characteristic UUIDs, MTU or BLE framing can be learned from it.
- Every "BLE" feature in the tool is something the **tracker** does with its own radio
  (scanning for BLE sensors and beacons, for example). The PC asks the tracker to do it over the
  normal serial link (binary command 0x0018 "scan BT/BLE devices" and 0x001A "connect",
  03-commands §7), and the tracker reports the results.
- There is therefore **no mapping** from a BLE-discovered device to a Classic SPP link in
  this tool.
- Advertising names, manufacturer IDs or service UUIDs that identify Teltonika trackers
  over BLE: **UNKNOWN from this artifact.** Whether the tracker firmware itself offers a BLE
  configuration channel (for example to a mobile app) is also UNKNOWN; it cannot be
  determined from the PC tool. An implementation must use Classic SPP (§2) or USB (§1).

## 2B. Input flushing before every command [observed]
On both transports, any unread input is **discarded immediately before each command is
written**. Late or unsolicited bytes from an earlier exchange or the debug log therefore
never leak into the next reply. Implementations should do the same: flush or drain the
input, write, then read.

## 3. Read framing at the byte-stream level [observed]

There is no single framing for everything. The tool uses three read strategies:

| Strategy            | Used for                                   | Completion rule                                                               |
|---------------------|--------------------------------------------|-------------------------------------------------------------------------------|
| Read until suffix   | text commands with a known terminator      | Keep reading until the received buffer **ends with** a given byte sequence    |
| Read until idle     | info query, log on/off, asset query        | Keep reading. Stop once data has arrived and then **200 ms** (USB) / **about 1 s** (BT) pass with no new bytes |
| Binary frame        | the binary ("FMBX") protocol               | Read a 12-byte header, then `length + 2` more bytes (02-framing)              |

Each whole operation also has an overall timeout (default **30 s**; per-command
exceptions are in 03-commands). Timing out or getting a malformed reply counts as failure.

All text commands are 7-bit ASCII. Replies are decoded as ASCII.

## 4. Concurrency [observed]

Commands are serialized: only one command and reply exchange is in flight at a time.
A new implementation should do the same, using one mutex per port.

## UNKNOWN / open

- Exact behaviour when DTR/RTS are left de-asserted.
- Whether any tracker model needs a specific baud rate on a real UART (not USB),
  for example via an RS-232 adapter.
- Bluetooth: legacy PIN vs Secure Simple Pairing on current firmware. The exact default
  "Local name" format.
- BLE identification of trackers (not observable in this tool; see §2A).
