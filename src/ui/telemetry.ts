// Telemetry helpers: tile values, I/O table, history for sparklines, CSV. Pure functions.
import type { IoElement, StatusField } from "../api.ts";

export const HISTORY_MS = 10 * 60 * 1000;
const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
const pad = (n: number) => String(n).padStart(2, "0");

export type Tile = { label: string; value: string; sub?: string; wait?: boolean; missing?: boolean };

export function formatVolts(mv: number): string {
  return `${(mv / 1000).toFixed(2)} V`;
}

export function formatDuration(totalSeconds: number): string {
  let s = Math.max(0, Math.round(totalSeconds));
  const d = Math.floor(s / 86400);
  s -= d * 86400;
  const h = Math.floor(s / 3600);
  s -= h * 3600;
  const m = Math.floor(s / 60);
  s -= m * 60;
  return d > 0 ? `${d} d ${h} h ${m} min` : h > 0 ? `${h} h ${m} min ${s} s` : m > 0 ? `${m} min ${s} s` : `${s} s`;
}

/** Unix seconds as `YYYY-MM-DD HH:MM:SS UTC`. */
export function formatUnix(seconds: number): string {
  const d = new Date(seconds * 1000);
  if (Number.isNaN(d.getTime())) return "—";
  return `${d.getUTCFullYear()}-${pad(d.getUTCMonth() + 1)}-${pad(d.getUTCDate())} ${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())}:${pad(d.getUTCSeconds())} UTC`;
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v < 10 ? 2 : 1)} ${units[i]}`;
}

export type Fields = Map<number, StatusField>;
export const byId = (fields: StatusField[]): Fields => new Map(fields.map((f) => [f.id, f]));

const num = (m: Fields, id: number) => m.get(id)?.num ?? null;
const DASH = "—";

/** Valid position, or null (each coordinate is valid only inside its range, 03 §7.6.4). */
export function position(m: Fields): { lat: number; lon: number } | null {
  const lat = num(m, 265);
  const lon = num(m, 266);
  return lat !== null && lon !== null && Math.abs(lat) <= 90 && Math.abs(lon) <= 180 ? { lat, lon } : null;
}

/** Tiles for the values 03 v6 defines. Raw enum numbers stay raw (texts are open question 70). */
export function tiles(fields: StatusField[]): Tile[] {
  const m = byId(fields);
  const n = (id: number, f: (v: number) => string) => {
    const v = num(m, id);
    return v === null ? DASH : f(v);
  };
  const out: Tile[] = [];
  const pos = position(m);
  const hasPos = m.has(265) || m.has(266);
  out.push({
    label: "Position",
    value: pos ? `${pos.lat.toFixed(7)}, ${pos.lon.toFixed(7)}` : hasPos ? "No valid position" : DASH,
    sub: pos ? "latitude, longitude" : undefined,
    missing: !pos,
  });
  out.push({ label: "Fix state", value: n(256, String), sub: `module state ${n(257, String)} · fix time ${n(258, formatDuration)} · texts not in the spec yet`, missing: !m.has(256) });
  const ext = num(m, 773);
  out.push({ label: "External voltage", value: ext === null ? DASH : ext === 0 ? "On battery" : formatVolts(ext), missing: ext === null });
  out.push({ label: "Battery voltage", value: n(774, formatVolts), missing: !m.has(774) });
  out.push({ label: "Battery level / charge", value: m.has(775) || m.has(776) ? `${n(775, String)} · charge state ${n(776, String)}` : DASH, sub: "raw values, no unit in the spec", missing: !m.has(775) && !m.has(776) });
  const time = num(m, 771);
  const up = num(m, 772);
  out.push({ label: "Device time", value: n(771, formatUnix), missing: time === null });
  out.push({ label: "Uptime", value: n(772, formatDuration), missing: up === null });
  out.push({ label: "Started", value: time !== null && up !== null ? formatUnix(time - up) : DASH, sub: "device time minus uptime", missing: time === null || up === null });
  const states = [512, 513, 515, 516].map((id) => n(id, String));
  out.push({ label: "GSM states (raw)", value: states.every((s) => s === DASH) ? DASH : `modem ${states[0]} · SIM ${states[1]} · signal ${states[2]} · link ${states[3]}`, sub: "texts not in the spec yet", missing: states.every((s) => s === DASH) });
  out.push({ label: "Data sent / received", value: m.has(544) || m.has(545) ? `${n(544, formatBytes)} / ${n(545, formatBytes)}` : DASH, missing: !m.has(544) && !m.has(545) });
  out.push({ label: "SMS sent / received", value: m.has(546) || m.has(547) ? `${n(546, String)} / ${n(547, String)}` : DASH, missing: !m.has(546) && !m.has(547) });
  out.push({ label: "Protocol", value: m.get(781)?.display ?? DASH, sub: "major.minor", missing: !m.has(781) });
  out.push({ label: "Features", value: m.has(784) || m.has(810) ? `${m.get(784)?.display ?? DASH} / ${m.get(810)?.display ?? DASH}` : DASH, sub: "status 784 / 810", missing: !m.has(784) && !m.has(810) });
  return out;
}

/** Not defined by the spec (open question 70). */
export const WAITING_TILES = ["Speed", "Altitude", "Satellites", "Heading", "GSM state texts"];

// ---------------------------------------------------------------- I/O table

export type IoRow = { id: number; name: string; value: string; group: string; numeric: boolean; known: boolean };

/** Records that are not device-status IDs are I/O elements; names and units come from this document's definitions. */
export function ioRows(fields: StatusField[], items: IoElement[] | undefined): IoRow[] {
  const map = new Map((items ?? []).map((i) => [i.id, i]));
  const rows: IoRow[] = [];
  for (const f of fields) {
    const item = map.get(f.id);
    if (!f.io && !(item && !f.defined)) continue;
    rows.push({
      id: f.id,
      name: item?.name ?? `I/O element #${f.id}`,
      value: item?.unit ? `${f.display} ${item.unit}` : f.display,
      group: item?.group ?? "Other",
      numeric: f.num !== null,
      known: !!item,
    });
  }
  return rows.sort((a, b) => a.group.localeCompare(b.group) || a.id - b.id);
}

export function ioTableHtml(rows: IoRow[], pinned: Set<number>): string {
  if (!rows.length) return `<p class="muted">No I/O element records in the last reply.</p>`;
  const body = rows
    .map((r) => `<tr><td>${esc(r.group)}</td><td>${esc(r.name)}${r.known ? "" : ` <span class="badge warn" title="Not in this document's definitions">unnamed</span>`}</td><td class="mono num">${r.id}</td><td class="mono">${esc(r.value)}</td><td>${r.numeric ? `<button class="link" data-action="pin-io" data-id="${r.id}" aria-pressed="${pinned.has(r.id)}">${pinned.has(r.id) ? "Unpin" : "Pin graph"}</button>` : ""}</td></tr>`)
    .join("");
  return `<div class="tablewrap"><table class="iotable"><thead><tr><th>Group</th><th>Name</th><th>ID</th><th>Value</th><th></th></tr></thead><tbody>${body}</tbody></table></div>`;
}

// ---------------------------------------------------------------- history and sparklines

export type Point = [number, number];

export class History {
  private series = new Map<string, Point[]>();
  push(key: string, atMs: number, value: number) {
    const s = this.series.get(key) ?? [];
    s.push([atMs, value]);
    const cutoff = atMs - HISTORY_MS;
    while (s.length && s[0][0] < cutoff) s.shift();
    this.series.set(key, s);
  }
  get(key: string): Point[] {
    return this.series.get(key) ?? [];
  }
  clear() {
    this.series.clear();
  }
}

/** Inline SVG line over a time window (x = time, newest on the right). */
export function sparkSeries(points: Point[], w = 220, h = 40, windowMs = HISTORY_MS): string {
  if (points.length < 2) return `<svg class="spark" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}" aria-hidden="true"></svg>`;
  const end = points[points.length - 1][0];
  const start = Math.max(end - windowMs, points[0][0]);
  const width = Math.max(end - start, 1);
  const vs = points.map((p) => p[1]);
  const min = Math.min(...vs);
  const max = Math.max(...vs);
  const span = max - min || 1;
  const pts = points
    .map(([t, v]) => `${(1 + ((t - start) / width) * (w - 2)).toFixed(1)},${(h - 3 - ((v - min) / span) * (h - 6)).toFixed(1)}`)
    .join(" ");
  return `<svg class="spark" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}" role="img" aria-label="Last 10 minutes"><polyline points="${pts}" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round" stroke-linecap="round"/></svg>`;
}

export type SparkSpec = { key: string; label: string; unit: string; scale: number };

export function sparkCardsHtml(specs: SparkSpec[], hist: History): string {
  return specs
    .map((s) => {
      const pts = hist.get(s.key);
      const last = pts.length ? pts[pts.length - 1][1] * s.scale : null;
      const shown = last === null ? DASH : `${Number(last.toFixed(2))}${s.unit ? ` ${s.unit}` : ""}`;
      return `<div class="tile"><div class="flabel">${esc(s.label)}</div><div class="fval">${esc(shown)}</div><div class="sparkrow">${sparkSeries(pts)}</div><div class="fid muted">last 10 min</div></div>`;
    })
    .join("");
}

// ---------------------------------------------------------------- CSV

export const CSV_HEADER = "time_utc,id,label,value,unit,kind";
const csvCell = (s: string) => (/[",\r\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s);

export function csvRows(atMs: number, fields: StatusField[], items: IoElement[] | undefined): string[] {
  const names = new Map((items ?? []).map((i) => [i.id, i]));
  const t = new Date(atMs).toISOString();
  return fields.map((f) => {
    const item = f.io ? names.get(f.id) : undefined;
    const value = f.display;
    const unit = item?.unit ?? f.unit ?? "";
    return [t, String(f.id), item?.name ?? f.label, value, unit, f.io ? "io" : "status"].map(csvCell).join(",");
  });
}

export const csvText = (rows: string[]) => [CSV_HEADER, ...rows].join("\r\n") + "\r\n";
