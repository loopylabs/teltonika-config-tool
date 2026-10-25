# 05 – Configuration files (.cfg)

> Status: released v1 (partial) — reviewed 2026-10-24. Items marked UNKNOWN/INFERRED are not yet confirmed.

Describes the `.cfg` file that Configurator 1.8.21 saves and loads
("Save to file" / "Load from file"). Facts are tagged **[observed]**, **[inferred]** or
**UNKNOWN**.

Related: 04-parameters (parameter IDs, value text forms, keyword parameter 10),
03-commands (§3 read, §4 write).

---

## 1. Container [observed]

- The file is a single **gzip** stream (RFC 1952, deflate). No other wrapper, no
  additional header or trailer.
- File extension `.cfg`; the save dialog's filter label is `TCP (*.cfg)` [observed]. The
  open dialog also offers `.config` files, a different format (§3.2).
- **No encryption, no signature, no checksum** beyond the gzip CRC-32. Nothing on the save
  or load path computes a hash or uses a key [observed]. (The tool computes hashes only
  for unrelated features.)
- Gzip header fields (timestamp, OS byte, compression level) are whatever the writer's
  deflate implementation produces; they carry no meaning. A reader must accept any valid
  gzip stream [inferred].
- The tool also has an uncompressed plain-text variant of the same content, but every
  observed save and load path uses gzip [observed]. A reader may accept plain text as a
  fallback (inferred, harmless).

## 2. Content [observed]

The decompressed payload is a single line of text: no line breaks, no BOM, ending with `;`.

```
ConfigurationVersion:<cfg_version>;HwVersion:<hw_code>;Title:<title>;FmType:<model_name>;SpecId:<spec_id>;<id>:<value>;<id>:<value>;…;
```

### 2.1 Character set
- Written as **ASCII**. A character outside ASCII cannot be stored; the writer substitutes
  `?` [observed ASCII output; substitution character inferred].
- Read as UTF-8, which is a superset of ASCII [inferred].

### 2.2 Header entries

Always exactly these five, always in this order, always first [observed]:

| Key | Value | Source when saving |
|---|---|---|
| `ConfigurationVersion` | dotted version, e.g. `13.0.0.0` | the definition version the configuration was composed from (04-parameters §3.2) [inferred: the selected version, not necessarily the device's raw info key 1] |
| `HwVersion` | hardware code, e.g. `FMB1:6` (escaped, see 2.3 → `FMB1_6`) | info key 2 after alias resolution [inferred] |
| `Title` | firmware hint text of the selected definition version, e.g. `03.25.04 or higher` [observed source] | |
| `FmType` | model name, e.g. `FMB140` | hardware table (04-parameters §3.1) |
| `SpecId` | decimal integer | info key 11 (1 = standard) |

### 2.3 Header value escaping [observed]

- On save, in the four **text** header values (`ConfigurationVersion`, `HwVersion`,
  `Title`, `FmType`), every `:` and every `;` is replaced with `_`. `SpecId` is written
  as a plain decimal integer.
- On load, in those four text values, every `_` is replaced with `:`.
- The mapping is lossy: a `;` becomes `:` after a round trip, and an original `_` also
  becomes `:`. Example: `HwVersion:FMB1_6;` loads as hardware code `FMB1:6`.

### 2.4 Parameter entries [observed]

- After the header: one entry per parameter, `<id>:<value>;`.
- `<id>` is the decimal parameter ID. `<value>` is the value's text form (04-parameters
  §5.1: decimal integers; floats with `.` and up to 15 fractional digits, no trailing
  zeros; strings raw).
- **Item values are not escaped.** A value containing `;` corrupts the file; a value
  containing `:` does not survive loading (see §3) [observed rules; consequence inferred].
- **Order**: entries are written sorted by parameter ID, ascending (numeric order
  inferred) [observed sort by ID], with one exception: the keyword entry (§4) may be
  appended at the very end.
- Every parameter the tool holds for the configuration is written, including ones equal to
  their default [inferred: the tool writes every parameter it holds].

## 3. Loading rules [observed]

The loaded file is interpreted by these rules [observed]:

- The payload is the gunzipped text. It is divided into pieces at every `;`. Empty pieces
  (for example after the final `;`) are ignored.
- Within a piece, the **key** is the text before the **first** `:` and the **value** is the
  text after the **last** `:`. A piece with no `:` has key = value = the whole piece.
  Consequence: `2004:a:b` loads as key `2004`, value `b`.
- A piece whose key is a decimal integer (optional sign and surrounding spaces tolerated
  [inferred detail]) is a **parameter entry**. Entries are kept in file order. Duplicate
  IDs are all kept; which one wins when applied is UNKNOWN (use the last one, inferred).
- Any other key is compared **case-insensitively** with the five header names.
  - `SpecId`: the value must be a decimal integer; if not, it is ignored.
  - The four text keys: `_` in the value becomes `:` (§2.3).
- **Unknown keys are ignored** (no error). The file still loads.
- Missing header keys: `SpecId` defaults to **1**; missing text keys stay empty.
- A file that is not a valid gzip stream fails to load as a whole.

Header position does not matter on load (header keys are recognised anywhere), but the
writer always puts them first [observed].

### 3.1 Checks after loading [observed flow; message wording paraphrased]

- **Version check**: if the file's `ConfigurationVersion` is **newer** than the version of
  the definitions currently open, the tool asks the user to confirm (the prompt warns that some
  parameters may not be loaded). Older or equal versions load silently. Which version
  components take part in the comparison is UNKNOWN (inferred: all four).
- **Hardware check**: if the file's `HwVersion` (after unescaping) differs from the
  current configuration's hardware code, the tool asks the user to confirm in the same way.
  Declining aborts the load [inferred].
- Spec ID is not checked [observed: no comparison on the load path].
- **Keyword**: if the file has a non-empty parameter 10 and a device is connected and
  initialised, and the file keyword differs from the device's current keyword, the tool
  asks the user to confirm. On confirmation it either sets the keyword (device has none)
  or changes it from the current one to the file's (03-commands §6) [observed calls;
  branch details inferred].
- Parameters in the file that the open definition set does not contain cannot be shown;
  whether they are kept and written back is UNKNOWN.

### 3.2 The `.config` alternative [observed]

The open dialog also offers `Configuration (*.config)`. Files with that extension are
read by a different, **XML-based** loader, not the format above. Its layout was not
analysed (UNKNOWN). Saving always produces `.cfg`.

## 4. Keyword (parameter 10) on save [observed]

When the definition set includes the keyword parameter (ID 10, 04-parameters §7):

- If a device is connected, initialised and supports the keyword feature (03-commands §6),
  the entry for ID 10 gets the device's **current keyword** as its value. If there was no
  entry for ID 10, one is **appended at the end** of the entry list (after the highest ID).
- Then, if the entry for ID 10 has an empty value, it is **removed**.

So a saved file carries the keyword only when one is set, and the keyword can appear out
of ID order at the end. The file is not encrypted, so a keyword stored in a `.cfg` is
readable in clear text [inferred].

## 5. Synthetic example

Decompressed content (synthetic values, not a real export):

```
ConfigurationVersion:13.0.0.0;HwVersion:FMB1_6;Title:03.29.00 or higher;FmType:FMB140;SpecId:1;102:2;800:2;802:5555;803:0;1000:300;2001:internet;20105:1000.5;
```

Loaded result:

| Field | Value |
|---|---|
| ConfigurationVersion | `13.0.0.0` |
| HwVersion | `FMB1:6` |
| Title | `03.29.00 or higher` |
| FmType | `FMB140` |
| SpecId | 1 |
| Entries | 102=`2`, 800=`2`, 802=`5555`, 803=`0`, 1000=`300`, 2001=`internet`, 20105=`1000.5` |

The file is this text, gzip-compressed, with no newline at the end.

## UNKNOWN / to confirm

1. Exact version-comparison components and hardware-code comparison case rule (§3.1).
2. `.config` XML layout (§3.2).
3. Which duplicate entry wins.
4. Whether the tool fills in missing parameters with defaults after loading a file that
   lists fewer IDs than the definition set.
