> Status: released v7 (partial) — reviewed 2026-10-24. Items marked UNKNOWN/INFERRED are not yet confirmed.

# 03 – Commands

**[observed]** = evident in the shipped tool; **[inferred]** =
reasoned, not yet confirmed on hardware; **UNKNOWN** = not established.
Framing and read-completion rules are defined in 02-framing; "until CR", "until idle"
and "until marker" refer to 02-framing §1.2.

Notation: `\r` = 0x0D. `<n>` = a decimal integer in ASCII.

## 0. Typical session [observed order, from the tool's flow]

```
open port (01-transport)
  -> ".log:0\r"            (read until idle, ignore)         [probe / before info & keyword status]
  -> ":cfg_info:?\r"        (read until idle)                 device info, §2
  -> ":cfg_connect\r"       (read until "<CFG_CONNECT>\r")    enter configuration session, §1
  -> ":sec_status\r"        (read until CR)  [only if keyword is supported, §6]
  -> ":sec_login:<kw>\r"    (read until CR)  [only if the device is protected and not yet authorized]
  -> ":cfg_getcfg\r"        (read until marker)               read full configuration, §3
  ... user edits ...
  -> ":cfg_setparam:...\r"  xN (read until CR each)           write changed parameters, §4
  -> ":cfg_save\r"          (read until CR)                    commit to flash, §4.3
  -> ":cfg_disconnect\r"    (no reply awaited)                 §1
close port
```

## 1. Session: connect / disconnect [observed]

| Command | Bytes sent | Reply | Success rule | Timeout |
|---|---|---|---|---|
| Connect | `:cfg_connect\r` | read until the received bytes end with `<CFG_CONNECT>\r` | reply text contains `<CFG_CONNECT>\r` | 30 s |
| Disconnect | `:cfg_disconnect\r` | none (the tool closes the port right after sending) | – | – |

## 2. Device info [observed]

Request: `:cfg_info:?\r`. Reply: read until idle (200 ms of silence on USB, about 1 s on Bluetooth; see 01-transport). Overall timeout 30 s.

Reply format: one or more lines separated by `\r`, each of the form

```
cfg_info:<key>:<value>
```

After the literal 9-character prefix `cfg_info:`, each line is split at the **first** `:`. The key must be a decimal integer. Lines whose value
contains non-printable characters are ignored. The value can itself contain `:`.

| Key | Meaning | Value format |
|---:|---|---|
| 0 | Firmware version | dotted version, e.g. `03.29.00` [example inferred] |
| 1 | Configuration (definition) version | dotted version |
| 2 | Hardware version code | e.g. `FMB1:6`. Maps to a model name through the hardware table (04-parameters) |
| 3 | IMEI | 15 digits |
| 4 | Active SIM | integer: `0` = SIM1 is active, any other value = SIM2 is active |
| 5 | SIM1 status | one of the status words below |
| 6 | SIM2 status | one of the status words below |
| 7 | Active SIM PIN retries left | integer (applies to the SIM named by key 4) |
| 8 | Keyword (password) state | integer. **Present = keyword feature supported**. `1` = session already authorized |
| 9 | Active SIM lock enabled | `1` = yes |
| 10 | Record reading enabled | `1` = yes |
| 11 | **Spec ID** | integer. Defaults to **1** if the key is absent |
| 12 | Firmware/hardware feature bitmask | unsigned 64-bit decimal. Bit *n* set = feature *n* available |
| 13 | Firmware revision | free text, shown to the user only; not interpreted |
| 14 | Hardware options | list of option tokens separated by `:`. Each token is matched against hardware-option conditions in the parameter definitions (04-parameters); it is not otherwise interpreted. No definition in this tool version uses such conditions, so the key can be stored and ignored |
| 16 | Supports compressed firmware upload | `1` = yes |

SIM status words [observed vocabulary]: `ABSENT`, `INIT`, `NEED PIN`, `ENTERING`,
`WORKING`, `NEED PUK`. Treat any other text as unknown and show it verbatim.

Keys not in the table (including 15) are not used by the reference tool. Ignore them.
Their meaning is UNKNOWN.

Integer keys (4, 7–11, 16) must parse as decimal integers. A line whose value does not
parse is ignored.

Validity [observed]: the reply is accepted only if keys **5 and 6** are both present and
non-empty. Otherwise the probe fails (the port is not treated as a tracker).

Two different feature masks [observed]:
- **Info key 12** (64-bit) controls which parameters and pages the definitions show for
  this device (04-parameters, "Feature" conditions). Known bit: bit 0 = BLE available
  [observed as a definition condition]. Other bit meanings are UNKNOWN beyond their use as
  definition conditions. Bit 14 also enables the BLE-chip version query below.
- **Status parameter 784** (binary protocol) controls which binary commands may be sent
  (§7). The two masks are independent and their bit numbers are unrelated.

Feature bit 14 [observed]: if set, the tool also sends `@asset_send:asset info\r`
(read until idle) and pulls a BLE-chip firmware version from the text after
`Code Version:`.

## 3. Read full configuration [observed]

Request: `:cfg_getcfg\r`.
Reply: read until the received bytes end with the 5-byte marker `17 1C 14 00 0D`.
Timeout: 30 s (USB), 90 s (Bluetooth) per attempt.

Retry [observed]: up to **10 attempts** in total. After a failed attempt (timeout or a
reply that fails the rules below), wait **1 s**, then send `:cfg_getcfg\r` again and read
again. The session is not re-opened and `:cfg_connect` is not repeated between attempts.
After the 10th failure the read fails.

Maximum reply size: no protocol limit is defined. The reference tool's receive buffer is
1 MiB, so replies are expected to be well below that [inferred].

Tag text [observed]: the reference tool never compares the opening or closing tag text of
this reply. It relies only on the structure below. The tag names are therefore not needed
for interoperability and remain UNKNOWN. Implementations must parse by structure.

Reply format rules (all observed):

- The reply contains an opening tag in angle brackets, then the parameter lines, then a
  closing tag in angle brackets. The first `<` and `>` delimit the opening tag; the last
  `<` and `>` delimit the closing tag, which comes after the opening tag.
- **Body** = the text strictly between the opening tag's `>` and the closing tag's `<`.
- **Count** = the decimal integer between the **last `:`** in the reply and the following
  `\r`.
- The body is `\r`-separated. Each non-empty line has the form `<id>:<value>` (split at the
  first `:`; the id must be non-empty) and is one parameter.
- The read is valid only if the number of parameters **equals Count**; otherwise it is a
  failure (and is retried, see above).

So the reply looks like this [inferred; the tag names are UNKNOWN]:

```
<OPEN_TAG>\r
101:4\r
102:2\r
...\r
<CLOSE_TAG>:<count>\r
17 1C 14 00 0D
```

Notes:
- The device returns **all** parameters it has, as raw strings. Typing and validation
  come from the definitions (04-parameters).
- Values can contain `:` (URLs, for example) because only the first `:` splits.
  Values never contain `\r`.

## 4. Write configuration [observed]

### 4.1 Set parameters
Format: `:cfg_setparam:` followed by `id:value` items separated by `;`, terminated by `\r`:

```
:cfg_setparam:101:4;102:2;2001:internet\r
```

- **Chunking [observed]:** each `:cfg_setparam:` command is **at most 1024 bytes in total,
  counting the `:cfg_setparam:` prefix, all items, the `;` separators and the final `\r`**.
  Items are packed in order. A new command is started when the next item would not fit
  (precisely: a new command starts when *current length including the trailing `;`* plus
  *the next item's length* reaches 1024 or more). An item is never split across commands.
  Example: ten items of 155 characters each (`8000:` plus 150 characters) are sent as two
  commands: 6 items (950 bytes) and 4 items (638 bytes).
- Items are sent in the order the application supplies them. No ordering is required
  [inferred].
- Items are formatted as `<decimal id>:<value text>` (locale-independent: `.` as the
  decimal separator) [observed].
- Escaping of `;` or `:` inside values: **none observed**. Treat values containing `;` or
  `\r` as invalid [inferred].
- The tool writes the parameters that changed (a diff against what was read), not the
  whole configuration [inferred from the UI flow; the protocol accepts any subset].

Reply to each chunk: read until CR. The text must match the pattern
`<WORD>:<digits>` (an angle-bracket tag, a colon, a decimal number). The number must
**equal the number of items in that chunk**. Otherwise the write fails. The tag is
believed to be `<SETPARAM_RESULT>` [inferred; not confirmed]. The tag text is not
compared, only the number.
Timeout per chunk: 30 s.

Failure handling [observed]:
- **No retries.** If a chunk times out or its count does not match, the whole write stops
  at once. Later chunks are not sent and **`:cfg_save` is not sent**, so nothing is
  committed to flash. Chunks the device already accepted stay in its working (unsaved)
  configuration [inferred].
- The device reports only an **accepted-item count** per chunk. There are no per-item
  error codes. If the count is lower than sent, which items were rejected is UNKNOWN. A
  reasonable recovery is to re-read the configuration (§3) and compare [inferred].

### 4.2 Values
- Integers and enums: decimal.
- Strings: raw ASCII.
- Lists (phone numbers, iButton IDs, operator codes): one parameter ID per element,
  in consecutive IDs (04-parameters).

### 4.3 Save (commit)
Request: `:cfg_save\r`. Reply: read until CR, pattern `<WORD>:<digits>`. The value
**`1` means success**; anything else is a failure. Timeout: 30 s by default; 20 s on
USB and 30 s on Bluetooth for the FMB640 family [observed].

## 5. Reset to defaults, reboot, log control [observed]

| Command | Bytes | Reply / rule |
|---|---|---|
| Reset to defaults | `:cfg_default\r` | Read until CR; expect the line `<DEFAULT_CFG_END>\r` (exact, case-sensitive match). Then read until CR again and parse a save-style `<WORD>:<digits>` reply where `1` = success. Timeout 30 s for each read. Any other first line means failure. |
| Reboot | `.reset\r` | No reply awaited. The device drops off USB and re-enumerates [inferred]. |
| Debug log off | `.log:0\r` | read until idle, content ignored |
| Debug log on | `.log:1\r` | none awaited |

## 6. Keyword (configuration password) [observed]

All keyword commands are read until CR. The status reply carries a line containing
`<SECSTAT>` (case-insensitive), followed by **5 comma-separated integers**:

```
<SECSTAT>r,s,a,l,n
```

| Pos | Meaning |
|---:|---|
| r | last operation succeeded (`1`) |
| s | device is secured (has a keyword) (`1`) |
| a | current session is authorized (`1`) |
| l | device is locked (too many attempts) (`1`) |
| n | retries left (integer) |

| Command | Bytes |
|---|---|
| Status | `:sec_status\r` (sent after `.log:0\r`) |
| Log in | `:sec_login:<keyword>\r` |
| Set keyword | `:sec_setkey:<a>,<b>\r`. Two fields; their meaning is UNKNOWN (probably the new keyword plus a confirmation or the old keyword) |
| Change keyword | `:sec_changekey:<new>,<old>\r` |
| Remove keyword | `:sec_removekey\r` |
| Get unlock code | `:sec_getunlock\r` (sent after `.log:0\r`) |
| Unlock | `:sec_unlock:<code>\r` |

Replies [observed]:
- **Status, log in, set, change, remove and unlock** all return the same `<SECSTAT>`
  status line described above. Success of the operation is field `r` = `1`. Then check `a`
  (authorized), `s` (secured) or `l` (locked) as appropriate.
- **Get unlock code** returns a line starting with `<UNLCKCODE>:`. The rest of the line
  is the unlock code. Any other reply is a failure. In the reference tool this exchange runs
  as a self-contained session: open the port, `.log:0\r` (read until idle),
  `:sec_getunlock\r` (read until CR), `.log:1\r`, close. How the unlock code is turned into
  the value for `:sec_unlock:` (for example by a vendor service) is UNKNOWN. The tool
  only shows the code.
- Timeout: 30 s per command. No retries.

Keyword character set and length: see the keyword validation rule in 04-parameters
(UNKNOWN until extracted). The keyword is sent in clear text.

## 7. Binary ("FMBX") commands [observed]

Frame format: 02-framing §2. "Min ver" = minimum binary protocol version (status
param 781). "Feature" = the bit(s) that must be set in the feature mask (status
param 784).

| Cmd | Name (descriptive) | Expected response | Min ver | Feature |
|---:|---|---:|:--:|---:|
| 0x0002 | Get status data | 0x0001 (count), then 0x0003 × count; see §7.6.2 | 0.0 | – |
| 0x0102 | Get extended status data (by group) | 0x0001 (count), then 0x0103 × count; see §7.6.2 | 0.1 | 0x800 |
| 0x0004 | Reset "sent data" counter | 0x0005 | 0.0 | – |
| 0x0006 | Reset "received data" counter | 0x0007 | 0.0 | – |
| 0x0008 | Reset "sent SMS" counter | 0x0009 | 0.0 | – |
| 0x000A | Reset "received SMS" counter | 0x000B | 0.0 | – |
| 0x000C | Format SD card | 0x000D (0x000F = failed) | 0.0 | – |
| 0x0010 | Get total records count | 0x0001 (ack), then 0x0011 (count); see §7.5 | 0.0 | – |
| 0x0012 | Get records (next batch) | 0x0039 (progress, zero or more), 0x0001 (ack), then 0x0013 (data) if the ack value is non-zero; see §7.5 | 0.0 | – |
| 0x0015 | Remove read records | 0x0001; see §7.5 | 0.0 | – |
| 0x0016 | Request LLS fuel sensor IDs | 0x0017 | 0.0 | – |
| 0x0018 | Scan BT/BLE devices | 0x0019 | 0.1 | 0x2 |
| 0x001A | Connect to BT/BLE device | 0x001B | 0.1 | 0x2 |
| 0x001C | Request device info | 0x001D (never sent by the tool; §7.6.5) | 0.1 | 0x1 |
| 0x001E | Request GNSS info | 0x001F (never sent by the tool; §7.6.5) | 0.1 | 0x1 |
| 0x0020 | Request GSM info | 0x0021 (never sent by the tool; §7.6.5) | 0.1 | 0x1 |
| 0x0022 | Request I/O info | 0x0001 (count), then 0x0023 × count; §7.6.5 | 0.1 | 0x1 |
| 0x0024 | Request OBD info | 0x0001 (count), then 0x0025 × count; §7.6.5 | 0.1 | 0x1 |
| 0x0026 | Request LV-CAN info | 0x0001 (count), then 0x0027 × count; §7.6.5 | 0.1 | 0x1 |
| 0x0028 | File management request | 0x0029 | 0.1 | 0xC |
| 0x002A | File data packet | 0x002D | 0.1 | 0xC |
| 0x002C | File/folder data ack/nack | 0x002D | 0.1 | 0xC |
| 0x002E | Toggle digital output | 0x002F | 0.1 | 0x4 |
| 0x0030 | Start accelerometer capture | 0x0031 | 0.1 | 0x4 |
| 0x0032 | Stop accelerometer capture | 0x0033 | 0.1 | 0x4 |
| 0x0034 | Get accelerometer result | 0x0035 | 0.1 | 0x4 |
| 0x0036 | Request Dallas temperature sensor IDs | 0x0037 | 0.1 | 0x10 |
| 0x0038 | **Get single status parameter** | 0x003B (0x003D = nack) | 0.0 | – |
| 0x0042 | Request login list IDs | 0x0043 | 0.1 | 0x200 |
| 0x0044 | Request FMS info | 0x0045 | 0.1 | 0x400 |
| 0x0046 | Request manual CAN info | 0x0047 | 0.1 | 0x400 |
| 0x0048 | Request tachograph info | 0x0049 | 0.1 | 0x400 |
| 0x004A | Request LV-CAN program numbers | 0x004B | 0.1 | 0x200000 |
| 0x0050 | Check TSM connection | 0x0051 | 0.0 | 0x800000 |
| 0x0056 | Get ICCID bind status | 0x0057 | 0.0 | 0x2000000 |
| 0x0058 | Bind ICCID | 0x0059 | 0.0 | 0x2000000 |
| 0x005A | Unlock ICCID | 0x005B | 0.0 | 0x2000000 |
| 0x005E | Reset spec ID | 0x005F | 0.1 | 0x4000000 |
| 0x0060 / 0x0062 / 0x0064 / 0x0066 | Lock: update status / unlock / change password / connect | +1 | 0.1 | 0x1000000 |
| 0x0071 | Scan (CAN) | 0x0072 | 0.0 | 0x200000 |
| 0x0073 | DTC command | 0x0074 | 0.0 | 0x200000000 |

Generic responses: 0x0001 = Ack, 0x0014 = Error, 0xFFFF = unknown.

### 7.1 Get single status parameter (0x0038) [observed]
- Request payload: **2 bytes**, big-endian signed 16-bit status parameter ID.
- Single reply frame, response 0x003B on success (no count frame first, unlike §7.6.2).
  0x003D = not acknowledged [observed in the command table; its payload is UNKNOWN].
- 0x003B payload layout [observed]. Unlike §7.2 there is **no leading skipped byte**:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 2 | `<status_id>`, big-endian (the tool reads it as signed 16-bit) |
| 2 | 1 | `<type>`, same codes as the §7.2 type table |
| 3 | per type | `<value>`; for variable-length types the length field(s) of §7.2 come first |

  How the tool reads the value for the IDs it requests this way [observed]:

| ID | Tool's check / read |
|---:|---|
| 784 | checks `<status_id>` = 784 and `<type>` = 7, else failure; value = 8 bytes at offset 3, big-endian unsigned 64-bit |
| 790 | value = byte at offset 3, non-zero = true (type not checked) |
| 810 | value = bytes from offset 3 to the end, read as a big-endian unsigned 64-bit number (8 bytes expected; type not checked) |
| 803 | value = bytes from offset 5 to the end, shown as hex; consistent with type 24 (2-byte length at offsets 3–4) [inferred] |

- Known status parameter IDs (see §7.6.4 for the full catalogue structure):

| ID | Meaning |
|---:|---|
| 773 | external power voltage, mV; value 0 means "running on battery only" [observed] |
| 781 | binary protocol version (byte0 = major, byte1 = minor) |
| 784 | binary protocol feature mask (u64) |
| 787 | LV-CAN adapter name |
| 788 | LV-CAN adapter ID |
| 790 | LV-CAN present (boolean) |
| 800 | CAN chip information frame |
| 803 | chip ID |
| 810 | additional feature bits (meaning UNKNOWN) |
| 823 | CAN config info |
| 825 | DTC status |
| 826 | DTC module numbers |

### 7.2 Status data payload (responses 0x0003 / 0x0103, and 0x0023 / 0x0025 / 0x0027) [observed]
This layout applies to every data frame that follows the count frame of §7.6.2. Each data
frame is parsed on its own and the records of all frames are concatenated.
- Byte 0: skipped (meaning UNKNOWN; possibly a group or status code).
- Then repeated records from offset 1 until the payload ends:

| Field | Size | Notes |
|---|---|---|
| Status ID | 2 bytes (u16 BE) for 0x0003; **4 bytes (u32 BE)** for 0x0103 | |
| Type | 1 byte | see table below |
| [Length] | 1 byte, or 2 bytes (BE) for type 24 | only for variable-length types |
| Value | per type | |

| Type | Value | Size |
|---:|---|---|
| 0 | u8 | 1 |
| 1 | i8 | 1 |
| 2 | i16 | 2 |
| 3 | u16 | 2 |
| 4 | i32 | 4 |
| 5 | u32 | 4 |
| 6 | i64 | 8 |
| 7 | u64 | 8 |
| 8 | float32 | 4 |
| 9 | float64 | 8 |
| 16 | text string (bytes decoded as ISO-8859-1) | variable (1-byte length) |
| 17 | u32 (alternate rendering) | 4 |
| 18 | 6-byte array (e.g. MAC) | 6 |
| 19 | byte array | variable (1-byte length) |
| 20 / 21 / 22 / 23 | hex rendering | 1 / 2 / 4 / 8 |
| 24 | string | variable (**2-byte** length) |
| 25 | hex rendering | variable (1-byte length) |
| 32 | string | variable (1-byte length) |

Value encoding [observed]:
- All integers are **big-endian**; types 1, 2, 4 and 6 are two's-complement signed.
- Type 8 is an IEEE-754 single and type 9 an IEEE-754 double, both big-endian.
- Types 16, 24 and 32 are text, decoded as ISO-8859-1 (one byte per character). Types 32 and
  24 are decoded the same way as 16; only the length field differs for 24.
- Types 20–23 and 25 are shown as `0x` followed by the value bytes in upper-case hex, in
  wire order, with no separators (an empty value shows as an empty string).
- Types 18 and 19 are kept as raw bytes.

Parse-stop rules [observed]. Parsing ends quietly and the records read so far are kept when:
- the type code is not in the table;
- the declared value would run past the end of the payload;
- the status ID field itself ends exactly at the end of the payload (no type byte follows).

There is no per-record error; a truncated frame simply yields fewer records.

### 7.3 Extended status request (0x0102) [observed]
Payload: an **8-byte big-endian bitmask** of groups: 0x1 device info, 0x2 GNSS, 0x4
GSM, 0x8 base I/O, 0x10 OBD, 0x20 LV-CAN, 0x40 FMS, 0x80 manual CAN, 0x100
tachograph, 0x200 beacons, 0x400 Modbus, 0x800 spec I/O, 0x1000 FMS eco-driving,
0xFFFF all. The reply is a count frame plus data frames (§7.6.2). What each group returns
is in §7.6.3.

### 7.4 File transfer: file management (0x0028), file data (0x002A), completion (0x002C) [observed unless marked]

Records are covered in §7.5. All multi-byte numbers are big-endian [inferred for item
IDs and values, consistent with §7.2; **observed** for the packet number].

#### 7.4.1 Typed item encoding (requests and replies)

File-management requests and replies are sequences of **typed items**:

| Field | Size | Notes |
|---|---|---|
| Item ID | 2 bytes | meaning depends on direction and command (tables below) |
| Type | 1 byte | same type codes as §7.2: 0 u8, 1 i8, 2 i16, 3 u16, 4 i32, 5 u32, 6 i64, 7 u64, 8 float32, 9 float64, 16 string |
| Length | 1 byte | **only for type 16 (string)**: number of bytes that follow |
| Value | per type | strings are ASCII, no terminator [inferred: ASCII] |

#### 7.4.2 File-management request (command 0x0028)

Payload: **1 byte sub-command**, then typed items back to back.

| Sub-command | Meaning |
|---:|---|
| 0 | delete file |
| 1 | upload file (host → device) |
| 2 | download file (device → host) |
| 3 | list directory |
| 4 | file information |

Request item IDs:

| ID | Meaning | Type used by the tool |
|---:|---|---|
| 1 | path on the device | string (16) |
| 2 | file size in bytes | u32 (5) |
| 3 | number of data packets that will follow | u32 (5) |
| 4 | data payload size per packet | u16 (3) [inferred type] |
| 5 | CRC of the whole file | u16 (3); CRC-16/ARC, the same algorithm as the frame checksum (02-framing) |
| 6 | upload target selector (certificates): 0 root certificate, 1 device certificate, 2 private key | UNKNOWN type |
| 7 | download target selector: 0 all, 1 continuous log, 2 root certificate, 3 device certificate, 4 device private key | UNKNOWN type |

Upload request (sub-command 1) carries: file size (2), payload size per packet (4) = **512**,
number of packets (3) = size divided by 512, rounded up (1 for sizes up to 512), file CRC (5), device path (1)
[observed values; item order inferred as listed]. Download request (sub-command 2) carries
the device path (1) only [observed]. Certificate transfers use items 6/7 instead of or in
addition to the path [inferred]. Delete, list and info layouts beyond the shared item format
are UNKNOWN except the info reply below.

Timeout: the tool allows **240 s** for the reply to a download request [observed]. Other
sub-commands: UNKNOWN (02-framing default applies).

#### 7.4.3 File-management reply (response 0x0029)

Reply items always start with: item 1 = **answer type** (u8; echoes the sub-command:
1 for upload, 2 for download, 4 for file info), then item 2 = **return code** (u8; 0 = OK).
The tool checks the item IDs and positions strictly and rejects a reply whose items are
out of order.

Upload reply (fixed positions):

| Offset | Content |
|---:|---|
| 0–3 | item 1, type u8, value 1 (upload) |
| 4–7 | item 2, type u8, return code |
| 8–11 | item 3, type u8, **number of data packets the host may send before it must wait for an ack** (0 = only after the last packet) |

Upload / file-info return codes: 0 OK, 1 general error, 2 file already exists (delete it
first), 5 wrong parameters, 6 not enough space; 3 and 4 are not named (treat as error).

Download reply (fixed positions, total 32 bytes):

| Offset | Content |
|---:|---|
| 0–3 | item 1, type u8, value 2 (download) |
| 4–7 | item 2, type u8, return code: 0 OK, 1 general error, 3 file not found, 4 wrong download path, 5 incorrect parameters (2: not named) |
| 8–14 | item 4, u32 = number of data chunks that will follow |
| 15–19 | item 5, u16 = chunk size |
| 20–24 | item 6, 2 bytes = CRC of the file (CRC-16) |
| 25–31 | item 7, u32 = file size |

Note: the reply item IDs (4–7) do **not** use the request ID meanings of §7.4.2 [observed].

File-info reply (sub-command 4; total 22 bytes): item 1 (u8 = 4), item 2 (u8 return code),
item 7 (i32 = file size), item 12 (i32 = time stamp; unit UNKNOWN, probably seconds since
1970). A size/time stamp of 0 is treated as "no file" [inferred].

#### 7.4.4 Upload exchange (host to device)

Frames: request 0x0028 (sub-command 1), reply 0x0029, data 0x002A, ack 0x002D.

| Direction | Frame | Content and conditions |
|---|---|---|
| host to device | 0x0028 | upload request (§7.4.2) |
| device to host | 0x0029 | upload reply (§7.4.3). Any return code other than 0 means no data follows. Item 3 gives N, the number of packets the host may send before it must wait for an ack; N = 0 means "send everything, ack only after the last packet". Whether this reply also uses the count-then-frames form of 02-framing §2.6 is UNKNOWN; accept both. |
| host to device | 0x002A, repeated | payload = **4-byte big-endian packet number** (first packet is 1) followed by up to **512 file bytes** (the last packet may be shorter). No reply is read for a packet unless it is covered by the next row. |
| device to host | 0x002D | sent for every packet whose number is a multiple of N, and for the last packet |

Ack payload (0x002D), 5 bytes:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 1 | return code (u8) |
| 1 | 4 | packet number (i32, big-endian) |

| Code | Meaning (reference tool behaviour) |
|---:|---|
| 0 | OK, continue |
| 1 | OK, continue (difference from 0 UNKNOWN; possibly "complete") |
| 2 | NACK: the host re-sends the packet named in bytes 1-4 and expects an ack again |
| 3 | data error: the upload has failed |
| other | unexpected: the upload is treated as failed |

The upload is complete when the ack for the last packet carries code 0 or 1. There is no
separate completion command for uploads [observed].

#### 7.4.5 Download exchange (device to host)

Frames: request 0x0028 (sub-command 2), reply 0x0029 (multi-part), chunks 0x002B,
completion 0x002C.

| Direction | Frame | Content and conditions |
|---|---|---|
| host to device | 0x0028 | download request with the device path (§7.4.2) |
| device to host | 0x0029 (first) | payload is only a **count** (1, 2 or 4 bytes, read as u8, u16 or u32 by payload length). For a download the count must be exactly 1 [observed]. |
| device to host | 0x0029 (second) | the download reply of §7.4.3. Return code other than 0 means no chunks follow. |
| device to host | 0x002B, chunk-count times | unsolicited, no request per chunk and **no per-chunk ack**. Payload = **4-byte u32** `<chunk_number>` followed by the chunk bytes (payload length minus 4). |
| host to device | 0x002C | completion: **1-byte payload, 0 = success, 3 = failure** (same code values as the upload ack). Sent **without waiting for any reply** [observed]. |

Assembling the file [observed]:
- After the last chunk, the host sorts the chunks by `<chunk_number>` (ascending) and joins
  their data bytes in that order. `<chunk_number>` is used **only as an ordering key**.
  Whether the device sends a running sequence number or a byte offset is UNKNOWN; ascending
  order gives the same result for both.
- The reference tool does **not** check the assembled file against the size or the
  CRC-16/ARC in the download reply, and does not check for gaps or duplicate numbers.
  (This replaces the v4 text, which inferred such a check.) A new implementation may add
  these checks as an extra safety measure; they are not needed for compatibility.
- The host does not read a reply to 0x002C. Whether the device sends one is UNKNOWN; if it
  does, the host must tolerate (discard) an unexpected 0x002D frame [inferred].

Response-ID pairing (open-questions row 13): 0x002D is the device's ack to file data packets
(0x002A); 0x002C is the host's completion code after a download, using the same code values.
The 0x002C to 0x002D entry in the command table is the reference tool's generic default.

### 7.5 Records: count (0x0010), read batch (0x0012), remove (0x0015) [observed unless marked]

Records are the device's stored AVL data. The host reads them in batches and may remove each
batch from the device after it has been read. All multi-byte numbers are big-endian
[inferred, consistent with §7.2].

#### 7.5.1 Payloads

| Frame | Direction | Payload |
|---:|---|---|
| 0x0010 | host to device | empty |
| 0x0012 | host to device | empty |
| 0x0015 | host to device | empty |
| 0x0001 | device to host | ack `<ack_value>`; its width follows the payload length: 1 byte = u8, 2 bytes = u16, 4 bytes = i32. Any other length cannot be read as a value and is treated as 0. |
| 0x0011 | device to host | `<total_records>`: 8-byte signed integer at offset 0 |
| 0x0039 | device to host | progress: `<progress_value>` (i16, offset 0) and a second i16 at offset 2 whose meaning is UNKNOWN (the reference tool ignores it) |
| 0x0013 | device to host | `<records_data>`: one or more AVL data records placed back to back, in the record layout of Teltonika's public Codec 8 data-sending protocol (1-byte I/O IDs). There is no preamble, length, codec ID or record-count header around them [observed from how the reference tool counts records in a saved file]. An empty payload means "no more data". |

#### 7.5.2 Exchange

Reading starts with one count exchange and then repeats the batch exchange until an end
condition is met. The host sends no ack for any device frame and does not retry.

| Phase | Host sends | Device replies | Host action |
|---|---|---|---|
| Count | 0x0010 | 0x0001 (ack; value not used), then 0x0011 | `<total_records>` = 0: tell the user there are no records and stop. Otherwise keep the total for progress display only (it does not limit or end the reading). |
| Batch | 0x0012 | 0x0039, zero or more times | update progress from `<progress_value>`; keep reading |
| | | then 0x0001 with `<ack_value>` = 0 (or an unreadable value) | end: success |
| | | then 0x0001 with `<ack_value>` non-zero, followed by one 0x0013 | empty 0x0013 payload: end, success. Otherwise append `<records_data>` to the result. |
| Remove (only when removal is enabled, after every non-empty batch) | 0x0015 | 0x0001 (value not checked) | send the next 0x0012 |
| Next batch (removal not enabled) | 0x0012 | as in the Batch row | |

- While waiting for the reply to 0x0012, the host accepts only 0x0039 and 0x0001 frames
  [observed]. The meaning of a non-zero `<ack_value>` beyond "data follows" is UNKNOWN
  (possibly the number of records in the batch).
- A timeout ends the reading with an error [inferred]. Whether records read before the
  failure are saved is UNKNOWN.

#### 7.5.3 Removal confirmation (feature bit 15)

| Feature mask (status parameter 784) bit 15 (0x8000) | Behaviour of the reference tool [observed] |
|---|---|
| set | the user is asked whether read records should be removed from the device; 0x0015 is sent after each batch only if the user agrees |
| clear | 0x0015 is sent after every non-empty batch, without asking |

Because devices without bit 15 always lose the records that were read, a new
implementation should warn the user before reading from such a device [recommendation, not
reference behaviour].

#### 7.5.4 Timeouts per transport

| Transport (01-transport) | Reply timeout for every frame in §7.5.2 |
|---|---|
| USB serial | 30 s |
| Bluetooth | 60 s |
| other | the normal reply timeout of 02-framing [inferred] |

The timeout applies to each awaited frame separately, not to the whole reading [observed].
The mapping of the two values to USB and Bluetooth is [inferred] from the transport type
codes.

#### 7.5.5 Saved file

The reference tool saves the result as a binary file (default extension `.bin`). It is
written once, after the reading has ended successfully; nothing is written if no records
were read.

| Offset | Size | Field |
|---:|---:|---|
| 0 | 2 | `<imei_length>`, u16 [big-endian inferred] |
| 2 | `<imei_length>` | IMEI of the device (the value of info key 3, §2 [inferred]), ASCII digits |
| 2 + `<imei_length>` | rest of file | every non-empty `<records_data>` payload, in the order received, with nothing between them |

Example (synthetic): IMEI `123456789012345` gives the header
`00 0F 31 32 33 34 35 36 37 38 39 30 31 32 33 34 35`, followed by the record bytes.

### 7.6 Live status [observed unless marked]

#### 7.6.1 How the reference tool drives its Status view

Polling lifecycle:

| Event | What the tool does |
|---|---|
| User opens the Status page (page becomes the selected page) | Starts one polling loop for that page, unless a loop for that page is already running |
| User leaves the Status page | Stops the loop. A request already on the wire is abandoned |
| Configuration session ends / port closes | The loop's requests fail. The loop does not stop on its own; it backs off (below) until it is stopped |

One poll cycle:

| State | Request | On success | On failure |
|---|---|---|---|
| Basic mode (start state) | 0x0002 (empty payload) | From the returned records: features = value of 784, protocol version = bytes of 781 (§2.7 of 02-framing), "battery only" = (value of 773 is 0). If feature bit 0x800 is set, switch to extended mode and, in the same cycle, send the extended request below. Otherwise show the 0x0002 records | Wait 30 s, then start a new cycle |
| Extended mode (sticky for the life of the page) | 0x0102 with `<group_mask>` = 0x000000000000FFFF | Show the records | Wait 30 s, then start a new cycle |
| After the view is updated | – | Wait **5 s**, then start a new cycle | – |

Notes:
- After the first successful 0x0002 in which feature 0x800 is set, the tool stops sending
  0x0002 and only sends 0x0102 with all groups each cycle.
- The interval is 5 s **after** the previous cycle finished, not a fixed 5 s period.
- Every cycle replaces the shown values with the new records [inferred].

Other pages that poll [observed]:
- The I/O configuration page polls every 5 s while it is shown, to show current I/O
  values next to the I/O settings. With feature 0x800 it sends 0x0102 with group 0x10 for
  OBD-type I/O tables and 0x20 for LV-CAN-type tables [inferred: the table type is picked
  from the page's label text]. Otherwise it sends 0x0022.
- A few other pages poll too (security/SIM, serial interfaces, 1-Wire sensors, Bluetooth).
  Their commands are outside this section.

Behaviour during other operations [observed]:
- Exchanges on the port are strictly one at a time: a binary or text exchange (request plus
  every reply frame) finishes before the next one starts. Status polls, configuration
  read/write, file transfer, records and log capture are therefore serialised. A poll that
  comes up while another operation is in progress waits for it. Exchanges never interleave
  on the wire.
- While a long user operation runs, the main window is disabled, so the user cannot switch
  pages. A Status-page loop that is already running keeps polling between the operation's
  exchanges [inferred].
- Polling sends neither `.log:0` nor `.log:1` [inferred: no such send in the polling path].
  It assumes the log stream is off (§7.7).

#### 7.6.2 Status exchange: count frame plus data frames

The same exchange is used for 0x0002, 0x0102, 0x0022, 0x0024 and 0x0026 (but not 0x0038):

| Phase | Direction | Frame | Rule |
|---|---|---|---|
| Request | host → device | request, new `<request_id>` | 0x0102 carries the 8-byte `<group_mask>`. The others have an empty payload |
| Count | device → host | 0x0001 (ack) with the same `<request_id>` | Payload = `<frame_count>`. Its width depends on the payload length: 1 byte = u8, 2 bytes = u16 BE, 4 bytes = u32 BE. Any other length gives `<frame_count>` = 0. A response ID other than 0x0001 fails the exchange |
| Data | device → host | `<frame_count>` data frames: 0x0003 / 0x0103 / 0x0023 / 0x0025 / 0x0027, with the same `<request_id>` | Each frame is parsed with §7.2 (4-byte IDs only for 0x0103) and the records are concatenated in arrival order |

- Any failed data frame (bad CRC, wrong response ID, timeout) fails the whole exchange.
  Records already received are discarded.
- Per-frame read timeout [observed]: each frame read (the count frame and every data frame)
  uses the command's own timeout if it has one, otherwise the link default of **30 s**. The
  default is the same for USB and Bluetooth. No own timeout was found for the status-type
  commands (0x0002, 0x0102, 0x0022/24/26, 0x0038), so they use 30 s per frame
  [inferred: only records and file transfer were seen setting their own]. No overall
  exchange timeout and no retry exist; the Status page's only retry is the 30 s back-off of
  §7.6.1. Shorter client timeouts (for example 3 s USB / 8 s Bluetooth) are a client choice
  and not in conflict with the reference tool, but a slow device that answers after more
  than that would be reported as failed where the reference tool would succeed.
- Text and binary on the same link [observed]: the tool sends FMBX frames on the same open
  port, straight after text commands, with **no** mode-switch command, no delay, no port
  re-open and no explicit flush in the status path. Both kinds are accepted interleaved
  (one exchange at a time). The status page is only reachable after
  the normal session start (§0: `.log:0`, `:cfg_info:?`, `:cfg_connect`, keyword), so in the
  reference tool 0x0002 always follows that sequence. Unread input is discarded
  before each write, see 01-transport §2B and §7.6.2a.
- `<frame_count>` = 0: the exchange returns no records. Whether the tool then counts it as a
  success is UNKNOWN.

#### 7.6.2a Host behaviour inside the exchange (0x0002, 0x0102, 0x0022, 0x0024, 0x0026) [observed]

The host writes exactly **one** frame per exchange: the request. Everything after it is
reads only. The reference tool sends no acknowledgement, "next frame" request, ready frame,
delay or poll frame between the count frame and the first data frame, or between data frames.

| Phase | Direction | Frame | Request ID | Notes |
|---|---|---|---|---|
| Request | host → device | 0x0002 / 0x0102 / 0x0022 / 0x0024 / 0x0026 | new `<request_id>` | the only host frame of the exchange |
| Count | device → host | 0x0001 | same `<request_id>` | `<frame_count>` read as in §7.6.2 |
| Data | device → host | 0x0003 / 0x0103 / 0x0023 / 0x0025 / 0x0027, `<frame_count>` times | same `<request_id>` | read back to back; nothing is sent by the host in between |
| End | – | – | – | the exchange is complete after the last data frame; there is no end marker and no host frame |

Read rules [observed]:
- Each frame is read from the byte stream as exactly 12 header bytes, then the payload length
  from the header plus 2 CRC bytes (02-framing §2.3). Bytes that arrive beyond the current
  frame stay in the receive stream and are the start of the next frame. A receiver must not
  discard the remainder of a received chunk: the count frame and the first data frames may
  arrive in one chunk.
- A header whose first 4 bytes are not `FMBX`, or whose request ID differs from the request,
  fails that read; there is no resynchronisation and no skipping of bytes.
- Pending input is discarded once, immediately before the request is written (01-transport
  §2B, both transports). Because no host frame is written inside an exchange, this never
  drops count or data frames. A host that also drains input at other times (for example
  before reading the count frame, or between data frames) would lose frames and must not.
- Timeouts: 30 s per frame read (count frame and each data frame), as in §7.6.2. If the
  count frame arrives and a data frame does not, the exchange fails after 30 s and no
  records are returned. The reference tool does not resend the request inside the exchange.
- The next exchange uses a new request ID (02-framing §2.2) and starts only after this one
  has finished or failed.

Synthetic data frame expected after a count frame (0x0003, request ID 1, skip byte `00`,
ID 781, type 3, value 1):
`46 4D 42 58 00 00 00 01 00 03 00 06 00 03 0D 03 00 01 60 01`.

Real-device observation (open-questions 77): a device on Bluetooth answered 0x0002 with
request ID 1 with the count frame `46 4D 42 58 00 00 00 01 00 01 00 01 06 BF 81`
(`<frame_count>` = 6, CRC valid), and no data frame was seen for 30 s. No host frame in the
reference tool's exchange could have been missing, so this is not explained by the host side
of this spec. Client-side causes to rule out first [inferred]: bytes following the count
frame in the same chunk were dropped, or the reader stopped after the first frame. A raw log
of every received chunk with timestamps settles this. If the device really sends nothing,
the cause is UNKNOWN (device firmware or transport specific).

Other flows [observed]: records (§7.5) is a true request/reply loop (one 0x0012 per batch,
then 0x0015 after a batch) and needs no change. File download (§7.4.5) follows the same
one-request-then-reads pattern (no host frame per chunk), so the stream rules above apply
to it, but nothing in its exchange changed.

#### 7.6.3 Extended groups (0x0102)

`<group_mask>` bits are listed in §7.3. The tool itself only ever sends 0xFFFF (Status page),
0x10 and 0x20 (I/O page) [observed]. What each group returns [inferred from which tool view
consumes the records; not checked on a device]:

| Bit | Group | Records returned (status IDs) |
|---:|---|---|
| 0x1 | Device info | 768–826 range: firmware version (770), RTC time (771), uptime (772), external voltage (773), battery voltage (774), battery level/charge state (775/776), protocol version/features (781/784), interface and chip info (782, 785–791, 803, 818/819) |
| 0x2 | GNSS | 256–288 range: fix state, fix time, satellites per constellation, latitude, longitude, altitude, speed, … |
| 0x4 | GSM | 512–547 range: modem state, SIM state, operator/signal, data-link state, last record sent time, last server reply time, sent/received data and SMS counters, socket states |
| 0x8 | Base I/O | records whose ID is an I/O element ID from the definitions (§7.6.4) |
| 0x10 | OBD | OBD records (VIN 550, protocol 551) plus OBD I/O element IDs |
| 0x20 | LV-CAN | adapter records (program number 552, software version 553, name 787, ID 788) plus CAN I/O element IDs and CAN flag elements |
| 0x40–0x1000 | FMS, manual CAN, tachograph, beacons, Modbus, spec I/O, FMS eco-driving | I/O element IDs belonging to that group |

UNKNOWN: whether a device returns only the requested groups' records or always everything;
the exact ID ranges per group.

#### 7.6.4 Status-parameter catalogue: how IDs get meaning, type and unit

There are two sources [observed]:

1. **Fixed device-status IDs (256–826)** are built into the tool. Nothing in the definitions
   data describes them. Each entry has a `<status_id>`, a user-visible label, a display
   group (tab) and an optional display rule. The wire type always comes from the record's
   own type byte. The catalogue never overrides it. Display rules seen:

| Rule | Applies to (examples) | Display |
|---|---|---|
| Unix time | 771 (RTC time), 542, 543 | value = seconds since 1970-01-01 00:00 UTC, shown as a date/time |
| Duration | 772 (uptime), 258 (fix time) | value = seconds, shown as a duration |
| Millivolts | 773 (external voltage), 774 (battery voltage) | value shown with the suffix ` mV`. No scaling |
| Coordinate | 265 latitude, 266 longitude | value is a floating-point number in degrees. It is shown only if within ±90 (latitude) or ±180 (longitude) [inferred], with up to 7 decimals |
| Enumerations | 256 fix state, 257 GNSS module state, 512 modem state, 513 SIM state, 515 signal level, 516 data-link state | small integer mapped to a text. The text tables are fixed in the tool and not reproduced here |
| Byte counts | 544/545 sent/received data | number of bytes, shown with a size unit [inferred] |
| Combined | 771+772 (start time = RTC − uptime), 775+776 (battery), 546+547 (SMS totals) | derived from two records |
| Version | 781 | byte 0 major, byte 1 minor |
| Features | 784, 810 | 64-bit bit masks (see 02-framing §2.7 and §7 here) |

   Example entries:

| ID | Meaning | Typical wire type [inferred] | Unit / scaling |
|---:|---|---|---|
| 773 | External power voltage | integer | mV, no scaling; 0 = no external power |
| 781 | Binary protocol version | 2-byte array or u16 | byte 0 major, byte 1 minor |
| 784 | Protocol feature mask | 7 (u64) [observed via 0x0038] | bit mask |
| 810 | Additional feature bits | 8-byte value | bit mask, big-endian |

2. **I/O element records** have a `<status_id>` equal to an I/O element ID from the
   device's definitions (04-parameters: I/O items; 08-definitions-source for where
   to read them in the user's Configurator). The tool looks each record ID up among the I/O
   items of the device's definitions and shows:
   - label = the I/O item's name;
   - value = the record's decoded value as text, then one space, then the I/O item's unit
     text. **No multiplier or offset is applied** in this view [observed];
   - tab = the I/O configuration group the item belongs to. A group's "external data" kind
     and its command-mask attribute choose the FMS / tachograph / CAN / OBD tabs. The
     command-mask is read as an integer and tested against the status features [inferred].
   Records whose ID matches neither the fixed list nor an I/O item are not shown.

So an implementation needs the fixed list (small, built in) plus the I/O items it already
builds for the parameter editor. I/O names and units come from the user's own Configurator
in the same way as 08.

#### 7.6.5 Info requests 0x001C–0x0026

- 0x0022 (I/O), 0x0024 (OBD), 0x0026 (LV-CAN): request payload empty. Exchange as in §7.6.2.
  Data frames 0x0023 / 0x0025 / 0x0027 use the §7.2 layout with **2-byte** IDs. 0x0022 is
  what the I/O page uses without feature 0x800. In 1.8.21 the tool never actually sends
  0x0024 or 0x0026, although it builds them [observed].
- 0x001C (device), 0x001E (GNSS), 0x0020 (GSM): defined in the command table (feature
  0x1, version 0.1) but never sent by the tool. Their reply layout is UNKNOWN. Inferred: the
  same as 0x0022 (count frame, then §7.2 records), returning the 768/256/512 ranges of
  §7.6.3. A client should get these through 0x0002 / 0x0102 instead.

### 7.7 Debug log stream (`.log:1`) [observed unless marked]

| Step | Bytes | Rule |
|---|---|---|
| Start | `.log:1\r` | Sent with no other exchange in progress (§7.6.1). No reply is awaited |
| Stream | device → host text | The tool reads CR-terminated chunks (each read ends at 0x0D) and appends the text exactly as received to a capture buffer. A read that times out or returns nothing does not end the capture |
| Stop | `.log:0\r` | Sent when the capture time is over or the user cancels. No reply is read at that point |
| End | – | The port is free for other exchanges again; the buffer is written to a text file |

- Capture time: a user setting in minutes, default **10 min**. The log is captured only by
  an explicit user action ("capture log"), never as part of status polling.
- Line format: free-form device text lines ending in CR (LF may follow) [inferred]. No
  framing, header or checksum. The tool does not parse the lines. Rate: UNKNOWN (device
  dependent).
- Separating log text from command replies: the tool never mixes them. During a capture
  no other exchange is in progress, so no command is in flight. Outside a capture the stream is off:
  - the probe and keyword flows send `.log:0\r` first and read until idle, discarding
    whatever arrives;
  - a binary reply whose first four bytes are not the frame marker is rejected (02-framing
    §2.3). Whether the reference tool ever skips stray text in front of a frame is UNKNOWN, so a
    new implementation must not rely on it: the log stream has to be off before any binary
    exchange.
- Safe way to turn it off: send `.log:0\r`, then read until idle (02-framing §1.2) and
  discard everything read. Only after that, send the next command. The reply to `.log:0`
  is not defined. Log lines already in the device's buffer may still arrive during the idle
  read.
- Note: at the end of its port probe the tool sends `.log:1\r` before closing the port
  (§0, 01-transport). So a device that has just been probed may have the stream on. Every
  later session must start with `.log:0\r` plus an idle read.

## 8. Other text commands [observed, low priority]

| Purpose | Bytes | Reply |
|---|---|---|
| BLE-chip info | `@asset_send:asset info\r` | read until idle; take the text after `Code Version:` |
| SIM PIN operation | `:cfg_pin_op:<op>,<current_pin>,<new_pin>\r` | until CR; line `<SIMPIN_OP_RESULT>:a,b` (2 fields, meaning UNKNOWN) |
| Bluetooth control | `:cfg_btcmd:<group>,<op>[,<arg>]\r`; group 0 = scan (op 0 stop, 1 start, 2 state, 3 discoverable), group 1 = paired list (0 discovered, 1 paired, 2 clear all, 3 clear by address), group 2 = pairing (0 status, 1 pair by address, 2 enter PIN) [group-to-op mapping inferred] | until CR; `<BTCMDRES>` followed by comma-separated fields |
| UART boot-mode status | `.run_uart_boot_mode?\r\n` | see 06-firmware |

## 9. Test vectors [derived]

All vectors below come from an independent implementation of the rules in this document
and 02-framing. None has been captured from a device yet. Spaces separate bytes (hex). The reviewer independently recomputed every binary frame CRC (CRC-16/ARC, 02-framing §2.4, check value 0xBB3D) and the ASCII byte dumps of the text requests; all match, and each binary frame's length field agrees with its payload.

### 9.1 Text requests

| Request | Bytes |
|---|---|
| `:cfg_connect\r` | `3A 63 66 67 5F 63 6F 6E 6E 65 63 74 0D` |
| `:cfg_info:?\r` | `3A 63 66 67 5F 69 6E 66 6F 3A 3F 0D` |
| `:cfg_getcfg\r` | `3A 63 66 67 5F 67 65 74 63 66 67 0D` |
| `:cfg_setparam:101:4;102:2;2001:internet\r` | `3A 63 66 67 5F 73 65 74 70 61 72 61 6D 3A 31 30 31 3A 34 3B 31 30 32 3A 32 3B 32 30 30 31 3A 69 6E 74 65 72 6E 65 74 0D` |
| `:cfg_save\r` | `3A 63 66 67 5F 73 61 76 65 0D` |
| `:cfg_default\r` | `3A 63 66 67 5F 64 65 66 61 75 6C 74 0D` |
| `:cfg_disconnect\r` | `3A 63 66 67 5F 64 69 73 63 6F 6E 6E 65 63 74 0D` |
| `.reset\r` | `2E 72 65 73 65 74 0D` |
| `.log:0\r` / `.log:1\r` | `2E 6C 6F 67 3A 30 0D` / `2E 6C 6F 67 3A 31 0D` |
| `:sec_status\r` | `3A 73 65 63 5F 73 74 61 74 75 73 0D` |
| `:sec_login:abcd\r` | `3A 73 65 63 5F 6C 6F 67 69 6E 3A 61 62 63 64 0D` |

### 9.2 Text replies (synthetic; they satisfy the parsing rules, but real tag text and spacing may differ)

```
Connect:        "<CFG_CONNECT>\r"
Info (3 lines): "cfg_info:3:123456789012345\rcfg_info:5:WORKING\rcfg_info:6:ABSENT\r"
Config read:    "<X>\r101:4\r102:2\r</X>:2\r" + 17 1C 14 00 0D     -> 2 parameters, count 2: valid
                "<X>\r101:4\r</X>:2\r" + 17 1C 14 00 0D             -> 1 parameter,  count 2: invalid (retry)
Set param:      "<SETPARAM_RESULT>:3\r"   after sending 3 items  -> success
Save:           "<CFG_SAVE>:1\r"                                  -> success (tag text illustrative)
Keyword status: "<SECSTAT>1,1,0,0,3\r"    -> ok, secured, not authorized, not locked, 3 retries left
```

### 9.3 Binary requests (one per command class)

| Class | Request ID | Frame |
|---|---:|---|
| Status data (0x0002, empty payload) | 1 | `46 4D 42 58 00 00 00 01 00 02 00 00 00 9C` |
| Extended status (0x0102, groups = 0x1) | 2 | `46 4D 42 58 00 00 00 02 01 02 00 08 00 00 00 00 00 00 00 01 39 57` |
| Extended status (0x0102, groups = 0xFFFF) | 3 | `46 4D 42 58 00 00 00 03 01 02 00 08 00 00 00 00 00 00 FF FF C8 95` |
| Single status parameter (0x0038, ID 781) | 4 | `46 4D 42 58 00 00 00 04 00 38 00 02 03 0D F0 84` |
| Counter reset (0x0004, empty payload) | 5 | `46 4D 42 58 00 00 00 05 00 04 00 00 C1 8D` |
| Device info (0x001C, empty payload) | 6 | `46 4D 42 58 00 00 00 06 00 1C 00 00 C6 49` |
| Records count (0x0010, empty payload) | 7 | `46 4D 42 58 00 00 00 07 00 10 00 00 05 B4` |
| Records read batch (0x0012, empty payload) | 8 | `46 4D 42 58 00 00 00 08 00 12 00 00 C4 41` |
| Records remove (0x0015, empty payload) | 9 | `46 4D 42 58 00 00 00 09 00 15 00 00 C5 CD` |

The three records vectors were computed for v5 with CRC-16/ARC over the bytes from the
request ID to the end of the payload (02-framing §2.4), the same coverage that reproduces
every earlier vector in this section.

Live-status requests (v6, same CRC coverage; the computation also reproduces the request ID 1
and 4 vectors above):

| Class | Request ID | Frame |
|---|---:|---|
| Single status parameter (0x0038, ID 784) | 10 | `46 4D 42 58 00 00 00 0A 00 38 00 02 03 10 39 AB` |
| Single status parameter (0x0038, ID 773) | 11 | `46 4D 42 58 00 00 00 0B 00 38 00 02 03 05 36 7A` |
| Single status parameter (0x0038, ID 790) | 12 | `46 4D 42 58 00 00 00 0C 00 38 00 02 03 16 3B 4D` |
| Single status parameter (0x0038, ID 803) | 13 | `46 4D 42 58 00 00 00 0D 00 38 00 02 03 23 EC 9D` |
| Single status parameter (0x0038, ID 810) | 14 | `46 4D 42 58 00 00 00 0E 00 38 00 02 03 2A EA 6E` |
| Extended status (0x0102, group 0x10 OBD) | 15 | `46 4D 42 58 00 00 00 0F 01 02 00 08 00 00 00 00 00 00 00 10 78 84` |
| Extended status (0x0102, group 0x20 LV-CAN) | 16 | `46 4D 42 58 00 00 00 10 01 02 00 08 00 00 00 00 00 00 00 20 F3 AE` |
| Device info (0x001C, empty) | 17 | `46 4D 42 58 00 00 00 11 00 1C 00 00 C5 3D` |
| GNSS info (0x001E, empty) | 18 | `46 4D 42 58 00 00 00 12 00 1E 00 00 05 D8` |
| GSM info (0x0020, empty) | 19 | `46 4D 42 58 00 00 00 13 00 20 00 00 09 84` |
| I/O info (0x0022, empty) | 20 | `46 4D 42 58 00 00 00 14 00 22 00 00 09 90` |
| OBD info (0x0024, empty) | 21 | `46 4D 42 58 00 00 00 15 00 24 00 00 C8 4D` |
| LV-CAN info (0x0026, empty) | 22 | `46 4D 42 58 00 00 00 16 00 26 00 00 08 A8` |
| Log on / log off (text, §7.7) | – | `2E 6C 6F 67 3A 31 0D` / `2E 6C 6F 67 3A 30 0D` |

### 9.4 Binary responses (synthetic)

Ack (0x0001), request ID 5, empty payload:
`46 4D 42 58 00 00 00 05 00 01 00 00 C0 9D`

Status data (0x0003), request ID 1. The payload is a leading byte `00` followed by three
records: ID 781 type 3 (u16) value 1; ID 784 type 5 (u32) value 5; ID 3 type 16 (string)
length 4 `"TEST"`:
```
46 4D 42 58 00 00 00 01 00 03 00 15
00
03 0D 03 00 01
03 10 05 00 00 00 05
00 03 10 04 54 45 53 54
46 5E
```

By §7.6.2, a real 0x0002 exchange is the count frame
`46 4D 42 58 00 00 00 01 00 01 00 01 01 7D C0` (ack, request ID 1, `<frame_count>` = 1)
followed by data frame(s) like the one above.

Live-status replies (v6, synthetic; request IDs assume the device echoes the request's ID):

| Frame | Bytes |
|---|---|
| Count frame for 0x0102 request ID 15, 2-byte `<frame_count>` = 2 | `46 4D 42 58 00 00 00 0F 00 01 00 02 00 02 C1 E2` |
| Count frame for 0x0022 request ID 20, `<frame_count>` = 1 | `46 4D 42 58 00 00 00 14 00 01 00 01 01 B8 C2` |
| 0x0003, request ID 1: skip byte, ID 773 type 3 value 12600 (12.6 V), ID 771 type 5 value 1700000000 | `46 4D 42 58 00 00 00 01 00 03 00 0D 00 03 05 03 31 38 03 03 05 65 53 F1 00 5A F7` |
| 0x0103, request ID 15: skip byte, ID 265 (4-byte) type 9 value 54.6872, ID 268 type 3 value 57 | `46 4D 42 58 00 00 00 0F 01 03 00 15 00 00 00 01 09 09 40 4B 57 F6 2B 6A E7 D5 00 00 01 0C 03 00 39 C6 5B` |
| 0x0023, request ID 20: skip byte, ID 1 type 0 value 1, ID 9 type 3 value 4123 | `46 4D 42 58 00 00 00 14 00 23 00 0A 00 00 01 00 01 00 09 03 10 1B 3E 31` |
| 0x003B for ID 784 (no skip byte): ID, type 7, u64 0x0FFF | `46 4D 42 58 00 00 00 0A 00 3B 00 0B 03 10 07 00 00 00 00 00 00 0F FF D6 34` |
| 0x003B for ID 790: type 0, value 1 (true) | `46 4D 42 58 00 00 00 0C 00 3B 00 04 03 16 00 01 43 0E` |
| 0x003B for ID 803: type 24, length 4, bytes DE AD BE EF (value read from offset 5) | `46 4D 42 58 00 00 00 0D 00 3B 00 09 03 23 18 00 04 DE AD BE EF A0 DD` |
| 0x003D (nack) to request ID 11, empty payload | `46 4D 42 58 00 00 00 0B 00 3D 00 00 0D 34` |

Records replies (§7.5), synthetic. The request IDs shown assume the device echoes the
request's ID, which is UNKNOWN for 0x0011, 0x0013 and 0x0039:

| Frame | Bytes |
|---|---|
| Ack to 0x0010, request ID 7, 1-byte value 0 | `46 4D 42 58 00 00 00 07 00 01 00 01 00 DB 01` |
| Count 0x0011, request ID 7, `<total_records>` = 3 | `46 4D 42 58 00 00 00 07 00 11 00 08 00 00 00 00 00 00 00 03 BA 7C` |
| Progress 0x0039, request ID 8, `<progress_value>` = 50, second field 0 | `46 4D 42 58 00 00 00 08 00 39 00 04 00 32 00 00 CC 9E` |
| Ack to 0x0012, request ID 8, value 1 (a 0x0013 frame follows) | `46 4D 42 58 00 00 00 08 00 01 00 01 01 E4 C0` |
| Ack to 0x0012, request ID 8, value 0 (end of reading) | `46 4D 42 58 00 00 00 08 00 01 00 01 00 24 01` |

## UNKNOWN / to confirm on hardware
- Live status (§7.6): whether count and data frames echo the request ID (assumed); the
  meaning of the skipped first byte of each data frame; the 0x003D payload; whether
  `<frame_count>` = 0 is a success; which records each 0x0102 group really returns, and
  whether the device filters by group; the reply layout of 0x001C/0x001E/0x0020 (never sent
  by the tool); the wire types the device uses for 781, 803 and 810; the text tables of the
  enumerated device statuses (fix state, modem state, SIM state, data-link state, signal
  level); why a real device on Bluetooth sent no data frames after a valid count frame (§7.6.2a).
- Debug log (§7.7): the device's line terminator (CR or CR LF), the line rate, whether the
  device sends any acknowledgement to `.log:0`/`.log:1`, and how long the stream keeps going
  after `.log:0`.
- Records (§7.5): whether 0x0011, 0x0013 and 0x0039 echo the request ID; whether one
  record can be split across two 0x0013 frames; the second field of 0x0039 and the unit of
  `<progress_value>`; the meaning of a non-zero ack value; whether records reading depends
  on any device-info key; whether a device can send records in a codec other than Codec 8
  (for example 2-byte I/O IDs).
- File transfer download (§7.4.5): whether `<chunk_number>` is a sequence number or a byte
  offset (ordering is the same either way), and whether the device replies to 0x002C.
- The exact tag text of the setparam and save replies (only the number is used, so this
  does not block implementation).
- Whether `:cfg_connect` is needed before `:cfg_getcfg` on every firmware version
  (the tool always sends it).
- Payload layouts of most binary responses beyond §7.2 and §7.4.
- File transfer (§7.4): item order in the upload request, delete/list payloads, and ack
  code 1 versus 0.
- Escaping rules for `;` in string values.
- File transfer (§7.4): the request ID used by data packets, acks, chunks and the completion
  frame (echo rules), and the timeout for the 0x002D ack and for each 0x002B chunk.
- File transfer: layouts of delete (sub-command 0) and directory list (sub-command 3) requests
  and replies, the types of request items 6 and 7, and the unit of the file-info time stamp.
