import test from "node:test";
import assert from "node:assert/strict";
import { builtinPages, compositionKey, defsDiffer, familyOf, hwFromHeader, lookupDef, controlFor, formatDefault, formatDisplay, formatRange, hhmmToMinutes, maskEncode, maskKind, maskSelected, matchesSearch, minutesToHhmm, parseInput, trimFloat, unitHint, withUnit } from "../src/values.ts";
import { validateValue } from "../src/validate.ts";

const base: any = { id: 1, label: "x", value_type: "", control: "number", default: null, min: null, max: null, min_id: null, max_id: null, units: null, text_min: null, text_max: null, validate: [], options: [], bitmask: false, mask: null, converter: null, list: null, slot: null };
const d = (o: object): any => ({ ...base, ...o });
const opts = (...v: [string, string][]) => v.map(([value, label]) => ({ value, label }));

test("enum label <-> raw", () => {
  const e = d({ control: "enum", options: opts(["0", "Off"], ["1", "Low"], ["2", "High"]) });
  assert.equal(controlFor(e), "radio");
  assert.equal(formatDisplay(e, "2"), "High");
  assert.equal(e.options.find((o: any) => o.label === "Low").value, "1");
  assert.equal(formatDisplay(e, "9"), "9");
  const many = d({ control: "enum", options: opts(["0", "a"], ["1", "b"], ["2", "c"], ["3", "d"], ["4", "e"]) });
  assert.equal(controlFor(many), "select");
});
test("toggle", () => {
  const t = d({ control: "enum", options: opts(["0", "Disabled"], ["1", "Enabled"]) });
  assert.equal(controlFor(t), "toggle");
  assert.equal(formatDisplay(t, "1"), "Enabled");
});
test("hh:mm <-> minutes", () => {
  assert.equal(minutesToHhmm("1440"), "24:00");
  assert.equal(minutesToHhmm("65"), "01:05");
  assert.equal(hhmmToMinutes("24:00"), "1440");
  assert.equal(hhmmToMinutes("1:05"), "65");
  assert.equal(hhmmToMinutes("12:75"), null);
  const t = d({ converter: "x" });
  assert.equal(controlFor(t), "time");
  assert.equal(parseInput(t, "02:30"), "150");
  assert.equal(formatDisplay(t, "150"), "02:30");
});
test("floats use '.' and no trailing zeros", () => {
  assert.equal(trimFloat("25.500"), "25.5");
  assert.equal(trimFloat("25.0"), "25");
  assert.equal(trimFloat("54.6872"), "54.6872");
  const f = d({ value_type: "float" });
  assert.equal(parseInput(f, "0.50"), "0.5");
  assert.equal(parseInput(f, "7"), "7");
});
test("units and hints", () => {
  assert.equal(withUnit("1440", "min"), "1440 min (1 d)");
  assert.equal(unitHint("1440", "min"), "1 d");
  assert.equal(withUnit("30000", "ms"), "30000 ms (30 s)");
  assert.equal(withUnit("13200", "mV"), "13200 mV (13.2 V)");
  assert.equal(withUnit("90", "s"), "90 s (1 min 30 s)");
  assert.equal(withUnit("2500", "m"), "2500 m (2.5 km)");
  assert.equal(withUnit("120", "km/h"), "120 km/h");
  assert.equal(withUnit("5", "s"), "5 s");
  assert.equal(withUnit("300", null), "300");
  assert.equal(unitHint("500", "ms"), null);
  assert.equal(unitHint("1500", "ms"), "1.5 s");
  assert.equal(formatRange(d({ min: "0", max: "65535", units: "s" })), "0–65535 s");
  assert.equal(formatRange(d({})), "");
  assert.equal(formatDefault(d({ default: "300", units: "s" })), "300 s (5 min)");
});
test("bit mask encoding", () => {
  const m = d({ control: "enum", bitmask: true, mask: "FFFFFFFE", options: opts(["1", "GPS"], ["2", "GSM"], ["4", "Bluetooth"]) });
  assert.equal(maskKind(m), "bits");
  assert.equal(controlFor(m), "mask");
  assert.deepEqual(maskSelected(m, "5"), ["1", "4"]);
  assert.equal(maskEncode(["1", "4"]), "5");
  assert.equal(maskEncode([]), "0");
  assert.equal(formatDisplay(m, "7"), "GPS, GSM, Bluetooth");
  assert.equal(formatDisplay(m, "0"), "None");
});
test("multi-field mask is read-only", () => {
  const m = d({ control: "enum", bitmask: true, mask: "32:4:4,20:12", options: opts(["1", "a"]) });
  assert.equal(maskKind(m), "multifield");
  assert.equal(controlFor(m), "mask-readonly");
});
test("secret, mac and builtin names", () => {
  assert.equal(controlFor(lookupDef(null, 'FMB1:6', 802)!), "secret");
  assert.equal(controlFor(lookupDef(null, 'FMB1:6', 830)!), "mac");
  assert.equal(controlFor(lookupDef(null, 'FMB1:6', 800)!), "radio");
  assert.equal(formatDisplay(lookupDef(null, 'FMB1:6', 800)!, "2"), "Enabled (visible)");
  assert.equal(lookupDef(null, 'FMB1:6', 832)!.slot, 3);
  assert.equal(lookupDef(null, 'FMB1:6', 9999), undefined);
});
test("search", () => {
  const f = { id: 802, name: "Local PIN", group: "Bluetooth", shown: "••••", raw: "5555" };
  for (const q of ["pin", "bluetooth", "802", "#802", "5555"]) assert.ok(matchesSearch(q, f), q);
  assert.ok(!matchesSearch("gsm", f));
});
test("validation", () => {
  const pin = lookupDef(null, 'FMB1:6', 802)!;
  assert.equal(validateValue(pin, "5555", () => undefined), null);
  assert.ok(validateValue(pin, "555", () => undefined));
  assert.ok(validateValue(lookupDef(null, 'FMB1:6', 830)!, "ZZ", () => undefined));
  assert.equal(validateValue(lookupDef(null, 'FMB1:6', 830)!, "", () => undefined), null);
  assert.ok(validateValue(lookupDef(null, 'FMB1:6', 1000)!, "10", () => undefined));
  assert.equal(validateValue(lookupDef(null, 'FMB1:6', 800)!, "2", () => undefined), null);
  assert.ok(validateValue(lookupDef(null, 'FMB1:6', 800)!, "7", () => undefined));
  const lo = d({ control: "number", value_type: "int32", min_id: 5 });
  assert.ok(validateValue(lo, "1", (id) => (id === 5 ? "10" : undefined)));
});

test("per-document definitions and family-keyed built-ins", () => {
  const A = { params: { "100": d({ id: 100, label: "Mode", control: "enum", options: opts(["0", "Eco"], ["1", "Fast"]) }) } };
  const B = { params: { "100": d({ id: 100, label: "Interval", control: "number", value_type: "uint16", min: "5", max: "60", units: "s" }) } };
  assert.equal(formatDisplay(lookupDef(A, "X:1", 100)!, "1"), "Fast");
  assert.equal(formatDisplay(lookupDef(B, "X:2", 100)!, "30"), "30 s");
  assert.ok(validateValue(lookupDef(B, "X:2", 100)!, "99", () => undefined));
  assert.ok(validateValue(lookupDef(A, "X:1", 100)!, "99", () => undefined));
  assert.ok(defsDiffer(lookupDef(A, "", 100), lookupDef(B, "", 100)));
  assert.ok(!defsDiffer(lookupDef(A, "", 100), lookupDef(A, "", 100)));
  assert.equal(lookupDef(A, "", 5), undefined);
  assert.equal(familyOf("FMB640:1"), "FMB640");
  assert.equal(lookupDef(null, "FMB640:1", 1000), undefined);
  assert.ok(lookupDef(null, "FMB640:1", 802));
  assert.ok(builtinPages(null).every((p) => p.title !== "Other known parameters"));
  assert.notEqual(compositionKey({ build: "a", hwCode: "FMB1:6", cfgVersion: "10.0.0.0", specId: 1, features: "1" }), compositionKey({ build: "a", hwCode: "FMB1:6", cfgVersion: "13.0.0.0", specId: 1, features: "1" }));
  assert.equal(hwFromHeader("FMB1_6"), "FMB1:6");
});
