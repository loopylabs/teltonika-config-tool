// Browser-only stand-in for the Tauri backend, for screenshots and UI work without a device.
// Active only outside Tauri and with `?mock` (usb, or `?mock=bt` for a Bluetooth link) in the URL.
type Handler = (payload: unknown) => void;

const w = window as unknown as Record<string, unknown>;
const params = new URLSearchParams(location.search);

if (!w.isTauri && params.has("mock")) {
  const bluetooth = params.get("mock") === "bt" || params.get("mock") === "idle";
  const idle = params.get("mock") === "idle"; // nothing connected: shows the before-connect hints
  const callbacks = new Map<number, Handler>();
  const listeners = new Map<string, number[]>();
  let nextId = 1;
  const emit = (event: string, payload: unknown) =>
    (listeners.get(event) ?? []).forEach((id) => callbacks.get(id)?.({ event, id, payload }));

  const status = {
    state: "connected", port: bluetooth ? "/dev/cu.FMB920_1234567" : "/dev/cu.usbmodem1101", kind: bluetooth ? "bluetooth" : "usb",
    info: { firmware: "03.29.00.Rev.18", configVersion: "03.29.00", hardwareCode: "FMB1:6", imei: "356307042441013", keywordSupported: true, specId: 1 },
    security: { lastOk: true, secured: false, authorized: true, locked: false, retriesLeft: 3 }, message: null,
  };

  const LOG = ["[GSM] registered, operator 63902", "[GPS] fix 3D sats=11 hdop=0.8", "[IO] ignition=1 ext_v=12.4", "[DATA] record #4821 saved", "[SRV] sent 1 record, ack ok", "[GSM] rssi=-71", "[BLE] scanning idle"];
  let logTimer: number | undefined;
  let pollTimer: number | undefined;
  let tick = 0;

  // Mirrors the backend's status events: a 0x0002 cycle first, then (feature bit 0x800 in 784)
  // 0x0102 with all groups for good. The real exchange is a count frame plus data frames.
  type F = { id: number; label: string; group: string; display: string; num: number | null; unit?: string; type?: number; io?: boolean; defined?: boolean };
  const field = (f: F) => ({
    id: f.id, label: f.label, group: f.group, display: f.display, rawHex: f.display, typeCode: f.type ?? 5,
    defined: f.defined ?? !f.io, unit: f.unit ?? null, num: f.num, io: f.io ?? false,
  });
  const n = (id: number, label: string, group: string, value: number, unit?: string): F => ({ id, label, group, display: String(value), num: value, unit });
  const startedAt = Math.floor(Date.now() / 1000) - 5 * 3600 - 1234;

  const basic = (): F[] => [
    { id: 781, label: "Binary protocol version", group: "Identity", display: "1.2", num: 258 },
    { id: 784, label: "Binary feature mask", group: "Identity", display: "0x80001805", num: null, type: 7 },
    n(773, "External voltage", "Power", 12300 + Math.round(Math.sin(tick / 3) * 250), "mV"),
    n(771, "Device time", "Time", Math.floor(Date.now() / 1000)),
    n(772, "Uptime", "Time", Math.floor(Date.now() / 1000) - startedAt, "s"),
  ];
  const extended = (): F[] => {
    // A short drive north-east of Vilnius; one lap every ~60 polls.
    const a = tick / 20;
    const lat = 54.6872 + Math.sin(a) * 0.004;
    const lon = 25.2797 + Math.sin(a * 2) * 0.006;
    return [
      ...basic(),
      n(256, "Fix state", "Position", 3),
      n(257, "GNSS module state", "Position", 1),
      n(258, "Fix time", "Position", 42 + tick, "s"),
      { id: 265, label: "Latitude", group: "Position", display: lat.toFixed(7), num: lat, unit: "°", type: 9 },
      { id: 266, label: "Longitude", group: "Position", display: lon.toFixed(7), num: lon, unit: "°", type: 9 },
      n(774, "Battery voltage", "Power", 4080 - Math.round(tick / 4), "mV"),
      n(775, "Battery level", "Power", 92),
      n(776, "Battery charge state", "Power", 1),
      n(512, "Modem state", "GSM", 2),
      n(513, "SIM state", "GSM", 1),
      n(515, "Signal level", "GSM", 4),
      n(516, "Data-link state", "GSM", 1),
      n(544, "Data sent", "GSM", 1_482_000 + tick * 310, "B"),
      n(545, "Data received", "GSM", 96_400 + tick * 40, "B"),
      n(546, "SMS sent", "GSM", 3),
      n(547, "SMS received", "GSM", 5),
      { id: 790, label: "LV-CAN present", group: "CAN", display: "No", num: 0, type: 0 },
      // I/O elements (IDs outside 256-826): value only, names come from the loaded definitions.
      { id: 66, label: "I/O #66", group: "Other", display: String(12300 + Math.round(Math.sin(tick / 3) * 250)), num: 12300, io: true, type: 3 },
      { id: 239, label: "I/O #239", group: "Other", display: "1", num: 1, io: true, type: 0 },
      { id: 21, label: "I/O #21", group: "Other", display: "4", num: 4, io: true, type: 0 },
      { id: 16, label: "I/O #16", group: "Other", display: String(48210 + tick), num: 48210 + tick, io: true, type: 5 },
    ];
  };

  const poll = () => {
    tick++;
    const rtt = (bluetooth ? 380 : 40) + Math.round(Math.sin(tick / 2) * (bluetooth ? 120 : 12));
    const wide = tick > 1;
    emit("device-status", {
      kind: status.kind, atMs: Date.now(), rttMs: rtt, failures: 0, ok: true, extended: wide,
      fields: (wide ? extended() : basic()).map(field),
    });
  };

  const commands: Record<string, (args: Record<string, unknown>) => unknown> = {
    list_ports: () => idle ? [
      { name: "/dev/cu.FMB920_1234567", kind: "bluetooth", likelyTracker: false, vid: null, pid: null, manufacturer: null, product: null, serialNumber: null },
      { name: "/dev/cu.usbmodem1101", kind: "usb", likelyTracker: true, vid: 4292, pid: 60000, manufacturer: "Teltonika", product: "FMB1xx", serialNumber: "123" },
    ] : [{ name: status.port, kind: status.kind, likelyTracker: true, vid: 4292, pid: 60000, manufacturer: "Teltonika", product: "FMB920", serialNumber: null }],
    connection_status: () => (idle ? { state: "disconnected", port: null, kind: null, info: null, security: null, message: null } : status),
    connect: () => ({ status, locked: false }),
    disconnect: () => null,
    definitions_summary: () => [],
    default_poll_interval: () => 5000,
    start_ble_scan: () => null,
    // Placeholder pictures only: the mock never carries vendor artwork.
    device_image: () => ({ source: "none", data_url: null }),
    device_model: (a) => ({ name: a.hwCode === "FMB1:6" ? "FMB140" : null, known: a.hwCode === "FMB1:6" }),
    model_hint: (a) => (String(a.fmType).toUpperCase() === "FMB920" ? { model: "FMB920", hw_code: "FMB9:21" } : null),
    start_status_polling: (a) => {
      clearInterval(pollTimer);
      poll();
      pollTimer = window.setInterval(poll, Number(a.intervalMs));
    },
    stop_status_polling: () => clearInterval(pollTimer),
    start_log: () => {
      clearInterval(logTimer);
      logTimer = window.setInterval(() => emit("device-log", [{ atMs: Date.now(), text: LOG[Math.floor(Math.random() * LOG.length)] }]), 450);
    },
    stop_log: () => clearInterval(logTimer),
    save_text: () => null,
  };

  w.__TAURI_INTERNALS__ = {
    transformCallback: (cb: Handler) => {
      const id = nextId++;
      callbacks.set(id, cb);
      return id;
    },
    unregisterCallback: (id: number) => callbacks.delete(id),
    invoke: async (cmd: string, args: Record<string, unknown> = {}) => {
      if (cmd === "plugin:event|listen") {
        const event = String(args.event);
        listeners.set(event, [...(listeners.get(event) ?? []), Number(args.handler)]);
        return Number(args.handler);
      }
      if (cmd.startsWith("plugin:")) return null;
      return commands[cmd]?.(args) ?? null;
    },
    convertFileSrc: (p: string) => p,
  };
  w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
}
