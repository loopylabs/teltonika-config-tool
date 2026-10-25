> Status: released v3 (partial) — reviewed 2026-10-24. Revised from released v2. Changes: §3.3 states that operations are applied once over the whole collected chain; §3.4 states that a reference with a blank patch number never takes the "first reference" place; §3.9 gives the full custom-group rule, including names supplied by IO templates; §3.10 figures corrected under that rule (FMB140 at 10.0.0.0 now equals the owner's file); §4.1 template merging made precise. Earlier: released v2 added version-level operations, extends rule, spec-reference matching, parameter membership (§3.9) and reference figures (§3.10). Items marked UNKNOWN/INFERRED are not yet confirmed. The catalogue of definitions is vendor data and is deliberately not reproduced; 05-cfg-files and 06-firmware are released separately.

# 04 – Parameters and parameter definitions

Scope: FM-series trackers handled by Configurator 1.8.21 (FMB family and
the FMB640 family). Every fact is tagged **[observed]** (seen directly in the tool or its
embedded data), **[inferred]** (follows from observed facts, not seen directly) or
**UNKNOWN**.

Related specs: 01-transport (Bluetooth parameters 800–834), 03-commands (§2 device info
keys, §3 configuration read, §4 set-param and save), 05-cfg-files (file format).

---

## 0. Overview

- A tracker configuration is a flat map of **parameter ID → value**. IDs are positive
  decimal integers. Values travel as text (03-commands §4.2; 05-cfg-files) [observed].
- The tracker itself knows nothing about labels, ranges, pages or grouping. All of that
  comes from **parameter definitions** that ship inside the vendor tool [observed].
- The vendor tool holds a catalogue of definitions (roughly 11,700 distinct definition
  entries across all FMB-family models and versions; about 4,500 parameters for a single
  model at one version, e.g. FMB140) as **vendor data embedded in the Configurator**
  [observed]. **This spec does not reproduce that catalogue.** It describes the data model,
  how the right subset is selected for a connected device, and the fixed rules that derive
  IDs. A small number of entries are quoted in §8 for orientation only.
- A raw "read all" (03-commands §3) returns every ID/value pair the device holds, without
  definitions. An implementation can therefore show, edit, save and restore a configuration
  generically (as ID/value pairs) without the catalogue [inferred]. Labels, ranges and
  grouping need a definition source the project is permitted to use (for example public
  Teltonika documentation). Which source the project uses is outside this spec.

---

## 1. Definition data model [observed unless marked]

### 1.1 Parameter definition

Each editable parameter has these properties. All are optional unless stated.

| Property | Meaning |
|---|---|
| `id` | Parameter ID (required). Either stated explicitly or derived by the rules in §4 |
| `value_type` | One of: signed 32-bit integer, unsigned 8-bit, unsigned 16-bit, unsigned 32-bit, signed 64-bit (rare), single-precision float, double-precision float, string, and a special fuel-sensor (LLS) setting text type. Type names are matched case-insensitively [inferred: both capitalisations of the unsigned 32-bit name occur] |
| `control` | How the value is edited: numeric input, text box, masked text box (fixed pattern, see §5.4), drop-down / radio list bound to an enumeration (§1.2), drop-down bound to a list (§1.4), domain-name text box, fuel-sensor text box |
| `label` | User-visible label (English) |
| `default` | Default value, as text in the encoding of §5 |
| `min`, `max` | Numeric bounds, inclusive |
| `min_from_id`, `max_from_id` | The bound is not a constant but the **current value of another parameter** (e.g. a "max" field bounded by a "min" field). May be combined with a constant bound [observed: e.g. parameter 1000 "Open link timeout" has min 30, max 259200 and an additional min-bound link to parameter 1003] |
| `step` | UI increment for numeric inputs (e.g. `0.000001` for coordinates) |
| `units` | Unit label shown next to the value (`s`, `km`, `L×10`, `ms`, …). Display only |
| `text_min_length`, `text_max_length` | Character-count bounds for string values |
| `validation` | Zero or more named validation rules (§5.3) |
| `display_converter` | Optional value converter; the only one used is "integer minutes shown as `hh:mm`" (§5.4) |
| `visibility` | Show/hide condition on other parameters' values (§1.5) |
| `enable_condition` | Disable condition on other parameters' values (§1.5) |
| `feature_condition` | A feature-bit number; the parameter exists only on devices that report that bit (§3.5) |
| `hardware_condition` | A hardware attribute name; see §3.6 |
| `layer_tag` | An opaque identifier text used only for layering (§3.3). Not shown, not sent |
| `help_url` | Optional link to the public Teltonika wiki page for the section |

### 1.2 Enumerations

An enumeration is a named, ordered list of **(value, label)** items with a value type
(32-bit signed, 8-bit unsigned or 32-bit unsigned). A parameter bound to an enumeration
accepts only its item values [observed]. Item properties:

- `value` (integer, written to the device as decimal) and `label` (user-visible text).
- Optional `hardware_condition`: the item is shown only if the device's hardware has that
  attribute (§3.6). Note: for enumeration items this hides the item; it does not delete it
  [observed].
- Optional `feature_condition` (§3.5).
- Optional enable condition `<param_id>:<v1>,<v2>…` — the item is selectable only while
  parameter `<param_id>` has one of the listed values — plus a flag "clear when disabled"
  (if the item becomes disabled while selected, the selection is cleared) [observed data;
  exact clearing behaviour inferred].
- Optional spec conditions: "only for spec IDs …" and "removed for spec IDs …" (§3.4).

Some enumerations are **bit-mask enumerations** (several items can be ticked; the value is
the OR of the item values) and carry a mask description; a few use a multi-field mask
notation such as `32:4:4,20:12`. The exact semantics of the multi-field notation are
UNKNOWN.

### 1.3 Groups and pages

Definitions are arranged in a tree used only for presentation [observed]:

```
page (navigation entry, e.g. "System", "GPRS", "Bluetooth", "I/O")
 └─ group box (titled section, optional help link, horizontal/vertical orientation)
     ├─ parameter
     ├─ table (columns with headings; each row is one IO element or list entry)
     ├─ tab group
     └─ nested group box
```

Pages and groups can carry the same visibility, feature and hardware conditions as a
parameter. Removing a page or group removes everything under it [observed]. Grouping has
no effect on the wire.

Special page kinds exist with fixed layouts: the **IO element table** (§4.1), the
**geofence zone editor** (§4.4), **lists** (§4.3), the **eco-driving calculator** (bound to
three explicit parameter IDs for acceleration, braking and cornering thresholds), the
**file transfer** page (device file system; see open-questions), and **sensor presets**
(§6) [observed].

### 1.4 Lists

A list is a named block of consecutive IDs with one value type and one validation rule.
Properties: `name`, `start_id`, `item_count`, `value_type` (string or 32-bit integer),
`text_max_length`, optional `validation`, optional `default` [observed]. IDs are
`start_id … start_id + item_count − 1` (§4.3).

### 1.5 Visibility and enable conditions [observed data; UI effect inferred]

Three equivalent condition forms occur:

- **Show-if**: `<param_id>` + set of values. Visible only while that parameter's current
  value is in the set.
- **Hide-if**: `<param_id>` + set of values. Hidden while the value is in the set.
- **Expression**: text of the form `<param_id>:<v>[,<v>…]` joined by ` AND `, e.g.
  `60087:1 AND 151:11`. Visible only while every clause holds.

A separate **disable-if** form (`<param_id>` + values) greys the control out instead of
hiding it. Value sets are comma-separated decimal integers.

There are also a few **cross-parameter rules** of the form "when parameter A is set to
value X and parameter B currently has value Y, show a message and then either reset A or
set B to Z". They are UI assistance only [observed data; behaviour inferred].

Visibility and enable conditions are presentational. Whether the reference tool omits
hidden parameters when writing to the device is UNKNOWN; this tool version's write path
sends the parameters it holds (03-commands §4).

---

## 2. Families and file set [observed]

- Definitions are organised in two **families**: the **FMB640 family** (models FMB640,
  FMB641, FMC640, FMC650, FMM640, FMM650; its hardware table also names MCX301 and MCX3M1)
  and the **FMB family** (every other model). Each family has its own, independent
  definition set.
- A family's definition set consists of: a **hardware table** (§3.1); a **base layer**; a
  **version-diff layer** (§3.2); an **enumeration library**; a **list library**; an **IO
  template library** (shared sub-definitions for IO elements); a set of numbered **spec
  patches** (§3.4); and, for the FMB family only, **sensor presets** (§6). A firmware
  package index is also present and belongs to 06-firmware.

---

## 3. Selecting the definition set for a connected device

Inputs come from the device-info reply (03-commands §2):

| Input | Source |
|---|---|
| `<hw_code>` | info key 2, e.g. `FMB1:6` |
| `<cfg_version>` | info key 1, dotted four-part version, e.g. `13.0.0.0` |
| `<spec_id>` | info key 11, default 1 |
| `<feature_mask>` | info key 12, 64-bit |
| `<hw_options>` | info key 14 (unused by this data set) |

For an offline .cfg file the same inputs come from the file header (05-cfg-files):
`HwVersion`, `ConfigurationVersion`, `SpecId`; the feature mask is not stored in the file
(UNKNOWN which mask the vendor tool assumes offline; treating all bits as set is a safe
inferred default for display).

### 3.1 Hardware table [observed]

Each row maps a hardware code to a model:

| Field | Meaning |
|---|---|
| `hw_code` | `<letters><digit>:<n>` as reported in info key 2, e.g. `FMB0:1`, `FMB1:6` |
| `model_name` | user-visible model, e.g. `FMB010`, `FMB140`, `FMB001 Buzzer` |
| `attributes` | comma-separated hardware attribute names, e.g. `DIN1,OBD,Call` (§3.6) |
| `alias_of` | optional second hardware code that this row also represents |
| `technology` | free text shown to the user (e.g. "2G, BLE, OBDII connector") |
| `legacy` | optional flag (2 rows); meaning UNKNOWN |
| `help_url` | public wiki link |

Alias rule: if a device reports a code that appears as some row's `alias_of`, the device is
treated as that row's own `hw_code` [observed]. Example (synthetic shape, real rule): a row
with `hw_code = FMB9:21`, `alias_of = FMB9:1`, model FMB900 means a device reporting
`FMB9:1` uses the `FMB9:21` definitions. Code comparison is case-insensitive [observed].

Family choice: the device belongs to the FMB640 family if its (alias-resolved) code is in
the FMB640 family's hardware table; otherwise the FMB family [observed].

### 3.2 Configuration-version chain [observed]

The version-diff layer is a set of **versions**. Each version has:
- `version` — dotted four-part number, compared numerically part by part;
- `parent` — the version it is a diff against (absent for the oldest);
- `title` — user-visible firmware hint, e.g. `03.25.04 or higher`;
- `end_of_life` flag (display only, inferred);
- optional **version-level operation lists** (add, remove, modify) that are not inside any
  hardware block (see §3.3);
- optional **version-level spec references** (see §3.4);
- a list of **hardware blocks**.

A hardware block applies to a set of hardware codes written as a comma-separated list of
`<prefix>:<n>` or `<prefix>:[<a>-<b>]` (inclusive numeric range), e.g.
`FMB1:[1-4],FMB1:[21-24],FMB2:[1-3]`. A block may also name another hardware code it **extends** (see "Extends rule" below). Each
block holds three operation lists — **add**, **remove**, **modify** — and optional **spec
references** (§3.4).

Selection rule: the hardware code is first resolved through the alias rule. Among the versions
that have a hardware block matching the device, the one with the highest version not greater
than `<cfg_version>` is selected [observed]. The selected version and its `parent` ancestors
back to the root form the applied chain; the chain is applied oldest first, newest (selected)
last, on top of the base layer, and within each version only hardware blocks matching the
device contribute [observed].

Example: device reports `<cfg_version>` = `6.1.6.0`; versions exist for `3.21.6.0` (root),
`6.1.4.0` (parent 3.21.6.0), `6.1.5.0` (parent 6.1.4.0), `6.1.8.0` (parent 6.1.5.0).
Selected = `6.1.5.0`; applied chain = `3.21.6.0 → 6.1.4.0 → 6.1.5.0` [rule observed;
version numbers are from the data].

FMB140 (`FMB1:6`) first appears at configuration version `8.9.0.0` [observed].

**Extends rule (history inheritance)** [observed]. "Extends" does not merge two blocks of the
same version. It makes a newer hardware code inherit the whole *history* of an older code:

- Every version in the applied chain has an **effective code**. At the version that declares
  `extends = <older_code>` and at every newer version, the effective code is the device's own
  (resolved) code. In every older version, starting from the first older version that has a
  block for `<older_code>`, the effective code is `<older_code>`.
- At each version, the hardware blocks that match the effective code contribute.
- The rule repeats if the extended code itself extends another code.
- Consistency rules in the data: the extended code must appear in some older version, and the
  device code must not appear in any version older than the one declaring the extends.

Example: FMB140 (`FMB1:6`) is declared at `8.9.0.0` with `extends = FMB1:22`. For FMB140 at
`10.0.0.0`, versions `8.9.0.0 … 10.0.0.0` contribute their `FMB1:6` blocks, and every version
in the chain from `3.21.6.0` to `6.1.9.0` contributes its blocks for `FMB1:22` instead
[observed]. This is how FMB140 receives the 1-Wire page, the iButton list and many other
groups that are only ever added for older codes. Ignoring this rule loses several thousand
IDs for FMB140 (§3.10).

### 3.3 Layer operations [observed; ordering details partly inferred]

Every definition element can carry an opaque **layer tag** (an identifier text). Operations
refer to tags:

- **Add**: a bundle of new elements with a tag. The base layer contains **anchors**
  (invisible markers) carrying tags. The added elements are inserted at the position of
  the anchor with the same tag (immediately before it). Anchors are removed after
  composition. Adds may themselves contain anchors for later adds (nested adds).
- **Remove**: a tag; every element carrying that tag is deleted with its subtree.
- **Modify**: a tag plus replacement children; children of the tagged element that have the
  same kind as a replacement child are replaced by it. Exact matching rules are UNKNOWN
  beyond this.

**Version-level operations** [observed]. Add, remove and modify lists placed directly in a
version (outside any hardware block) apply to **every hardware code of the family** whenever
that version is in the applied chain. They carry no hardware or spec condition. For each
version, its operations are collected in this order: version-level operations first, then
those of the matching hardware blocks (per the extends rule, §3.2), then those of the matching
spec patches (§3.4). Example: one version-level operation pair replaces the 500-entry iButton
list with a 4,000-entry one (§3.10); another removes the SMS text and SMS phone sub-fields
from all IO elements (§3.9).

**Removes also apply to IO templates** [observed]. Once removes have been applied to the definition
tree, the same collected remove tags are applied to the IO-template set (§4.1): any template
part carrying a removed tag is deleted, so it is no longer supplied when templates are
expanded.

**Ordering rules** [observed]:
- **Collect first, apply once.** Operations are first collected from every version of the
  applied chain into three lists (adds, removes, modifies). Each list is then applied **once**
  to the base tree, as a whole. Versions are **not** applied one after another. Consequences:
  a remove from an older version also deletes elements carrying that tag that a newer version
  adds; a modify from an older version also affects elements a newer version adds; and an
  element added and removed anywhere in the chain is absent, whatever the version order.
- Operations are collected over the applied chain, oldest version first; within a version,
  version-level operations come first, then those of hardware blocks (by effective code), then
  those of spec patches.
- Nested adds (an add whose anchor is supplied by another add) are resolved before insertion.
- On the definition tree, adds take effect before modifies, and modifies before removes.
  Duplicate base entries are dropped.
- The collected removes are applied to the IO templates before the templates are expanded.
- The hardware-attribute filter (§3.6) applies to enumeration items first, then to other
  elements. Brand conditions (§3.8) are ignored. Spec-limited enumeration items (§3.4) are
  then filtered.
- IO templates are expanded and enumeration and list references are resolved after those
  filters; anchors are discarded.
- The feature filter (§3.5) and then the hardware-option filter (§3.7) apply last.
- The geofence output-list length is also adjusted during composition; the exact effect is
  UNKNOWN.

An implementation that reproduces the resulting **set of parameters and their properties**
is sufficient; the order only matters where operations overlap [inferred].

### 3.4 Spec IDs (customer variants) [observed]

- `<spec_id>` 1 means the standard variant: no patch.
- A spec ID > 1 selects a numbered **spec patch** set: a list of patches, each with its own
  number and add/remove/modify lists of the same form as §3.3. A hardware block (or a
  version directly) lists `(spec_id, patch_number)` references; only referenced patches are
  applied, in the version-chain order.
- **Reference matching** [observed]:
  - `spec_id` and `patch_number` are both read as **integers**, and patches are matched by
    integer equality (not by text), so `07` and `7` match.
  - A reference **without a patch number** (or with a blank one) contributes nothing; it is
    skipped. Several such references occur in the data (for example a spec reference listed
    for one hardware code at an older version).
  - A reference whose spec file or patch number does not exist makes the whole definition
    set invalid (the vendor tool refuses to build it). A non-numeric patch number is also
    invalid.
  - Within one hardware block, only the **first** reference whose `spec_id` equals the
    device's spec ID is used. References with a blank or missing patch number are discarded
    when the definitions are read, **before** this choice. So they never take the "first
    reference" place: if a blank reference comes before a numbered one with the same
    `spec_id`, the numbered one is used [observed]. (No hardware block in the 1.8.21 data
    has both kinds for the same `spec_id`, so no reference figure depends on this.)
  - References placed directly in a version (outside hardware
    blocks) all apply when their `spec_id` matches.
  - Only versions in the applied chain contribute. References in versions outside the chain
    are never used, whatever their hardware code.
  - Through the extends rule (§3.2), a hardware block of the **extended** code at an older
    version contributes its spec references too, exactly like its other operations.
  - A spec ID of 1 never applies patches, even if a reference names spec 1 [inferred].
- Enumeration items can be limited to, or removed for, listed spec IDs.
- The vendor tool applies a local access check before opening definitions for spec IDs > 1.
  That check is not part of the device protocol and is not specified here.
- Figures for FMB140 are in §3.10.

### 3.5 Feature-bit filter [observed]

An element with a `feature_condition` of `<n>` is kept only if bit `<n>` (0 = least
significant, taken modulo 64) of `<feature_mask>` is set; otherwise it is removed with its
subtree. Bit meanings are not named in the data. Known: bit 0 = BLE available (see
03-commands §2); other meanings UNKNOWN. Example: two IO elements for "OEM total mileage"
and "OEM fuel level" carry feature 15, so they appear only on devices reporting bit 15.

### 3.6 Hardware-attribute filter [observed]

Elements and enumeration items may carry a `hardware_condition` naming one attribute
(e.g. `DIN3`, `OBD`, `OneWire`). If the device's hardware row (§3.1) does not list that
attribute:
- an enumeration item is **hidden** (kept in the data, not offered);
- any other element is **removed**.

### 3.7 Hardware options (info key 14) [observed]

The definition format allows a condition on hardware-option tokens from info key 14, but no
definition in this tool version uses it. Store and ignore.

### 3.8 Brand conditions [observed]

Some elements carry "show for brand X" / "disabled for brand X" conditions. They apply only
in branded builds of the vendor tool. **The standard tool ignores them**; an interoperable
tool must ignore them too (they appear on ordinary parameters such as the APN and the
ignition IO element, which must stay visible).

### 3.9 Which composed elements are device parameters

After composition (§3.3), the device parameter set is every parameter ID that the composed
definitions give rise to. The vendor tool registers a parameter for each editable field it
builds from the composed tree, and its configuration read, write and `.cfg` save cover exactly
those registered IDs [observed]. Rules for the doubtful cases:

| Case | Parameter? | Status |
|---|---|---|
| Ordinary element with its own parameter ID, anywhere in the composed tree, including on pages that are never shown | yes | observed |
| IO elements in an IO page marked **not visible** (hidden pages, e.g. CAN data and the various vehicle flag pages) | **yes**. The page's fields are built whatever its visibility; the flag only hides the menu entry | observed |
| IO elements in an IO page marked **external data** | **yes**. The flag changes how live status values are grouped, not whether fields exist | observed |
| IO element whose **effective custom-group name** (see below) is non-empty | only if the composed tree contains a **custom IO table** with exactly that name. Its fields are then built through that table. Such elements are never built on the ordinary IO page. With no matching table they produce **no** parameters | observed |
| IO sub-fields (priority, operand, high, low, event-only, averaging, time interval, CAN fields) | one ID per sub-field **present** on the element after template expansion (§4.1); absent sub-fields produce nothing | observed |
| SMS text and SMS phone of an IO element | one ID each, **only if** the element still has those sub-fields after composition and template expansion. From configuration version `10.0.0.0` on, a version-level remove deletes both sub-fields from every IO element and template in the FMB family, so at `10.0.0.0` and later **no IO element has SMS text or phone IDs** | observed |
| List reference | `item_count` IDs (§4.3), using the list length named by the composed element | observed |
| Geofence zone block | `zone_count` × (number of zone sub-field kinds present in the block) IDs (§4.4) | observed (data); count rule inferred |
| Elements removed by the feature filter (§3.5) | no; depends on the device's feature mask | observed |
| Anchors, enumeration items, labels, layout-only elements | no | observed |

Duplicate IDs (the same ID produced by two elements) count once [inferred].

**Custom-group rule in full** [observed unless marked]:
- **Effective name.** An IO element's effective custom-group name comes from its own
  custom-group attribute if the element carries that attribute, even with an empty value.
  Otherwise it comes from the IO template the element names: template expansion (§4.1) copies
  every template attribute the element does not already carry, and the custom-group name is
  one of them. A **template-supplied name counts exactly like a name written on the element.**
  Template expansion happens after removes have been applied to the templates (§3.3), so a
  removed template part supplies nothing.
- **Empty effective name**: the element is an ordinary IO element. Its fields are built on its
  IO page (hidden or not, §3.9 table).
- **Non-empty effective name**: the element is left off its own IO page. Each custom IO table
  in the composed tree builds fields for every IO element, on any IO page of the composed set,
  whose effective name equals the table's name. Matching is exact text equality, case
  sensitive. The table may sit on any page, visible or not.
- **No custom IO table with that name** anywhere in the composed tree: the element yields
  **no parameter IDs** at all. The vendor tool does not read, write or save them in `.cfg`
  files.
- Custom IO tables are ordinary composed elements. Adds, removes, the hardware-attribute
  filter and the feature filter decide whether they exist for a model. If a filter removes a
  table, its elements lose their parameters too [inferred for the feature filter; no 1.8.21
  case seen].
- Examples (1.8.21 data, observed by composition):
  - **FMB140 at 10.0.0.0 and 13.0.0.0.** Six IO elements take a custom-group name from their
    template, and no table of that name exists for FMB140. Their 30 sub-field IDs (five per
    element, within 605850–605914) are absent. The owner's vendor-saved FMB140 file
    confirms this: it has none of them.
  - **FMB920 (`FMB9:22`) and FMB010 (`FMB0:1`) at 13.0.0.0.** 314 IO elements carry a
    custom-group name and have no matching table on these models. 97 of them carry the name
    themselves (CAN-adapter IO data). 217 get it from their template (sensor-group IO data in
    six groups). Together they yield 1,590 IDs that do not exist on these models. Of the
    custom IO tables, only the four tables for one sensor-tag group are present on these
    models.

### 3.10 Reference figures (FMB family, 1.8.21 data) [observed by composition]

These figures come from a reference composition applying §3.2–§3.9 in full, **including the
custom-group rule of §3.9**. Every figure counts **distinct parameter IDs**, with all feature
bits set (the feature filter not applied), no brand, and the hardware-attribute filter applied
for the stated model. Spec 1 unless stated.

| Model (code) | Version | Spec | Distinct IDs |
|---|---|---|---:|
| FMB140 (`FMB1:6`) | 13.0.0.0 | 1 | 8,885 |
| FMB140 (`FMB1:6`) | 13.0.0.0 | 107 | 8,887 |
| FMB140 (`FMB1:6`) | 10.0.0.0 | 1 | **8,514** |
| FMB920 (`FMB9:22`) | 13.0.0.0 | 1 | 2,741 |
| FMB010 (`FMB0:1`) | 13.0.0.0 | 1 | 2,480 |

Breakdowns (plain / list items / geofence / IO sub-field / IO SMS):

| Model, version | Plain | List items | Geofence | IO sub-field | IO SMS |
|---|---:|---:|---:|---:|---:|
| FMB140 13.0.0.0 s1 | 866 | 4,415 | 600 | 3,004 | 0 |
| FMB140 13.0.0.0 s107 | 868 | 4,415 | 600 | 3,004 | 0 |
| FMB140 10.0.0.0 s1 | 856 | 4,415 | 600 | 2,643 | 0 |
| FMB920 13.0.0.0 s1 | 853 | 415 | 600 | 873 | 0 |
| FMB010 13.0.0.0 s1 | 801 | 415 | 600 | 664 | 0 |

FMB140 list items include the iButton list IDs 30000–33999. Geofence is 50 zones × 12 fields.

Notes:
- **Corrections.** The released v2 figures (FMB140 8,915 / 8,917 / 8,544; FMB920 4,331; FMB010
  4,070) came from a composition that **did not apply the custom-group rule**. They are wrong.
  The differences are exactly the IDs of §3.9's examples: 30 for FMB140 and 1,590 each for
  FMB920 and FMB010. Applying the rule only to names written on the element itself (ignoring
  template-supplied names) is also wrong. It keeps the 30 FMB140 IDs and drops only 97 of the
  314 elements on FMB920 and FMB010.
- The feature filter removes up to 219 of the FMB140 IDs, depending on the device's feature
  mask. Examples: the beacon-list page, the 1-Wire temperature and mode groups, the immobiliser
  group, low-power mode, and the jamming and call-control groups.
- Earlier figures of about 4,550 (spec 1) and about 4,590 (spec 107) for FMB140 at 13.0.0.0
  were also wrong. They ignored version-level operations and the extends rule.
- A small number of add tags (5 to 12 per model over all 70 FMB-family codes at 13.0.0.0) find no anchor.
  Whether these adds are meant for other models or reveal a remaining rule gap is UNKNOWN.
  They are not counted.
- Template sub-field merging (§4.1) does not change any figure: appending all template
  sub-fields and taking only missing kinds give the same ID sets for these models.

**Real-file cross-check** [observed by composition; real file reported by the owner]. A
vendor-saved FMB140 file at 10.0.0.0, spec 1, holds 8,514 IDs. The full rules, including the
custom-group rule, compose **exactly 8,514 IDs**. An independent comparison of the composed
set with the file's IDs reported an exact match, with nothing missing and nothing extra. A composition
without version-level operations and the extends rule missed 4,151 of them. They break down as
follows:
- **30000–33999** (4,000 IDs): the 4,000-entry iButton list, start 30000. At configuration version
  `8.9.0.0` (in the FMB140 chain), a version-level remove deletes the 500-entry list. A
  version-level add then inserts the 4,000-entry list at an anchor inside the 1-Wire page.
  FMB140 only has that page because of the extends rule: it is added for `FMB1:22` at
  `3.21.6.0`. The ID rule is `30000 + i`, `i` = 0…3999. (A parallel version line, `6.2.0.0`,
  has the same pair, but it is not in the FMB140 chain.)
- **50xxx, 45xxx, 11xxx–13xxx, 60xxx, 110, 111**: ordinary parameters and IO elements added by
  hardware blocks of the extended code (`FMB1:22`) in versions older than `8.9.0.0`, plus
  version-level adds. IO IDs follow §4.1. All of these are produced by the full rules.

The 30 IDs that the released v2 counted beyond the file (605850–605854, 605860–605864,
605880–605884, 605890–605894, 605900–605904, 605910–605914) are the custom-group case of §3.9.
They are not feature-gated. The owner's device needs no feature-mask assumption to match.

---

## 4. Derived parameter IDs [observed]

Several kinds of definition state only a base number; the actual parameter IDs follow
fixed rules. The common form is:

```
id = <base> + <index> × <stride> + <sub_offset>
```

### 4.1 IO elements

An IO element has an `io_id` (base) and an optional `sms_event_id`. Its sub-fields:

| Sub-field | `<sub_offset>` |
|---|---:|
| Priority | 0 |
| Operand (event trigger) | 1, or an explicit offset given in the definition |
| High level | 2 |
| Low level | 3 |
| Event only | 4, or an explicit offset given in the definition |
| Averaging constant | 5 |
| Send-period / time interval | 6 |
| SMS event text | **= `sms_event_id`** (not offset-based) |
| SMS event phone index | **= `sms_event_id` − 1000** |

The high-level field's lower bound is the low-level field's current value and vice versa
(`min_from_id` / `max_from_id`) [observed].

CAN-type IO elements (user-defined CAN rows) use a different sub-field set: CAN type 0,
data mask 1, CAN ID 2, CAN source 3; their operand uses an explicit offset (e.g. 4).

Worked example (ignition, FMB family): `io_id` = 50000, `sms_event_id` = 8000 →

| ID | Field | Default |
|---:|---|---|
| 50000 | Priority | 1 |
| 50001 | Operand | 5 |
| 50002 | High level | 0 (max 1) |
| 50003 | Low level | 0 (min 0) |
| 50004 | Event only | 0 |
| 50005 | Averaging constant | 10 (0–65535) |
| 8000 | SMS event text | `Ignition` (≤ 160 chars) |
| 7000 | SMS event phone | index into the list "SMS event numbers" (§4.3) |

The SMS phone field holds the **index** of an entry in the SMS-event-numbers list, not a
phone number (value type unsigned 8-bit; the list itself is IDs 6000–6009 in the FMB
family) [observed data; index base 0 vs 1 UNKNOWN].

IO templates: several IO elements share a template (a reusable set of sub-field
definitions). Expanding a template gives the same sub-fields with the element's own
`io_id` [observed]. When a template is expanded into an element [observed]:
- each template attribute the element does **not** already carry is copied to it. The
  element's own attributes always win. One copied attribute can be the custom-group name, which
  decides membership (§3.9);
- the template's sub-fields are appended to the element's own. Duplicate IDs count once. In
  the 1.8.21 data this gives the same ID set as taking only sub-field kinds the element lacks
  [observed by composition]. Template entries can carry feature conditions [observed]. Remove
operations also apply to templates (§3.3). Which IO elements yield parameters is defined in §3.9.

### 4.2 SMS events
Covered by §4.1: text at `sms_event_id`, phone index at `sms_event_id − 1000`.

### 4.3 Lists

Item *i* (0-based) of a list is `start_id + i`, for `i` in `0 … item_count − 1`.
Examples from the FMB family [observed]:

| List | IDs | Value |
|---|---|---|
| Authorised phone numbers | 4000–4049 (or 4000–4199 on models with 200 entries) | string ≤ 16, phone rule |
| Operator list | 5000–5049 (or 5000–5099) | integer ≤ 6 digits, operator rule |
| Operator list, SIM2 | 9500–9549 (or 9500–9599) | as above |
| SMS event numbers | 6000–6009 | string ≤ 16, phone rule |
| Bluetooth authorised MAC list | 830–834 | 12 hex characters |

The same list name can appear with different lengths for different models/versions; the
selected definition set decides.

### 4.4 Geofence zones

A zone block has `start_id`, `stride` and `zone_count`. Zone *k* (0-based, inferred) uses
`start_id + k × stride + <sub_offset>`:

| `<sub_offset>` | Field | Status |
|---:|---|---|
| 0 | Priority | observed |
| 1 | Generate event (on entry/exit/both) | observed |
| 2 | Eventual records | observed |
| 3 | Frame border | observed |
| 4 | Shape (circle / rectangle) | observed |
| 5 | Radius | observed |
| 6 | Longitude X1 | observed |
| 7 | Latitude Y1 | observed |
| 8 | Longitude X2 | observed |
| 9 | Latitude Y2 | observed |
| 10 | Overspeeding enable | observed |
| 11 | Max speed | observed |
| 12–16 | output control / output event / output on / output off / repeat | inferred, less certain |

Worked example (FMB family): `start_id` = 20100, `stride` = 20, `zone_count` = 50 →
zone 0 priority = 20100, zone 0 radius = 20105, zone 1 priority = 20120, zone 49 max
speed = 20100 + 49 × 20 + 11 = 21091.

Polygon zones: some models add a polygon block (`start_id`, `stride`, `point_count` = 8,
coordinate step 0.000001, latitude −90…90, longitude −180…180; example `start_id` 60310).
The ID layout of polygon points is UNKNOWN.

### 4.5 Other explicit-offset blocks
A "delay" block (start ID + stride per index) occurs in one spec patch; its layout follows
the common formula but is otherwise UNKNOWN.

---

## 5. Values: encoding and validation

### 5.1 Text encoding (device and .cfg) [observed unless marked]

| Type | Text form |
|---|---|
| Integer types, enumerations | decimal, `-` for negatives, no separators |
| Float / double | decimal point `.`, no thousands separators, no exponent, at most 15 fractional digits, trailing zeros dropped (e.g. `54.6872`, `25`, `0.5`) — observed for .cfg output; inferred to match what is sent with set-param |
| String | raw characters; the file writer emits ASCII (05-cfg-files) |
| Hex-entered fields (e.g. CAN ID) | entered as hex digits; whether the stored value is the hex text or its decimal equivalent is UNKNOWN — copy what the device returned in a read |

A value containing `;` cannot be represented in set-param or .cfg (both use `;` as a
separator) [inferred]. The vendor tool performs no escaping of item values [observed for
.cfg].

### 5.2 Range checks [observed]
- Numeric: `min ≤ value ≤ max` (inclusive), with linked bounds (§1.1) using the other
  parameter's current value.
- Enumerations: value must be one of the item values.
- Strings: `text_min_length ≤ length ≤ text_max_length`.

### 5.3 Validation rules [observed names; exact checks inferred unless marked]

A parameter's validation is an ordered list of rules; **all must pass**, the first failure
is reported [observed]. Fixed-name rules are matched case-insensitively and ignoring
surrounding blanks [observed]. Rule kinds:

| Rule kind | Check |
|---|---|
| Hex | hexadecimal digits only |
| Length set (list of allowed lengths) | value length must equal one of the listed numbers [observed] |
| MAC address | 12 hex characters (Bluetooth address without separators) |
| PIN | exactly 4, exactly 8, or 4–8 digits; optional variants also accept empty |
| Phone number | optional leading `+`, then digits (length limited by the parameter) |
| Operator code | mobile operator code (MCC+MNC digits) |
| Digits | digits only |
| iButton ID | iButton ID (hex) |
| VIN | vehicle identification number: 17 characters [observed length] |
| Time / date | `hhmm`, `mmdd` or `yyyymmdd` digit layout |
| Fuel-sensor settings (RS-485 / 1-Wire) | structured fuel-sensor settings; format UNKNOWN |
| Mask (with pattern) | fixed pattern; details UNKNOWN |
| Regular expression | value must match the regular expression. Expressions seen: APN must not start with a space; domain name; up to 10 comma-separated numbers; `+` optional followed by ≤ 16 digits |

Quirk [observed]: the length-set, mask and regex rules are recognised only when the rule
text starts exactly at the beginning (no leading blank). Data such as "length (0 or 8);
␠length (8); hex" therefore effectively means "length 0 or 8, and hex": the second length
rule (preceded by a blank) is silently ignored and passes. To match the vendor tool,
ignore an unrecognised rule (treat as pass).

### 5.4 Display conversions [observed data; behaviour inferred]
- **Minutes as `hh:mm`**: the stored value is an integer number of minutes; the UI shows
  and accepts `hh:mm` (mask `00:00`).
- **Units** are labels only; the stored value is never scaled by the tool, except where a
  unit label itself states a factor (e.g. `L×10` means the device value is in tenths of a
  litre).

---

## 6. Sensor presets [observed]

The FMB family includes **presets** for third-party BLE sensors (e.g. temperature, fuel,
TPMS, beacons). A preset is a named table of `(row, column, value)` cells that fills a
sensor configuration table. Each preset has a numeric preset ID. Tables reference presets
by ID. The cell-to-parameter mapping is UNKNOWN (not analysed); presets are a convenience
and not needed to read or write a configuration.

---

## 7. Special parameters

- **Keyword (configuration password)** is parameter **10** in both families [observed].
  It is handled specially when saving files (05-cfg-files §4) and by the keyword flow
  (03-commands §6).
- **Spec ID, configuration version, hardware code** are not parameters; they come from
  device info (03-commands §2) and are stored in the .cfg header (05-cfg-files).

---

## 8. Example parameters (orientation only; not a catalogue)

All [observed] in the FMB-family definitions; some properties vary by model/version.

| ID | Label | Type / control | Default | Range / rule |
|---:|---|---|---|---|
| 102 | Sleep mode | enumeration | 2 or 3 (varies) | sleep mode options |
| 1000 | Open link timeout | integer | 300 | 30–259200 s; lower bound also linked to parameter 1003 |
| 2001 | APN | string | `internet` or empty (varies) | ≤ 32 chars; must not start with a blank |
| 11000 | Scenario settings | enumeration | 0 | |
| 50000–50005 | Ignition IO element | §4.1 | | |
| 20100… | Geofence zones | §4.4 | | |

Bluetooth group (consistent with 01-transport v2 §2):

| ID | Label | Type | Default | Values / rule |
|---:|---|---|---|---|
| 800 | BT radio | enumeration | 2 | 0 = disabled, 1 = enabled (hidden), 2 = enabled (visible) |
| 801 | Local name | string | empty | ≤ 16 or ≤ 30 characters depending on model/version |
| 802 | Local PIN | string | `5555` | exactly 4 digits |
| 803 | Security mode | enumeration | 0 | 0 = `PIN only`, 1 = `PIN + MAC list`, 2 = `MAC list only`, 3 = `None` |
| 804 | External MAC | string | empty | 12 hex characters |
| 805 | External name | string | empty | ≤ 30 characters |
| 806 | External PIN | string | empty | empty or 4–8 digits (01-transport §2 says 0–8 digits; the definition rule is empty or 4–8, exact edge unconfirmed) |
| 807 | Connection mode | enumeration | 0 | options vary by model/version (e.g. auto-connect, barcode scanner, OBD dongle) |
| 830–834 | Authorised MAC list | list (§4.3) | empty | 12 hex characters each |

IDs 808–829 are not defined in this tool version's data [observed: no definitions found].

---

## UNKNOWN / to confirm on hardware

1. Exact "modify" matching (§3.3). (Extends: resolved, §3.2.)
2. Multi-field bit-mask enumeration notation (§1.2).
3. Polygon zone ID layout; geozone sub-offsets 12–16 (§4.4).
4. Storage form of hex-entered integer fields (CAN ID) (§5.1).
5. SMS phone index base (0 or 1) (§4.1).
6. Whether hidden parameters are excluded from writes (§1.5).
7. Feature-mask assumption when opening a .cfg offline (§3).
8. Sensor preset cell-to-parameter mapping (§6).
9. Effect of the geofence output-list length adjustment (§3.3).
10. Add tags with no anchor (5 to 12 per model) (§3.10).
11. (Resolved, §3.9/§3.10: the 30-ID surplus was the custom-group rule, not the feature mask.)
12. Whether a feature-filtered custom IO table also removes its elements' parameters (§3.9;
    no 1.8.21 case seen).
