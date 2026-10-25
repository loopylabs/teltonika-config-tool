// Human-readable value formatting and parsing (04 §5). Pure functions, no DOM.
// Raw values keep the exact 04/05 text encoding; display text is derived from them.
import type { ParamDef } from "./api";

export type Control = "toggle" | "select" | "radio" | "mask" | "mask-readonly" | "number" | "time" | "secret" | "mac" | "text" | "raw";

const SECRET_IDS = new Set([10, 802, 806]);

/** Drops trailing zeros of a decimal (04 §5.1): "25.500" -> "25.5", "25.0" -> "25". */
export function trimFloat(s: string): string {
  const t = s.trim();
  if (!/^-?\d+\.\d+$/.test(t)) return t;
  return t.replace(/0+$/, "").replace(/\.$/, "");
}

/** Which bit-mask encoding a definition uses: plain OR of item values, or the unconfirmed multi-field form. */
export function maskKind(def: ParamDef): "bits" | "multifield" | null {
  if (def.control !== "enum" || !def.bitmask) return null;
  // TODO(spec): open question 56 — the multi-field mask notation (e.g. "32:4:4,20:12") is not understood.
  return def.mask && def.mask.includes(":") ? "multifield" : "bits";
}

/** One control type per definition type; the registry in controls.ts is keyed by this. */
export function controlFor(def: ParamDef): Control {
  if (def.control === "enum") {
    const mk = maskKind(def);
    if (mk === "bits") return "mask";
    if (mk === "multifield") return "mask-readonly";
    const vals = def.options.map((o) => o.value).sort().join(",");
    if (vals === "0,1") return "toggle";
    return def.options.length >= 2 && def.options.length <= 4 ? "radio" : "select";
  }
  if (def.control === "number") return def.converter ? "time" : "number";
  if (def.control === "text") {
    if (SECRET_IDS.has(def.id) || /\b(pin|password|keyword)\b/i.test(def.label)) return "secret";
    if (def.validate.some((r) => r.trim().toLowerCase().startsWith("mac"))) return "mac";
    return "text";
  }
  return "raw";
}

// ---- time (minutes <-> hh:mm)

export function minutesToHhmm(raw: string): string | null {
  if (!/^\d+$/.test(raw)) return null;
  const m = Number(raw);
  return `${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;
}

export function hhmmToMinutes(text: string): string | null {
  const m = /^(\d{1,3}):([0-5]\d)$/.exec(text.trim());
  return m ? String(Number(m[1]) * 60 + Number(m[2])) : null;
}

// ---- bit masks (04 §1.2: value is the OR of ticked item values)

export function maskSelected(def: ParamDef, raw: string): string[] {
  if (!/^\d+$/.test(raw)) return [];
  const v = BigInt(raw);
  return def.options.filter((o) => /^\d+$/.test(o.value) && BigInt(o.value) !== 0n && (v & BigInt(o.value)) === BigInt(o.value)).map((o) => o.value);
}

export function maskEncode(values: string[]): string {
  return values.reduce((acc, v) => acc | BigInt(v), 0n).toString();
}

// ---- units

const plural = (n: number, unit: string) => `${n} ${unit}`;

function duration(totalSeconds: number): string {
  const parts: string[] = [];
  let s = totalSeconds;
  for (const [size, unit] of [[86400, "d"], [3600, "h"], [60, "min"], [1, "s"]] as const) {
    const n = Math.floor(s / size);
    if (n) parts.push(plural(n, unit));
    s -= n * size;
  }
  return parts.join(" ");
}

/** Friendly equivalent of a large value, or null. Only conversions that are certain. Never changes the stored value. */
export function unitHint(raw: string, unit: string | null): string | null {
  if (!unit || !/^-?\d+(\.\d+)?$/.test(raw)) return null;
  const n = Number(raw);
  const a = Math.abs(n);
  switch (unit.trim()) {
    case "ms":
      return a >= 1000 ? (n % 1000 === 0 ? duration(n / 1000) : `${trimFloat((n / 1000).toFixed(3))} s`) : null;
    case "s":
      return a >= 60 && Number.isInteger(n) ? duration(n) : null;
    case "min":
      return a >= 60 && Number.isInteger(n) ? duration(n * 60) : null;
    case "h":
      return a >= 24 && Number.isInteger(n) ? duration(n * 3600) : null;
    case "mV":
      return a >= 1000 ? `${trimFloat((n / 1000).toFixed(3))} V` : null;
    case "m":
      return a >= 1000 ? `${trimFloat((n / 1000).toFixed(3))} km` : null;
    default:
      return null;
  }
}

/** "1440 min (24 h)": the value with its unit and, for large values, the friendly hint. */
export function withUnit(raw: string, unit: string | null): string {
  const hint = unitHint(raw, unit);
  const base = unit ? `${raw} ${unit}` : raw;
  return hint ? `${base} (${hint})` : base;
}

/** "0–65535 s"; empty when there are no bounds. */
export function formatRange(def: ParamDef): string {
  if (def.control === "text") {
    return def.text_max !== null ? `up to ${def.text_max} characters` : "";
  }
  if (def.min === null && def.max === null) return "";
  const u = def.units ? ` ${def.units}` : "";
  return `${def.min ?? "…"}–${def.max ?? "…"}${u}`;
}

// ---- display and parsing

/** Human text for a raw value: enum label, On/Off, hh:mm, value with unit. */
export function formatDisplay(def: ParamDef, raw: string): string {
  const c = controlFor(def);
  if (c === "toggle") {
    const item = def.options.find((o) => o.value === raw);
    return item && !/^(0|1)$/.test(item.label.trim()) ? item.label : raw === "1" ? "On" : raw === "0" ? "Off" : raw;
  }
  if (c === "select" || c === "radio") return def.options.find((o) => o.value === raw)?.label ?? raw;
  if (c === "mask" || c === "mask-readonly") {
    const labels = def.options.filter((o) => maskSelected(def, raw).includes(o.value)).map((o) => o.label);
    return labels.length ? labels.join(", ") : raw === "0" ? "None" : raw;
  }
  if (c === "time") return minutesToHhmm(raw) ?? raw;
  if (c === "number") return withUnit(def.value_type === "float" || def.value_type === "double" ? trimFloat(raw) : raw, def.units);
  if (c === "secret") return raw ? "•".repeat(Math.min(raw.length, 8)) : "";
  return raw;
}

/** Raw value to store for text typed in a control; null if it does not parse. */
export function parseInput(def: ParamDef, input: string): string | null {
  const c = controlFor(def);
  if (c === "time") return hhmmToMinutes(input);
  if (c === "number") {
    const t = input.trim();
    return def.value_type === "float" || def.value_type === "double" ? (/^-?\d+(\.\d+)?$/.test(t) ? trimFloat(t) : t) : t;
  }
  return input;
}

/** Display text of the default value. */
export function formatDefault(def: ParamDef): string {
  return def.default === null ? "" : formatDisplay(def, def.default);
}

// ---- names

export function unnamedLabel(id: number): string {
  return `Unnamed parameter #${id}`;
}

/** Search needle match over name, group, display text, raw value and ID. */
export function matchesSearch(needle: string, fields: { id: number; name: string; group: string; shown: string; raw: string }): boolean {
  const n = needle.trim().toLowerCase().replace(/^#/, "");
  if (!n) return true;
  return String(fields.id) === n || String(fields.id).startsWith(n) && /^\d+$/.test(n) && n.length >= 2
    || [fields.name, fields.group, fields.shown, fields.raw].some((s) => s.toLowerCase().includes(n));
}

// ---- built-in names for parameters our released specs describe (04 §8, 01 v2), own words

const base = { value_type: "", default: null, min: null, max: null, min_id: null, max_id: null, units: null, text_min: null, text_max: null, validate: [] as string[], options: [], bitmask: false, mask: null, converter: null, list: null, slot: null };
const en = (id: number, label: string, def: string, items: [string, string][]): ParamDef => ({
  ...base, id, label, control: "enum", default: def, value_type: "byte", options: items.map(([value, label]) => ({ value, label })),
});
const text = (id: number, label: string, max: number | null, def: string | null, validate: string[] = []): ParamDef => ({
  ...base, id, label, control: "text", value_type: "string", default: def, text_max: max, validate,
});
const num = (id: number, label: string, def: string | null, min: string | null, max: string | null, units: string | null): ParamDef => ({
  ...base, id, label, control: "number", value_type: "uint32", default: def, min, max, units,
});
const list = (name: string, start: number, count: number, max: number, validate: string[]): ParamDef[] =>
  Array.from({ length: count }, (_, i) => ({ ...text(start + i, name, max, "", validate), list: name, slot: i + 1 }));

type Family = "FMB" | "FMB640";
/** Pages of the built-in table; `families` = where our released specs say the entry holds. */
export const BUILTIN_PAGES: { title: string; families: Family[]; defs: ParamDef[] }[] = [
  {
    // 01-transport v2 §2 and 04 §8.
    title: "Bluetooth",
    families: ["FMB", "FMB640"],
    defs: [
      en(800, "Bluetooth radio", "2", [["0", "Disabled"], ["1", "Enabled (hidden)"], ["2", "Enabled (visible)"]]),
      text(801, "Local name", 30, ""),
      text(802, "Local PIN", 4, "5555", ["PIN4"]),
      en(803, "Security mode", "0", [["0", "PIN only"], ["1", "PIN + MAC list"], ["2", "MAC list only"], ["3", "None"]]),
      text(804, "External device MAC", 12, "", ["MAC"]),
      text(805, "External device name", 30, ""),
      text(806, "External device PIN", 8, "", ["PIN"]),
      { ...num(807, "Connection mode", "0", "0", "255", null), value_type: "byte" },
      ...list("Authorised MAC list", 830, 5, 12, ["MAC"]),
    ],
  },
  // 04 §7: the keyword is parameter 10 in both families.
  { title: "Security", families: ["FMB", "FMB640"], defs: [text(10, "Configuration keyword", null, null)] },
  {
    // 04 §4.3 and §8 describe these for the FMB family only.
    title: "Other known parameters",
    families: ["FMB"],
    defs: [
      { ...num(102, "Sleep mode", null, null, null, null), control: "other" },
      num(1000, "Open link timeout", "300", "30", "259200", "s"),
      text(2001, "APN", 32, null),
      ...list("Authorised phone numbers", 4000, 50, 16, ["Phone"]),
      ...list("SMS event numbers", 6000, 10, 16, ["Phone"]),
    ],
  },
];

const FMB640_PREFIXES = ["FMB640", "FMB641", "FMC640", "FMC650", "FMM640", "FMM650", "MCX3"];

/** 04 §2 family of a hardware code, judged by model prefix only (no definitions loaded). */
export function familyOf(hwCode: string | null): Family | null {
  if (!hwCode) return null;
  const prefix = hwCode.split(":")[0].toUpperCase();
  return FMB640_PREFIXES.some((p) => prefix.startsWith(p)) ? "FMB640" : "FMB";
}

/** Built-in pages that apply to the document's family. Unknown family: only entries valid for both. */
export function builtinPages(hwCode: string | null): { title: string; defs: ParamDef[] }[] {
  const fam = familyOf(hwCode);
  return BUILTIN_PAGES.filter((p) => (fam ? p.families.includes(fam) : p.families.length === 2));
}

/** The one place a definition is looked up: the document's own composition, else the family's built-in table. */
export function lookupDef(model: { params: Record<string, ParamDef> } | null, hwCode: string | null, id: number): ParamDef | undefined {
  if (model) return model.params[String(id)];
  for (const p of builtinPages(hwCode)) {
    const d = p.defs.find((x) => x.id === id);
    if (d) return d;
  }
  return undefined;
}

/** Key of a composition: everything that can change what an ID means. */
export function compositionKey(k: { build: string; hwCode: string; cfgVersion: string; specId: number; features: string }): string {
  return [k.build, k.hwCode.toUpperCase(), k.cfgVersion, k.specId, k.features].join("|");
}

/** True when the same ID means something different on two documents. */
export function defsDiffer(a: ParamDef | undefined, b: ParamDef | undefined): boolean {
  if (!a || !b) return false;
  const pick = (d: ParamDef) => JSON.stringify([d.label, d.value_type, d.control, d.min, d.max, d.units, d.options, d.text_max, d.mask]);
  return pick(a) !== pick(b);
}

/** `FMB1_6` (05 header form) back to `FMB1:6`. */
export function hwFromHeader(v: string): string {
  return v.replace(/^([A-Za-z]+\d*)_(\d+)$/, "$1:$2");
}
