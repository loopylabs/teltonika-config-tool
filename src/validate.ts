// Value validation (04 §5.2, §5.3). Pure functions; unrecognised rules pass (04 §5.3 quirk).
import type { ParamDef } from "./api";

const INT_RANGE: Record<string, [bigint, bigint]> = {
  byte: [0n, 255n],
  uint8: [0n, 255n],
  uint16: [0n, 65535n],
  uint32: [0n, 4294967295n],
  int32: [-2147483648n, 2147483647n],
  int64: [-(2n ** 63n), 2n ** 63n - 1n],
};

function checkRule(raw: string, v: string): string | null {
  const t = raw.trim().toLowerCase();
  if (t.startsWith("length")) {
    // 04 §5.3 quirk: only a rule starting exactly at the beginning is recognised.
    if (raw !== raw.trimStart()) return null;
    const lens = t.match(/\d+/g)?.map(Number) ?? [];
    return lens.length && !lens.includes(v.length) ? `Length must be ${lens.join(" or ")} characters` : null;
  }
  if (t.startsWith("hex") || t.startsWith("ibutton")) return /^[0-9a-f]*$/i.test(v) ? null : "Hexadecimal digits only";
  if (t.startsWith("mac")) return v === "" || /^[0-9a-f]{12}$/i.test(v) ? null : "Must be 12 hexadecimal characters, or empty";
  if (t.startsWith("pin")) {
    const empty = v === "";
    const allowEmpty = t.includes("empty") || t === "pin";
    if (empty) return allowEmpty ? null : "PIN required";
    const [lo, hi] = t.includes("4") && !t.includes("8") ? [4, 4] : t.includes("8") && !t.includes("4") ? [8, 8] : [4, 8];
    return /^\d+$/.test(v) && v.length >= lo && v.length <= hi
      ? null
      : lo === hi
        ? `PIN must be exactly ${lo} digits`
        : `PIN must be ${lo} to ${hi} digits`;
  }
  if (t.startsWith("phone")) return /^\+?\d*$/.test(v) ? null : "Optional + then digits only";
  if (t.startsWith("digits") || t.startsWith("operator")) return /^\d*$/.test(v) ? null : "Digits only";
  if (t.startsWith("vin")) return v === "" || v.length === 17 ? null : "VIN must be 17 characters";
  return null;
}

/** Error text for `value`, or null when valid. `lookup` reads another parameter's current value. */
export function validateValue(def: ParamDef, value: string, lookup: (id: number) => string | undefined): string | null {
  if (def.control === "enum") {
    if (def.bitmask) return /^\d+$/.test(value) ? null : "Whole number expected";
    if (!def.options.some((o) => o.value === value)) return "Not one of the allowed options";
    return null;
  }
  if (def.control === "number") {
    const float = def.value_type === "float" || def.value_type === "double";
    if (!(float ? /^-?\d+(\.\d+)?$/ : /^-?\d+$/).test(value)) return float ? "Number expected" : "Whole number expected";
    const range = INT_RANGE[def.value_type];
    if (range && (BigInt(value) < range[0] || BigInt(value) > range[1])) return `Out of range for ${def.value_type} (${range[0]} to ${range[1]})`;
    const n = Number(value);
    const min = def.min !== null ? Number(def.min) : null;
    const max = def.max !== null ? Number(def.max) : null;
    if (min !== null && !Number.isNaN(min) && n < min) return `Minimum is ${def.min}`;
    if (max !== null && !Number.isNaN(max) && n > max) return `Maximum is ${def.max}`;
    for (const [id, dir] of [[def.min_id, 1], [def.max_id, -1]] as const) {
      const other = id === null ? undefined : lookup(id);
      if (other !== undefined && /^-?\d+(\.\d+)?$/.test(other) && (n - Number(other)) * dir < 0) {
        return dir === 1 ? `Must be at least parameter ${id} (${other})` : `Must be at most parameter ${id} (${other})`;
      }
    }
    return null;
  }
  if (def.control === "text") {
    if (def.text_min !== null && value.length < def.text_min) return `At least ${def.text_min} characters`;
    if (def.text_max !== null && value.length > def.text_max) return `At most ${def.text_max} characters`;
    if (value.includes(";")) return "';' cannot be stored in a configuration";
    const rules = def.validate;
    for (const r of rules) {
      const e = checkRule(r, value);
      if (e) return e;
    }
  }
  return null;
}
