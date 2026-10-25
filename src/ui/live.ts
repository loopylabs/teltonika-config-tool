// Live data views: the Status dashboard and the Device log console.
import type { LogLine, PortKind, StatusEvent, StatusField } from "../api.ts";
import { WAITING_TILES, byId, ioTableHtml, position, sparkCardsHtml, tiles as tilesOf, type History, type IoRow, type SparkSpec, type Tile } from "./telemetry.ts";
const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
import { icon } from "./icons.ts";

export const LOG_CAP = 10_000;
export const POLL_CHOICES = [1000, 2000, 5000, 10000];
/** Warn after this many failed polls in a row. */
export const WARN_AFTER = 3;

const pad = (n: number, w = 2) => String(n).padStart(w, "0");

/** HH:MM:SS.mmm in local time. */
export function stamp(atMs: number): string {
  const d = new Date(atMs);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${pad(d.getMilliseconds(), 3)}`;
}

export const lineText = (l: LogLine) => `${stamp(l.atMs)}  ${l.text}`;

/** Appends and keeps only the newest `cap` lines. */
export function pushLines(buffer: LogLine[], lines: LogLine[], cap = LOG_CAP): LogLine[] {
  const all = buffer.concat(lines);
  return all.length > cap ? all.slice(all.length - cap) : all;
}

export function filterLines(lines: LogLine[], needle: string): LogLine[] {
  const n = needle.trim().toLowerCase();
  return n ? lines.filter((l) => l.text.toLowerCase().includes(n)) : lines;
}

export function agoText(nowMs: number, atMs: number | null): string {
  if (atMs === null) return "not yet";
  const s = Math.max(0, Math.round((nowMs - atMs) / 1000));
  return s <= 1 ? "just now" : `${s} s ago`;
}

export type LinkHealth = { text: string; level: "ok" | "warn" | "off" };
export function linkHealth(ev: StatusEvent | null, lost: boolean): LinkHealth {
  if (lost) return { text: "Link lost", level: "warn" };
  if (!ev) return { text: "Waiting for first reply", level: "off" };
  if (ev.failures >= WARN_AFTER) return { text: `${ev.failures} polls in a row got no reply`, level: "warn" };
  if (ev.failures > 0) return { text: `${ev.failures} missed ${ev.failures === 1 ? "poll" : "polls"}`, level: "warn" };
  return { text: ev.rttMs === null ? "OK" : `Last round trip ${ev.rttMs} ms`, level: "ok" };
}

// ---------------------------------------------------------------- log

export type LogView = { running: boolean; connected: boolean; busy: boolean; filter: string; total: number; shown: number; following: boolean };

export function logPaneHtml(v: LogView): string {
  return `<section class="pane">
    <header class="phead"><div><h1>Device log</h1><p class="muted">The tracker's debug output, stamped when received. It pauses while a command runs.</p></div></header>
    <section class="card logcard">
      <div class="logbar">
        <button class="${v.running ? "" : "primary"}" data-action="${v.running ? "log-stop" : "log-start"}" ${v.connected ? "" : "disabled"}>${v.running ? "Stop" : "Start"}</button>
        <label class="logfilter"><span class="sr-only">Filter log</span><input data-role="log-filter" type="search" placeholder="Filter lines" value="${esc(v.filter)}" autocomplete="off" spellcheck="false" /></label>
        <button data-action="log-clear" ${v.total ? "" : "disabled"}>Clear</button>
        <button data-action="log-save" ${v.total ? "" : "disabled"}>Save log…</button>
      </div>
      <div class="logbox mono" data-role="log-box" tabindex="0" aria-label="Device log" aria-live="off"></div>
      <div class="logfoot muted" data-role="log-foot">${logFootText(v)}</div>
    </section></section>`;
}

export const logRowHtml = (l: LogLine) => `<div class="ll"><span class="lt">${stamp(l.atMs)}</span><span class="lx">${esc(l.text)}</span></div>`;

export function logFootText(v: LogView): string {
  const state = !v.connected ? "Connect to a device to start the log." : v.running ? (v.busy ? "Paused while a command runs…" : "Streaming") : "Stopped";
  const count = v.filter.trim() ? `${v.shown.toLocaleString()} of ${v.total.toLocaleString()} lines` : `${v.total.toLocaleString()} lines`;
  return `${state} · ${count}${v.running && !v.following ? " · scrolling paused (scroll to the bottom to follow)" : ""} · keeps the last ${LOG_CAP.toLocaleString()}`;
}

// ---------------------------------------------------------------- status dashboard

export type StatusView = {
  connected: boolean;
  kind: PortKind | null;
  paused: boolean;
  intervalMs: number;
  last: StatusEvent | null;
  lost: boolean;
  rtt: number[];
  now: number;
  message: string | null;
  /** Polling is on but nothing has come back for longer than a poll should take. */
  waiting: boolean;
  io: IoRow[];
  pinned: Set<number>;
  hist: History;
  sparks: SparkSpec[];
  /** Offline map fallback: show the coordinates instead. */
  offline: boolean;
  recording: boolean;
  recordedRows: number;
};

export function sparkline(values: number[], w = 120, h = 28): string {
  if (values.length < 2) return `<svg class="spark" width="${w}" height="${h}" aria-hidden="true"></svg>`;
  const min = Math.min(...values);
  const max = Math.max(...values);
  const span = max - min || 1;
  const pts = values.map((v, i) => `${((i / (values.length - 1)) * (w - 2) + 1).toFixed(1)},${(h - 3 - ((v - min) / span) * (h - 6)).toFixed(1)}`).join(" ");
  return `<svg class="spark" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}" aria-hidden="true"><polyline points="${pts}" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round" stroke-linecap="round"/></svg>`;
}

function fieldCard(f: StatusField): string {
  const raw = f.defined ? "" : `<span class="badge warn" title="The released spec doesn't define this value">raw</span>`;
  return `<div class="fcard ${f.defined ? "" : "undef"}"><div class="flabel">${esc(f.label)}${raw}</div><div class="fval ${f.defined ? "" : "mono"}">${esc(f.display)}${f.unit ? `<small>${esc(f.unit)}</small>` : ""}</div><div class="fid mono">#${f.id} · type ${f.typeCode}</div></div>`;
}

function tileHtml(t: Tile): string {
  return `<div class="tile ${t.missing ? "dim" : ""}"><div class="flabel">${esc(t.label)}</div><div class="fval ${t.value.length > 16 ? "small" : ""}">${esc(t.value)}</div>${t.sub ? `<div class="fid muted">${esc(t.sub)}</div>` : ""}</div>`;
}

export function statusBodyHtml(v: StatusView): string {
  const health = linkHealth(v.last, v.lost);
  const wire = v.kind === "bluetooth" ? "Bluetooth" : "USB";
  const warn = v.lost
    ? `<div class="notice warn" role="alert">${icon("warn")}<div><strong>The ${wire} link dropped.</strong> Polling has stopped. <button class="link" data-action="goto-devices">Reconnect</button></div></div>`
    : health.level === "warn" && v.last && v.last.failures >= WARN_AFTER
      ? `<div class="notice warn" role="alert">${icon("warn")}<div><strong>No reply from the tracker.</strong> ${v.last.failures} polls in a row failed; trying again every 30 s${v.kind === "bluetooth" ? ". Check range and that no other program is connected" : ""}.</div></div>`
      : "";
  const fields = v.last?.fields ?? [];
  const tiles = fields.length ? [...tilesOf(fields).map(tileHtml), ...WAITING_TILES.map((t) => `<div class="tile wait"><div class="flabel">${esc(t)}</div><div class="fval muted">Waiting for spec</div></div>`)].join("") : "";
  const groups = new Map<string, StatusField[]>();
  for (const f of fields) if (!f.io && (f.group === "Other" || f.group === "CAN")) groups.set(f.group, [...(groups.get(f.group) ?? []), f]);
  const cards = [...groups.keys()].map((g) => `<section class="fgroup"><h3>${g === "Other" ? "Other status values (not in the spec yet)" : esc(g)}</h3><div class="fgrid">${groups.get(g)!.map(fieldCard).join("")}</div></section>`).join("");
  const rttNow = v.rtt.length ? v.rtt[v.rtt.length - 1] : null;
  const state = v.waiting ? `<span class="muted" data-role="waiting">Waiting for the device… (a slow link can take up to 30 s per reply)</span>` : "";
  const pos = fields.length ? position(byId(fields)) : null;
  const coords = pos ? `${pos.lat.toFixed(7)}, ${pos.lon.toFixed(7)}` : "no valid position yet";
  const empty = fields.length ? "" : `<p class="muted empty">${v.connected ? "Waiting for the first status reply…" : "Connect to a device to see live status."}</p>`;
  const sparks = sparkCardsHtml(v.sparks, v.hist);
  return `${warn}
    <div class="linkrow"><span class="linkbadge">${icon(v.kind === "bluetooth" ? "bluetooth" : "usb")}${wire}</span><span class="lh ${health.level}"><span class="sdot ${health.level === "ok" ? "on" : health.level === "warn" ? "pending" : "off"}"></span>${esc(health.text)}</span><span class="muted" data-role="ago">Updated ${agoText(v.now, v.last?.atMs ?? null)}</span>${v.last?.extended ? `<span class="muted">extended status (all groups)</span>` : ""}${state}</div>
    ${empty}
    <div class="tiles">${tiles}</div>
    <section class="card mapcard"><div class="card-head"><h3>Map</h3><span class="muted mono">${esc(coords)}</span></div>
      <div data-role="map-slot" class="mapslot ${v.offline ? "off" : ""}"></div>
      ${v.offline ? `<p class="mapfallback">${icon("warn")}<span>Map tiles are not reachable (offline). Current position: <strong class="mono">${esc(coords)}</strong></span></p>` : ""}
    </section>
    <div class="tiles sparks">${sparks}</div>
    <section class="card"><div class="card-head"><h3>I/O elements</h3><span class="muted">${v.io.length} in the last reply</span></div>${ioTableHtml(v.io, v.pinned)}</section>
    <section class="card"><div class="card-head"><h3>Link round trip</h3><span class="muted">${rttNow === null ? "no data" : `${rttNow} ms`}</span></div><div class="sparkrow">${sparkline(v.rtt, 360, 44)}</div></section>
    ${cards}`;
}

export function statusPaneHtml(v: StatusView): string {
  const opts = POLL_CHOICES.map((ms) => `<option value="${ms}" ${ms === v.intervalMs ? "selected" : ""}>${ms / 1000} s</option>`).join("");
  return `<section class="pane">
    <header class="phead"><div><h1>Telemetry</h1><p class="muted">Live values polled from the tracker while this page is open. Each poll starts the chosen time after the previous one finished.</p></div>
      <div class="pollctl"><label>Every <select data-role="poll-interval" aria-label="Poll interval">${opts}</select></label>
      <button data-action="poll-toggle" ${v.connected && !v.lost ? "" : "disabled"}>${v.paused ? "Resume" : "Pause"}</button>
      <button data-action="record-toggle" ${v.connected && !v.lost || v.recording ? "" : "disabled"}>${v.recording ? `Stop and save CSV (${v.recordedRows} rows)` : "Record session…"}</button></div></header>
    <div data-role="live-body">${statusBodyHtml(v)}</div></section>`;
}
