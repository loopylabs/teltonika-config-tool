import assert from "node:assert/strict";
import { test } from "node:test";
import type { StatusField } from "../src/api.ts";
import { History, byId, csvRows, csvText, formatBytes, formatDuration, formatUnix, formatVolts, ioRows, position, sparkSeries, tiles } from "../src/ui/telemetry.ts";

const f = (id: number, num: number | null, display = String(num), extra: Partial<StatusField> = {}): StatusField => ({
  id, label: `L${id}`, group: "Other", display, rawHex: "", typeCode: 3, defined: id >= 256 && id <= 826, unit: null, num, io: id < 256 || id > 826, ...extra,
});
const tile = (fields: StatusField[], label: string) => tiles(fields).find((t) => t.label === label)!;

test("formats", () => {
  assert.equal(formatVolts(12600), "12.60 V");
  assert.equal(formatDuration(3725), "1 h 2 min 5 s");
  assert.equal(formatDuration(90061), "1 d 1 h 1 min");
  assert.equal(formatUnix(1_700_000_000), "2023-11-14 22:13:20 UTC");
  assert.equal(formatBytes(1536), "1.50 KB");
});

test("external voltage: mV shown as V, zero means on battery", () => {
  assert.equal(tile([f(773, 12600)], "External voltage").value, "12.60 V");
  assert.equal(tile([f(773, 0)], "External voltage").value, "On battery");
  assert.equal(tile([], "External voltage").value, "—");
});

test("position needs both coordinates within range", () => {
  const ok = [f(265, 54.6872), f(266, 25.2797)];
  assert.deepEqual(position(byId(ok)), { lat: 54.6872, lon: 25.2797 });
  assert.equal(position(byId([f(265, 54.6872), f(266, null, "no valid position")])), null);
  assert.equal(tile([f(265, 54.6872), f(266, null)], "Position").value, "No valid position");
  assert.match(tile(ok, "Position").value, /^54\.6872000, 25\.2797000$/);
});

test("start time is device time minus uptime; enums stay raw", () => {
  const fields = [f(771, 1_700_000_000), f(772, 3600), f(256, 3), f(512, 2), f(513, 1), f(515, 4), f(516, 1)];
  assert.equal(tile(fields, "Started").value, "2023-11-14 21:13:20 UTC");
  assert.equal(tile(fields, "Fix state").value, "3");
  assert.equal(tile(fields, "GSM states (raw)").value, "modem 2 · SIM 1 · signal 4 · link 1");
});

test("I/O rows use this document's names and units", () => {
  const items = [{ id: 66, name: "External voltage", unit: "mV", group: "Base I/O" }];
  const rows = ioRows([f(66, 12000), f(9999, 5), f(773, 1)], items);
  assert.deepEqual(rows.map((r) => [r.id, r.name, r.value, r.group, r.known]), [
    [66, "External voltage", "12000 mV", "Base I/O", true],
    [9999, "I/O element #9999", "5", "Other", false],
  ]);
  assert.equal(ioRows([f(66, 1)], undefined)[0].name, "I/O element #66");
});

test("history keeps ten minutes and the sparkline draws a line", () => {
  const h = new History();
  h.push("a", 0, 1);
  h.push("a", 5 * 60_000, 2);
  h.push("a", 11 * 60_000, 3);
  assert.deepEqual(h.get("a").map((p) => p[1]), [2, 3]);
  assert.ok(sparkSeries(h.get("a")).includes("polyline"));
  assert.ok(!sparkSeries([[0, 1]]).includes("polyline"));
});

test("CSV rows quote cells and name I/O elements", () => {
  const rows = csvRows(0, [f(66, 1, "12000"), f(3, null, 'a,"b"', { io: true })], [{ id: 66, name: "Ext, V", unit: "mV", group: null }]);
  assert.equal(rows[0], '1970-01-01T00:00:00.000Z,66,"Ext, V",12000,mV,io');
  assert.equal(rows[1], '1970-01-01T00:00:00.000Z,3,L3,"a,""b""",,io');
  assert.ok(csvText(rows).startsWith("time_utc,id,label,value,unit,kind\r\n"));
});
