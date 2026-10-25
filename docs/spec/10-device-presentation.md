> Status: released v1 — reviewed 2026-10-24. Items marked UNKNOWN/[inferred] are not yet confirmed. This spec describes a file format and display behaviour only; it reproduces no vendor image and ships no vendor data. The application reads device pictures at run time from the Configurator download that the user supplies, caches them locally for that user, and never redistributes them.

# 10 – Device presentation: model names, device pictures and the device list

Scope: Teltonika Configurator 1.8.21 (Windows) as the reference build. Each fact is tagged
**[observed]** (seen directly in the build or its embedded data), **[inferred]** (follows from
observed facts) or **UNKNOWN**.

Purpose: let the application show, for every device in its lists (USB serial, paired
Bluetooth Classic, BLE), a model name and a device picture comparable to the reference tool.
Pictures come from the user's own Configurator download, located with the same container walk
as 08-definitions-source. Model names come from the hardware table that 08 already extracts.

Related specs: 01-transport (discovery and probe), 03-commands (device info keys),
04-parameters §3.1 (hardware table), 08-definitions-source (container walk, §2.1–§2.3, §3.7).

Placeholders are written `<snake_case>`. All multi-byte integers are little-endian.

---

## 1. Where the pictures live (format walk)

```
<configurator_exe>                         (08 §1–§2.1: PE + single-file bundle)
 └─ Brand.dll                              (08 §2.2: the definition library)
     └─ manifest resource "Brand.g.resources"   (08 §2.3: same ManifestResource walk)
         └─ binary resource set (§1.2)
             └─ entry "devices/<image_key>.png" (PNG image bytes)
```

### 1.1 Container [observed]

- The pictures are in the **same library** that holds the definition documents (08 §2.2).
  No other library in the bundle holds device pictures.
- Inside it they are in the single manifest resource whose name ends in `.g.resources`
  (1.8.21: exactly `Brand.g.resources`). 08 §2.3 tells the definition extractor to ignore
  this resource; the picture extractor reads it with the same ManifestResource walk
  (u32 length prefix, then the bytes).
- The resource is stored uncompressed and unencrypted.

### 1.2 Binary resource-set layout [observed]

All offsets are from the start of the resource blob (after the u32 length prefix of 08 §2.3).
"7-bit length" means an unsigned integer in little-endian base-128 groups: each byte carries
7 value bits, and a set top bit means another byte follows.

| Part | Layout |
|---|---|
| Magic | u32 `0xBEEFCACE` |
| Reader header | i32 header version (1.8.21: `1`), i32 `<header_skip>`, then `<header_skip>` bytes holding two type-description strings. Skip them. (1.8.21: 145 bytes) |
| Set header | i32 set version (1.8.21: `2`), i32 `<entry_count>`, i32 `<type_count>` |
| Type names | `<type_count>` strings, each a 7-bit byte length then UTF-8 text (1.8.21: `<type_count>` = 0) |
| Padding | zero or more bytes so that the next field starts at an offset that is a multiple of 8 |
| Name hashes | `<entry_count>` × i32 (not needed for reading) |
| Name positions | `<entry_count>` × i32: offset of each entry's name record, relative to the start of the name section |
| Data-section offset | i32: absolute offset (from the blob start) of the data section |
| Name section | starts right after the data-section offset field. Each name record = 7-bit byte length, the name in UTF-16LE, then i32 `<data_offset>` relative to the data-section start |
| Data section | at each `<data_offset>`: a 7-bit `<type_code>`. For `<type_code>` 0x21 (stream) or 0x20 (byte array): i32 `<length>` then `<length>` bytes. Other type codes carry non-picture values; skip them |

In 1.8.21 every entry has `<type_code>` 0x21 [observed]. The extractor should accept 0x20 as
well and ignore any other code [inferred].

### 1.3 Entry naming [observed]

- Entry names are **lower-case**, use `/` as separator, and are relative paths.
- Device pictures: `devices/<image_key>.png`.
  `<image_key>` = the hardware code (03 info key 2, e.g. `FMB1:6`) with every `:` replaced
  by `_`, then lower-cased. Example: `FMB1:6` → `devices/fmb1_6.png`; `FMB640:1` →
  `devices/fmb640_1.png`.
- Fallback picture: `devices/unknown.png`.
- Other folders in the same set, not device pictures: `misc/` (UI icons, including a USB
  connection icon and a Bluetooth connection icon), `logos/`, `brands/` (compiled UI markup),
  plus two loose markup documents at the root. The application ignores them and uses its own
  icons for connection type [inferred: recommended; these are vendor artwork too].
- Picture lookup in the reference tool is case-insensitive, while the stored names are
  lower-case [observed]. The application must lower-case the derived name before lookup.

### 1.4 Picture format [observed]

- All device pictures are **PNG** (signature `89 50 4E 47 0D 0A 1A 0A`). No SVG, vector
  markup or icon formats are used for device pictures.
- Raster sizes vary per model; most are about 130 × 100 pixels. A few are very large
  (over 4,000 pixels wide). The application must scale to its own display size and must not
  assume a fixed size or aspect ratio.
- Several entries are byte-identical copies under different names (one picture per hardware
  code even when models look the same).

---

## 2. Choosing a picture for a device

### 2.1 Rule [observed]

The picture is chosen **by hardware code only**, by name, with no mapping table:

- With no hardware code known, the picture is `devices/unknown.png`.
- Otherwise the picture is `devices/<image_key>.png` (§1.3), built from the code the device
  reported. The code is used as reported; it is not first translated through the hardware
  table's `BaseOf` alias (04 §3.1, 08 §3.7). Example: a device reporting `FMB9:1` uses
  `devices/fmb9_1.png`, although its model name comes from the `FMB9:21` row.
- The hardware table has no picture attribute. The definitions do not take part in picture
  selection.

The schema may allow a further hardware-row attribute that overrides the code used for
picture lookup. No hardware row in 1.8.21 carries it [observed]. Ignore it.

### 2.2 Missing picture [observed / inferred]

- The fallback picture is used only when there is no hardware code (or the code contains
  characters that are invalid in a file path) [observed].
- When a code is known but no `devices/<image_key>.png` entry exists, the reference tool
  still looks up that name, and the picture is simply missing (empty area) [inferred].
- **Recommended for the application:** if the derived entry does not exist, show
  `devices/unknown.png` from the user's copy; if the user has supplied no Configurator
  download, show the application's own generic device icon [inferred: recommendation].

### 2.3 Offline (no device) lists [observed]

In the model list for working without a device (one entry per model name, §4.2), the picture
comes from the hardware code of the first configuration available for that model [inferred:
exact choice of "first" not pinned].

---

## 3. Model name

### 3.1 Source [observed]

- The shown model name is the hardware table's `FmType` text (08 §3.7, 04 §3.1) for the
  device's hardware code. It is shown **verbatim**, including suffix words. Examples from
  1.8.21: `FMB1:6` → `FMB140`; `FMB0:3` → `FMB001 Buzzer`; `FMB1:4` →
  `FMB125 no BAT DualSIM`.
- The name is taken from the configuration that the definitions offer for the device's
  hardware code, configuration version and spec ID. A code that appears only as a `BaseOf`
  value takes the model name of the row that names it (example: `FMB9:1` → `FMB900`)
  [observed for the alias rule, 04 §3.1].

### 3.2 Unknown or unsupported device [observed]

When the definitions offer no configuration for the device (unknown hardware code, or no
usable version), the reference tool still lists the device, but:

- the model name is **empty**;
- a "not supported" label replaces the configuration version;
- the picture still follows §2.1 from the raw hardware code.

The raw hardware code is not shown as a substitute name [observed in the list card; inferred
for other views]. **Recommended for the application:** show the raw hardware code, for example
`FMB1:6`, when no model name is known [inferred: recommendation].

### 3.3 Other displayed version text [observed]

- Firmware version: each of major, minor and build as at least two digits, joined by `.`
  (example `03.29.01`), then `Rev:<firmware_revision>`. If the version cannot be read, the
  text is `00.00.00`.
- Configuration: the configuration version that the definitions selected (04), or "not
  supported" (§3.2).
- BLE-chip firmware version: shown only for devices whose feature set has a BLE chip
  (03 status/features).

---

## 4. Device lists in the reference tool

### 4.1 Connected-device list (before the user opens a device) [observed]

- **Every discovered port is probed before it is listed.** The tool runs the
  identification probe of 01-transport/03-commands on each new USB serial port and
  Bluetooth port. Only ports whose probe returned device info are added to the list.
  Ports that do not answer are not shown.
- So the list is never a list of raw ports. Each card already shows identified data:
  - device picture (§2);
  - IMEI;
  - BLE-chip firmware (when applicable) and modem firmware (§3.3);
  - configuration version or "not supported" (§3.2);
  - for spec-extended devices, the spec description (§4.3);
  - connection type icon: Bluetooth or USB;
  - the port identifier: the serial port name for USB [observed], or the Bluetooth address
    [inferred].
- Whether the model-name text itself is on the home card is UNKNOWN (§6). It is shown in
  the device status panel after connecting (§4.4).

### 4.2 Friendly names and Bluetooth names [observed]

- **USB:** the operating system's friendly device name is read only to extract the port
  name inside the trailing parentheses (for example `(COM7)`). It is not displayed and is
  not used to guess a model.
- **Bluetooth Classic:** inquiry results are kept by Bluetooth address and last-seen time
  (01 discovery filter: authenticated, service class value 0x308, seen in the last 1 min).
  The reference tool does **not** derive a model from the Bluetooth device name, and no
  naming pattern is used for that purpose. The model only becomes known from the probe
  (§3). Whether the name is shown anywhere at all is UNKNOWN (01 §2.2 item 6 says the name
  is shown as-is; the analysis found no use of it in the device list; see UNKNOWN item 7).
- **BLE:** the reference tool has no PC-side BLE (01) [observed]. Nothing to mirror.

### 4.3 Grouping [observed]

- Connected devices are split into two groups:
  - **Base:** spec ID ≤ 1;
  - **Extended:** spec ID > 1. The card also shows the spec description.
- There is no grouping by family (FMB vs FMB640) or by product line in the device list
  [inferred: no such grouping found].
- **Offline model list** (create a configuration without a device):
  - a fixed home entry comes first;
  - then there is one entry per hardware-table row that has at least one configuration,
    titled with its `FmType`, in hardware-table file order. The table carries a comment saying
    the offline sort follows the file;
  - each entry has a picture (§2.3) and a link to the row's `WikiUrl`;
  - a free-text box filters the entries by model name. The exact matching rule is UNKNOWN.
  - inside a model, configurations are again split into Base and Extended (spec) lists.

### 4.4 After connecting [observed]

The device status panel shows:

- the picture from the same rule (§2.1);
- IMEI;
- firmware (§3.3);
- spec ID;
- configuration version.

When no device is connected but a configuration is open, the panel shows the picture,
the model name (labelled as device type) and the configuration version.

---

## 5. Application guidance (clean side) [inferred]

- Extract pictures with §1 from the same user-supplied download used for 08. Cache them per
  Configurator version in the user's local application data. Never bundle them with the
  application, sync them or upload them.
- Key the cache by `<image_key>` (§1.3). Look up with the reported hardware code. If there is
  no entry, use `unknown`; if no download was supplied, use the application's own generic icon.
- USB serial and paired Bluetooth: to match the reference tool, show a device only after the
  01/03 probe. Then show the picture and the model name from §3. Before or without a probe
  (for example a port that is still being probed, or a probe failure), the application may
  show its own placeholder with the port name.
- BLE (application-only feature): the reference tool gives no rule. Showing a provisional
  model parsed from the advertised name is an application design decision. If used, treat
  it as a hint and replace it with the probe result once connected. This spec gives no
  vendor rule for it.

---

## 6. Test guidance (1.8.21, against the user's own file)

| Check | Expected |
|---|---|
| Resource blob `Brand.g.resources` | 2,772,396 bytes, SHA-256 `1feaed8bfa7064ba45cfd5c03954a8f5e9d32e6b7756fd3f394e8c63f79978a3` |
| Reader header | version 1, skip 145; set version 2; 0 type names |
| Entry count | 179 (devices 131, misc 29, brands 14, logos 3, root documents 2) |
| Device pictures | 131 PNG entries including `unknown`; 90 distinct by content; 2,486,615 bytes total; smallest 2,500 bytes, largest 439,499 bytes |
| Hardware-table coverage | all 76 hardware codes counted by the analysis team (70 FMB family + 6 FMB640 family; 08 §3.7 counts 7 FMB640-family rows, difference unexplained, UNKNOWN item 8) have a picture; 54 further pictures exist for codes not in the table (including `BaseOf` codes such as `fmb9_1`, `fmb9_2`, `fmb1_34`) |
| `devices/fmb1_6.png` (FMB140) | 2,500 bytes, PNG 128 × 100 |
| `devices/fmb0_1.png` (FMB010) | 5,903 bytes, PNG 134 × 101 |
| `devices/fmb640_1.png` (FMB640) | 6,833 bytes, PNG 127 × 100 |
| `devices/unknown.png` | 3,719 bytes, PNG 101 × 101, SHA-256 `9ad71e2bfeb7cb8cf3e38bba18bc7c6674c19236a42b3932f6297286722944a2` |
| Largest pictures | five `devices/tat1_*.png` entries, up to 4,232 × 3,314 pixels |
| Name rule | `FMB1:6` → `devices/fmb1_6.png`; no code → `devices/unknown.png` |

---

## UNKNOWN / to confirm

1. Whether the home-screen device card shows the model name as text, or only the picture
   (the card shows the picture, IMEI, firmware, configuration and port; no
   model-name text was found on that card).
2. Which configuration supplies the offline-list picture when a model has several hardware
   codes (§2.3).
3. Exact matching rule of the offline-list text filter.
4. Whether the port identifier on a Bluetooth card is the address or another string.
5. Picture behaviour when the code is known but has no entry: the reference tool most likely
   shows nothing (§2.2), but this has not been seen in a running tool.
6. Whether later Configurator versions move pictures to another resource or format. Check
   for a missing `devices/` prefix or a non-PNG signature and report a format change.
7. Conflict with 01 §2.2 item 6: that item says the Bluetooth device name is shown to the
   user as-is, while the analysis found the name unused in the device list. Which view
   shows it, if any, and the default advertised-name format, are UNKNOWN.
8. The FMB640 family's hardware table has 7 rows (08 §3.7) but the coverage figure in §6
   counts 6 codes; which row is not counted, and whether it has a picture, is UNKNOWN.
9. The model-name source in §3.1 (configuration offered by the definitions) differs in
   emphasis from 04 §3.1 (hardware-table lookup by code). For a code present in the table
   but with no usable configuration, the reference tool shows an empty name (§3.2); the
   application should show the table's model name instead [inferred: recommendation].
