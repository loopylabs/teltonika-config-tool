import test from "node:test";
import assert from "node:assert/strict";
import { buildReviewRows, canWrite, entriesToSend, writeLabel, type ReviewView } from "../src/ui/review.ts";

const base: any = { id: 1, label: "x", value_type: "", control: "number", default: null, min: null, max: null, min_id: null, max_id: null, units: null, text_min: null, text_max: null, validate: [], options: [], bitmask: false, mask: null, converter: null, list: null, slot: null };
const defs: Record<number, any> = {
  100: { ...base, id: 100, control: "number", units: "min", value_type: "int" },
  101: { ...base, id: 101, control: "enum", options: [{ value: "0", label: "Disabled" }, { value: "1", label: "Enabled" }] },
};
const src = (entries: [string, string][], original: string[]) => ({
  entries: entries.map(([key, value]) => ({ key, value })),
  original,
  defOf: (id: number) => defs[id],
  nameOf: (id: number) => (defs[id] ? `Setting ${id}` : `Parameter ${id}`),
  groupOf: () => "System › Sleep",
});

test("only changed parameters become rows, with human labels and units", () => {
  const rows = buildReviewRows(src([["Title", "x"], ["100", "1440"], ["101", "1"], ["102", "5"]], ["y", "10", "0", "5"]));
  assert.equal(rows.length, 2);
  assert.deepEqual(rows[0], { id: 100, name: "Setting 100", group: "System › Sleep", before: "10 min", after: "1440 min (1 d)", secret: false });
  assert.equal(rows[1].before, "Disabled");
  assert.equal(rows[1].after, "Enabled");
});

test("unknown parameters show raw values; empty shows (empty)", () => {
  const rows = buildReviewRows(src([["200", ""]], ["7"]));
  assert.equal(rows[0].before, "7");
  assert.equal(rows[0].after, "(empty)");
});

test("the keyword is masked", () => {
  const rows = buildReviewRows(src([["10", "hunter2"]], ["old"]));
  assert.equal(rows[0].secret, true);
  assert.ok(!rows[0].after.includes("hunter2") && !rows[0].before.includes("old"));
});

test("only ticked rows are sent", () => {
  const e = src([["100", "1"], ["101", "1"], ["Title", "t"]], ["0", "0", "t"]).entries;
  assert.deepEqual(entriesToSend(e, new Set([101])).map((x) => x.key), ["101"]);
});

test("write gating: block, confirm, selection count", () => {
  const v: ReviewView = { rows: [], selected: new Set([1, 2]), warnings: [], confirmed: false, phase: "review" };
  assert.equal(canWrite(v), true);
  assert.equal(writeLabel(v), "Write 2 changes");
  assert.equal(writeLabel({ ...v, selected: new Set([1]) }), "Write 1 change");
  assert.equal(canWrite({ ...v, selected: new Set() }), false);
  assert.equal(canWrite({ ...v, warnings: [{ level: "block", text: "x" }], confirmed: true }), false);
  assert.equal(canWrite({ ...v, warnings: [{ level: "confirm", text: "x" }] }), false);
  assert.equal(canWrite({ ...v, warnings: [{ level: "confirm", text: "x" }], confirmed: true }), true);
  assert.equal(canWrite({ ...v, warnings: [{ level: "note", text: "x" }] }), true);
  assert.equal(canWrite({ ...v, phase: "writing" }), false);
});
