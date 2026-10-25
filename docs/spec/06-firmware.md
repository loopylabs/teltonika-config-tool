# 06 – Firmware update

> Status: released v1 — reviewed 2026-10-24. Items marked UNKNOWN/INFERRED are not yet confirmed. Implementations must treat UNKNOWN/INFERRED steps as unsafe until verified on a sacrificial device. This spec describes externally visible behaviour only; it contains no firmware bytes, keys or credentials and describes no way around any device-side or package-side security check.

Describes how Configurator 1.8.21 updates the firmware of an FMB-family
tracker (and of the tracker's BLE co-processor). Facts are tagged **[observed]**,
**[inferred]** or **UNKNOWN**.

Related: 01-transport (serial/USB and Bluetooth links, port discovery), 02-framing,
03-commands (§1 device info keys, `.run_uart_boot_mode?`), 04-parameters (feature bits,
families), 08-definitions-source (where the package index lives).

Scope note: our app does not bundle firmware. The user supplies the package file (or a
later, separately decided fetch mechanism supplies it). Nothing here requires vendor
firmware bytes.

---

## 1. Overview [observed]

A firmware update is host-driven and passes through these phases:

| Phase | Link state | Where |
|---|---|---|
| Package chosen, local checks, user confirmation | normal configuration session | §2, §7 |
| Device asked whether it may enter update mode | normal configuration session | §3.1, §7.2 |
| Hand-over: configuration session closed, same link reopened for the **update protocol** (a separate protocol from the 02 binary framing and from the 03 §7.4 file transfer) | update link | §3 |
| Device identification and package acceptance | update link | §3.4, §4.1 |
| Image transfer | update link | §4.2 |
| Device reboots into the new firmware; tool waits, reconnects in normal mode and re-reads device info | normal configuration session | §5 |
| Optional: when the device reports that an intermediate firmware is required, an intermediate package is flashed first and the original attempt is repeated | as above | §6.3 |

The 03 §7.4 file-transfer commands (0x0028–0x002D) are **not** used for firmware updates
[observed: the update flow does not use those commands].

## 2. Firmware package format

### 2.1 File names [observed]

- The tool's open dialog offers `*.e.XIM` (main firmware) and, on devices with feature
  bit 14, also `*.bin` (BLE co-processor firmware, §4.4).
- The format is chosen by file-name suffix. The tool's own pre-checks compare
  case-insensitively [observed]; whether the name is lower-cased before the format is
  chosen is UNKNOWN, so implementations should compare case-insensitively [inferred]:

| Suffix | Format |
|---|---|
| `.e.xim` | container format §2.2 if the first 4 bytes are ASCII `TLTF`; otherwise the legacy encrypted format §2.3 |
| `.e.cfw`, `.e.bin` | legacy encrypted format §2.3 |
| `.cfw`, `.bin` | plain image, no metadata (version and device sets default/empty) |
| `.xim` | plain image; if it starts with bytes `4D 4D 4D 01` it carries a fixed-offset header (§2.4); bytes `BB AA 01 F4` mark another plain variant [inferred] |
| other, with `.e` before the last extension | as `.e.xim` |
| anything else | plain image |

- The `e` infix means "encrypted" [inferred]. Vendor-distributed tracker firmware uses
  `.e.XIM` [observed in the tool's file filter and package index]. Only `.e.XIM` (and
  `.bin` for BLE) is offered by the tool's open dialog; the plain-image rows above describe
  how a file of that name would be read, not an input the tool offers.

### 2.2 Container format (`TLTF`) [observed layout; roles partly inferred]

All integers little-endian.

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | magic, ASCII `TLTF` |
| 4 | 4 | UNKNOWN |
| 8 | 4 | UNKNOWN |
| 12 | 4 | format major version; must be 1, otherwise reject |
| 16 | … | four blocks, each `<length:i32>` followed by `<length>` bytes |

- The four blocks carry key-derivation salt, IV, ciphertext and a signature [inferred
  roles; order UNKNOWN].
- Confidentiality: AES-256-CBC (ISO 10126 padding); the key is derived with
  PBKDF2-HMAC-SHA1, 1000 iterations, 32-byte output, from a vendor-held secret
  [observed]. Authenticity: the package carries an RSA signature with SHA-512 that the
  vendor tool verifies against a vendor public key [observed].
- No key, secret or key location appears in any spec (see §2.6). A package that fails the
  signature or format check must be rejected, never sent to a device.
- Decrypted body: a sequence of frames until the end of data:

| Field | Size |
|---|---|
| `<frame_id>` | 16 (all bytes zero except the last) |
| `<kind>` | 4, i32 LE |
| `<length>` | 4, i32 LE |
| data | `<length>` |

Frame IDs (last byte of the ID) and kinds:

| ID | Meaning | Kind and data |
|---|---|---|
| 00 | Version | kind 1: four i32 LE (major, minor, build, revision) |
| 01 | Device ID | kind 2: u32 LE (the `<device_id>` of §4.1) |
| 02 | Devices | kind 4 (set): IMEI list restricting the package [inferred from §4.1 use] |
| 03 | Firmware names | kind 4 (set): allowed current firmware versions [inferred from §4.1 use] |
| 04 | Image | kind 3 (blob): the bytes sent to the bootloader |
| 05 | Properties | kind 5 (map) |
| 06 | Spec ID | kind 2: u32 LE |

Set (kind 4): `<count:i32>` then `<count>` strings. Map (kind 5): `<count:i32>` then
`<count>` key/value string pairs. String: length as a 7-bit variable-length integer
(low 7 bits first, high bit = more), then UTF-8 bytes. Unknown kinds/IDs are kept and
ignored [observed].

### 2.3 Legacy encrypted format [observed]

- Last 128 bytes: RSA-1024 signature (SHA-1) over everything before them.
- Body: `<l1:i32>`, `<l2:i32>`, then `l1` bytes (RSA-encrypted AES key), `l2` bytes
  (RSA-encrypted IV [inferred]), then AES-CBC ciphertext up to the signature. The RSA key
  material is vendor-held and not given here. The signature must verify before the
  package is used.
- Plaintext: ASCII `<n1>;` then `n1` bytes = `;`-separated device-code list
  (case-insensitive); `<n2>;` then `n2` bytes = `;`-separated firmware-name list; the
  rest is the image. Bad lengths → invalid package.

### 2.4 Plain `.xim` with fixed header [observed, partial]

When the first four bytes are `4D 4D 4D 01`: offset 0x11C holds an ASCII integer
(`<device_id>` [inferred]), 0x124 an ASCII version string, 0x138 and 0x13C each an i32
LE of UNKNOWN meaning. The whole file is the image [inferred].

### 2.5 BLE co-processor image (`.bin`) [observed]

Little-endian header:

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | magic 0x96F3B83D |
| 4 | 4 | ignored |
| 8 | 2 | header length `H` |
| 10 | 2 | trailer-related length `T` |
| 12 | 4 | image length `I` |
| 16 | 4 | ignored |
| 20 | 1 | version major |
| 21 | 1 | version minor |
| 22 | 2 | version patch |
| 24 | 8 | ignored |

Validity: after the 32-byte header skip `H − 32` bytes, then `I` image bytes, then a u16
that must equal 0x6907; and `H + I + T` must not exceed the file length. Version =
`major.minor.patch`. The whole file is sent (§4.4) [inferred].

### 2.6 What the device receives, and obtaining images [observed / owner decision]

- The bootloader receives the **decrypted image** (frame 04 / legacy plaintext image),
  not the package file [observed: the transfer header size (§4.2) is the image size].
- Opening `.e.XIM` packages requires vendor-held key material that this spec deliberately
  does not contain. Owner options: (a) obtain key material or unencrypted images
  legitimately from the vendor; (b) accept only unencrypted images supplied by the
  vendor. Whether a bootloader would accept an encrypted package unchanged is UNKNOWN and
  must not be tried on a device that cannot be recovered.
- An image without package metadata (§2.1 plain rows) carries no device or version
  restriction. An implementation must then not skip the pre-flight checks of §4.1 and §7;
  where a check needs package metadata that is absent, it should refuse, or require an
  explicit user confirmation that names the risk [inferred, recommended].
- Optional size-reduced upload: when device info key 16 (03 §1, "supports compressed
  firmware upload") is `1`, the vendor tool can send a smaller image with the same
  metadata [observed]. The method is UNKNOWN. Always sending the full image is the safe
  choice [inferred: key 16 only advertises support].

### 2.7 Fit to a device

- Package side: version, device ID, spec ID, IMEI list and allowed-current-version list
  (§2.2). Device side: IMEI and current firmware version from the bootloader (§3.4).
  Rules: §4.1, plus the device's own verdict via the update query [observed].
- There is no hardware-code (e.g. `FMB1:6`) check in the tool; hardware fit is decided by
  `<device_id>` and by the device itself [observed].
- Source of packages: the user picks a local file [observed]. The tool downloads **no
  firmware** from any server [observed: its only web API, `https://api.teltonika.lt`, is
  used for account checks and CAN-adapter data, never on the update path]. The
  local package index (08 §3.10) is described in §6.3.



## 3. Entering update mode

### 3.1 Pre-flight in configuration mode [observed]

See §7.2: `.run_uart_boot_mode?\r\n` → `<UPDATEFWST>:<digit>`; only `0` allows the update.
The question form (with `?`) only asks; it does not switch modes.

### 3.2 Hand-over [observed]

- The tool stops using the configuration session and closes the port. The update
  protocol then opens the **same port address** itself (serial port name or Bluetooth
  device address) [observed].
- Serial settings for the update session on FMB-family devices: **921600 baud**, 8N1, no
  flow control, DTR and RTS asserted, 5 s read and write timeouts [observed]. (The
  configuration session uses 115200, 01-transport; other device classes handled by the
  same updater use 115200 or 19200 and are out of scope.) Over a USB CDC port the baud
  value is nominal [inferred]; over Bluetooth SPP it has no effect [inferred].
- Whether a port is serial or Bluetooth is decided by its address form [observed]; our app
  should use its own link type instead.

### 3.3 Boot-mode command [observed]

Exchange on the update link, after opening the port and waiting **200 ms**:

| Step | Host sends | Device answers | Host rule |
|---|---|---|---|
| Request | text `.run_uart_boot_mode` + CR LF (no `?`), write time-out 10 s | text, read after a 200 ms pause | read until one of the two answers below |
| Ready | | receiver-ready characters `CCCC` (see §4) | bootloader ready, go to §4 |
| Restarting | | text `Starting local bootloader...` | close the port, wait **10 s**, reopen the same port, wait up to **10 s** for `CCCC` |
| Neither | | nothing matching | entry failed; the entry step is attempted up to **4** times [inferred] |

- USB re-enumeration: in the "restarting" case the device drops and re-appears; the tool
  expects it under the **same port name** (it does not search for a new VID/PID)
  [observed]. Whether the bootloader exposes a different USB VID/PID is UNKNOWN; the
  01-transport VID/PID list includes several vendor/product pairs, and an implementation
  on macOS/Linux, where device paths may change, should re-run port discovery and match
  the device by its IMEI (§4.1) [inferred, recommended].

### 3.4 Device identification in update mode [observed]

Before transferring, the updater identifies the device on the update link (overall
time-out 5 s):

| Request (text) | Expected reply |
|---|---|
| `.imei\r\n` | `IMEI:<imei_15_digits>\r` |
| `.tst_device_info\r\n` | `tst:<kind>:<version>` terminated by `,` or a line end; `<kind>` is `fm_device_inf` (tracker), `at_device_inf` or `at_ver` (other device classes) |

- The IMEI must equal the IMEI read in normal mode, otherwise abort with an IMEI-mismatch error
  [observed].
- `<version>` is the device's current firmware version used by the checks in §4.1
  [inferred from use].



## 4. Transfer protocol

### 4.1 Package acceptance checks [observed]

The following are evaluated, using the package fields (§2) and the device data (§3.4),
before any image block is sent. All must pass.

| Check | Condition |
|---|---|
| IMEI list | if the package carries an IMEI list, the device IMEI must be in it |
| Allowed current versions | if the package carries a list of allowed current firmware versions, the device's current version text must be in it |
| Compatibility, tracker class only | device version < 1.2 and package `<device_id>` = 1: package version must be < 2.0. Package `<device_id>` = 0 or absent: reject. Device version < 3.0 and package `<device_id>` = 1: package `<spec_id>` must be 0. Otherwise compatible |
| Update query, only when device version >= 1.2 | device must answer the query below with code 0 |

Update query: the host sends the text
`:cfg_updquery:<device_id>,<MM>.<mm>.<bb>[,<spec_id>]` + CR LF, where `<MM>.<mm>.<bb>` is
the **package** version with each field as two zero-padded decimal digits and
`,<spec_id>` is present only when the package spec ID is non-zero. Time-out 5 s. The
device replies `<UPDQUERY>:<code>` followed by CR or LF, `<code>` a signed decimal:

| `<code>` | Tool result |
|---|---|
| 0 | accepted |
| -5 | firmware does not match the device [inferred meaning] |
| -6 | update not allowed / an intermediate firmware is required [inferred: -6 is the `ErrorId` of the package index, §6.3] |
| -8 | an extended update is needed [inferred meaning] |
| -7 or other | not accepted (unknown reason) |

The exact meanings of -5/-7/-8 are UNKNOWN; show them as distinct errors.

Any failed check aborts before flashing; the device stays in its bootloader until reset
(§6) [inferred].

### 4.2 Packet transfer [observed]

The image transfer is standard **YMODEM** with 1 KiB blocks and CRC-16:

| Field | Size | Value |
|---|---|---|
| start byte | 1 | 0x01 for a 128-byte block, 0x02 for a 1024-byte block |
| block number | 1 | `<block_no>` mod 256 |
| complement | 1 | 255 − block number |
| data | 128 or 1024 | see below |
| CRC | 2 | CRC-16/XMODEM (poly 0x1021, init 0x0000, no reflection, no final XOR) over the data bytes only, **big-endian** |

Exchange:

| Phase | Host sends | Device must reply |
|---|---|---|
| Header block | block 0 with 128 data bytes: ASCII `<file_name>`, a NUL byte, the image size in decimal ASCII, then zero padding. `<file_name>` is the package's base name with its extension replaced by `.CFW`; the size is that of the **extracted image** (§2), not of the package file | `06 43` (ACK then `C`) |
| Data blocks | blocks numbered from 1, wrapping mod 256, 1024 data bytes each; the last block is padded with 0x1A | `06` to each block |
| End of transmission | the single byte 0x04, once | `06 43` |
| Closing block | block 0 with 128 zero bytes | `06` |

- Every reply has a **20 s** time-out [observed].
- The tool has no NAK (0x15) handling and no per-block retry: any unexpected or missing
  reply fails the update [observed: only the success replies are accepted; inferred that
  a NAK therefore fails]. A more tolerant implementation may resend a block on NAK, as
  YMODEM allows, but this is not required to interoperate [inferred].
- Progress = data blocks sent / total data blocks [observed: progress is reported per data
  block].
- After the closing block the tool closes the port (§5).

### 4.3 Bluetooth vs USB [observed, shallow]

- The same boot-mode entry and YMODEM exchange run over Bluetooth SPP; "port re-appears"
  means the same Bluetooth device address is discovered again [observed]. No packet-size
  or timing differences were found [observed: none found]. Expect a much
  longer transfer time over Bluetooth [inferred].
- Whether the tool's UI allows starting a main-firmware update over Bluetooth is
  UNKNOWN (no restriction was found).

### 4.4 BLE co-processor firmware (`.bin`) [observed]

On devices with feature bit 14, a separate path flashes the tracker's BLE chip through the
tracker:

| Phase | Host sends | Device must reply |
|---|---|---|
| Enter transparent mode | after 200 ms and opening the port: `@asset_transp` + CR | `ASSET TRANSPARENT MODE` + CR within 10 s |
| Start transfer | `image ymodem` + CR; then wait 200 ms | `CCCC` within 10 s (retried) |
| Image | same exchange as §4.2 | as §4.2 |
| Result | | `@asset_img_ymodem:<word>` within 10 s; `<word>` must contain `done` |
| Reboot and verify | `asset reboot` + CR; after 200 ms `asset info` + CR | text containing `Code Version:<major>.<minor>.<build>` (time-out 10 s), taken as the new BLE firmware version |



## 5. After flashing: reboot and reconnection [observed]

- On success the tool reopens its normal configuration link on the same port address it
  used before the update [observed]. If that reopen fails the update is reported as
  failed, with a reconnect-after-flashing error [observed].
- The tool then shows a waiting state with a countdown of **3 minutes**, ticking once per
  second, while it tries to reconnect [observed].
- Reconnection attempts: each attempt is a normal connect (01/03 probe) with a
  **1 second** time-out; the tool makes up to **30** attempts [observed for the 1 s
  timeout and the 30 limit; inferred that the limit is an attempt count]. If none
  succeeds the user is told the device was not detected [observed].
- After a successful reconnect the tool re-reads device info (03 §1) and treats the new
  firmware version as the result [observed].
- Implementations should apply the same timing to Bluetooth, but must expect the
  Bluetooth link to be dropped by the device's reboot; re-pairing is not required
  [inferred].

## 6. Interruption, refusal and recovery

### 6.1 Abort points [observed]

| Point | Device state afterwards |
|---|---|
| Pre-flight `<UPDATEFWST>` ≠ 0, or local file checks | normal mode, nothing changed |
| Boot-mode command sent, no `CCCC` | UNKNOWN; probably bootloader or rebooting [inferred] |
| IMEI mismatch or §4.1 refusal | bootloader waiting for YMODEM [inferred]; resetting power is expected to return to the old firmware [inferred, UNKNOWN] |
| During data blocks | partial image; behaviour UNKNOWN |

### 6.2 Recovery [inferred]

- No explicit recovery routine exists in the tool [observed: none found]. After a failure
  the user is told the update failed; reconnection uses the same §5 logic.
- Because entry may print `CCCC` directly, a device stuck in its bootloader can be
  re-flashed by opening the port and waiting for `CCCC` without sending the boot-mode
  command again [inferred from the protocol; untested].
- If the device staged the image on storage (§7.2 digits 2/4) an interrupted transfer
  should leave the running firmware intact [inferred, UNKNOWN].

### 6.3 Intermediate packages (package index) [observed]

- The definitions source (08 §3.10) holds a small per-family index: elements `Package`
  with `Path` (a package file path relative to the tool's install folder), `Min`, `Max`
  (dotted versions) and `ErrorId` (−6 in all entries).
- When an update fails, the tool looks for an intermediate package [observed]:
  - if the failure is the tool's intermediate-firmware-required result, it takes the
    first entry with `Min` < current device version < `Max` (strict on both sides);
  - otherwise, if the failure carries a device error code, it takes the first entry whose
    `ErrorId` equals that code and whose range contains the device version the same way.
  If found, it flashes that package, then the user's package again [observed]. That the
  device error code is the update-query code of §4.1 (−6) is [inferred].
- The referenced package files are **not** shipped inside the 1.8.21 executable
  [observed]. Our app should instead tell the user that an intermediate firmware in that
  range is required and let them supply it [inferred, recommended].



## 7. Safety checks and warnings

### 7.1 Before anything is sent [observed]

| Check | Behaviour |
|---|---|
| File type | Main firmware: file name ending `.e.XIM` (case-insensitive). BLE co-processor firmware: `.bin`, offered only when the device has **feature bit 14** (key 12, 04-parameters) |
| Major-version jump | If the package version's major number is ≥ 4 and the device's current firmware major number is < 4, the tool shows a specific major-version warning instead of the generic confirmation. Otherwise it shows a generic confirmation |
| Same BLE firmware | For a `.bin` BLE package: if the package version equals the device's current BLE firmware version (03 §1, BLE version), the tool asks whether to re-flash the same version; "no" cancels |
| Package loads | The package must parse (§2); otherwise the tool reports an invalid package |
| BLE package validity | A `.bin` BLE package is validated (§2.5); invalid → the tool reports an invalid file |

### 7.2 Device-side pre-flight (normal mode) [observed]

Before closing the configuration session, on devices that support it (FMB family and TAT
family yes; FMB640 family no), the tool sends the 03 text command
`.run_uart_boot_mode?\r\n` and expects a reply starting `<UPDATEFWST>:` followed by
exactly **one** digit:

| Digit | Meaning | Tool behaviour |
|---|---|---|
| `0` | Ready | continue |
| `1` | A FOTA (over-the-air) update is running | abort, tell the user |
| `2` | No SD card | abort, tell the user |
| `3` | An update is already running from another port | abort, tell the user |
| `4` | Not enough free space | abort, tell the user |

- A reply that is shorter or longer than the prefix plus one character, or whose digit is
  not in the table, is treated as a failure of the check [observed]; the tool then reports
  the update as failed [observed].
- Digits 2 and 4 suggest the device stages the image on its SD card / storage before
  flashing [inferred]. Whether a device without an SD card can still be updated over the
  UART path is UNKNOWN (the digit is only a blocking condition in the tool).

### 7.3 In update mode [observed]

- The IMEI reported by the bootloader must equal the IMEI read in normal mode, otherwise
  the update aborts with an IMEI-mismatch error (§3.4).
- The device itself may refuse the package; the refusal reasons the tool distinguishes are
  listed in §4 (firmware does not match, update not allowed, must be extended/
  intermediate needed, version not accepted, device ID not accepted, spec ID not
  accepted, unknown).

### 7.4 Warnings our app should show [inferred, recommended]

- Do not disconnect power or the cable during the update; the device may stay in its
  bootloader (§6).
- Keep the host awake; a Bluetooth update is slower and more fragile than USB.
- Show the device's current firmware version and the package version side by side before
  confirmation.

## 8. Test guidance

### 8.1 Synthetic vectors (no vendor bytes)

- CRC-16/XMODEM: ASCII `123456789` → 0x31C3 (standard check value). On the wire after
  data: `31 C3`.
- YMODEM header for `<file_name>` = `test.CFW`, size 2048: start `01`, `00 FF`, then
  `test.CFW` `00` `2048` and zero padding to 128 bytes, then CRC of those 128 bytes.
- Data block 1 of a 1500-byte image: `02 01 FE`, 1024 bytes; block 2: `02 02 FD`,
  476 bytes + 548 × 0x1A. Block 256 wraps to number `00 FF`.
- Header block CRC (computed): the 128 data bytes `test.CFW` `00` `2048` + zero padding
  give CRC 0x9E5F, so the block ends `9E 5F`. A different CRC family is used in 02-framing
  (CRC-16/ARC, check value 0xBB3D for `123456789`); do not mix the two.
- Closing block: `01 00 FF`, 128 × 0x00, `00 00` (CRC of zeros is 0).
- Update query: package 3.29.4, device ID 1, spec 0 → `:cfg_updquery:1,03.29.04\r\n`;
  with spec 7 → `:cfg_updquery:1,03.29.04,7\r\n`.
- Pre-flight reply parsing: `<UPDATEFWST>:0` ok; `<UPDATEFWST>:4` no space;
  `<UPDATEFWST>:` and `<UPDATEFWST>:01` invalid.
- Container parser: build a synthetic decrypted frame list (version 1.2.3.4, device ID 1,
  image of 3 bytes, a set with two strings, an unknown kind 9) and check round-trip; the
  unknown frame must be kept and ignored.
- BLE header: synthesise H=32, I=16, T=2 with the trailer u16 0x6907 → valid, version
  from bytes 20–23; change the trailer → invalid.

### 8.2 Without bricking a device

- Run the whole flow against a loopback simulator that implements §3.3 replies (`CCCC`),
  §3.4 replies and a YMODEM receiver (any standard YMODEM receiver can verify packet
  framing and CRCs).
- On a real device, the non-destructive steps are: pre-flight query (§7.2, read-only),
  package parsing and checks (§2, §4.1, local), and the update query
  `:cfg_updquery:` — it is UNKNOWN whether it is accepted in normal mode; send it only in
  the bootloader as the tool does.
- Abort point for a dry run: stop after receiving `CCCC` and the IMEI check, before the
  YMODEM header. Then power-cycle and confirm the device returns to normal mode (§6.1,
  inferred). Do this first on a spare device.
- First real flash: re-flash the version already installed, over USB, with a supply that
  cannot drop.



## 9. UNKNOWN list

- Roles and order of the four `TLTF` blocks; the two UNKNOWN header words.
- Whether the bootloader accepts the encrypted package unchanged (§2.6) — critical for an
  implementation without the vendor secret.
- Minified image algorithm (key 16).
- Meanings of update-query codes −5, −7, −8 and of other non-zero codes.
- Whether the bootloader uses a different USB VID/PID; macOS/Linux port naming after
  re-enumeration.
- Device behaviour after an abort at each point of §6.1; a dedicated recovery mode.
- Whether a NAK or `C` retransmission request is ever sent by the bootloader.
- Whether main-firmware update over Bluetooth is allowed by the vendor UI; Bluetooth
  timing.
- Exact digit set of `<UPDATEFWST>` beyond 0–4.
- Reconnect timing after flashing: the 3-minute countdown versus 30 one-second attempts
  (§5) leave it unclear whether attempts are spaced out; the device's real reboot time is
  not stated.
- Refusal reasons for version, device ID and spec ID not accepted (§7.3) have no matching update-query codes in §4.1; their source is UNKNOWN.
- Whether the device validates the image signature itself, and what a device does with an
  image that fails its own check; behaviour on power loss during the final stage.
- Purpose of the second `.xim` marker (`BB AA 01 F4`) and fields at 0x138/0x13C.

