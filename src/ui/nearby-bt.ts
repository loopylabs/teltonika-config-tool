// "Nearby Bluetooth" card (01-transport §2.2): Bluetooth Classic inquiry results plus OS-paired
// devices, like the "Nearby Devices" list in macOS Bluetooth settings. Self-contained: main.ts
// only renders `nearbyBtSection()` on the Devices screen and calls `initNearbyBt()` once.
import { invoke } from "@tauri-apps/api/core";
import type { PortInfo } from "../api";
import { nameModelHint } from "./bt-name";
import { pictureHtml } from "./device-picture";
import { icon } from "./icons";

export type NearbyBtDevice = {
  address: string;
  name: string | null;
  classOfDevice: number;
  rssi: number | null;
  paired: boolean;
  likelyTracker: boolean;
  modelHint: { model: string; serial: string; source: string } | null;
  lastSeen: number | null;
};
export type NearbyBtSnapshot = { supported: boolean; scanning: boolean; message: string | null; devices: NearbyBtDevice[] };

export type NearbyBtView = {
  snap: NearbyBtSnapshot;
  /** Bluetooth serial ports by normalized device name, for the Connect button. */
  ports: PortInfo[];
  pairingFor: string | null;
  othersOpen: boolean;
  busy: boolean;
  /** PIN typed for in-app pairing (tracker parameter 802, default 5555). */
  pin: string;
  pair: { state: "idle" | "pairing" | "paired" | "failed"; message: string | null } | null;
  /** macOS can open RFCOMM natively ("bt:<address>") when no serial port exists. */
  nativeRfcomm: boolean;
};

const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
const norm = (s: string) => s.toLowerCase().replace(/[^a-z0-9]/g, "");
export const DEFAULT_PIN_HINT = "Default PIN is 5555 unless changed";

/** RSSI in dBm to 1–4 bars; null when the stack gave no reading. */
export function rssiBars(rssi: number | null): number | null {
  if (rssi === null) return null;
  return rssi >= -60 ? 4 : rssi >= -70 ? 3 : rssi >= -80 ? 2 : 1;
}

/** The OS serial port for a paired device (macOS names it /dev/cu.<name>…), if any. */
export function portFor(d: NearbyBtDevice, ports: PortInfo[]): PortInfo | null {
  if (!d.name) return null;
  const n = norm(d.name);
  return ports.find((p) => p.kind === "bluetooth" && n.length > 0 && norm(p.name).includes(n)) ?? null;
}

/** The Bluetooth address of a paired device whose OS serial port is `port`, if the snapshot knows it. */
export function addressForPort(port: PortInfo, devices: NearbyBtDevice[]): string | null {
  const found = devices.filter((d) => d.paired && portFor(d, [port]));
  return found.length === 1 ? found[0].address : null;
}

function rowHtml(d: NearbyBtDevice, v: NearbyBtView): string {
  const name = d.name ? esc(d.name) : `<span class="muted">Unnamed</span>`;
  const bars = rssiBars(d.rssi);
  const signal = bars === null
    ? `<span class="muted">${d.lastSeen === null ? "Not in range" : "In range"}</span>`
    : `<span class="bars" title="${d.rssi} dBm" aria-label="Signal ${bars} of 4">${[1, 2, 3, 4].map((n) => `<i class="${n <= bars ? "on" : ""}"></i>`).join("")}</span>`;
  const hint = d.modelHint
    ? `<span>${esc(d.modelHint.model)}</span><span class="muted dsub" title="Our guess from the device's Bluetooth name; connect to confirm">from Bluetooth name</span>`
    : `<span class="muted">—</span>`;
  const port = d.paired ? portFor(d, v.ports) : null;
  const primary = d.likelyTracker ? "primary" : "";
  // Direct RFCOMM is the default, as in 01-transport §2.1: the reference tool never uses
  // the OS serial port. The OS port stays available as a secondary path.
  const direct = `<button class="${primary}" data-action="connect" data-port="bt:${esc(d.address)}" data-kind="bluetooth" title="Direct Bluetooth serial (RFCOMM) connection">Connect</button>`;
  const viaPort = port
    ? `<button data-action="connect" data-port="${esc(port.name)}" data-kind="bluetooth" title="Connect through ${esc(port.name)}">Via serial port</button>`
    : "";
  const action = d.paired && v.nativeRfcomm
    ? direct + viaPort
    : port
      ? `<button class="${primary}" data-action="connect" data-port="${esc(port.name)}" data-kind="bluetooth" title="${esc(port.name)}">Connect</button>`
      : d.paired
        ? `<button data-nbt="refresh" title="Refresh to pick up its serial port">Refresh</button>`
        : `<button class="${primary}" data-nbt="pair" data-addr="${esc(d.address)}">Pair…</button>`;
  const pic = d.modelHint ? pictureHtml({ hintModel: d.modelHint.model, model: d.modelHint.model }) : `<span class="dicon" aria-hidden="true">${icon("bluetooth")}</span>`;
  const pairing = v.pairingFor === d.address ? pairRowHtml(d, v) : "";
  return `
    <div class="nbt-row" role="row" data-nbt-addr="${esc(d.address)}">
      <div class="dname">${pic}<span><span class="trunc" title="${d.name ? esc(d.name) : ""}">${name}</span><span class="dsub nbt-tags">${d.likelyTracker ? `<span class="nbt-badge">Tracker</span>` : ""}<span class="muted">${d.paired ? "Paired" : "Not paired"}</span></span></span></div>
      <div class="mono trunc nbt-addr" title="${esc(d.address)}">${esc(d.address)}</div>
      <div class="nbt-sig">${signal}</div>
      <div class="dmodel">${hint}</div>
      <div class="dact">${action}</div>
    </div>${pairing}`;
}

function pairRowHtml(d: NearbyBtDevice, v: NearbyBtView): string {
  const p = v.pair;
  const working = p?.state === "pairing";
  const status = p?.message
    ? `<p class="${p.state === "failed" ? "error" : p.state === "paired" ? "nbt-ok" : "muted"}" role="status">${working ? `<span class="nbt-spin"></span> ` : ""}${esc(p.message)}</p>`
    : "";
  const label = d.name ? esc(d.name) : esc(d.address);
  return `<div class="nbt-pair" role="group" aria-label="Pair ${label}">
      <p>Pair <strong>${label}</strong> with this computer. ${DEFAULT_PIN_HINT} (tracker parameter 802).</p>
      <div class="nbt-pair-act">
        <label class="nbt-pin">PIN <input class="mono" data-nbt-pin inputmode="numeric" autocomplete="off" maxlength="16" value="${esc(v.pin)}" ${working ? "disabled" : ""}></label>
        <button class="primary" data-nbt="pair-go" data-addr="${esc(d.address)}" ${working ? "disabled" : ""}>${working ? "Pairing…" : "Pair"}</button>
        <button data-nbt="open-settings" title="Pair in the OS Bluetooth settings instead">Bluetooth Settings</button>
        <button data-nbt="pair-close">Close</button>
      </div>
      ${status}
    </div>`;
}

const HEAD = `<div class="nbt-row dhead" role="row"><div>Device</div><div>Address</div><div>Signal</div><div>Model</div><div></div></div>`;

/** The whole card. Pure: easy to test and to re-render in place. */
export function nearbyBtHtml(v: NearbyBtView): string {
  const { snap } = v;
  const trackers = snap.devices.filter((d) => d.likelyTracker);
  const others = snap.devices.filter((d) => !d.likelyTracker);
  const head = snap.supported
    ? `${snap.scanning ? `<span class="nbt-spin" role="status" aria-label="Scanning"></span><span class="muted nbt-scanning">Scanning…</span>` : ""}<button data-nbt="${snap.scanning ? "stop" : "scan"}" ${v.busy ? "disabled" : ""}>${snap.scanning ? "Stop" : "Scan"}</button>`
    : `<button data-nbt="open-settings">Bluetooth settings</button>`;
  const table = (list: NearbyBtDevice[]) => `<div class="dtable nbt-table" role="table">${HEAD}${list.map((d) => rowHtml(d, v)).join("")}</div>`;
  let body: string;
  if (!snap.supported) body = `<p class="muted dnone">${esc(snap.message ?? "Nearby Bluetooth discovery is not supported on this OS.")}</p>`;
  else {
    const err = snap.message ? `<p class="error scard-err">${esc(snap.message)}</p>` : "";
    const none = trackers.length
      ? ""
      : `<p class="muted dnone">${snap.scanning ? "Looking for trackers… Make sure the tracker's Bluetooth is on and visible." : "No trackers found yet. Click Scan to search for nearby Bluetooth devices."}</p>`;
    const otherBlock = others.length
      ? `<details class="nbt-others" ${v.othersOpen ? "open" : ""}><summary>Other devices <span class="count-pill">${others.length}</span></summary>${table(others)}</details>`
      : "";
    body = `${err}${trackers.length ? table(trackers) : none}${otherBlock}`;
  }
  return `<section class="scard" data-role="nearby-bt"><header class="scard-head"><span class="sicon">${icon("bluetooth", 18)}</span><div class="grow"><h3>Nearby Bluetooth${trackers.length ? ` <span class="count-pill">${trackers.length}</span>` : ""}</h3><p class="muted">Bluetooth Classic devices in range. Pair a tracker once, then connect to it.</p></div><div class="scard-act nbt-head">${head}</div></header>${body}</section>`;
}

// ---- runtime ------------------------------------------------------------------------------

const w = (typeof window === "undefined" ? {} : window) as unknown as Record<string, unknown>;
const MOCK = typeof location !== "undefined" && !w.isTauri && new URLSearchParams(location.search).has("mock");

const IS_MAC = typeof navigator !== "undefined" && /Mac/i.test(navigator.userAgent);
const view: NearbyBtView = {
  snap: { supported: true, scanning: false, message: null, devices: [] }, ports: [], pairingFor: null, othersOpen: false, busy: false,
  pin: "5555", pair: null, nativeRfcomm: IS_MAC,
};
let pairTimer: ReturnType<typeof setInterval> | undefined;
type PairResult = { state: "idle" | "pairing" | "paired" | "failed"; address: string | null; message: string | null };
let timer: ReturnType<typeof setInterval> | undefined;

/** Browser mock: the owner's tracker plus two everyday non-trackers. */
export function mockSnapshot(scanning: boolean, now = Date.now()): NearbyBtSnapshot {
  const dev = (address: string, name: string, cod: number, rssi: number | null, paired: boolean, tracker: boolean): NearbyBtDevice => ({
    address, name, classOfDevice: cod, rssi, paired, likelyTracker: tracker, lastSeen: now - 4000,
    modelHint: nameModelHint(name),
  });
  return {
    supported: true, scanning, message: null,
    devices: [
      dev("A4:C1:38:96:01:07", "FMB920_1234567", 0x610000, -63, false, true),
      dev("40:1C:83:2E:5A:10", "MIBOX4 nown", 0x080414, -71, false, false),
      dev("F4:7D:EF:12:9B:44", "[TV] Samsung TV", 0x08043c, -84, false, false),
    ],
  };
}

async function call(cmd: string, args?: Record<string, unknown>): Promise<NearbyBtSnapshot | null> {
  if (MOCK) {
    if (cmd === "bt_classic_stop") return mockSnapshot(false);
    if (cmd === "bt_classic_open_settings") return null;
    if (cmd === "bt_classic_pair" || cmd === "bt_classic_pair_status") return { state: "pairing", address: null, message: "Pairing…" } as never;
    return mockSnapshot(cmd === "bt_classic_start" || view.snap.scanning);
  }
  return invoke<NearbyBtSnapshot | null>(cmd, args);
}

function patch() {
  const el = document.querySelector('[data-role="nearby-bt"]');
  if (el) el.outerHTML = nearbyBtHtml(view);
}

async function refreshPorts() {
  try {
    view.ports = MOCK ? [] : (await invoke<PortInfo[]>("list_ports")).filter((p) => p.kind === "bluetooth");
  } catch {
    view.ports = [];
  }
}

function applySnap(s: NearbyBtSnapshot | null) {
  if (!s) return;
  view.snap = s;
  if (s.scanning && !timer) timer = setInterval(() => void poll(), 1000);
  if (!s.scanning && timer) (clearInterval(timer), (timer = undefined));
}

async function poll() {
  // Stop polling once the Devices screen (and so this card) is gone.
  if (!document.querySelector('[data-role="nearby-bt"]')) {
    void run("bt_classic_stop");
    return;
  }
  await run("bt_classic_poll");
}

async function run(cmd: string) {
  try {
    applySnap(await call(cmd));
  } catch (e) {
    view.snap = { ...view.snap, scanning: false, message: String(e) };
    applySnap(view.snap);
  }
  if (view.snap.devices.some((d) => d.paired)) await refreshPorts();
  view.busy = false;
  patch();
}

async function startPair(address: string) {
  view.pair = { state: "pairing", message: "Pairing… Accept any prompt from macOS." };
  patch();
  try {
    const r = (await call("bt_classic_pair", { address, pin: view.pin })) as unknown as PairResult;
    view.pair = { state: r.state, message: r.message };
  } catch (e) {
    view.pair = { state: "failed", message: String(e) };
  }
  patch();
  if (view.pair.state !== "pairing") return;
  pairTimer = setInterval(async () => {
    const r = (await call("bt_classic_pair_status").catch(() => null)) as unknown as PairResult | null;
    if (!r || r.state === "pairing") return;
    clearInterval(pairTimer);
    pairTimer = undefined;
    view.pair = { state: r.state, message: r.message };
    if (r.state === "paired") {
      view.pair.message = "Paired. Its serial port appears under Bluetooth (paired) once macOS creates it; or click Connect.";
      // Let the Devices screen pick up the new /dev/cu.<name> port.
      document.querySelector<HTMLElement>('[data-action="refresh-ports"]')?.click();
    }
    await run("bt_classic_poll");
  }, 500);
}

/** For the "Bluetooth (paired)" rows: direct RFCOMM address (when the native transport exists and the address is known). */
export function directAddressFor(port: PortInfo): { native: boolean; address: string | null } {
  return { native: view.nativeRfcomm, address: view.nativeRfcomm ? addressForPort(port, view.snap.devices) : null };
}

/** Markup for the Devices screen. Starts a first (non-scanning) refresh of the paired list. */
export function nearbyBtSection(): string {
  return nearbyBtHtml(view);
}

// Styles live here (not styles.css) to keep this feature self-contained; they reuse the theme tokens.
const CSS = `
.nbt-row { display: grid; grid-template-columns: minmax(160px, 2fr) minmax(130px, 1.1fr) 76px minmax(80px, 1fr) 112px; gap: var(--s2); align-items: center; padding: 12px 20px; border-top: 1px solid var(--border); }
.nbt-row.dhead { font-size: 12px; color: var(--muted); padding-top: 8px; padding-bottom: 8px; border-top: none; }
.nbt-table { border-top: 1px solid var(--border); }
.nbt-tags { display: flex; gap: 6px; align-items: center; margin-top: 2px; }
.nbt-badge { font-size: 11px; font-weight: 600; padding: 0 7px; border-radius: 9px; color: var(--accent); background: color-mix(in srgb, var(--accent) 14%, transparent); }
.nbt-addr { font-size: 12px; }
.nbt-head { display: flex; align-items: center; gap: 8px; }
.nbt-scanning { font-size: 12px; }
.nbt-spin { width: 14px; height: 14px; border-radius: 50%; border: 2px solid color-mix(in srgb, var(--muted) 30%, transparent); border-top-color: var(--accent); animation: nbt-spin .8s linear infinite; }
@keyframes nbt-spin { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) { .nbt-spin { animation-duration: 3s; } }
.nbt-others { border-top: 1px solid var(--border); }
.nbt-others > summary { cursor: pointer; padding: 10px 20px; color: var(--muted); font-size: 13px; }
.nbt-others .nbt-table { border-top: 1px solid var(--border); }
.nbt-pair { display: flex; gap: 12px; align-items: center; flex-wrap: wrap; padding: 10px 20px 12px 56px; background: color-mix(in srgb, var(--accent) 6%, transparent); font-size: 13px; }
.nbt-pair p { margin: 0; flex: 1 1 320px; }
.nbt-pair-act { display: flex; gap: 6px; align-items: center; flex-wrap: wrap; }
.nbt-pin { display: inline-flex; align-items: center; gap: 6px; font-size: 13px; }
.nbt-pin input { width: 72px; }
.nbt-pair .nbt-ok { color: var(--ok); }
.nbt-pair > p.error, .nbt-pair > p[role="status"] { flex-basis: 100%; margin: 0; }
@media (max-width: 720px) { .nbt-row { grid-template-columns: minmax(0, 1fr) auto; } .nbt-row > :nth-child(2), .nbt-row > :nth-child(3), .nbt-row > :nth-child(4) { display: none; } .nbt-pair { padding-left: 20px; } }
`;

let inited = false;
export function initNearbyBt() {
  if (inited || typeof document === "undefined") return;
  inited = true;
  const style = document.createElement("style");
  style.textContent = CSS;
  document.head.appendChild(style);
  void run("bt_classic_poll");
  document.addEventListener("toggle", (e) => {
    const t = e.target as HTMLElement;
    if (t.classList?.contains("nbt-others")) view.othersOpen = (t as HTMLDetailsElement).open;
  }, true);
  document.addEventListener("input", (e) => {
    const t = e.target as HTMLInputElement;
    if (t.matches?.("[data-nbt-pin]")) view.pin = t.value.replace(/\D/g, "");
  });
  document.addEventListener("click", (e) => {
    const b = (e.target as HTMLElement).closest<HTMLElement>("[data-nbt]");
    if (!b || (b as HTMLButtonElement).disabled) return;
    switch (b.dataset.nbt) {
      case "scan":
        view.busy = true;
        patch();
        return void run("bt_classic_start");
      case "stop":
        return void run("bt_classic_stop");
      case "refresh":
        view.pairingFor = null;
        return void run("bt_classic_poll");
      case "pair":
        view.pairingFor = b.dataset.addr ?? null;
        view.pair = null;
        return patch();
      case "pair-go":
        return void startPair(b.dataset.addr!);
      case "open-settings":
        return void call("bt_classic_open_settings").catch(() => undefined);
      case "pair-close":
        view.pairingFor = null;
        view.pair = null;
        if (pairTimer) (clearInterval(pairTimer), (pairTimer = undefined));
        return patch();
    }
  });
}
