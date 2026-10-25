import assert from "node:assert/strict";
import { test } from "node:test";
import { bluetoothHint, categoryOf, modelFromBluetoothName } from "../src/ui/device-picture";

test("model hint from the Bluetooth name pattern", () => {
  assert.equal(modelFromBluetoothName("FMB920_1234567"), "FMB920");
  assert.equal(modelFromBluetoothName("FMC1300_12345678"), "FMC1300");
  assert.equal(modelFromBluetoothName("FMB920-SPP"), null);
  assert.equal(modelFromBluetoothName("FMB920_12345"), null);
  assert.equal(modelFromBluetoothName("fmb920_1234567"), null);
  assert.equal(modelFromBluetoothName("FMB9_1234567"), null);
  assert.equal(modelFromBluetoothName(null), null);
});

test("Bluetooth ports take the name from the product or the port path", () => {
  assert.equal(bluetoothHint({ name: "/dev/cu.FMB920_1234567", product: null }), "FMB920");
  assert.equal(bluetoothHint({ name: "COM7", product: "FMB920_1234567" }), "FMB920");
  assert.equal(bluetoothHint({ name: "/dev/cu.Bluetooth-Incoming-Port", product: null }), null);
  assert.equal(bluetoothHint({ name: "/dev/rfcomm0", product: null }), null);
});

test("categories only pick one of our icons", () => {
  assert.equal(categoryOf("FMB003"), "obd");
  assert.equal(categoryOf("FMB920"), "compact");
  assert.equal(categoryOf("FMB140"), "wired");
  assert.equal(categoryOf("FMB640"), "heavy");
  assert.equal(categoryOf("FMB125 no BAT DualSIM"), "wired");
  assert.equal(categoryOf(null), "unknown");
  assert.equal(categoryOf("Teltonika"), "unknown");
});
