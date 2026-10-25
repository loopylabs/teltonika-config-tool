import assert from "node:assert/strict";
import { test } from "node:test";
import { nameModelHint } from "../src/ui/bt-name.ts";
import { mockSnapshot, nearbyBtHtml, portFor, rssiBars, type NearbyBtView } from "../src/ui/nearby-bt.ts";

const view = (over: Partial<NearbyBtView> = {}): NearbyBtView => ({
  snap: mockSnapshot(true, 1000), ports: [], pairingFor: null, othersOpen: false, busy: false, pin: "5555", pair: null, nativeRfcomm: false, ...over,
});

test("model hint from the Bluetooth name", () => {
  assert.deepEqual(nameModelHint("FMB920_1234567"), { model: "FMB920", serial: "1234567", source: "from Bluetooth name" });
  assert.equal(nameModelHint("FMC1301_12345678")?.model, "FMC1301");
  for (const bad of ["MIBOX4 nown", "[TV] Samsung TV", "FMB920", "fmb920_1234567", "FMB920_12345", null]) assert.equal(nameModelHint(bad), null);
});

test("rssi bars", () => {
  assert.equal(rssiBars(null), null);
  assert.equal(rssiBars(-55), 4);
  assert.equal(rssiBars(-65), 3);
  assert.equal(rssiBars(-75), 2);
  assert.equal(rssiBars(-90), 1);
});

test("trackers listed first; non-trackers collapsed under Other devices", () => {
  const html = nearbyBtHtml(view());
  const [main, others] = html.split('class="nbt-others"');
  assert.match(main, /FMB920_1234567/);
  assert.match(main, /Tracker/);
  assert.doesNotMatch(main, /MIBOX4/);
  assert.match(others, /Other devices <span class="count-pill">2<\/span>/);
  assert.match(others, /MIBOX4 nown/);
  assert.match(others, /\[TV\] Samsung TV/);
  assert.doesNotMatch(html, /<details class="nbt-others" open/);
  assert.match(html, /Scanning…/);
});

test("pair row is pre-filled with 5555; paired tracker connects over its serial port or RFCOMM", () => {
  const snap = mockSnapshot(false, 1000);
  assert.match(nearbyBtHtml(view({ snap, pairingFor: snap.devices[0].address })), /value="5555"/);
  snap.devices[0].paired = true;
  const port = { name: "/dev/cu.FMB920_1234567", kind: "bluetooth" as const, likelyTracker: false, vid: null, pid: null, manufacturer: null, product: "FMB920_1234567", serialNumber: null };
  assert.equal(portFor(snap.devices[0], [port])?.name, port.name);
  assert.match(nearbyBtHtml(view({ snap, ports: [port] })), /data-port="\/dev\/cu.FMB920_1234567"/);
  assert.match(nearbyBtHtml(view({ snap, nativeRfcomm: true })), /data-port="bt:A4:C1:38:96:01:07"/);
});

test("unsupported OS shows the message", () => {
  const html = nearbyBtHtml(view({ snap: { supported: false, scanning: false, message: "Nearby Bluetooth discovery not supported on this OS yet — pair in Windows Settings", devices: [] } }));
  assert.match(html, /pair in Windows Settings/);
});
