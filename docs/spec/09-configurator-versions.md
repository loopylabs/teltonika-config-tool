> Status: released v1 — reviewed 2026-10-24. Items marked UNKNOWN/INFERRED are not yet confirmed. This spec compares two Configurator releases and reproduces no vendor catalogue; the application still reads definitions at run time from a download the user supplies (08-definitions-source). 06-firmware is not yet released.

# 09 – Configurator versions: 1.8.10 vs 1.8.21

Compares two vendor tool releases so one implementation can serve the devices of both.
Facts are tagged **[observed]**, **[inferred]** or **UNKNOWN**. Terminology follows 04: the
**FMB family** (every model except the next) and the **FMB640 family**.

Releases compared:

| Label | Tool release | Build date | Role |
|---|---|---|---|
| `old` | 1.8.10 | February 2025 | older release, obtained for "basic tracker" support |
| `new` | 1.8.21 | later | reference release for specs 01–05 and 08 |

Related: 01-transport, 02-framing, 03-commands, 04-parameters, 05-cfg-files, 08-definitions-source.

Method: static comparison of both packages only. No device was used. "Identical" below
means identical behaviour as far as static comparison shows; internal renumbering and
cosmetic changes are ignored.

---

## 1. Summary

- Both releases contain the **same set of components** with the same layout and the same
  definition container [observed].
- Transport, framing, CRC, probe sequence, binary command table and text commands are
  **identical**, with one exception: the sub-operation byte of the "clear DTC" request
  (§3.4) [observed].
- The **device model tables are almost identical**. Neither release adds a model family
  over the other. `old` has one extra hardware code (§2.2). `new` adds hardware
  attributes to existing models (§2.3) [observed].
- `old` has **no model or family that `new` lacks**, apart from that one hardware code.
  What `old` offers is the definition set as it stood in early 2025 (§4). For the
  two "basic" models compared (FMB010, FMB920) both releases produce the same parameter ID
  set at configuration version 12.0.0.0 (§4.4); other models and versions are not all checked.
- The `.cfg` format is identical (§5) [observed].
- One implementation can cover the devices of both releases (§6).

## 2. Device coverage

### 2.1 Families [observed]

Both releases have the same two definition families: the FMB family and the
FMB640 family. Family membership of every model is the same in both.

### 2.2 Hardware codes present in only one release [observed]

| Hardware code | Model label | In `old` | In `new` | Family |
|---|---|---|---|---|
| `FMM0:31` | FMM001 BG95M1 | yes | **no** | FMB |

There is no hardware code in `new` that is missing from `old`. The hardware-code aliases
(04 §3.1 alias rule) are identical in both. For example, the row `FMB9:22` (FMB920) also
represents `FMB9:2`, and the row `FMB9:21` (FMB900) also represents `FMB9:1`.

`old` composes a full parameter set for `FMM0:31` (about 4 000 entries at configuration
version 12.0.0.0, the same order as other FMM001 variants) [observed]. In `new` the code
is absent from both the hardware table and every hardware-specific change block, so a
device reporting it is "unknown hardware" to `new` [observed]. What `new` then does
(refuse, fall back to the FMB family) is UNKNOWN; 04-parameters gives no rule for a hardware
code missing from the table.

Total entries: `old` 77, `new` 76 [observed].

### 2.3 Hardware attribute changes [observed]

Attributes gate parameters per model (04-parameters, hardware-attribute filter). `new`
adds attributes to these existing codes; `old` never has an attribute that `new` lacks:

| Change in `new` | Hardware codes (model) |
|---|---|
| `+AIN1, +AIN2` | `FMB1:7` (FMB150), `FMB1:134` (FMB250), `FMC1:6` (FMC150 2 DOUT), `FMC1:133` (FMC250 2 DOUT), `FMM1:5` (FMM150), `FMM1:132` (FMM250) |
| `+AIN1` | `FMC1:7` (FMC150 3 DOUT), `FMC1:134` (FMC250 3 DOUT) |
| `+BT_OBD` | `FMC8:1` (FMC800), `FMM8:1` (FMM800), `FMM8:3` (FMM80A) |
| `+Meig_Quectel` | `FMC0:3`, `FMC1:5`, `FMC1:6`, `FMC1:7`, `FMC1:130`, `FMC1:131`, `FMC1:133`, `FMC1:134`, `FMC2:1`, `FMC8:1`, `FMC8:2`, `FMC9:1`, `FMM1:5`, `FMM1:23`, `FMM1:24`, `FMM1:25`, `FMM1:132`, `FMM1:151`, `FMM8:1`, `FMM8:2`, `FMM8:3`, `FMM9:1` |

Consequence [inferred]: with `old` definitions, parameters gated on these attributes are
hidden for these models. Using the `new` hardware table with `old`-era configuration
versions is safe, because the attribute filter is independent of the version chain.

### 2.4 "Basic tracker" models

Neither release labels any model as "basic" [observed]. The hardware table's free-text
technology tags ("Easy", "Slim", "Ultra-small", "OBDII connector/data") identify the
compact, low-I/O models. Read as "basic trackers" [inferred], they are:

| Group | Hardware codes (model) |
|---|---|
| Compact "Easy"/"Slim" | `FMB9:1`/`FMB9:21` (FMB900), `FMB9:2`/`FMB9:22` (FMB920), `FMB9:7` (FMB910), `FMC9:1` (FMC920), `FMM9:1` (FMM920), `FMT1:1` (FMT100), `FMP1:1` (FMP100) |
| OBD plug-in | `FMB0:1` (FMB010), `FMB0:2`/`FMB0:3` (FMB001), `FMB0:4` (FMB003), `FMB0:5` (FMB002), `FMB0:6` (FMB020), `FMC0:1` (FMC001), `FMC0:3` (FMC003), `FMC0:4` (FMC00A), `FMM0:1`/`FMM0:2`/`FMM0:21` (FMM001), `FMM0:22`/`FMM0:25`/`FMM0:41`/`FMM0:51` (FMM003), `FMM0:31` (FMM001 BG95M1, `old` only), `FMM0:34` (FMM00A) |

Coverage: every one of these is supported by both releases, except `FMM0:31`, which only
`old` supports [observed]. Only `FMC0:3`, `FMC9:1` and `FMM9:1` differ in attributes
(`+Meig_Quectel` in `new`, §2.3).

## 3. Protocol differences

### 3.1 Transport (01-transport) [observed]

Identical: USB discovery VID/PID list, serial settings (115200 8N1, DTR/RTS),
read/write timeouts, open-retry policy, Bluetooth SPP parameters.

### 3.2 Framing and CRC (02-framing) [observed]

Identical: binary frame layout, byte order, request-ID rule, CRC-16 parameters,
per-transport idle timing.

### 3.3 Probe and text commands (03-commands) [observed]

Identical: probe sequence, info keys and their parsing, text commands, configuration
read terminator and retries, chunked parameter write rule and reply checks, save
command, keyword handling.

### 3.4 Binary commands (03-commands §7) [observed]

The table of binary command IDs, expected response IDs, minimum protocol versions and
feature bits is **identical** in both releases. The status-record value-type table and
the file-management reply layouts are identical.

One payload differs:

| Command | Purpose | Payload byte 0 in `old` | Payload byte 0 in `new` |
|---|---|---|---|
| `0x0073` (DTC command; response `0x0074`, feature bit 33) | read DTC list | `1` | `1` |
| `0x0073` | **clear DTC list** | **`2`** | **`3`** |

- Which firmware expects `2` and which expects `3` is UNKNOWN. Neither release checks
  firmware version or protocol version before choosing the value; each sends its own
  constant unconditionally [observed].
- Inferred: the firmware changed the sub-operation code, with `2` the older code. An
  implementation that offers "clear DTC" needs either a firmware-version threshold
  (UNKNOWN; confirm on a device) or a device test: send `3`, check the response status,
  and fall back to `2` (inferred, untested).

### 3.5 Other behaviour changes (not on the device protocol) [observed]

- `new` adds some user-interface notifications and status display rows. The status record
  format is unchanged, so nothing here affects the device protocol.
- Cloud-service defaults changed (not device protocol).

## 4. Definitions

### 4.1 Packaging [observed]

Same in both releases: the definitions are XML documents stored as named resources inside
one component of the application bundle, uncompressed, grouped by family, with the same
naming scheme (`<family>` + `<file>`: Base, BaseExtension, Collections, Lists, Hardware,
Templates, UpdateConfiguration, `Specs.<id>`, and `Presets.<name>` for the FMB family).
The 08 extraction walk works on `old` unchanged and yields 138 definition documents (FMB 75,
FMB640 63, derived from §4.2). `new` yields 147 (FMB 76, FMB640 71, as in 08). This answers the
question in open-questions row 37 for 1.8.10 [observed]; the 1.8.10 executable's size, hash and
version-resource values are not recorded here.

### 4.2 File-set differences [observed]

| Family | Files only in `new` |
|---|---|
| FMB | `Specs.206` |
| FMB640 | `Specs.1057`–`1060`, `Specs.1111`, `Specs.1152`, `Specs.1214`, `Specs.1226` |

`old` has no definition file that `new` lacks. Firmware-package bounds
(UpdateConfiguration) carry the same values in both.

### 4.3 Schema differences [observed]

- `new` documents declare a schema location attribute on the root; `old` documents
  mostly do not. This has no effect on content.
- FMB family: `new` adds a CAN-configuration view element and orientation/visibility
  attributes on view/table elements (presentation only).
- FMB640 family:
  - `old` puts a minimum-tool-version attribute on every software-version entry (181
    entries, all `0.9.0.0`). `new` dropped it. It has no observed effect in either release [inferred]. Implementations ignore it.
  - `old` Base contains 2 elements whose tag is a misspelling of the placeholder
    element. Implementations treat it as a placeholder [inferred].
  - `new` adds IO templates for this family (`old`'s template file is empty), an
    IO-template reference attribute on IO items, min/max on IO level fields, a
    feature attribute on group boxes, and tab groups.
- Collection elements in `old` FMB640 definitions may carry Min/Max. `new` dropped them.

### 4.4 Version coverage and example counts

Highest configuration version (= newest firmware line covered) [observed]:

| Family | `old` newest | `new` newest | Software entries `old` / `new` |
|---|---|---|---|
| FMB | 12.0.0.0 (firmware 04.00.00+) | 13.0.0.0 (firmware 04.01.00+) | 32 / 33 |
| FMB640 | 4.41.0.0 | 4.45.00.0 (firmware 03.01.03+) | 181 / 187 |

Composed parameter counts (the analysis team's composition; approximate, see 04-parameters):

| Model (code) | Config version | `old` | `new` |
|---|---|---|---|
| FMB010 (`FMB0:1`) | 12.0.0.0 | 3761 | 3761 (identical ID set) |
| FMB920 (`FMB9:22`) | 12.0.0.0 | 4057 | 4057 (identical ID set) |
| FMB140 (`FMB1:6`) | 12.0.0.0 | 4548 | 4546 (`new` drops 2 IDs at this version) |
| FMB640 (`FMB640:1`) | 4.41.0.0 | 5205 | 5551 |

Inferred from the two basic models above only (FMB010, FMB920): `new` definitions give the
same result at older configuration versions. FMB140, which is not a basic model, differs. The FMB640
family was revised retroactively, partly because IO definitions moved into the new
template mechanism. Part of the 346-entry gap may be composer approximation (UNKNOWN).

### 4.5 Composition-rule differences [observed, detail inferred]

The composition pipeline is the same in outline (04-parameters). `new` adds:
- a step that corrects a parameter's default value when the default names an option the
  hardware does not support (it picks a supported option);
- hardware-attribute removal applied to more element kinds than collection items.

One FMB-family spec patch contains a delay-configuration element, in both releases'
definitions [observed]. Whether the two releases treat it differently is UNKNOWN; it
affects only that spec ID.

## 5. `.cfg` files [observed]

Identical in both releases: gzip container, ASCII header with the same keys and order
(ConfigurationVersion, HwVersion, Title, FmType, SpecId), `:`/`;` → `_` in header values,
`id:value;` entries, and the same loading rules. Files are interchangeable between the releases. A file's
ConfigurationVersion may be newer than `old` knows (for example 13.0.0.0); `old` then
applies its "version newer" confirmation (05-cfg-files §load checks) [inferred].

## 6. Recommendation (spec terms)

One implementation can cover both releases' devices:

1. **Protocol: single implementation.** Use 01–03 as released. The only per-device
   variable is the clear-DTC sub-operation (§3.4). Key it by firmware version once the
   threshold is known, otherwise probe with fallback.
2. **Definitions: use the 1.8.21 set** (read from the user's download, per 08; nothing is
   shipped), and accept a 1.8.10 download as a source too (§4.1). `FMM0:31` (FMM001 BG95M1,
   attributes `OBD,Meig_Quectel`, FMB family) exists only in a 1.8.10 download, so it needs
   that source or a hardware-table row built from these facts. Where the two sets differ
   for a model and version (FMB140 at 12.0.0.0, FMB640 at 4.41.0.0; §4.4), which one matches
   a real device is UNKNOWN. Older firmware is already covered because the version chain selects the
   highest configuration version ≤ the device's reported version (04-parameters). `old`
   definitions are otherwise needed only if a device on an FMB640-family configuration version ≤ 4.41 is found to
   reject parameters that `new` adds retroactively (UNKNOWN; validate against a real
   device read).
3. **Capability selection:**

| Selector | Source | Decides |
|---|---|---|
| hardware code (info key 2), alias-resolved | hardware table | family, model label, attribute filter |
| configuration version (info key 1) | version chain | parameter set (highest version ≤ device) |
| spec ID | spec patches | per-customer patch |
| feature bits (info key 12, status 784) and protocol version (status 781) | device | binary command availability (existing 03 table, same in both releases) |
| firmware version (info key 0) | device | clear-DTC sub-operation (UNKNOWN threshold) |

No selection by tool release is needed. No model needs a different transport, framing
or command set.

## 7. UNKNOWN / to confirm on hardware

- Firmware threshold between clear-DTC sub-operation `2` and `3`.
- How `new` handles a device reporting `FMM0:31`.
- Whether FMB640-family devices on older firmware accept the parameters `new` adds
  retroactively at the same configuration version.
- Whether the "basic tracker" meant by the project owner is a model outside both tables.
  Neither release contains any model family the other lacks.
- Whether `FMM0:31` devices exist in the field and what the 1.8.21 tool does with one.
