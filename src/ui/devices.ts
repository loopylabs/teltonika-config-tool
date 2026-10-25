// Devices screen markup (ui-brief §3): USB / Nearby Bluetooth / Bluetooth (paired) sections (BLE lives under Tools), empty state, details card.
import type { BleDevice, ConnectionStatus, PortInfo } from "../api";
import { bluetoothHint, lastKnownModel, modelFromBluetoothName, modelLabel, pictureHtml } from "./device-picture";
import { icon } from "./icons";

export const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

export type Os = "mac" | "windows" | "linux";
export function detectOs(ua: string = typeof navigator === "undefined" ? "" : navigator.userAgent): Os {
  return /Mac/i.test(ua) ? "mac" : /Win/i.test(ua) ? "windows" : "linux";
}

export const PAIRING_HELP: Record<Os, string> = {
  mac: "Open System Settings → Bluetooth, put the tracker in pairing mode and click Connect next to it. It then appears here as a serial port.",
  windows: "Open Settings → Bluetooth & devices → Add device → Bluetooth. After pairing, the tracker appears here as a COM port.",
  linux: "Pair the tracker with your desktop's Bluetooth settings or bluetoothctl. If it doesn't show up here, bind it to an RFCOMM serial port first.",
};

export function lastSeenText(ms: number, now = Date.now()): string {
  const secs = Math.max(0, Math.round((now - ms) / 1000));
  return secs < 3 ? "just now" : secs < 60 ? `${secs} s ago` : `${Math.round(secs / 60)} min ago`;
}

const hex = (n: number | null) => (n === null ? "" : n.toString(16).padStart(4, "0"));
const KIND_LABEL = { usb: "USB", bluetooth: "Bluetooth", other: "Serial" } as const;

export const menu = (items: { label: string; action: string; attrs?: string; disabled?: boolean }[], label = "More actions") =>
  `<div class="menu-wrap"><button class="icon" data-action="menu" aria-haspopup="menu" aria-label="${esc(label)}">${icon("more")}</button><div class="menu" role="menu">${items
    .map((i) => `<button role="menuitem" data-action="${i.action}" ${i.attrs ?? ""} ${i.disabled ? "disabled" : ""}>${esc(i.label)}</button>`)
    .join("")}</div></div>`;

export const dtableHead = `<div class="drow dhead" role="row"><div>Device</div><div>Port</div><div>Status</div><div>Model</div><div></div></div>`;

export type PortCtx = {
  conn: ConnectionStatus;
  connectingPort: string | null;
  seenAt: number;
  model: string | null;
  /** Paired Bluetooth rows: whether native RFCOMM exists and the device address mapped from the port (null = unknown). */
  direct?: (port: PortInfo) => { native: boolean; address: string | null };
};

/** The action buttons of a port row that is not connected. Bluetooth defaults to direct RFCOMM (01-transport §2.1). */
export function connectActionsHtml(port: PortInfo, c: PortCtx, disabled: boolean): string {
  const dis = disabled ? "disabled" : "";
  const serial = (label: string, primary = false) =>
    `<button ${primary ? 'class="primary"' : ""} data-action="connect" data-port="${esc(port.name)}" data-kind="${port.kind}" ${dis} title="Connect through ${esc(port.name)}">${label}</button>`;
  if (port.kind !== "bluetooth") return serial("Connect");
  const d = c.direct?.(port) ?? { native: false, address: null };
  if (d.native && d.address) {
    return `<button class="primary" data-action="connect" data-port="bt:${esc(d.address)}" data-kind="bluetooth" ${dis} title="Direct Bluetooth serial (RFCOMM) connection">Connect</button>${serial("Via serial port")}`;
  }
  const hint = d.native
    ? `<button data-action="goto-nearby" title="This port's Bluetooth address is unknown. Connect from Nearby Bluetooth for a direct connection.">Use Nearby Bluetooth</button>`
    : "";
  return serial("Connect") + hint;
}

/** What a port row shows about the device: title, second line, model column and picture slot. */
function portLook(port: PortInfo, c: PortCtx, mine: boolean) {
  const product = port.product?.replace(/^Teltonika\s+/i, "") ?? null;
  if (mine) {
    const hw = c.conn.info?.hardwareCode ?? null;
    const l = modelLabel(hw);
    const text = c.model ?? l?.text ?? "Device";
    return { title: text, second: hw ?? "", model: [text, l?.hint === "Not in definitions" ? l.hint : null], pic: pictureHtml({ hw, model: c.model ?? l?.model ?? product }) };
  }
  const bt = port.kind === "bluetooth" ? bluetoothHint(port) : null;
  if (bt) return { title: bt, second: "", model: [bt, "from Bluetooth name"], pic: pictureHtml({ hintModel: bt }) };
  const known = lastKnownModel({ port: port.name, serial: port.serialNumber });
  if (known) return { title: known.model, second: "last connected here", model: [known.model, "last connected"], pic: pictureHtml({ hw: known.hw, model: known.model }) };
  const label = port.product ?? port.manufacturer ?? "Serial device";
  return { title: label, second: "", model: [null, null], pic: pictureHtml({ model: product }) };
}

export function portRowHtml(port: PortInfo, c: PortCtx): string {
  const ids = port.vid !== null ? ` · ${hex(port.vid)}:${hex(port.pid)}` : "";
  const connecting = c.conn.state === "connecting" && c.connectingPort === port.name;
  const mine = c.conn.port === port.name && (c.conn.state === "connected" || c.conn.state === "busy");
  const busy = c.conn.state === "connecting" || c.conn.state === "connected" || c.conn.state === "busy";
  const status = mine ? ["on", "Connected"] : connecting ? ["pending", "Connecting…"] : ["off", port.likelyTracker ? "Tracker" : "Available"];
  const look = portLook(port, c, mine);
  const [mText, mSub] = look.model;
  const model = mine
    ? `<span>${esc(mText ?? "Device")}</span>${mSub ? `<span class="muted dsub">${esc(mSub)}</span>` : ""}${c.conn.info?.imei ? `<span class="muted dsub mono">${esc(c.conn.info.imei)}</span>` : ""}`
    : mText
      ? `<span>${esc(mText)}</span><span class="muted dsub">${esc(mSub ?? "")}</span>`
      : `<span class="muted">—</span>`;
  const action = mine
    ? `<button data-action="disconnect">Disconnect</button>`
    : connecting
      ? `<button data-action="cancel-connect">Cancel</button>`
      : connectActionsHtml(port, c, busy);
  return `
    <div class="drow" role="row" tabindex="0" data-port-row="${esc(port.name)}">
      <div class="dname">${look.pic}<span><span class="trunc" title="${esc(look.title)}">${esc(look.title)}</span><span class="muted dsub trunc">${KIND_LABEL[port.kind]}${look.second ? ` · ${esc(look.second)}` : ""}${esc(ids)}</span></span></div>
      <div class="mono trunc" title="${esc(port.name)}">${esc(port.name)}</div>
      <div class="dmodel"><span class="nowrap"><span class="sdot ${status[0]}"></span>${status[1]}</span><span class="muted dsub nowrap" data-seen="${c.seenAt}">${lastSeenText(c.seenAt)}</span></div>
      <div class="dmodel">${model}</div>
      <div class="dact">${action}${menu([{ label: "Copy port name", action: "copy", attrs: `data-text="${esc(port.name)}"` }])}</div>
    </div>`;
}

export function bleRowHtml(d: BleDevice): string {
  const bars = [1, 2, 3, 4].map((n) => `<i class="${n <= d.bars ? "on" : ""}"></i>`).join("");
  const rssi = d.rssi === null ? "no signal reading" : `${d.rssi} dBm`;
  const name = d.name ? esc(d.name) : `<span class="muted">Unnamed</span>`;
  const mfr = d.manufacturerData.length ? `mfr ${esc(d.manufacturerData.join(", "))}` : "";
  const addr = d.address ?? d.id;
  const hint = modelFromBluetoothName(d.name);
  const pic = pictureHtml({ hintModel: hint, model: hint });
  const modelCell = hint ? `<span>${esc(hint)}</span><span class="muted dsub">from Bluetooth name</span>` : `<span class="muted">—</span>`;
  return `
    <div class="drow ble-device" role="row" tabindex="0" data-ble-id="${esc(d.id)}">
      <div class="dname">${pic}<span><span class="trunc">${name}</span><span class="muted dsub trunc">Bluetooth LE${mfr ? ` · ${mfr}` : ""}</span></span></div>
      <div class="mono trunc" title="${esc(addr)}">${esc(addr)}</div>
      <div class="dmodel"><span class="nowrap"><span class="bars" title="${esc(rssi)}" aria-label="Signal ${d.bars} of 4">${bars}</span></span><span class="muted dsub nowrap" data-seen="${d.lastSeen}">${lastSeenText(d.lastSeen)}</span></div>
      <div class="dmodel">${modelCell}</div>
      <div class="dact">${menu([{ label: "Copy address", action: "copy", attrs: `data-text="${esc(addr)}"` }])}</div>
    </div>`;
}

export function emptyStateHtml(os: Os): string {
  return `
    <section class="hero">
      <div class="hero-icon" aria-hidden="true">${icon("plug", 28)}</div>
      <h3>Connect a tracker</h3>
      <p class="muted">Plug it in over USB, or pair it over Bluetooth, then refresh. Nothing is sent to it until you connect.</p>
      <div class="hero-actions"><button class="primary" data-action="refresh-ports">${icon("refresh")} Refresh</button><button data-action="open-file">${icon("folder")} Open a .cfg file</button></div>
      <p class="pairing"><strong>${os === "mac" ? "macOS" : os === "windows" ? "Windows" : "Linux"}:</strong> ${esc(PAIRING_HELP[os])}</p>
    </section>`;
}

/** A section card: icon, title, helper line, optional action, then the body. */
export function sectionCard(ic: "usb" | "bluetooth" | "ble", title: string, helper: string, body: string, count?: number, action = "", role = ""): string {
  return `<section class="scard" ${role ? `data-role="${role}"` : ""}><header class="scard-head"><span class="sicon">${icon(ic, 18)}</span><div class="grow"><h3>${esc(title)}${count === undefined ? "" : ` <span class="count-pill">${count}</span>`}</h3><p class="muted">${esc(helper)}</p></div>${action}</header>${body}</section>`;
}

/** Cards for the serial ports. `nearby` (the Nearby Bluetooth card) goes between USB and paired Bluetooth. */
export function portSectionsHtml(ports: PortInfo[], c: PortCtx, nearby = ""): string {
  const usb = ports.filter((p) => p.kind !== "bluetooth");
  const bt = ports.filter((p) => p.kind === "bluetooth");
  const table = (list: PortInfo[], none: string) => (list.length ? `<div class="dtable" role="table">${dtableHead}${list.map((p) => portRowHtml(p, c)).join("")}</div>` : `<p class="muted dnone">${none}</p>`);
  return (
    sectionCard("usb", "USB", "Trackers plugged in over USB or a serial adapter.", table(usb, "No USB or serial trackers found."), usb.length) +
    nearby +
    sectionCard("bluetooth", "Bluetooth (paired)", "Paired in your OS Bluetooth settings; they show up as a serial port.", table(bt, "No paired Bluetooth trackers."), bt.length)
  );
}

export type CardFacts = { title: string; sub: string; picture?: string; badge?: string; facts: [string, string][]; mono?: string[]; notes?: string[] };

/** One device/file card: used by Devices (after connecting) and Overview, so facts never appear twice. */
export function deviceCardHtml(c: CardFacts): string {
  return `
    <section class="card dcard">
      <header class="dcard-head">${c.picture ?? `<span class="dcard-icon">${icon("devices", 22)}</span>`}<div class="grow"><h2>${esc(c.title)}</h2><p class="muted">${esc(c.sub)}</p></div>${c.badge ? `<span class="pill ok"><span class="sdot on"></span>${esc(c.badge)}</span>` : ""}</header>
      <dl class="facts">${c.facts.map(([k, v]) => `<dt>${esc(k)}</dt><dd class="${c.mono?.includes(k) ? "mono" : ""}">${esc(v)}</dd>`).join("")}</dl>
      ${(c.notes ?? []).map((n) => `<p class="note">${esc(n)}</p>`).join("")}
    </section>`;
}

export function connectionFacts(conn: ConnectionStatus): [string, string][] {
  const i = conn.info;
  if (!i) return [];
  const sec = conn.security;
  return [
    ["IMEI", i.imei ?? "Not reported"],
    ["Firmware", i.firmware ?? "Not reported"],
    ["Hardware code", i.hardwareCode ?? "Not reported"],
    ["SIM status", "Not reported by the protocol"],
    ["Connection", `${conn.kind ? KIND_LABEL[conn.kind] : "Serial"}${conn.port ? ` · ${conn.port}` : ""}`],
    ["Keyword", sec?.secured ? (sec.authorized ? "Protected · unlocked" : "Protected · locked") : "Not set"],
  ];
}
