import {
  type BleDevice,
  type BleScanStatus,
  type ConnectionStatus,
  type DefinitionsSummary,
  type EditorModel,
  type EditorNode,
  type ParamDef,
  composeDefinitions,
  type LogLine,
  type StatusEvent,
  defaultPollInterval,
  onDeviceLog,
  onDeviceStatus,
  pickTextToSave,
  pickCsvToSave,
  saveText,
  startLog,
  startStatusPolling,
  stopLog,
  stopStatusPolling,
  definitionsSummary,
  loadDefinitions,
  removeDefinitions,
  pickConfigurator,
  type Entry,
  type PortInfo,
  type Progress,
  connectDevice,
  connectionStatus,
  disconnectDevice,
  listPorts,
  loginDevice,
  onBleDevice,
  onBleScanState,
  onConnectionChanged,
  onProgress,
  openCfg,
  pickCfgToOpen,
  pickCfgToSave,
  readDeviceConfig,
  rebootDevice,
  startBleScan,
  stopBleScan,
  resetDefaults,
  checkWriteTarget,
  saveCfg,
  writeDeviceConfig,
} from "./api";
import { readControl, renderControl } from "./controls";
import { validateValue } from "./validate";
import { installPictureObserver, modelFor, modelLabel, onPresentationChange, pictureHtml, rememberModel, resetPresentation } from "./ui/device-picture";
import { bleRowHtml, connectionFacts, deviceCardHtml, detectOs, dtableHead, emptyStateHtml, esc as escHtml, lastSeenText, menu, portSectionsHtml, sectionCard } from "./ui/devices";
import { icon } from "./ui/icons";
import { LOG_CAP, type LogView, type StatusView, filterLines, logFootText, logPaneHtml, logRowHtml, pushLines, statusBodyHtml, statusPaneHtml, lineText, agoText } from "./ui/live";
import { History, byId, csvRows, csvText, ioRows, position, type SparkSpec } from "./ui/telemetry";
import { attachMap, detachMap, isOffline, showPosition } from "./ui/map";
import "./mock";
import { diagnosticsHtml, forwardConsole, initDiagnostics } from "./ui/diagnostics";
import { directAddressFor, initNearbyBt, nearbyBtSection } from "./ui/nearby-bt";
import { type NavGroup, actionsHtml, chipHtml, searchHtml, sidebarHtml } from "./ui/shell";
import { KEYWORD_ID, type ReviewView, type Warning, buildReviewRows, canWrite, entriesToSend, progressHtml, reviewHtml } from "./ui/review";
import {
  builtinPages, compositionKey, controlFor, formatDefault, formatDisplay, formatRange, hwFromHeader, lookupDef,
  matchesSearch, unitHint, unnamedLabel,
} from "./values";

type Section = "devices" | "overview" | "status" | "log" | "parameters" | "ble" | "diagnostics" | `page:${number}`;

type Doc = {
  /** Where it was last saved as a file; null for a configuration read from a device. */
  path: string | null;
  source: "file" | "device";
  entries: Entry[];
  /** Values as loaded, by entry index, to track unsaved edits. */
  original: string[];
  /** This document's own composed definitions (never shared between documents). */
  model: EditorModel | null;
  /** Hardware code used for composition and for the built-in names. */
  hw: string | null;
  /** Why there are no loaded definitions for it, or a caveat (shown in the editor). */
  notes: string[];
  index: Map<number, number>;
  pages: EditorNode[];
  groupOf: Map<number, string>;
  errors: Map<number, string>;
  parseErrors: Map<number, string>;
};

const FIRMWARE_HINT = "Firmware update isn't available yet";
const NOT_CONNECTED_HINT = "Connect to a device first";
const TABLE_LIMIT = 500;

/** Live data: status polling (only while the Status page is open) and the device log. */
const live = {
  intervalMs: 0,
  paused: false,
  polling: false,
  last: null as StatusEvent | null,
  lost: false,
  rtt: [] as number[],
  hist: new History(),
  /** Numeric I/O elements the user pinned for a graph. */
  pinned: new Set<number>(),
  offline: false,
  /** Record session: CSV rows kept in memory until saved. */
  recording: false,
  recorded: [] as string[],
  logRunning: false,
  logLines: [] as LogLine[],
  logFilter: "",
  following: true,
};

const state = {
  section: "devices" as Section,
  /** Collapsed sidebar groups. */
  collapsed: new Set<string>(),
  /** When the serial port list was last refreshed ("last seen" for ports). */
  portsAt: Date.now(),
  ports: [] as PortInfo[],
  portsError: "",
  defs: [] as DefinitionsSummary[],
  /** Bumped when builds are loaded/removed: part of the composition cache key. */
  defsEpoch: 0,
  defsBusy: false,
  defsError: "",
  /** Nearby Bluetooth LE devices, in the order first seen (rows never jump around). */
  ble: { status: { state: "idle", message: null } as BleScanStatus, devices: new Map<string, BleDevice>() },
  doc: null as Doc | null,
  filter: "",
  message: "",
  conn: { state: "disconnected", port: null, kind: null, info: null, security: null, message: null } as ConnectionStatus,
  /** Port being connected, for the "connecting…" state. */
  connectingPort: null as string | null,
  /** A device operation started from this window (read, write, reboot, reset, login). */
  working: "",
  progress: null as Progress | null,
  /** Non-blocking warnings from the last save, shown in a dismissible banner under the toolbar. */
  warnings: [] as string[],
  /** An open dialog; the keyword itself is never kept here. */
  /** The review-before-write dialog, when open. */
  review: null as ReviewView | null,
  modal: null as null | {
    kind: "login" | "reboot" | "reset" | "notice" | "save-options";
    text?: string;
    error?: string;
    /** save-options: whether the dialog came from Save as. */
    saveAs?: boolean;
  },
};

const app = document.querySelector<HTMLDivElement>("#app")!;

const esc = escHtml;
const PLATFORM_KEY = /Mac/i.test(navigator.userAgent) ? "⌘" : "Ctrl+";

function changedCount(doc: Doc): number {
  return doc.entries.filter((entry, i) => entry.value !== doc.original[i]).length;
}

function fileName(path: string | null): string {
  return path ? (path.split(/[\\/]/).pop() ?? path) : "Device configuration";
}

function docName(doc: Doc): string {
  return doc.source === "device" && !doc.path ? "Device configuration (not saved to a file)" : fileName(doc.path);
}

const isConnected = () => state.conn.state === "connected";
const isLocked = () => !!state.conn.security && state.conn.security.secured && !state.conn.security.authorized;
const canUseDevice = () => isConnected() && !state.working && !isLocked();

// ---------------------------------------------------------------- views

const isConnectedish = () => state.conn.state === "connected" || state.conn.state === "busy";

/** Model of the connected device: definitions when its document is loaded, else the USB product text. */
function deviceModel(): string | null {
  if (!isConnectedish()) return null;
  if (state.doc?.source === "device" && state.doc.model) return state.doc.model.model_name;
  const fromDefs = modelFor(state.conn.info?.hardwareCode)?.name;
  if (fromDefs) return fromDefs;
  const product = state.ports.find((p) => p.name === state.conn.port)?.product;
  return product ? product.replace(/^Teltonika\s+/i, "") : null;
}

const deviceName = () => deviceModel() ?? state.conn.info?.hardwareCode ?? "Device";

/** Second line of a device card: the hardware code, plus a hint when no loaded definitions know it. */
function deviceSub(): string {
  const hw = state.conn.info?.hardwareCode ?? "";
  return modelLabel(hw)?.hint === "Not in definitions" ? `${hw} · Not in definitions` : hw;
}

const devicePicture = (size: "sm" | "md" | "lg" = "lg") => pictureHtml({ hw: state.conn.info?.hardwareCode, model: deviceModel(), size });

/** Devices screen: hero or device card, then section cards for USB / Nearby Bluetooth (Classic) / Bluetooth (paired), with file and definitions beside them. */
function startView(): string {
  const noneFound = !state.portsError && !state.ports.length;
  const ctx = { conn: state.conn, connectingPort: state.connectingPort, seenAt: state.portsAt, model: deviceModel(), direct: directAddressFor };
  const connecting = state.conn.state === "connecting"
    ? `<p class="note" role="status"><span class="nbt-spin"></span> Connecting to ${esc(state.conn.port ?? state.connectingPort ?? "the device")}… <button data-action="cancel-connect">Cancel</button></p>`
    : "";
  const connError = state.conn.state === "error" && state.conn.message ? `<p class="error" role="alert">${esc(state.conn.message)}</p>` : "";
  const connected = isConnectedish() && !!state.conn.info;
  const top = connected
    ? deviceCardHtml({ title: deviceName(), sub: deviceSub(), picture: devicePicture(), badge: "Connected", facts: connectionFacts(state.conn), mono: ["IMEI", "Hardware code"] })
    : noneFound
      ? emptyStateHtml(detectOs())
      : "";
  return `
    <main class="pane devices">
      <header class="phead"><div><h1>Devices</h1><p class="muted">Connect a tracker to read and change its configuration.</p></div><button data-action="refresh-ports">${icon("refresh")}<span>Refresh</span></button></header>
      ${connError}${connecting}
      <div class="dgrid">
        <div class="dmain">
          ${top}
          ${state.portsError ? `<p class="error" role="alert">${esc(state.portsError)}</p>${nearbyBtSection()}` : portSectionsHtml(state.ports, ctx, nearbyBtSection())}
        </div>
        <aside class="dside">
          <section class="card"><h3>Configuration file</h3><p class="muted">View and edit a saved <span class="mono">.cfg</span> without a device.</p><button data-action="open-file">${icon("folder")}<span>Open a .cfg file</span></button></section>
          ${definitionsCard()}
        </aside>
      </div>
    </main>`;
}

// ---------------------------------------------------------------- Bluetooth LE discovery

function bleHead(): string {
  const { status } = state.ble;
  const scanning = status.state === "scanning";
  const problem = status.message && status.state !== "scanning" && status.state !== "idle";
  return `<button data-action="${scanning ? "ble-stop" : "ble-scan"}">${scanning ? "Stop" : "Scan"}</button>${problem ? `<p class="error scard-err">${esc(status.message!)}</p>` : ""}`;
}

function bleEmptyText(): string {
  const { status, devices } = state.ble;
  if (devices.size) return "";
  return status.state === "scanning"
    ? "Scanning…"
    : "Press Scan to look for Bluetooth LE devices that are advertising nearby.";
}

/** Tools → BLE scanner. Trackers are configured over USB or Bluetooth Classic, never BLE (01-transport §2A). */
function bleView(): string {
  return `
    <main class="pane devices">
      <header class="phead"><div><h1>BLE scanner</h1><p class="muted">Find Bluetooth LE sensors and beacons near the vehicle. Trackers are configured over USB or Bluetooth Classic, not BLE.</p></div></header>
      ${bleSection()}
    </main>`;
}

function bleSection(): string {
  const rows = [...state.ble.devices.values()].map(bleRowHtml).join("");
  const body = `<p class="muted dnone" data-role="ble-empty">${bleEmptyText()}</p><div class="dtable" role="table">${dtableHead}<div class="dcontents" data-role="ble-list">${rows}</div></div>`;
  return sectionCard("ble", "Nearby BLE devices", "Bluetooth LE sensors and beacons advertising nearby.", body, undefined, `<div data-role="ble-head" class="scard-act">${bleHead()}</div>`, "ble");
}

/** Event handlers patch the list in place; the rest of the start screen is untouched. */
function patchBleHead() {
  const head = app.querySelector('[data-role="ble-head"]');
  if (head) head.innerHTML = bleHead();
  patchBleEmpty();
}

function patchBleEmpty() {
  const empty = app.querySelector('[data-role="ble-empty"]');
  if (empty) empty.textContent = bleEmptyText();
}

function patchBleDevice(d: BleDevice) {
  state.ble.devices.set(d.id, d);
  const list = app.querySelector('[data-role="ble-list"]');
  if (!list) return;
  const existing = [...list.children].find((el) => (el as HTMLElement).dataset.bleId === d.id);
  if (existing) existing.outerHTML = bleRowHtml(d);
  else list.insertAdjacentHTML("beforeend", bleRowHtml(d));
  patchBleEmpty();
}

async function toggleBleScan(start: boolean) {
  try {
    if (start) {
      state.ble.devices.clear();
      app.querySelector('[data-role="ble-list"]')!.innerHTML = "";
      await startBleScan();
    } else {
      await stopBleScan();
    }
  } catch (error) {
    state.ble.status = { state: "error", message: `Bluetooth scan failed: ${error}` };
    patchBleHead();
  }
}

// "Last seen" is the only text that changes without an event.
setInterval(() => {
  for (const el of app.querySelectorAll<HTMLElement>("[data-seen]")) el.textContent = lastSeenText(Number(el.dataset.seen));
}, 2000);


function definitionsCard(): string {
  const rows = state.defs
    .map(
      (d) => `<li class="port"><div><div class="port-name">Configurator ${esc(d.product_version)}${d.primary ? ` <span class="badge">primary</span>` : ""}</div>
        <div class="muted">${esc(d.families.join(", "))}, ${d.document_count} documents${d.from_cache ? " (cached)" : ""}${d.extra_hardware.length ? `; adds hardware ${esc(d.extra_hardware.join(", "))}` : ""}</div>
        ${d.warnings.map((w) => `<div class="muted">${esc(w)}</div>`).join("")}</div>
        <button data-action="remove-definitions" data-version="${esc(d.product_version)}">Remove</button></li>`,
    )
    .join("");
  return `
      <section class="card">
        <h3>Parameter definitions</h3>
        <p class="muted">Names, units and ranges come from your own copy of Teltonika Configurator. Nothing is uploaded. With several builds the newest is primary.</p>
        ${rows ? `<ul class="ports">${rows}</ul>` : ""}
        ${state.defsError ? `<p class="error">${esc(state.defsError)}</p>` : ""}
        <button data-action="load-definitions" ${state.defsBusy ? "disabled" : ""}>${state.defsBusy ? "Loading…" : state.defs.length ? "Add another build…" : "Load definitions…"}</button>
      </section>`;
}

// Loaded builds are remembered by path (per-viewer storage in the app's webview) and reloaded on start.
const DEFS_KEY = "fmconfig.definitionPaths";
const savedPaths = (): Record<string, string> => {
  try {
    return JSON.parse(localStorage.getItem(DEFS_KEY) ?? "{}");
  } catch {
    return {};
  }
};
const savePaths = (paths: Record<string, string>) => {
  try {
    localStorage.setItem(DEFS_KEY, JSON.stringify(paths));
  } catch {
    /* storage unavailable: builds just aren't remembered */
  }
};

/** Reload remembered builds that the backend doesn't already hold; paths that no longer load are forgotten. */
async function restoreDefs() {
  const paths = savedPaths();
  let failed = false;
  for (const [version, path] of Object.entries(paths)) {
    if (state.defs.some((d) => d.product_version === version)) continue;
    try {
      state.defs = await loadDefinitions(path);
    } catch {
      delete paths[version];
      failed = true;
    }
  }
  if (failed) savePaths(paths);
  state.defsEpoch++;
  resetPresentation();
  render();
}

async function loadDefs() {
  const path = await pickConfigurator();
  if (!path) return;
  state.defsBusy = true;
  state.defsError = "";
  render();
  try {
    const before = new Set(state.defs.map((d) => d.product_version));
    state.defs = await loadDefinitions(path);
    const added = state.defs.find((d) => !before.has(d.product_version));
    if (added) savePaths({ ...savedPaths(), [added.product_version]: path });
    state.defsEpoch++;
  resetPresentation();
  } catch (e) {
    state.defsError = String(e);
  } finally {
    state.defsBusy = false;
    render();
  }
}

async function removeDefs(version: string) {
  state.defs = await removeDefinitions(version);
  const paths = savedPaths();
  delete paths[version];
  savePaths(paths);
  state.defsEpoch++;
  resetPresentation();
  render();
}

// ---------------------------------------------------------------- per-document definitions

const isParameter = (entry: Entry) => /^\d+$/.test(entry.key);

/** Compositions by key (build epoch, hardware, config version, spec, features): switching documents is instant. */
const compositions = new Map<string, EditorModel | string>();

function emptyDocParts(): Pick<Doc, "model" | "hw" | "notes" | "index" | "pages" | "groupOf" | "errors" | "parseErrors"> {
  return { model: null, hw: null, notes: [], index: new Map(), pages: [], groupOf: new Map(), errors: new Map(), parseErrors: new Map() };
}

const valueOf = (doc: Doc, id: number) => doc.entries[doc.index.get(id) ?? -1]?.value;
const defOf = (doc: Doc, id: number): ParamDef | undefined => lookupDef(doc.model, doc.hw, id);
const nameOf = (doc: Doc, id: number) => {
  const d = defOf(doc, id);
  return d ? (d.list ?? d.label) : unnamedLabel(id);
};

/** Composition inputs: DeviceInfo for device documents, the file header for files (04 §3). */
function compositionInputs(doc: Doc) {
  const header = (k: string) => doc.entries.find((e) => e.key === k)?.value;
  if (doc.source === "device" && state.conn.info?.hardwareCode) {
    const i = state.conn.info;
    return { hw: i.hardwareCode!, cfg: i.configVersion ?? header("ConfigurationVersion") ?? "", spec: i.specId || 1, features: i.features, fromFile: false };
  }
  const hw = header("HwVersion");
  return { hw: hw ? hwFromHeader(hw) : null, cfg: header("ConfigurationVersion") ?? "", spec: Number(header("SpecId") ?? 1) || 1, features: undefined, fromFile: true };
}

function pruneNode(n: EditorNode, present: Set<number>): EditorNode | null {
  const children = n.children.map((c) => pruneNode(c, present)).filter((c): c is EditorNode => !!c);
  const ids = n.ids.filter((id) => present.has(id));
  return ids.length || children.length ? { ...n, ids, children } : null;
}

function walkGroups(n: EditorNode, path: string[], out: Map<number, string>) {
  const here = n.title ? [...path, n.title] : path;
  for (const id of n.ids) if (!out.has(id)) out.set(id, here.join(" › "));
  n.children.forEach((c) => walkGroups(c, here, out));
}

/** (Re)compose this document's definitions and rebuild everything derived from them. */
async function attachModel(doc: Doc) {
  Object.assign(doc, emptyDocParts());
  doc.entries.forEach((e, i) => isParameter(e) && doc.index.set(Number(e.key), i));
  const inp = compositionInputs(doc);
  doc.hw = inp.hw;
  const present = new Set(doc.index.keys());
  if (state.defs.length && inp.hw) {
    const key = compositionKey({ build: `lib${state.defsEpoch}`, hwCode: inp.hw, cfgVersion: inp.cfg, specId: inp.spec, features: inp.features ?? "all" });
    let model = compositions.get(key);
    if (model === undefined) {
      try {
        model = await composeDefinitions({ hwCode: inp.hw, cfgVersion: inp.cfg, deviceSpecId: inp.spec > 1 ? inp.spec : undefined, featureMask: inp.features });
      } catch (e) {
        model = String(e);
      }
      compositions.set(key, model);
    }
    if (typeof model === "string") doc.notes.push(`No definitions for this model: ${model}`);
    else {
      doc.model = model;
      doc.notes.push(`Definitions: Configurator ${model.build}, ${model.model_name}, configuration version ${inp.cfg || "unknown"}.`);
    }
  } else if (!state.defs.length) {
    doc.notes.push("Load a Teltonika Configurator on the Devices screen for names, units and ranges of every parameter.");
  }
  if (inp.fromFile && doc.model) doc.notes.push("A file does not store the device's feature flags, so all are assumed present; some parameters may not apply to the real device.");
  doc.pages = (doc.model ? doc.model.pages : builtinPages(inp.hw).map((p) => ({ kind: "page" as const, title: p.title, ids: p.defs.map((d) => d.id), children: [] })))
    .map((n) => pruneNode(n, present))
    .filter((n): n is EditorNode => !!n);
  doc.pages.forEach((n) => walkGroups(n, [], doc.groupOf));
  recomputeErrors(doc);
}

function recomputeErrors(doc: Doc) {
  doc.errors = new Map();
  for (const [id, i] of doc.index) {
    const def = defOf(doc, id);
    if (!def) continue;
    const err = validateValue(def, doc.entries[i].value, (other) => valueOf(doc, other));
    if (err) doc.errors.set(id, err);
  }
}

/** Entries that would be sent to a device (changed ones for a device document, all for a file). */
function toSend(doc: Doc): number[] {
  return [...doc.index.entries()].filter(([, i]) => doc.source !== "device" || doc.entries[i].value !== doc.original[i]).map(([id]) => id);
}

const blockingErrors = (doc: Doc) => toSend(doc).filter((id) => doc.errors.has(id) || doc.parseErrors.has(id));

async function openDoc(doc: Doc) {
  await attachModel(doc);
  state.doc = doc;
  state.section = "overview";
  state.filter = "";
}

// ---------------------------------------------------------------- editor views

const idTag = (id: number) => `<small class="idbadge mono">#${id}</small>`;

function pageIds(n: EditorNode): number[] {
  return [...n.ids, ...n.children.flatMap(pageIds)];
}

const isChanged = (doc: Doc, id: number) => {
  const i = doc.index.get(id);
  return i !== undefined && doc.entries[i].value !== doc.original[i];
};

function navGroups(): NavGroup[] {
  const doc = state.doc;
  const settings = doc
    ? doc.pages.map((p, i) => ({ section: `page:${i}`, label: p.title || "Other", icon: "list" as const, count: pageIds(p).filter((id) => isChanged(doc, id)).length }))
    : [];
  const callout = !doc
    ? { text: "Open a file or read a device to see its settings.", action: "open-file", label: "Open a .cfg file" }
    : !state.defs.length
      ? { text: "Load Teltonika Configurator definitions to see every setting grouped and named.", action: "load-definitions", label: "Load definitions" }
      : undefined;
  return [
    { id: "device", title: "Device", items: [{ section: "devices", label: "Devices", icon: "devices" }, { section: "overview", label: "Overview", icon: "overview" }, { section: "status", label: "Telemetry", icon: "status" }, { section: "log", label: "Device log", icon: "records" }] },
    { id: "settings", title: "Settings", items: settings, callout },
    {
      id: "tools",
      title: "Tools",
      items: [{ section: "parameters", label: "All parameters", icon: "list" }, { section: "ble", label: "BLE scanner", icon: "ble" }, { section: "diagnostics", label: "Diagnostics", icon: "info" }, { section: "records", label: "Records", icon: "records", disabled: "Reading records isn't available yet" }, { section: "firmware", label: "Firmware", icon: "firmware", disabled: FIRMWARE_HINT }],
    },
  ];
}

const sidebar = () => sidebarHtml(navGroups(), state.section, state.collapsed);

/** Switch the content pane (and highlight) without rebuilding the shell. */
function setSection(section: Section) {
  state.section = section;
  const side = app.querySelector("aside.sidebar");
  if (side) side.innerHTML = sidebar();
  const content = app.querySelector('[data-role="content"]');
  if (content) {
    content.innerHTML = contentHtml();
    content.scrollTop = 0;
  } else render();
  if (section === "log") paintLog();
  syncPolling();
}

function dangerZone(): string {
  const ok = canUseDevice();
  const why = ok ? "" : isLocked() ? "Enter the keyword first" : NOT_CONNECTED_HINT;
  const row = (ic: "power" | "reset" | "firmware", title: string, text: string, button: string) => `<div class="danger-row"><span class="danger-ico">${icon(ic, 18)}</span><div class="grow"><div class="danger-title">${title}</div><div class="muted">${text}</div></div>${button}</div>`;
  return `
    <section class="danger" aria-labelledby="dz">
      <h3 id="dz">${icon("warn")} Danger zone</h3>
      ${row("power", "Reboot device", "Restarts the tracker and closes this connection. Changes not yet saved on the device are discarded.", `<button class="danger-btn" data-action="reboot" ${ok ? "" : "disabled"} title="${esc(why)}">Reboot…</button>`)}
      ${row("reset", "Reset to defaults", "Erases the device's configuration and restores factory defaults. This can't be undone.", `<button class="danger-btn" data-action="reset" ${ok ? "" : "disabled"} title="${esc(why)}">Reset to defaults…</button>`)}
      ${row("firmware", "Update firmware", "Not available yet.", `<button class="danger-btn" disabled title="${FIRMWARE_HINT}">Update firmware…</button>`)}
    </section>`;
}

/** One card for everything known about the device and/or the open configuration (never two cards). */
function overview(doc: Doc | null): string {
  const connected = isConnectedish() && !!state.conn.info;
  if (!doc && !connected) {
    return `<section class="pane"><header class="phead"><div><h1>Overview</h1><p class="muted">Nothing is loaded yet.</p></div></header>
      <section class="hero"><div class="hero-icon">${icon("overview", 28)}</div><h3>No configuration loaded</h3><p class="muted">Read the configuration from a connected tracker, or open a saved .cfg file.</p>
      <div class="hero-actions"><button class="primary" data-action="goto-devices">${icon("devices")} Go to Devices</button><button data-action="open-file">${icon("folder")} Open a .cfg file</button></div></section>${dangerZone()}</section>`;
  }
  const h = (k: string) => doc?.entries.find((e) => e.key === k)?.value;
  const info = state.conn.info;
  const useDevice = connected && (!doc || doc.source === "device");
  const facts: [string, string][] = [];
  if (useDevice) facts.push(...connectionFacts(state.conn));
  const hw = (useDevice ? info?.hardwareCode : null) ?? h("HwVersion");
  if (!useDevice && hw) facts.push(["Hardware code", hw]);
  const cfg = (useDevice ? info?.configVersion : null) ?? h("ConfigurationVersion");
  if (cfg) facts.push(["Configuration version", cfg]);
  const spec = useDevice && info ? String(info.specId) : h("SpecId");
  if (spec) facts.push(["Spec", spec === "1" ? "1 (standard)" : spec]);
  if (useDevice && info?.features) facts.push(["Feature flags set", String([...BigInt(info.features).toString(2)].filter((c) => c === "1").length)]);
  if (h("Title")) facts.push(["Title", h("Title")!]);
  if (doc) {
    const unknown = [...doc.index.keys()].filter((id) => !defOf(doc, id)).length;
    facts.push(["Parameters", `${doc.index.size.toLocaleString()}${doc.model ? ` (${unknown.toLocaleString()} unknown to definitions)` : ""}`]);
    facts.push([doc.source === "device" ? "Source" : "File", doc.source === "device" ? `Read from device${doc.path ? `, saved to ${doc.path}` : ", not saved to a file"}` : (doc.path ?? "")]);
  }
  const title = doc ? (doc.model?.model_name ?? h("FmType") ?? (useDevice ? deviceName() : docName(doc))) : deviceName();
  const sub = doc ? (doc.source === "device" ? "Configuration read from the connected device" : `File · ${fileName(doc.path)}`) : "Connected device";
  return `
    <section class="pane">
      <header class="phead"><div><h1>Overview</h1><p class="muted">${esc(sub)}</p></div></header>
      ${deviceCardHtml({ title, sub: useDevice ? deviceSub() : (hw ?? ""), picture: pictureHtml({ hw, model: doc?.model?.model_name ?? deviceModel(), size: "lg" }), badge: useDevice ? "Connected" : undefined, facts, mono: ["IMEI", "Hardware code", "Configuration version", "File"], notes: doc?.notes })}
      ${dangerZone()}
    </section>`;
}

function statusViewModel(): StatusView {
  const waiting = live.polling && live.intervalMs > 0 && (live.last === null ? true : live.last.ok && Date.now() - live.last.atMs > live.intervalMs + 3000);
  const items = state.doc?.model?.io_items;
  const sparks: SparkSpec[] = [
    { key: "773", label: "External voltage", unit: "V", scale: 0.001 },
    { key: "774", label: "Battery voltage", unit: "V", scale: 0.001 },
    ...[...live.pinned].map((id) => {
      const item = items?.find((i) => i.id === id);
      return { key: `io${id}`, label: item?.name ?? `I/O element #${id}`, unit: item?.unit ?? "", scale: 1 };
    }),
  ];
  return {
    connected: isConnected(), kind: state.conn.kind, paused: live.paused, intervalMs: live.intervalMs, last: live.last, lost: live.lost, rtt: live.rtt, now: Date.now(), message: state.conn.message,
    waiting: waiting && isConnected(), io: ioRows(live.last?.fields ?? [], items), pinned: live.pinned, hist: live.hist, sparks, offline: live.offline,
    recording: live.recording, recordedRows: live.recorded.length,
  };
}

const statusView = () => statusPaneHtml(statusViewModel());

function logViewModel(): LogView {
  const shown = filterLines(live.logLines, live.logFilter).length;
  return { running: live.logRunning, connected: isConnected() || state.conn.state === "busy", busy: state.conn.state === "busy", filter: live.logFilter, total: live.logLines.length, shown, following: live.following };
}

const logView = () => logPaneHtml(logViewModel());

function paintLog() {
  const box = app.querySelector<HTMLElement>('[data-role="log-box"]');
  if (!box) return;
  box.innerHTML = filterLines(live.logLines, live.logFilter).map(logRowHtml).join("");
  if (live.following) box.scrollTop = box.scrollHeight;
  paintLogFoot();
}

function paintLogFoot() {
  const foot = app.querySelector('[data-role="log-foot"]');
  if (foot) foot.textContent = logFootText(logViewModel());
}

/** Polls only while the Status page is visible, connected, not paused and idle. */
function syncPolling() {
  const want = state.section === "status" && isConnected() && !live.paused && !document.hidden;
  if (want && !live.polling) {
    live.polling = true;
    live.lost = false;
    void (async () => {
      if (!live.intervalMs) live.intervalMs = await defaultPollInterval().catch(() => 2000);
      const sel = app.querySelector<HTMLSelectElement>('[data-role="poll-interval"]');
      if (sel) sel.value = String(live.intervalMs);
      if (live.polling) await startStatusPolling(live.intervalMs).catch(() => (live.polling = false));
    })();
  } else if (!want && live.polling) {
    live.polling = false;
    void stopStatusPolling().catch(() => {});
  }
}

function connectionDropped() {
  live.logRunning = false;
  if (live.polling) {
    live.polling = false;
    live.lost = state.conn.state === "error";
  }
  if (state.conn.state === "disconnected") {
    live.last = null;
    live.rtt = [];
    live.hist.clear();
    live.lost = false;
  }
}

async function toggleLog(on: boolean) {
  try {
    if (on) {
      await startLog();
      live.logRunning = true;
      live.following = true;
    } else {
      live.logRunning = false;
      await stopLog();
    }
  } catch (e) {
    live.logRunning = false;
    toast(String(e));
  }
  const content = app.querySelector('[data-role="content"]');
  if (content && state.section === "log") content.innerHTML = logView(), paintLog();
}

async function toggleRecording() {
  if (!live.recording) {
    live.recording = true;
    live.recorded = [];
    return paintStatus();
  }
  live.recording = false;
  const rows = live.recorded;
  paintStatus();
  if (!rows.length) return toast("Nothing was recorded (no successful poll)");
  const path = await pickCsvToSave(`telemetry-${new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19)}.csv`);
  if (!path) return;
  try {
    await saveText(path, csvText(rows));
    toast(`Saved ${rows.length} rows to ${path}`);
  } catch (e) {
    toast(String(e));
  }
}

async function saveLog() {
  const path = await pickTextToSave(`device-log-${new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19)}.txt`);
  if (!path) return;
  try {
    await saveText(path, filterLines(live.logLines, live.logFilter).map(lineText).join("\n") + "\n");
    toast(`Saved ${path}`);
  } catch (e) {
    toast(String(e));
  }
}

/** One logical row: a single parameter, or all present slots of one list. */
function rowHtml(doc: Doc, ids: number[], showGroup: boolean): string {
  const first = ids[0];
  const def = defOf(doc, first);
  const idxs = ids.map((id) => doc.index.get(id)!);
  const changed = idxs.some((i) => doc.entries[i].value !== doc.original[i]);
  const name = def ? nameOf(doc, first) : "Unnamed parameter";
  const idText = ids.length > 1 ? `#${ids[0]}–${ids[ids.length - 1]}` : `#${first}`;
  const badge = !def && doc.model ? `<span class="badge warn">unknown</span>` : "";
  const ctl = (id: number) => {
    const i = doc.index.get(id)!;
    const d = defOf(doc, id);
    const v = doc.entries[i].value;
    const control = d ? renderControl(d, v, i) : renderControl({ id, label: "", value_type: "", control: "other", default: null, min: null, max: null, min_id: null, max_id: null, units: null, text_min: null, text_max: null, validate: [], options: [], bitmask: false, mask: null, converter: null, list: null, slot: null }, v, i);
    const hint = d && controlFor(d) === "number" ? unitHint(v, d.units) : null;
    const shown = d ? formatDisplay(d, v) : v;
    return `<span class="ctl" title="Raw value: ${esc(v)}">${control}${hint ? `<span class="hint" data-hint="${id}">(${esc(hint)})</span>` : ""}</span><span class="sr-only">${esc(shown)}</span>`;
  };
  const body =
    ids.length > 1
      ? ids.map((id, n) => `<div class="slot" data-pid="${id}"><span class="slotno">${defOf(doc, id)?.slot ?? n + 1}</span>${ctl(id)}<button class="link" data-action="clear-slot" data-index="${doc.index.get(id)}">Clear</button><span class="perr" data-err="${id}"></span></div>`).join("")
      : `${ctl(first)}<span class="perr" data-err="${first}"></span>`;
  const range = def ? formatRange(def) : "";
  const dflt = def && def.default !== null ? `default ${esc(formatDefault(def))}` : "";
  const grp = showGroup ? esc(doc.groupOf.get(first) ?? "") : "";
  const desc = [grp, range ? `range ${esc(range)}` : "", dflt, !def ? (doc.model ? "Not described by the loaded definitions" : "Load definitions for its name and range") : ""].filter(Boolean).join(" · ");
  const canReset = ids.some((id) => { const d = defOf(doc, id); return d && d.default !== null && valueOf(doc, id) !== d.default; });
  return `
    <div class="prow ${changed ? "changed" : ""}" data-ids="${ids.join(",")}">
      <div class="plabel">
        <div class="pname">${esc(name)} ${idTag(first).replace(`#${first}`, idText)}<span class="modchip" ${changed ? "" : "hidden"}>Modified</span>${badge}</div>
        <div class="pdesc muted">${desc}</div>
      </div>
      <div class="pctl">${body}</div>
      <div class="pact">
        <button class="icon" data-action="revert" data-ids="${ids.join(",")}" title="Revert to the loaded value" aria-label="Revert ${esc(name)}" ${changed ? "" : "hidden"}>${icon("undo")}</button>
        ${menu([{ label: "Reset to default", action: "reset-default", attrs: `data-ids="${ids.join(",")}"`, disabled: !canReset }], `More actions for ${name}`)}
      </div>
    </div>`;
}

/** Collapses consecutive slots of one list into a single logical row. */
function logicalRows(doc: Doc, ids: number[]): number[][] {
  const out: number[][] = [];
  const lists = new Map<string, number[]>();
  for (const id of ids) {
    const list = defOf(doc, id)?.list;
    if (list) {
      const existing = lists.get(list);
      if (existing) existing.push(id);
      else {
        const g = [id];
        lists.set(list, g);
        out.push(g);
      }
    } else out.push([id]);
  }
  return out;
}

function groupNode(doc: Doc, n: EditorNode, depth: number, budget: { left: number }): string {
  const rows = logicalRows(doc, n.ids).slice(0, Math.max(0, budget.left));
  budget.left -= rows.length;
  const own = rows.map((r) => rowHtml(doc, r, false)).join("");
  const kids = n.children.map((c) => groupNode(doc, c, depth + 1, budget)).join("");
  if (depth === 0) return (own ? `<section class="cardx">${own}</section>` : "") + kids;
  if (depth === 1) return `<section class="cardx"><h3>${esc(n.title || "Other")}</h3>${own}${kids}</section>`;
  return `<h4 class="subhead">${esc(n.title || "Other")}</h4>${own}${kids}`;
}

function pageCounts(doc: Doc, page: EditorNode): string {
  const ids = pageIds(page);
  const settings = logicalRows(doc, ids).length;
  const modified = logicalRows(doc, ids.filter((id) => isChanged(doc, id))).length;
  return `${settings.toLocaleString()} setting${settings === 1 ? "" : "s"} · ${modified} modified`;
}

function pageView(doc: Doc, i: number): string {
  const page = doc.pages[i];
  if (!page) return overview(doc);
  const budget = { left: 600 };
  return `
    <section class="pane">
      <header class="phead"><div><h1>${esc(page.title || "Other")}</h1><p class="muted" data-role="pcount">${pageCounts(doc, page)}</p></div></header>
      ${doc.notes.filter((n) => n.startsWith("A file")).map((n) => `<p class="note">${esc(n)}</p>`).join("")}
      <div class="rows" data-role="rows">${groupNode(doc, page, 0, budget)}</div>
      ${budget.left <= 0 ? `<p class="muted">Showing the first 600 rows of this group. Use All parameters with a search to find the rest.</p>` : ""}
    </section>`;
}

function allRows(doc: Doc): string {
  const needle = state.filter;
  const ids = [...doc.index.keys()].filter((id) => {
    if (!needle.trim()) return true;
    const d = defOf(doc, id);
    const raw = valueOf(doc, id) ?? "";
    return matchesSearch(needle, { id, name: nameOf(doc, id), group: doc.groupOf.get(id) ?? "", shown: d ? formatDisplay(d, raw) : raw, raw });
  });
  const logical = logicalRows(doc, ids);
  const cap = (rows: number[][]) => rows.slice(0, TABLE_LIMIT);
  const head = "";
  let html: string;
  if (doc.model) {
    const sections = new Map<string, number[][]>();
    for (const r of logical) {
      const top = (doc.groupOf.get(r[0]) ?? "").split(" › ")[0] || (defOf(doc, r[0]) ? "Other" : "Unknown to definitions");
      if (!sections.has(top)) sections.set(top, []);
      sections.get(top)!.push(r);
    }
    html = [...sections.entries()].map(([t, rows]) => `<details open class="sec"><summary>${esc(t)} <span class="muted">${rows.length}</span></summary><div class="cardx">${cap(rows).map((r) => rowHtml(doc, r, true)).join("")}</div></details>`).join("");
  } else {
    html = `<div class="cardx">${cap(logical).map((r) => rowHtml(doc, r, true)).join("")}</div>`;
  }
  const more = logical.length > TABLE_LIMIT ? `<p class="muted">Showing up to ${TABLE_LIMIT} rows per group of ${logical.length.toLocaleString()}. Narrow the search to see more.</p>` : "";
  return logical.length ? head + html + more : `<p class="muted">No parameters match.</p>`;
}

function parameters(doc: Doc): string {
  return `
    <section class="pane">
      <header class="phead"><div><h1>All parameters</h1><p class="muted">${doc.index.size.toLocaleString()} parameters${state.filter.trim() ? ` · matching “${esc(state.filter.trim())}”` : ""}. Search from the top bar (${PLATFORM_KEY}K).</p></div></header>
      <div class="rows" data-role="rows">${allRows(doc)}</div>
    </section>`;
}

function contentHtml(): string {
  const doc = state.doc;
  switch (state.section) {
    case "devices":
      return startView();
    case "overview":
      return overview(doc);
    case "status":
      return statusView();
    case "log":
      return logView();
    case "parameters":
      return doc ? parameters(doc) : startView();
    case "ble":
      return bleView();
    case "diagnostics":
      return diagnosticsHtml();
    default:
      return doc ? pageView(doc, Number(state.section.slice(5))) : startView();
  }
}

/** After an edit: refresh errors, hints, changed marks and the toolbar without rebuilding inputs. */
function refreshRows(doc: Doc) {
  recomputeErrors(doc);
  for (const el of app.querySelectorAll<HTMLElement>("[data-err]")) {
    const id = Number(el.dataset.err);
    const msg = doc.parseErrors.get(id) ?? doc.errors.get(id) ?? "";
    el.textContent = msg;
    el.closest(".ctl, .slot, .pctl")?.classList.toggle("invalid", !!msg);
  }
  for (const row of app.querySelectorAll<HTMLElement>(".prow[data-ids]")) {
    const idxs = row.dataset.ids!.split(",").map((id) => doc.index.get(Number(id))!);
    const changed = idxs.some((i) => doc.entries[i].value !== doc.original[i]);
    row.classList.toggle("changed", changed);
    row.querySelector<HTMLButtonElement>('[data-action="revert"]')!.hidden = !changed;
    const chip = row.querySelector<HTMLElement>(".modchip");
    if (chip) chip.hidden = !changed;
  }
  const page = state.section.startsWith("page:") ? doc.pages[Number(state.section.slice(5))] : null;
  const pc = app.querySelector('[data-role="pcount"]');
  if (pc && page) pc.textContent = pageCounts(doc, page);
  for (const el of app.querySelectorAll<HTMLElement>("[data-hint]")) {
    const id = Number(el.dataset.hint);
    const h = unitHint(valueOf(doc, id) ?? "", defOf(doc, id)?.units ?? null);
    el.textContent = h ? `(${h})` : "";
  }
}

function chipView() {
  const { state: s, security } = state.conn;
  if (s === "connecting") return { state: "pending" as const, text: "Connecting…", title: "Devices" };
  if (s !== "connected" && s !== "busy") return { state: "off" as const, text: "No device", title: "Choose a device" };
  const locked = security?.secured && !security.authorized;
  const imei = state.conn.info?.imei;
  return { state: "on" as const, picture: pictureHtml({ hw: state.conn.info?.hardwareCode, model: deviceModel(), size: "sm" }), text: `${deviceName()}${locked ? " · locked" : ""}`, title: `${state.conn.info?.hardwareCode ?? ""}${imei ? ` · IMEI ${imei}` : ""}` };
}

/** The primary button follows state: connect, read, or review & write (ui-brief §1). */
function primaryView() {
  const doc = state.doc;
  if (!isConnectedish() && state.conn.state !== "connecting") return { label: "Connect device", action: "goto-devices", disabled: false, title: "Choose a device to connect" };
  if (state.conn.state === "connecting") return { label: "Connecting…", action: "goto-devices", disabled: true, title: "" };
  if (isLocked()) return { label: "Enter keyword", action: "login", disabled: false, title: "This device is protected" };
  const busy = !!state.working;
  if (doc && doc.source === "device" && changedCount(doc)) {
    const bad = blockingErrors(doc).length;
    return { label: `Review & write (${changedCount(doc)})`, action: "save-device", disabled: busy || bad > 0, title: bad ? `${bad} invalid value${bad > 1 ? "s" : ""}: fix them first` : `Review before writing (${PLATFORM_KEY}⇧W)` };
  }
  if (doc && doc.source === "file") return { label: "Review & write file", action: "save-device", disabled: busy || blockingErrors(doc).length > 0, title: "Send this file to the connected device" };
  return { label: "Read from device", action: "load-device", disabled: busy, title: "Read the configuration from the device" };
}

const canSaveFile = () => !!state.doc && (changedCount(state.doc) > 0 || (state.doc.source === "device" && !state.doc.path));
/** Replace the top bar actions only when they changed, so a click that follows an edit's blur isn't lost to a re-render. */
function patchActions() {
  const actions = app.querySelector('[data-role="tb-actions"]');
  if (!actions) return;
  const html = actionsMarkup();
  if (actions.getAttribute("data-html") === html) return;
  actions.setAttribute("data-html", html);
  actions.innerHTML = html;
}
const actionsMarkup = () => actionsHtml(primaryView(), canSaveFile(), !!state.doc, isConnectedish(), PLATFORM_KEY);

function topbar(): string {
  return `<header class="topbar"><div data-role="tb-chip">${chipHtml(chipView())}</div>${searchHtml(state.filter, !!state.doc, PLATFORM_KEY)}<div class="tb-actions" data-role="tb-actions" data-html="${esc(actionsMarkup())}">${actionsMarkup()}</div></header>`;
}

function progressBar(): string {
  const p = state.progress;
  if (!state.working) return "";
  const known = p && p.total > 0;
  return `<div class="progress" role="progressbar"><div class="progress-text">${esc(p?.message ?? state.working)}${known ? ` (${p!.done}/${p!.total})` : "…"}</div><div class="bar ${known ? "" : "indeterminate"}"><div style="width:${known ? Math.round((p!.done / p!.total) * 100) : 40}%"></div></div></div>`;
}

function statusBar(): string {
  const doc = state.doc;
  const left = doc ? esc(docName(doc)) : "No configuration loaded";
  const right = doc ? `${changedCount(doc)} changed · ${doc.entries.length.toLocaleString()} entries` : "";
  return `
    <footer class="status">
      <span>${left}</span>
      <span class="message">${esc(state.message)}</span>
      <span data-role="counts">${right}</span>
    </footer>`;
}

function warningsBanner(): string {
  if (!state.warnings.length) return "";
  return `<div class="warnings" role="status"><ul>${state.warnings.map((w) => `<li>${esc(w)}</li>`).join("")}</ul><button class="link" data-action="dismiss-warnings">Dismiss</button></div>`;
}

/** Full render: only for view changes (open, close, load). Rebuilds the shell and the active pane. */
function render() {
  if (!state.doc && (state.section === "parameters" || state.section.startsWith("page:"))) state.section = "devices";
  app.innerHTML = `
    <div class="shell">
      <aside class="sidebar">${sidebar()}</aside>
      <div class="main">
        ${topbar()}
        <div data-role="progress">${progressBar()}</div>
        <div data-role="warnings">${warningsBanner()}</div>
        <div class="content" data-role="content">${contentHtml()}</div>
        <div class="skeleton-layer" data-role="skeleton" hidden></div>
        ${statusBar()}
      </div>
    </div>`;
  renderModal();
  refreshProgress();
  if (state.section === "log") paintLog();
  syncPolling();
}

/** Status and progress updates patch the chrome only, so inputs keep focus. */
let lastRemembered = "";
/** Remembers the probed model per port, serial number and IMEI, for the list before the next connect. */
function noteProbedModel() {
  const hw = state.conn.info?.hardwareCode ?? null;
  const model = isConnectedish() ? modelFor(hw)?.name : null;
  if (!model) return;
  const serial = state.ports.find((p) => p.name === state.conn.port)?.serialNumber ?? null;
  const sig = `${state.conn.port}|${serial}|${state.conn.info?.imei}|${model}`;
  if (sig === lastRemembered) return;
  lastRemembered = sig;
  rememberModel({ port: state.conn.port, serial, imei: state.conn.info?.imei }, model, hw);
}

function refreshStatus() {
  noteProbedModel();
  const chip = app.querySelector('[data-role="tb-chip"]');
  const actions = app.querySelector('[data-role="tb-actions"]');
  const statusEl = app.querySelector("footer.status");
  if (!chip || !actions || !statusEl) return render();
  chip.innerHTML = chipHtml(chipView());
  patchActions();
  statusEl.outerHTML = statusBar();
  refreshProgress();
  // Screens that show connection state are plain markup (no inputs), so repaint them.
  if (state.section === "devices" || state.section === "overview" || state.section === "status" || state.section === "log") {
    const content = app.querySelector('[data-role="content"]');
    if (content) content.innerHTML = contentHtml();
  }
}

function refreshProgress() {
  const slot = app.querySelector('[data-role="progress"]');
  if (slot) slot.innerHTML = progressBar();
  const sk = app.querySelector<HTMLElement>('[data-role="skeleton"]');
  if (sk) {
    const reading = state.working === "Reading configuration";
    sk.hidden = !reading;
    sk.innerHTML = reading ? Array.from({ length: 8 }, () => `<div class="sk-row"><div class="sk sk-l"></div><div class="sk sk-r"></div></div>`).join("") : "";
  }
}

// Dialogs live outside #app so a re-render never wipes what is being typed.
const modalRoot = document.createElement("div");
document.body.append(modalRoot);

function renderModal() {
  const m = state.modal;
  if (state.review) {
    modalRoot.innerHTML = `<div class="backdrop"><div class="dialog review" role="dialog" aria-modal="true" aria-label="Review changes">${reviewHtml(state.review)}</div></div>`;
    modalRoot.querySelector<HTMLElement>("[data-action='rv-write']:not([disabled]), [data-action='rv-close']")?.focus();
    return;
  }
  if (!m) {
    modalRoot.innerHTML = "";
    return;
  }
  // Keep the keyword field (and its focus) when only the error line changes.
  if (m.kind === "login" && modalRoot.querySelector('[data-role="keyword"]')) {
    const err = modalRoot.querySelector(".error");
    if (err) err.textContent = m.error ?? "";
    return;
  }
  const retries = state.conn.security?.retriesLeft;
  const body = {
    login: `<h3>Device keyword</h3><p class="muted">This device is protected. Enter its keyword to read or change its configuration.${retries !== undefined && retries < 5 ? ` ${retries} attempts left.` : ""}</p>
      <input type="password" data-role="keyword" autocomplete="off" spellcheck="false" autofocus />
      <p class="error">${esc(m.error ?? "")}</p>`,
    reboot: `<h3>Reboot device?</h3><p>The device restarts and the connection closes. Unsaved changes on the device are discarded.</p>`,
    reset: `<h3>Reset to defaults?</h3><p>This erases the device's configuration and restores factory defaults. It can't be undone.</p><label class="muted" for="reset-confirm">Type RESET to confirm</label><input id="reset-confirm" data-role="reset-confirm" autocomplete="off" spellcheck="false" />`,
    "save-options": `<h3>Save options</h3>
      <label class="check"><input type="checkbox" data-role="include-keyword" /> Include device keyword</label>
      <p class="muted">Off by default. The file is not encrypted, so a saved keyword can be read by anyone who opens it.</p>`,
    notice: `<h3>${esc(m.text?.split("\n")[0] ?? "")}</h3><p>${esc(m.text?.split("\n").slice(1).join(" ") ?? "")}</p>`,
  }[m.kind];
  const confirmLabel = { login: "Log in", reboot: "Reboot", reset: "Reset", notice: "OK", "save-options": "Save" }[m.kind];
  const cancel = m.kind === "notice" ? "" : `<button data-action="modal-cancel">Cancel</button>`;
  modalRoot.innerHTML = `<div class="backdrop"><div class="dialog" role="dialog">${body}<div class="dialog-actions">${cancel}<button class="primary" data-action="modal-ok">${confirmLabel}</button></div></div></div>`;
  modalRoot.querySelector<HTMLInputElement>('[data-role="keyword"]')?.focus();
  const typed = modalRoot.querySelector<HTMLInputElement>('[data-role="reset-confirm"]');
  if (typed) {
    typed.focus();
    modalRoot.querySelector<HTMLButtonElement>('[data-action="modal-ok"]')!.disabled = true;
  }
}

/** Update counts, nav badges and the action buttons without re-rendering the rows, so inputs keep focus. */
function refreshChrome() {
  const doc = state.doc!;
  const counts = app.querySelector('[data-role="counts"]');
  if (counts) counts.textContent = `${changedCount(doc)} changed · ${doc.entries.length.toLocaleString()} entries`;
  patchActions();
  doc.pages.forEach((p, i) => {
    const el = app.querySelector<HTMLElement>(`[data-nav-count="page:${i}"]`);
    if (!el) return;
    const n = pageIds(p).filter((id) => isChanged(doc, id)).length;
    el.textContent = String(n);
    el.hidden = n === 0;
  });
}

// ---------------------------------------------------------------- toasts

const toastRoot = document.createElement("div");
toastRoot.className = "toasts";
toastRoot.setAttribute("role", "status");
toastRoot.setAttribute("aria-live", "polite");
document.body.append(toastRoot);

/** Bottom-right, auto-dismissing feedback. Failures use inline messages or dialogs instead. */
function toast(text: string, kind: "info" | "success" = "info") {
  const el = document.createElement("div");
  el.className = `toast ${kind}`;
  el.textContent = text;
  toastRoot.append(el);
  setTimeout(() => el.remove(), 4000);
}

// ---------------------------------------------------------------- actions

async function refreshPorts() {
  try {
    state.ports = await listPorts();
    state.portsAt = Date.now();
    state.portsError = "";
  } catch (error) {
    state.portsError = `Could not list serial ports: ${error}`;
  }
  render();
}

async function openFile() {
  const doc = state.doc;
  if (doc && changedCount(doc) && !confirm("Discard unsaved changes?")) return;
  const path = await pickCfgToOpen();
  if (!path) return;
  try {
    const file = await openCfg(path);
    await openDoc({ path, source: "file", entries: file.entries, original: file.entries.map((e) => e.value), ...emptyDocParts() });
    state.warnings = [];
    state.message = `Opened ${fileName(path)}`;
  } catch (error) {
    state.message = `Could not open file: ${error}`;
  }
  render();
}

const hasKeyword = (doc: Doc) => doc.entries.some((e) => e.key === "10");

async function saveTo(path: string, includeKeyword: boolean) {
  const doc = state.doc!;
  try {
    state.warnings = await saveCfg(path, { entries: doc.entries }, includeKeyword);
    doc.path = path;
    doc.original = doc.entries.map((e) => e.value);
    state.message = `Saved ${fileName(path)}${state.warnings.length ? " with warnings" : ""}`;
  } catch (error) {
    state.warnings = [];
    notice(`Not saved\n${String(error).replace(/^Not saved: /, "")}`);
    state.message = "Could not save";
  }
  render();
}

/** Save or Save as; asks about the keyword first when the configuration has one. */
function startSave(saveAs: boolean) {
  if (hasKeyword(state.doc!)) {
    state.modal = { kind: "save-options", saveAs };
    return renderModal();
  }
  void finishSave(saveAs, false);
}

async function finishSave(saveAs: boolean, includeKeyword: boolean) {
  const doc = state.doc!;
  const path = !saveAs && doc.path ? doc.path : await pickCfgToSave(doc.path ?? undefined);
  if (path) await saveTo(path, includeKeyword);
}

const save = () => startSave(false);
const saveAs = () => startSave(true);

// ---------------------------------------------------------------- device actions

function closeModal() {
  state.modal = null;
  renderModal();
}

function notice(text: string) {
  state.modal = { kind: "notice", text };
  renderModal();
}

function openLogin() {
  state.modal = { kind: "login" };
  renderModal();
}

/** Runs one device operation with the busy/progress display around it. */
async function device<T>(label: string, job: () => Promise<T>): Promise<T | undefined> {
  state.working = label;
  state.progress = null;
  refreshStatus();
  try {
    return await job();
  } finally {
    state.working = "";
    state.progress = null;
    refreshStatus();
  }
}

async function connect(port: string, kind: PortInfo["kind"]) {
  state.connectingPort = port;
  state.message = "";
  try {
    const result = await connectDevice(port, kind);
    state.conn = result.status;
    state.message = `Connected to ${port}`;
    refreshStatus();
    if (result.locked) openLogin();
  } catch (error) {
    state.message = /cancelled/i.test(String(error)) ? "Connection cancelled." : `Could not connect: ${error}`;
    refreshStatus();
  } finally {
    state.connectingPort = null;
  }
}

/** Aborts a connect in progress: the backend stops the session and closes the port. */
async function cancelConnect() {
  state.message = "Cancelling…";
  try {
    await disconnectDevice();
  } catch (error) {
    state.message = `Could not cancel: ${error}`;
  }
  refreshStatus();
}

async function disconnect() {
  try {
    await disconnectDevice();
    state.message = "Disconnected";
  } catch (error) {
    state.message = `Could not disconnect: ${error}`;
  }
  closeModal();
  refreshStatus();
}

async function loadFromDevice() {
  const doc = state.doc;
  if (doc && changedCount(doc) && !confirm("Discard unsaved changes?")) return;
  try {
    const file = await device("Reading configuration", readDeviceConfig);
    if (!file) return;
    await openDoc({ path: null, source: "device", entries: file.entries, original: file.entries.map((e) => e.value), ...emptyDocParts() });
    state.message = `Read ${file.entries.length.toLocaleString()} entries from the device`;
  } catch (error) {
    state.message = `Could not read the device: ${error}`;
  }
  render();
}


async function saveToDevice() {
  const doc = state.doc!;
  const bad = blockingErrors(doc);
  if (bad.length) {
    const lines = bad.slice(0, 6).map((id) => `${nameOf(doc, id)} (#${id}): ${doc.parseErrors.get(id) ?? doc.errors.get(id)}`);
    return notice(`Fix invalid values first\n${bad.length} value${bad.length > 1 ? "s are" : " is"} invalid, so nothing was sent. ${lines.join("; ")}${bad.length > 6 ? "; …" : ""}`);
  }
  if (doc.source === "device" && !changedCount(doc)) {
    toast("Nothing changed, so nothing to send");
    return;
  }
  // "Write to device" always opens the review first; the existing safety check feeds its warnings.
  const warnings: Warning[] = [];
  try {
    const check = await checkWriteTarget({ entries: doc.entries });
    if (check.verdict === "block") warnings.push({ level: "block", text: `Wrong hardware, not sent. ${check.message}` });
    else if (check.verdict === "confirm") warnings.push({ level: "confirm", text: check.message });
  } catch (error) {
    // A device read always matches itself; for a file the check failing is a reason not to send.
    if (doc.source === "file") warnings.push({ level: "block", text: `Could not check the file: ${error}` });
  }
  let view: ReviewView;
  if (doc.source === "device") {
    const rows = buildReviewRows({ entries: doc.entries, original: doc.original, defOf: (id) => defOf(doc, id), nameOf: (id) => nameOf(doc, id), groupOf: (id) => doc.groupOf.get(id) ?? "" });
    if (rows.some((r) => r.id === KEYWORD_ID)) warnings.push({ level: "note", text: "This changes the device keyword (shown masked). Write it down: without it you cannot read or change the device again." });
    view = { rows, selected: new Set(rows.map((r) => r.id)), warnings, confirmed: false, phase: "review" };
  } else {
    // A file was never read from this device, so there is no baseline to diff against.
    if (hasKeyword(doc)) warnings.push({ level: "note", text: "This file contains a device keyword; it will replace the keyword on the device." });
    view = { rows: [], selected: new Set(), warnings, confirmed: false, phase: "review", wholeFile: doc.entries.filter(isParameter).length };
  }
  state.review = view;
  renderModal();
}

function closeReview() {
  state.review = null;
  renderModal();
}

/** Repaint the parts of the open review that depend on selection, without touching the checkboxes. */
function patchReview() {
  const v = state.review;
  if (!v) return;
  const count = modalRoot.querySelector('[data-role="rv-count"]');
  if (count) count.textContent = `${v.selected.size} of ${v.rows.length} selected`;
  modalRoot.querySelectorAll<HTMLElement>(".rv-row").forEach((li) => {
    const id = Number(li.querySelector<HTMLInputElement>("[data-id]")?.dataset.id);
    li.classList.toggle("off", !v.selected.has(id));
  });
  const write = modalRoot.querySelector<HTMLButtonElement>('[data-action="rv-write"]');
  if (write) {
    write.disabled = !canWrite(v);
    write.textContent = v.phase === "writing" ? "Writing…" : v.wholeFile ? `Write ${v.wholeFile.toLocaleString()} parameters` : `Write ${v.selected.size} change${v.selected.size === 1 ? "" : "s"}`;
  }
  const discard = modalRoot.querySelector<HTMLButtonElement>('[data-action="rv-discard"]');
  if (discard) discard.disabled = !v.selected.size;
}

function discardSelected() {
  const v = state.review, doc = state.doc;
  if (!v || !doc) return;
  for (const id of v.selected) {
    const i = doc.index.get(id);
    if (i === undefined) continue;
    doc.entries[i].value = doc.original[i];
    doc.parseErrors.delete(id);
  }
  v.rows = v.rows.filter((r) => !v.selected.has(r.id));
  v.selected = new Set();
  recomputeErrors(doc);
  rerenderRows();
  refreshChrome();
  toast("Selected changes discarded");
  if (!v.rows.length) return closeReview();
  renderModal();
}

async function writeReviewed() {
  const v = state.review, doc = state.doc;
  if (!v || !doc || !canWrite(v)) return;
  const send = v.wholeFile ? doc.entries.filter(isParameter) : entriesToSend(doc.entries, v.selected);
  v.phase = "writing";
  v.progress = null;
  renderModal();
  try {
    await device("Writing configuration", () => writeDeviceConfig(send));
    // Only what was sent is now the device's value; unticked edits stay pending.
    const sent = new Set(send.map((e) => e.key));
    doc.entries.forEach((e, i) => sent.has(e.key) && (doc.original[i] = e.value));
    v.phase = "done";
    v.sent = send.length;
    state.message = `Saved ${send.length.toLocaleString()} parameters to the device`;
    toast(`Saved ${send.length.toLocaleString()} parameter${send.length === 1 ? "" : "s"} to the device`, "success");
  } catch (error) {
    v.phase = "aborted";
    v.error = String(error);
    state.message = "Writing to the device failed";
  }
  if (state.review === v) renderModal();
  rerenderRows();
  refreshChrome();
}

async function confirmModal() {
  const m = state.modal;
  if (!m) return;
  if (m.kind === "login") return void submitLogin();
  if (m.kind === "reset" && modalRoot.querySelector<HTMLInputElement>('[data-role="reset-confirm"]')?.value.trim() !== "RESET") return;
  const includeKeyword = modalRoot.querySelector<HTMLInputElement>('[data-role="include-keyword"]')?.checked ?? false;
  closeModal();
  if (m.kind === "save-options") return void finishSave(!!m.saveAs, includeKeyword);
  if (m.kind === "reboot") {
    try {
      await device("Rebooting", rebootDevice);
      state.message = "Device is rebooting; reconnect in a few seconds";
    } catch (error) {
      state.message = `Reboot failed: ${error}`;
    }
  } else if (m.kind === "reset") {
    try {
      await device("Resetting to defaults", resetDefaults);
      state.message = "Device reset to defaults. Load from device to see the new values";
    } catch (error) {
      state.message = `Reset failed: ${error}`;
    }
  }
  render();
}

async function submitLogin() {
  const field = modalRoot.querySelector<HTMLInputElement>('[data-role="keyword"]');
  if (!field || !field.value) return;
  const keyword = field.value;
  field.value = "";
  try {
    await device("Logging in", () => loginDevice(keyword));
    closeModal();
    state.message = "Logged in";
  } catch (error) {
    if (state.modal) state.modal.error = String(error);
    renderModal();
    modalRoot.querySelector<HTMLInputElement>('[data-role="keyword"]')?.focus();
  }
}

function close() {
  const doc = state.doc;
  if (doc && changedCount(doc) && !confirm("Discard unsaved changes?")) return;
  state.doc = null;
  state.section = "devices";
  state.filter = "";
  state.warnings = [];
  state.message = "";
  render();
  void refreshPorts();
}

function revert(ids: number[]) {
  const doc = state.doc!;
  for (const id of ids) {
    const i = doc.index.get(id)!;
    doc.entries[i].value = doc.original[i];
    doc.parseErrors.delete(id);
  }
  recomputeErrors(doc);
  rerenderRows();
  refreshChrome();
}

function rerenderRows() {
  const doc = state.doc!;
  const rows = app.querySelector('[data-role="rows"]');
  if (!rows) return render();
  const page = state.section.startsWith("page:") ? doc.pages[Number(state.section.slice(5))] : null;
  rows.innerHTML = page ? groupNode(doc, page, 0, { left: 600 }) : allRows(doc);
  refreshRows(doc);
}

function closeMenus() {
  for (const m of app.querySelectorAll(".menu-wrap.open")) m.classList.remove("open");
}

app.addEventListener("click", (event) => {
  if (!(event.target as HTMLElement).closest('[data-action="menu"]')) closeMenus();
  const target = (event.target as HTMLElement).closest<HTMLElement>("[data-action]");
  if (!target || (target as HTMLButtonElement).disabled) return;
  switch (target.dataset.action) {
    case "open-file":
      return void openFile();
    case "load-definitions":
      return void loadDefs();
    case "save-file":
      return save();
    case "connect":
      return void connect(target.dataset.port!, target.dataset.kind as PortInfo["kind"]);
    case "disconnect":
      return void disconnect();
    case "cancel-connect":
      return void cancelConnect();
    case "goto-nearby":
      return void document.querySelector("[data-nbt-addr]")?.scrollIntoView({ block: "center" });
    case "login":
      return openLogin();
    case "load-device":
      return void loadFromDevice();
    case "save-device":
      return saveToDevice();
    case "reboot":
      return void (state.modal = { kind: "reboot" }, renderModal());
    case "reset":
      return void (state.modal = { kind: "reset" }, renderModal());
    case "save-as":
      return saveAs();
    case "dismiss-warnings":
      state.warnings = [];
      return void (app.querySelector('[data-role="warnings"]')!.innerHTML = "");
    case "close":
      return close();
    case "ble-scan":
      return void toggleBleScan(true);
    case "ble-stop":
      return void toggleBleScan(false);
    case "refresh-ports":
      return void refreshPorts();
    case "log-start":
      return void toggleLog(true);
    case "log-stop":
      return void toggleLog(false);
    case "log-clear":
      live.logLines = [];
      return void paintLog();
    case "log-save":
      return void saveLog();
    case "pin-io": {
      const id = Number(target.dataset.id);
      if (live.pinned.has(id)) live.pinned.delete(id);
      else live.pinned.add(id);
      return paintStatus();
    }
    case "record-toggle":
      return void toggleRecording();
    case "poll-toggle": {
      live.paused = !live.paused;
      syncPolling();
      const pane = app.querySelector('[data-role="content"]');
      if (pane && state.section === "status") pane.innerHTML = statusView();
      return;
    }
    case "section":
      return setSection(target.dataset.section as Section);
    case "goto-devices":
      return setSection("devices");
    case "nav-group": {
      const g = target.dataset.group!;
      if (!state.collapsed.delete(g)) state.collapsed.add(g);
      return void (app.querySelector("aside.sidebar")!.innerHTML = sidebar());
    }
    case "menu": {
      const wrap = target.closest(".menu-wrap")!;
      const open = wrap.classList.contains("open");
      closeMenus();
      if (!open) wrap.classList.add("open");
      return;
    }
    case "copy": {
      void navigator.clipboard?.writeText(target.dataset.text ?? "").then(() => toast("Copied"), () => toast("Could not copy"));
      return;
    }
    case "reset-default": {
      const doc = state.doc!;
      for (const id of target.dataset.ids!.split(",").map(Number)) {
        const d = defOf(doc, id);
        const i = doc.index.get(id);
        if (!d || d.default === null || i === undefined) continue;
        doc.entries[i].value = d.default;
        doc.parseErrors.delete(id);
      }
      recomputeErrors(doc);
      rerenderRows();
      return refreshChrome();
    }
    case "revert":
      return revert(target.dataset.ids!.split(",").map(Number));
    case "clear-slot": {
      const doc = state.doc!;
      const id = Number(doc.entries[Number(target.dataset.index)].key);
      doc.entries[Number(target.dataset.index)].value = "";
      doc.parseErrors.delete(id);
      rerenderRows();
      return refreshChrome();
    }
    case "reveal": {
      const field = target.previousElementSibling as HTMLInputElement;
      field.type = field.type === "password" ? "text" : "password";
      target.textContent = field.type === "password" ? "Show" : "Hide";
      return;
    }
    case "remove-definitions":
      return void removeDefs(target.dataset.version!);
  }
});

function onEdit(event: Event) {
  const target = event.target as HTMLElement;
  const role = (target as HTMLInputElement).dataset?.role;
  if (role === "log-filter") {
    live.logFilter = (target as HTMLInputElement).value;
    return paintLog();
  }
  if (role === "poll-interval") {
    live.intervalMs = Number((target as HTMLSelectElement).value);
    if (live.polling) {
      live.polling = false;
      void stopStatusPolling().catch(() => {}).then(syncPolling);
    }
    return;
  }
  const doc = state.doc;
  if (!doc) return;
  if ((target as HTMLInputElement).dataset?.role === "filter") {
    state.filter = (target as HTMLInputElement).value;
    if (state.section !== "parameters" && state.filter.trim()) {
      // Global search: results appear in All parameters. Only the content pane changes, so the box keeps focus.
      state.section = "parameters";
      const side = app.querySelector("aside.sidebar");
      if (side) side.innerHTML = sidebar();
      app.querySelector('[data-role="content"]')!.innerHTML = contentHtml();
      return;
    }
    if (state.section === "parameters") rerenderRows();
    return;
  }
  const el = target.closest<HTMLElement>("[data-index]");
  if (!el) return;
  const index = Number(el.dataset.index);
  const id = Number(doc.entries[index].key);
  const def = defOf(doc, id);
  const raw = def ? readControl(el, def) : (el as HTMLInputElement).value;
  if (raw === null) {
    doc.parseErrors.set(id, "Use the format hh:mm");
  } else {
    doc.parseErrors.delete(id);
    doc.entries[index].value = raw;
  }
  refreshRows(doc);
  refreshChrome();
}
app.addEventListener("input", onEdit);
app.addEventListener("change", onEdit);

window.addEventListener("keydown", (event) => {
  if (event.key === "Escape") closeMenus();
  const active = document.activeElement as HTMLElement | null;
  if ((event.key === "ArrowDown" || event.key === "ArrowUp") && active && !(active instanceof HTMLInputElement) && !(active instanceof HTMLSelectElement)) {
    // Arrow keys move through the sidebar and the device list.
    const list = active.closest("aside.sidebar") ? [...app.querySelectorAll<HTMLElement>("aside.sidebar .nav-item:not([disabled])")] : active.closest(".drow") ? [...app.querySelectorAll<HTMLElement>(".drow:not(.dhead)")] : null;
    const at = list ? list.findIndex((el) => el === active || el.contains(active)) : -1;
    if (list && at >= 0) {
      event.preventDefault();
      list[Math.min(list.length - 1, Math.max(0, at + (event.key === "ArrowDown" ? 1 : -1)))].focus();
    }
  }
  if (event.key === "Enter" && active?.classList.contains("drow")) active.querySelector<HTMLButtonElement>('[data-action="connect"]:not([disabled])')?.click();
  if (!(event.metaKey || event.ctrlKey)) return;
  if (event.key.toLowerCase() === "k") {
    event.preventDefault();
    const box = app.querySelector<HTMLInputElement>('[data-role="filter"]');
    if (box && !box.disabled) {
      box.focus();
      box.select();
    } else toast("Open a file or read a device to search its settings");
  } else if (event.key === "o") {
    event.preventDefault();
    void openFile();
  } else if (event.key.toLowerCase() === "w" && event.shiftKey && state.doc && canUseDevice()) {
    event.preventDefault();
    void saveToDevice();
  } else if (event.key === "s" && state.doc) {
    event.preventDefault();
    (event.shiftKey ? saveAs() : save());
  }
});

modalRoot.addEventListener("click", (event) => {
  const action = (event.target as HTMLElement).closest<HTMLElement>("[data-action]")?.dataset.action;
  if (action === "rv-write") void writeReviewed();
  else if (action === "rv-close") closeReview();
  else if (action === "rv-discard") discardSelected();
  else if (action === "rv-back") {
    if (state.review) state.review.phase = "review";
    renderModal();
  } else if (action === "modal-cancel") closeModal();
  else if (action === "modal-ok") void confirmModal();
});
modalRoot.addEventListener("input", (event) => {
  const el = event.target as HTMLInputElement;
  if (el.dataset.role === "reset-confirm") modalRoot.querySelector<HTMLButtonElement>('[data-action="modal-ok"]')!.disabled = el.value.trim() !== "RESET";
});
modalRoot.addEventListener("change", (event) => {
  const v = state.review;
  const el = event.target as HTMLInputElement;
  if (!v) return;
  if (el.dataset.role === "rv-check") {
    const id = Number(el.dataset.id);
    if (el.checked) v.selected.add(id);
    else v.selected.delete(id);
    patchReview();
  } else if (el.dataset.role === "rv-confirm") {
    v.confirmed = el.checked;
    patchReview();
  }
});
modalRoot.addEventListener("keydown", (event) => {
  if (event.key === "Enter" && state.modal?.kind === "login") void confirmModal();
});
window.addEventListener("keydown", (event) => {
  if (event.key !== "Escape") return;
  if (state.review) {
    if (state.review.phase !== "writing") closeReview();
  } else if (state.modal) closeModal();
});

void onConnectionChanged((status) => {
  state.conn = status;
  if (status.state !== "connected" && status.state !== "busy") connectionDropped();
  syncPolling();
  refreshStatus();
  if (state.section === "log") paintLog();
});
void onDeviceLog((lines) => {
  live.logLines = pushLines(live.logLines, lines, LOG_CAP);
  if (state.section !== "log") return;
  const box = app.querySelector<HTMLElement>('[data-role="log-box"]');
  if (!box) return;
  if (live.logFilter.trim()) return paintLog();
  box.insertAdjacentHTML("beforeend", filterLines(lines, "").map(logRowHtml).join(""));
  while (box.childElementCount > LOG_CAP) box.firstElementChild!.remove();
  if (live.following) box.scrollTop = box.scrollHeight;
  paintLogFoot();
});
void onDeviceStatus((ev) => {
  live.last = ev;
  live.lost = false;
  if (ev.ok && ev.rttMs !== null) live.rtt = [...live.rtt, ev.rttMs].slice(-300);
  if (ev.ok) {
    for (const f of ev.fields) {
      if (f.num === null) continue;
      if (f.id === 773 || f.id === 774) live.hist.push(String(f.id), ev.atMs, f.num);
      else if (f.io && live.pinned.has(f.id)) live.hist.push(`io${f.id}`, ev.atMs, f.num);
    }
    if (live.recording && live.recorded.length < 500_000) live.recorded.push(...csvRows(ev.atMs, ev.fields, state.doc?.model?.io_items));
    const pos = position(byId(ev.fields));
    if (pos && state.section === "status") void showPosition(pos.lat, pos.lon);
  }
  if (state.section !== "status") return;
  paintStatus();
});

function paintStatus() {
  const body = app.querySelector('[data-role="live-body"]');
  if (body) body.innerHTML = statusBodyHtml(statusViewModel());
  const rec = app.querySelector<HTMLButtonElement>('[data-action="record-toggle"]');
  if (rec) rec.textContent = live.recording ? `Stop and save CSV (${live.recorded.length} rows)` : "Record session…";
}

/** Keeps the map element inside its slot after any re-render of the Telemetry page. */
new MutationObserver(() => {
  const slot = state.section === "status" ? app.querySelector<HTMLElement>('[data-role="map-slot"]') : null;
  if (!slot) return detachMap();
  if (slot.firstElementChild?.classList.contains("mapbox")) return;
  attachMap(slot, (offline) => {
    if (offline === live.offline) return;
    live.offline = offline;
    paintStatus();
  });
}).observe(app, { childList: true, subtree: true });
window.addEventListener("offline", () => ((live.offline = true), state.section === "status" && paintStatus()));
window.addEventListener("online", () => ((live.offline = isOffline()), state.section === "status" && paintStatus()));
// Pause-on-scroll-up: the log follows new lines only while scrolled to the bottom.
app.addEventListener("scroll", (e) => {
  const el = e.target as HTMLElement;
  if (el.dataset?.role !== "log-box") return;
  const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
  if (atBottom !== live.following) {
    live.following = atBottom;
    paintLogFoot();
  }
}, true);
document.addEventListener("visibilitychange", syncPolling);
window.addEventListener("beforeunload", () => {
  void stopLog().catch(() => {});
  void stopStatusPolling().catch(() => {});
});
setInterval(() => {
  const el = app.querySelector('[data-role="ago"]');
  if (el && state.section === "status") el.textContent = `Updated ${agoText(Date.now(), live.last?.atMs ?? null)}`;
  if (state.section === "status" && statusViewModel().waiting !== !!app.querySelector('[data-role="waiting"]')) paintStatus();
}, 1000);
void onBleDevice(patchBleDevice);
void onBleScanState((status) => {
  state.ble.status = status;
  patchBleHead();
});
void onProgress((progress) => {
  state.progress = progress;
  refreshProgress();
  if (state.review?.phase === "writing") {
    state.review.progress = progress;
    const slot = modalRoot.querySelector('[data-role="rv-progress"]');
    if (slot) slot.innerHTML = progressHtml(progress);
  }
});

installPictureObserver();
onPresentationChange(() => refreshStatus());
render();
initNearbyBt();
initDiagnostics();
forwardConsole();
void connectionStatus().then((status) => {
  state.conn = status;
  refreshStatus();
});
void refreshPorts();

void definitionsSummary().then((d) => {
  state.defs = d;
  state.defsEpoch++;
  resetPresentation();
  render();
  void restoreDefs();
});
