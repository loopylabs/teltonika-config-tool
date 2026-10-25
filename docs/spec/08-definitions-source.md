> Status: released v3 — reviewed 2026-10-24. Revised from released v2. Changes: §6 composition figures corrected for the custom-group rule of 04 §3.9 (FMB140 at 10.0.0.0 now equals the owner's file), FMB640-family figures added. Earlier: released v2 revised §3.3 `Extends` and version-level operations, §3.4 spec references, and the §6 figures. Items marked UNKNOWN/INFERRED are not yet confirmed. This spec describes a file format only; it reproduces no vendor catalogue and ships no vendor data. The application reads definitions at run time from a Configurator download that the user supplies. 06-firmware is released separately.

# 08 – Reading parameter definitions from a user-supplied Configurator download

Scope: Teltonika Configurator 1.8.21 (Windows) as the
reference build. Each fact is tagged **[observed]** (seen directly in the build or its
embedded data), **[inferred]** (follows from observed facts) or **UNKNOWN**.

Purpose: the application never ships vendor definition data. Instead the user points it at
their own copy of the Configurator download, and the application extracts the definition
documents locally and turns them into the definition model of 04-parameters. This document
is the file-format walk needed to build that extractor from scratch: input identification,
container layers, document schema and the mapping to 04 concepts.

Related specs: 04-parameters (data model, composition rules, derived IDs), 05-cfg-files.

Placeholders are written `<snake_case>`. Offsets are byte offsets from the start of the
named container. All multi-byte integers are little-endian unless stated.

---

## 1. Inputs and build identification

### 1.1 Accepted inputs

| Input | Handling |
|---|---|
| The download archive (`.zip`) | Standard ZIP (PKWARE APPNOTE). Look for a member whose name ends in `.exe` and starts with `Teltonika.Configurator_` (case-insensitive); extract it in memory and continue as below. The archive layout (single member or folder) is UNKNOWN; search all members. [inferred: the archive was not available for this analysis; only the exe was] |
| The executable (`Teltonika.Configurator_<ver>_B.<b>_R.<r>.exe`) | Parse as described in §2 [observed name for 1.8.21: `Teltonika.Configurator_1.8.21_B.4.01_R.10.exe`] |
| An already-extracted definition library file (`Brand.dll`, see §2.3) | Optional convenience: skip §2.1–§2.2 [inferred] |

The input can be about 40 MB; the extractor needs random access to the whole file
(the container index sits at the end, the definitions in the middle) [observed].

### 1.2 Recognising a supported build

A supported input satisfies all of the following conditions; the extractor reports the first
one that fails with a clear message and stops.

- **PE image**: bytes 0–1 are `MZ`, and the 32-bit value at offset 0x3C points to `PE\0\0`.
  1.8.21: PE header at 0x00E8, machine 0x8664 (x86-64), optional-header magic 0x020B (PE32+)
  [observed]. A 32-bit (0x014C / 0x010B) image is not expected but nothing in the walk below
  depends on it [inferred].
- **Version resource** (standard `VS_VERSIONINFO` in the PE resource section, string table
  in UTF-16LE). 1.8.21 values [observed]:
  - `CompanyName` = `JSC Teltonika`
  - `FileVersion` = `ProductVersion` = `1.8.21.0`
  - `ProductName` = `FileDescription` = the exe file name
  - `OriginalFilename` = `InternalName` = the main application library name (not the exe name).

  A version string of the form `1.<minor>.<patch>.0` identifies the Configurator line
  [inferred]. `ProductVersion` is the build identifier shown to the user.
- **Single-file bundle marker** (§2.1) is present. If absent, the build packs its libraries in
  a way this spec does not cover: report "unsupported Configurator build (no application
  bundle)" [observed for 1.8.21: present exactly once].
- **Definition library and definition resources** are present (§2.2, §2.3). If either is
  missing, report "unsupported Configurator build (no definition data found)".
- **Schema sanity** holds (§5.3). If root elements or required attributes are missing, report
  a format change and refuse to use the data rather than producing a partial catalogue
  [inferred: recommended policy].

Builds other than 1.8.21 are accepted if steps 1–5 pass, with a warning that only 1.8.21
was verified [inferred: recommended policy].

---

## 2. Container layers (exe → definition documents)

```
exe (PE32+, .NET 8 single-file app host)
 └─ bundle (appended files + index; public .NET single-file bundle format, version 6)
     └─ Brand.dll  (managed PE / ECMA-335 assembly)
         └─ manifest resources  "Brand.Configurations.<family>.<file>.xml"
             └─ XML documents (UTF-8)
```

### 2.1 Layer 1: .NET single-file bundle [observed]

The exe is a .NET 8 "single-file" application host. The format is the public .NET runtime
single-file bundle format (documented in the dotnet/runtime repository, "single-file bundle"
design notes and host sources). Facts needed:

- **Marker**: the 32-byte bundle signature
  `8B 12 02 B9 6A 61 20 38 72 7B 93 02 14 D7 A0 32 13 F5 B9 E6 EF AE 33 18 EE 3B 2D CE 24 B3 6A AE`
  occurs in the host image. The 8 bytes immediately **before** the signature are a signed
  64-bit file offset of the **bundle header**. 1.8.21: signature at 0x204D0, header at
  0x0272E124 [observed].
- **Bundle header**: major version (u32), minor version (u32), file count (i32), bundle ID
  (length-prefixed UTF-8 string, length as 7-bit-encoded integer, the same encoding as .NET
  binary-writer strings). For major ≥ 2 the header continues with 40 bytes: offset and size
  of the deps.json entry (i64, i64), offset and size of the runtimeconfig.json entry (i64,
  i64), and a 64-bit flags word. 1.8.21: version 6.0, 60 entries, flags 0 [observed].
- **File entries** follow, one per file: offset (i64), size (i64), and for major ≥ 6 a
  compressed size (i64; 0 = stored uncompressed), then a 1-byte file type, then the relative
  path (7-bit-length-prefixed UTF-8, forward or back slashes possible [inferred]).
  File type values in 1.8.21: 1 = managed assembly (56 entries), 2 = native library (2),
  3 = deps.json (1), 4 = runtimeconfig.json (1) [observed counts; type meanings per the
  public format].
- **Compression**: when compressed size ≠ 0, the entry's bytes are **raw deflate** (RFC 1951,
  no zlib or gzip header) of length compressed-size, inflating to size. In 1.8.21 every entry
  is stored uncompressed (compressed size 0) [observed]. An extractor must still support
  deflate, because the bundler option that compresses entries may be turned on in other
  builds [inferred].
- Offsets are absolute file offsets in the exe [observed].

### 2.2 Layer 2: the definition library [observed]

- The bundle entry whose path is `Brand.dll` (root of the bundle, no directory) holds all
  parameter definitions. 1.8.21: offset 0x017BC000, size 6,386,176 bytes, uncompressed
  [observed].
- Other entries (application and protocol libraries, satellite resource folders such as
  `de/…`, `ru/…`) contain no definition documents and can be ignored [observed].
- Some libraries in the bundle also carry other libraries embedded as manifest resources
  (§2.3 describes how to enumerate resources of any managed library). None of these hold
  definitions in 1.8.21 [observed]. If a future build moves the definition library into such
  an embedded resource, the extractor may look there as a fallback [inferred]; how those
  embedded libraries are named and stored is UNKNOWN beyond "plain PE bytes in 1.8.21".

### 2.3 Layer 3: manifest resources of the definition library [observed]

`Brand.dll` is a managed PE (ECMA-335, Partition II). Its embedded resources are described
by the following structure:

- The PE optional header's data directory 14 gives the CLI header's RVA; RVAs map to file
  offsets through the section table.
- The CLI header holds the metadata RVA/size at offset 8 and the resources RVA/size at
  offset 24.
- The metadata root starts with the `BSJB` signature and lists stream headers, including the
  `#~` (or `#-`) table stream and the `#Strings` heap. Row counts come from the valid-mask;
  row sizes follow ECMA-335 §II.22/§II.24 (heap-size flags, coded-index widths).
- The **ManifestResource table (0x28)** has columns Offset (u32), Flags (u32), Name (string
  index), Implementation (coded index). Rows with Implementation = 0 are embedded in this
  file.
- Each embedded resource sits at `resources_base + Offset` as a **u32 length** followed by
  that many bytes.

1.8.21: 148 manifest resources in `Brand.dll`; 147 are definition documents, 1 is a
compiled UI resource bundle (name ends `.g.resources`) to be ignored [observed].
Resources are stored **uncompressed**; no per-resource compression or encryption [observed].

### 2.4 Resource naming convention [observed]

`Brand.Configurations.<family>.<file>.xml`

- `<family>` is `FMB` or `FMB640` (04 §2). Split on the first dot after the prefix; family
  names themselves contain no dots [observed].
- `<file>` is one of:

| `<file>` | Content (04 concept) | FMB | FMB640 |
|---|---|---|---|
| `Base` | base layer (04 §1, §3.3) | 1 | 1 |
| `BaseExtension` | version-diff layer (04 §3.2–3.3) | 1 | 1 |
| `Collections` | enumeration library (04 §1.2) | 1 | 1 |
| `Lists` | list library (04 §1.4) | 1 | 1 |
| `Hardware` | hardware table (04 §3.1) | 1 | 1 |
| `Templates` | IO template library (04 §4.1) | 1 | 1 |
| `UpdateConfiguration` | firmware package index (06-firmware) | 1 | 1 |
| `Specs.<spec_id>` | spec patch for `<spec_id>` (04 §3.4) | 32 | 64 |
| `Presets.<preset name>` | sensor preset (04 §6); names may contain spaces, dots and `+` | 37 | 0 |

  Totals 1.8.21: FMB 76, FMB640 71, together 147 [observed]. FMB spec IDs are in the range
  100–206; FMB640 spec IDs are 1000–1226 [observed]. Because preset names can contain dots,
  identify the file kind by the first component after the family (`Specs`, `Presets`, or a
  fixed name) and take the rest up to the final `.xml` as the spec ID or preset name.

### 2.5 Document encoding [observed]

- XML 1.0, UTF-8. 98 of 147 documents start with a UTF-8 BOM (EF BB BF), the rest start
  directly with `<?xml`. Line ends CRLF. Use a standard XML parser that accepts an
  optional BOM.
- Documents start with an XML comment before the root; ignore comments. Some comments
  contain element-like text (for example upper-case pseudo tags); a real XML parser handles
  this correctly, a text search would not.
- The root element carries `xsi:noNamespaceSchemaLocation` (and sometimes `xmlns:xsi`,
  `xmlns:xsd`). The referenced schema files are **not** shipped [observed]. Elements are not
  in a namespace.
- Attribute values are plain text; booleans are `true`/`false`; numbers are decimal unless
  stated; versions are dotted.

---

## 3. Document schema and mapping to 04-parameters

Element and attribute names below are the literal names in the documents. "→" gives the
04 concept. All observed unless marked.

### 3.1 Common attributes (any definition element)

| Attribute | Meaning → 04 |
|---|---|
| `Guid` | layer tag (opaque text, not a real GUID; e.g. `Add_…`, `Remove_…`) → `layer_tag` (§3.3) |
| `Feature` | decimal feature-bit number → `feature_condition` (§3.5) |
| `ShowFor`, `DisabledFor` | brand names → brand conditions; **ignore** (§3.8) |
| `HWAttribute` | hardware attribute name (on enumeration items) → `hardware_condition` (§3.6) |
| `VisibilityDependOnId` + `VisibilityDependOnValues` | show-if: parameter ID + comma-separated values (§1.5) |
| `VisibilityDependOnId` + `InvisibilityDependOnValues` | hide-if (§1.5) |
| `VisibilityDependOn` | expression form `<id>:<v>[,<v>…]` joined by ` AND ` (§1.5) |
| `DisableOnValues` / `EnableOnValues` | disable-if / enable-if values; the controlling parameter is the one named by `VisibilityDependOnId` on the same element [inferred] |
| `WikiUrl` | `help_url` |

### 3.2 Base layer (`Base`, root `Configuration`)

| Element | Attributes | → 04 |
|---|---|---|
| `Configuration` | — | root of the presentation tree |
| `Keyword` | `ParameterId` | the keyword (password) parameter ID; 10 in both families → 04 §7, 05 |
| `ViewConfiguration` | `Name`, `Guid`, `Orientation`, `Feature`, `VisibilityDependOn` | page |
| `GroupBox` | `GroupName`, `Orientation` (`Horizontal`/`Vertical`), `WikiUrl`, conditions | group box |
| `Group`, `TabGroup` | `Orientation`, conditions | nested group / tab group |
| `TableControl` with `Column Name=…` and `TableLabel Label=…` | `PresetId`, `DisableOnValues` | table; `PresetId` links a sensor preset table (§6) [inferred] |
| `Placeholder` | `Guid`, optional `Feature` | anchor for adds (§3.3); never a parameter |
| `NumberInput` | `ParameterId`, `ParameterType`, `Label`, `Default`, `Min`, `Max`, `MinId`, `MaxId`, `Increment`, `Units` | numeric parameter (`min_from_id`/`max_from_id` = `MinId`/`MaxId`, `step` = `Increment`) |
| `TextBox` | `ParameterId`, `ParameterType`, `Label`, `Default`, `TextMinLength`, `TextMaxLength`, `TextWrap`, `Validate` | text parameter; `Validate` = rule names separated by `;` (§5.3) |
| `TextBoxMasked` | as `TextBox` + `TextMask`, `Converter` (a display-converter name; the one value seen presents an integer as minutes), `Min`, `Max` | masked text / display converter (§5.4) |
| `TextBoxDomain` | as `TextBox` | domain-name text box |
| `Lls` | `ParameterId`, `ParameterType` (`LLS`), `Label`, `Default`, `Validate` | fuel-sensor setting text |
| `UiControl` | `ParameterId`, `ParameterType`, `Label`, `Min`, `Max`, `Default` | plain parameter with a special editor [inferred] |
| `Collection` | `ParameterId`, `ParameterType`, `Label`, `Default`, `Collection` | parameter bound to an enumeration (radio/drop-down); `Collection` = enumeration `Name` |
| `DropDownList` | `ParameterId`, `ParameterType`, `Label`, `Default`, `Collection` or `SourceBindToList` | drop-down bound to an enumeration (`Collection`) or to a list (`SourceBindToList` = list `Name`) |
| `List` | `List` (= list `Name`), conditions | list block placement (§1.4, §4.3) |
| `IO` | `Name`, `Visible`, `ExternalData` | IO element table (§4.1); `Visible="false"` = not shown [inferred] |
| `IOItem` | `IoId`, `Name`, `Units`, `SmsEventId`, `IOTemplate`, `CustomIoItemName`, `DefaultText` | one IO element (§4.1, §4.2) |
| IO sub-elements `Priority`, `Operand`, `HighLevel`, `LowLevel`, `EventOnly`, `AveragingConstant`, `TimeInterval`; CAN sub-elements `CanType`, `DataMask`, `CanId`, `CanSrc` | `Default`, `Min`, `Max`, `ParameterType`, `Collection`, `ControlType`, optional `Offset` | derived IO parameters (§4.1); `Offset` overrides the fixed sub-index |
| `Text`, `Phone` (inside `IOItem`) | `ParameterType`, `TextMaxLength`, `SourceBindToList`, `ControlType` | SMS event text / phone parameters (§4.2) |
| `Geofencing` > `Geozones` | `StartId`, `Offset`, `Count` | zone block (§4.4); children `Priority`, `GenerateEvent`, `EventualRecords`, `FrameBorder`, `ShapeType`, `Radius`, `LongitudeX1`, `LatitudeY1`, `LongitudeX2`, `LatitudeY2`, `Overspeeding`, `MaxSpeed`, … are the per-zone fields in sub-index order |
| `Polygon` | `StartId`, `Offset`, `PointsCount`, `Increment`, `MinLat`, `MaxLat`, `MinLong`, `MaxLong` | polygon zone block; ID layout UNKNOWN (04 open item) |
| `EcoCalculator` | `AccelerationId`, `BrakingId`, `CorneringId` (+ default/limit children) | eco-driving calculator (§1.3) |
| `SendSmsTo`, `SmsText` | `SourceBindToList`, `ParameterType` | SMS helpers [inferred: UI only] |
| `FileTransfer` | `Label`, `Path`, `TransferType` (`Download`/`UploadDownload`), `AllowDelete` | file-transfer page (03-commands file transfer) |
| `CanConfiguration`, `AutoScan`, `CustomButton` (`RequestId`, `ResponseAckId`, `Timeout`), `RangeSlider`/`Thumb`, `CustomIoTable`, `Bluetooth`, `Bluetooth40`, `Dallas`, `Rs232Rs485` | — | special pages / named sections; parameters inside them follow the normal rules |
| `Rules` > `Rule` (`ForParameterId`, `WhenValue`) > `If` (`ParameterId`, `ValueIs`, `Message`) + `DoAction`/`NoAction` (`Type` = `Reset`/`SetValue`/`SetParameter`, `ParameterId`, `Value`) | | cross-parameter rules (§1.5) |

**Which elements are parameters**: every element with a `ParameterId` attribute, **except**
`Keyword`, `Thumb`, `If`, `NoAction`, and `Rule` (these reference parameters defined
elsewhere) [observed in the data; recommended rule]. Plus all IDs derived from `IOItem`,
`List`, `Geozones` (04 §4).

`ParameterType` vocabulary → 04 `value_type`: `Int32`, `Byte`, `UInt16`, `UInt32` (also
spelled `Uint32`), `Int64`, `Float`, `Double`, `String`, `LLS`. Compare case-insensitively.
A `Collection`/`DropDownList` without `ParameterType` takes the enumeration's
`CollectionType` [observed in the data; rule inferred].

### 3.3 Version-diff layer (`BaseExtension`, root `BaseExtensions`)

| Element | Attributes | → 04 §3.2–3.4 |
|---|---|---|
| `Software` | `Version` (dotted 4-part cfg version), `DiffFor` (parent version), `Title` (firmware hint), `EOL` (`true`) | version node of the chain |
| `Hardware` (child of `Software`) | `Version` = hardware code list (`<prefix>:<n>` or `<prefix>:[<a>-<b>]`, comma-separated); optional `Extends` = an older hardware code whose history the block's codes inherit in older versions (04 §3.2 extends rule) | hardware block |
| `Add` > `Item Guid=…` > *definition elements* | | add: the `Item` children are inserted before each `Placeholder` with the same `Guid` |
| `Remove` > `Item Guid=…` (empty) | | remove every element carrying that `Guid` (not anchors) |
| `Modify` > `Item Guid=…` > *replacement elements* | | modify (1 occurrence in 1.8.21) |
| `Spec` (child of `Hardware`, or directly of `Software`) | `Id` (spec ID, integer), `PatchNo` (integer; blank or absent = reference ignored) | applies patch `PatchNo` of `Specs.<Id>` at this version (04 §3.4) |

`Add`/`Remove`/`Modify` also occur **directly under `Software`** (not inside a `Hardware`
block): 43 adds, 26 removes, 1 modify in the FMB family [observed]. They apply to every
hardware code of the family whenever that `Software` is in the applied chain, before that
version's `Hardware` operations (04 §3.3) [observed].

FMB family: 33 `Software` nodes from `3.21.6.0` (root, no `DiffFor`) to `13.0.0.0`; about
220 `Hardware` blocks. FMB640: 187 `Software` nodes [observed].

### 3.4 Spec patches (`Specs.<spec_id>`, root `Spec`)

`Spec Id=<spec_id>` > `Patch No=<n>` > `Add` / `Remove` / `Modify` > `Item Guid=…`, same
semantics as §3.3. Only patches referenced by a matching `Spec` reference in the applied chain
are used. Matching is by integer `Id` and integer `PatchNo`, and the first matching reference
per `Hardware` block wins (04 §3.4) [observed]. `Patch No` is matched as an integer.

### 3.5 Enumeration library (`Collections`, root `CollectionConfigurations`)

| Element | Attributes | → 04 §1.2 |
|---|---|---|
| `CollectionConfiguration` | `Name` (referenced by `Collection=`), `CollectionType` (`Int32`/`Byte`/`UInt32`), `CollectionControlType` (`DropDownList`; absent = radio buttons [inferred]), `ButtonsInRow`, `Mask` (bit-mask enumeration: hex mask such as `FFFFFFFE`, or multi-field form such as `32:4:4,20:12`) | enumeration |
| `CollectionItem` | `Value`, `Text` (label), `HWAttribute`, `EnableDependOn` (`<id>:<v>,…`), `ClearOnDisable` | item |

Spec-limited items: the reference tool also recognises per-item "only for spec IDs" and
"removed for spec IDs" attributes, but no item in 1.8.21 uses them; their exact attribute
names are UNKNOWN from the data [observed absence]. Treat any unknown attribute on
`CollectionItem` as "keep item" and log it.

### 3.6 List library (`Lists`, root `ListConfigurations`)

`ListConfiguration`: `Name`, `StartId`, `NumberOfItems`, `ListType` (`String`/`Int32`),
`TextMaxLength`, `Validate`, `Default`, `Mask`, `ItemsInColumn`, `Image` → 04 §1.4, §4.3
(IDs `StartId … StartId + NumberOfItems − 1`) [observed].

### 3.7 Hardware table (`Hardware`, root `HardwareVersionTypes`)

`HardwareVersionType`: `Version` (hardware code, e.g. `FMB1:6`) → `hw_code`; `FmType` →
`model_name`; `Attributes` (comma-separated) → `attributes`; `BaseOf` → `alias_of`;
`Technology`; `Legacy`; `WikiUrl` → 04 §3.1 [observed]. FMB about 70 rows, FMB640 7 rows.

### 3.8 IO templates (`Templates`, root `IOTemplates`)

`IOTemplate Name=…` > one `IOItem` (attributes `Units`, `CustomIoItemName`, `Feature`) with
IO sub-elements. An `IOItem` in a layer with `IOTemplate=<name>` takes every sub-element of
the template that it does not define itself (own sub-elements win) [observed in data; merge
rule inferred].

### 3.9 Sensor presets (`Presets.<name>`, root `Preset`)

`Preset Id=…` > `Cell Row Column Value` grid (values are hex or decimal text per column)
→ 04 §6. Linked to a table via `TableControl PresetId` [inferred].

### 3.10 Firmware package index (`UpdateConfiguration`)

`Configuration` > `Package Path ErrorId Min Max` (firmware version range). Belongs to
06-firmware; the referenced package files are **not** in `Brand.dll` [observed].

---

## 4. Output of the extractor

The extractor should produce, per family, the documents above parsed into the 04 model
(hardware table, version chain with hardware blocks and operations, base tree, enumeration
library, list library, IO templates, spec patches, presets) and cache it per build
identifier (`ProductVersion` + SHA-256 of the input) in the user's application data folder.
It must never be bundled with or uploaded by the application [requirement from the product
decision]. Composition for a given device follows 04 §3.

---

## 5. Version and robustness notes

### 5.1 Likely to change between Configurator versions [inferred]

- The bundle version (major 6 for .NET 6–8 hosts; a newer runtime may bump it) and whether
  entries are compressed.
- The definition library's file name, or its move into an embedded-library resource.
- The resource prefix (`Brand.Configurations.`), the family set (a new family name), and new
  `<file>` kinds.
- New definition element kinds and attributes (the data already uses about 80 element
  kinds); new `ParameterType` names; new `Software` versions (routine).

### 5.2 Not expected to change [inferred]

The XML vocabulary of §3 is shared by all documents and has been extended rather than
renamed (old and new attributes coexist, e.g. `Uint32`/`UInt32`).

### 5.3 Format-change detection (recommended checks)

- Required roots: `Configuration` (Base), `BaseExtensions`, `CollectionConfigurations`,
  `ListConfigurations`, `HardwareVersionTypes`, `IOTemplates`, `Spec`, `Preset`.
- Each family has all seven fixed files.
- `Software` nodes have `Version`; every `DiffFor` names an existing `Software.Version`;
  exactly one root per family.
- Every `Collection=` and `SourceBindToList=`/`List=` reference resolves; every `IOTemplate=`
  resolves.
- Unknown element names inside the tree: keep as presentation-only, list them in a
  diagnostics report. Unknown element with a `ParameterId`: treat as a generic parameter
  using the common attributes.
- Any check failure on the fixed files → refuse the build with "definition format not
  recognised (Configurator <version>)".

---

## 6. Test guidance (against the user's own file)

All values observed on the 1.8.21 download unless marked.

| Check | Expected for 1.8.21 |
|---|---|
| Exe size | 41,086,351 bytes |
| Exe SHA-256 | `92c8d43bdbbad476e190d54ee4fb56495744d5fe1036aa7cef3282fe9b6de123` |
| Version resource | `ProductVersion` `1.8.21.0`, `CompanyName` `JSC Teltonika` |
| Bundle | signature at 0x204D0, header at 0x0272E124, version 6.0, 60 entries, none compressed |
| Definition library entry | `Brand.dll`, offset 0x017BC000, 6,386,176 bytes, SHA-256 `0c2b8c613b5799976cc1029765416f9fedb5040b0f6d78e165bbd749542385b7` |
| Manifest resources in it | 148 (147 definition documents + 1 non-definition) |
| Documents per family | FMB 76 (7 fixed + 32 spec + 37 preset); FMB640 71 (7 fixed + 64 spec) |
| Total definition bytes | 3,601,111 |
| Fixed-file sizes (bytes) | FMB: Base 202,156; BaseExtension 556,381; Collections 62,939; Hardware 14,701; Lists 4,097; Templates 20,884; UpdateConfiguration 483. FMB640: Base 37,861; BaseExtension 1,377,550; Collections 296,158; Hardware 981; Lists 4,487; Templates 8,848; UpdateConfiguration 483 |
| `Keyword` parameter ID | 10 (both families) |
| FMB version chain | root `3.21.6.0`; newest `13.0.0.0`; for `FMB1:6` at `13.0.0.0` the applied chain has 22 versions |
| FMB140 (`FMB1:6`, attributes include `DIN1`, `OBD`, `CAN`, `Call`, `OneWire`) at cfg version 13.0.0.0, spec 1 | **8,885** distinct parameter IDs. Breakdown: 866 plain, 4,415 list items, 600 geofence, 3,004 IO sub-field, 0 IO SMS |
| Same, spec 107 | **8,887** (2 more plain IDs) |
| FMB140 at cfg version **10.0.0.0**, spec 1 (applied chain: 17 versions, `3.21.6.0` … `10.0.0.0`) | **8,514**. Breakdown: 856 plain, 4,415 list items, 600 geofence, 2,643 IO sub-field, 0 IO SMS. Equal to the owner's real file (see below) |
| FMB920 (`FMB9:22`) at 13.0.0.0, spec 1 | **2,741**. Breakdown: 853 plain, 415 list items, 600 geofence, 873 IO sub-field |
| FMB010 (`FMB0:1`) at 13.0.0.0, spec 1 | **2,480**. Breakdown: 801 plain, 415 list items, 600 geofence, 664 IO sub-field |
| **Composed union**: distinct IDs over all 70 FMB-family hardware codes in `Hardware`, each composed at 13.0.0.0, spec 1 | **10,070** (unchanged by the custom-group rule: every ID it removes for one model exists on another) |
| FMB640 family: applied chain for every code at its newest cfg version `4.45.00.0` | 172 versions (of 187 in `BaseExtension`) |
| FMB640 (`FMB640:1`) at 4.45.00.0, spec 1 | **6,025** |
| FMB641 (`FMB641:1`) at 4.45.00.0, spec 1 | **7,438** |
| FMC640 (`FMC640:1`) / FMM640 (`FMM640:1`) at 4.45.00.0, spec 1 | **6,744** / **6,737** |
| FMC650 (`FMC650:1`) / FMM650 (`FMM650:1`) at 4.45.00.0, spec 1 | **11,265** / **11,264** |
| FMB640-family composed union (6 codes, 4.45.00.0, spec 1) | **11,272** |
| **Raw definition union** (no composition): distinct IDs from every parameter-bearing element in `Base`, in every `Item` of `BaseExtension` and in every `Item` of every `Specs.*`. Lists are counted at the length named by each list reference. IO elements are counted with all sub-fields present on them or supplied by their template, including SMS text and phone. Geofence blocks are counted at full zone count. Presets, `Collections` and `Hardware` contribute nothing. Every IO template is used only through IO elements that name it | **11,717** |
| Alias example | a device reporting `FMB9:1` resolves to the row whose code is `FMB9:21` (model FMB900) |

What every composed figure counts [observed by composition]: distinct parameter IDs produced
under the 04 §3.9 membership rules (including the custom-group rule: an IO element whose own or
template-supplied custom-group name has no custom IO table in the composed set yields no IDs), after the full composition of 04 §3.3. That composition
includes version-level operations, the extends rule, spec-reference matching, removes applied
to IO templates, and the hardware-attribute filter for the stated model. All feature bits are
set (the feature filter is not applied), with no brand conditions. The raw definition union is
an upper bound for data checks only. It is **not** a device parameter set: it mixes mutually
exclusive variants and keeps IO SMS fields that version `10.0.0.0` removes.

Corrections. The released v2 figures (FMB140 8,915 / 8,917 / 8,544, FMB920 4,331, FMB010 4,070)
were wrong because the custom-group rule was not applied. The excess was 30 IDs for FMB140 and
1,590 each for FMB920 and FMB010 (04 §3.9, §3.10). The still earlier figures (about 4,550 / 4,590
for FMB140 at 13.0.0.0) were wrong because version-level operations and the extends rule were
missing. Between 5 and 12 add tags per model in the FMB family (0 to 3 in the FMB640 family)
still find no anchor and are not counted (04 §3.10).

Real-file check [real file reported by the owner]: an owner-saved FMB140 file at 10.0.0.0, spec
1, holds 8,514 IDs. The composition gives the same 8,514, and an independent comparison reported
an exact match by ID. No feature-mask assumption is needed.

FMB640-family figures [observed by composition; not cross-checked against a real file]. The six
hardware rows of that family list no hardware attributes, so the hardware-attribute filter
removes every element that names one. The custom-group rule changes none of these figures.

The figures above for the stated models and versions are exact targets. A tolerance of ±1 % applies only to other models and versions. The definitive check is a real device read (03-commands §3): every ID a
device returns should exist in the composed set, and vice versa except for hidden or
feature-gated entries [inferred].

---

## UNKNOWN / to confirm

- Download archive layout and its hash (only the exe was analysed).
- Exact `Modify` semantics (see 04 §3.3).
- Whether the 5 to 12 unanchored add tags per model indicate a remaining rule gap.
- FMB640-family figures have no real-file cross-check yet.
- Attribute names for spec-limited enumeration items (unused in 1.8.21).
- Polygon zone ID layout; `Legacy` meaning; `ExternalData` meaning.
