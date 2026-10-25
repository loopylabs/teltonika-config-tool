import assert from "node:assert/strict";
import { test } from "node:test";
import { agoText, filterLines, linkHealth, pushLines, sparkline, stamp } from "../src/ui/live.ts";

const line = (text: string, atMs = 0) => ({ atMs, text });

test("buffer keeps the newest lines", () => {
  const out = pushLines([line("a"), line("b")], [line("c"), line("d")], 3);
  assert.deepEqual(out.map((l) => l.text), ["b", "c", "d"]);
});

test("filter is case-insensitive and empty shows all", () => {
  const ls = [line("GSM ok"), line("gps fix")];
  assert.equal(filterLines(ls, "gsm").length, 1);
  assert.equal(filterLines(ls, "  ").length, 2);
});

test("stamp has millisecond precision", () => {
  assert.match(stamp(Date.now()), /^\d\d:\d\d:\d\d\.\d{3}$/);
});

test("ago text", () => {
  assert.equal(agoText(10_000, null), "not yet");
  assert.equal(agoText(10_000, 9_500), "just now");
  assert.equal(agoText(10_000, 4_000), "6 s ago");
});

test("link health warns after repeated timeouts", () => {
  const ev = { kind: "bluetooth", atMs: 0, rttMs: 420, failures: 0, ok: true, extended: false, fields: [] } as const;
  assert.equal(linkHealth(ev, false).level, "ok");
  assert.match(linkHealth(ev, false).text, /420 ms/);
  assert.equal(linkHealth({ ...ev, failures: 3, ok: false }, false).level, "warn");
  assert.equal(linkHealth(ev, true).text, "Link lost");
});

test("sparkline needs two points", () => {
  assert.ok(!sparkline([1]).includes("polyline"));
  assert.ok(sparkline([1, 2, 3]).includes("polyline"));
});
