// Review-before-write dialog (ui-brief §4): the diff model (pure, unit-tested) and the dialog markup.
import type { Entry, ParamDef } from "../api";
import { controlFor, formatDisplay } from "../values.ts";

const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

export type ReviewRow = {
  /** Parameter ID. */
  id: number;
  name: string;
  group: string;
  /** Human labels with units (secrets are masked, never shown). */
  before: string;
  after: string;
  /** True when the value is a keyword/PIN: shown masked. */
  secret: boolean;
};

export type DiffSource = {
  entries: Entry[];
  original: string[];
  defOf: (id: number) => ParamDef | undefined;
  nameOf: (id: number) => string;
  groupOf: (id: number) => string;
};

/** Parameter ID of the device keyword (06 §login); its value is never displayed. */
export const KEYWORD_ID = 10;

const isParameter = (e: Entry) => /^\d+$/.test(e.key);

/** One row per changed parameter, in file order, with Before and After as human labels. */
export function buildReviewRows(src: DiffSource): ReviewRow[] {
  const rows: ReviewRow[] = [];
  src.entries.forEach((e, i) => {
    if (!isParameter(e) || e.value === src.original[i]) return;
    const id = Number(e.key);
    const def = src.defOf(id);
    const secret = id === KEYWORD_ID || (!!def && controlFor(def) === "secret");
    const show = (raw: string) => (secret ? (raw ? "••••••" : "(empty)") : def ? formatDisplay(def, raw) || "(empty)" : raw || "(empty)");
    rows.push({ id, name: src.nameOf(id), group: src.groupOf(id), before: show(src.original[i]), after: show(e.value), secret });
  });
  return rows;
}

/** Entries to send for the ticked rows only. */
export function entriesToSend(entries: Entry[], selected: Set<number>): Entry[] {
  return entries.filter((e) => isParameter(e) && selected.has(Number(e.key)));
}

export type Warning = { level: "block" | "confirm" | "note"; text: string };

export type ReviewView = {
  rows: ReviewRow[];
  selected: Set<number>;
  warnings: Warning[];
  /** Whether the "newer version" confirmation was ticked. */
  confirmed: boolean;
  phase: "review" | "writing" | "done" | "aborted";
  /** Whole-file send (no baseline): rows are replaced by a summary. */
  wholeFile?: number;
  progress?: { message: string; done: number; total: number } | null;
  sent?: number;
  error?: string;
};

export const canWrite = (v: ReviewView): boolean => {
  const n = v.wholeFile ? 1 : v.selected.size;
  return v.phase === "review" && n > 0 && !v.warnings.some((w) => w.level === "block") && (v.confirmed || !v.warnings.some((w) => w.level === "confirm"));
};

export function writeLabel(v: ReviewView): string {
  if (v.wholeFile) return `Write ${v.wholeFile.toLocaleString()} parameters`;
  const n = v.selected.size;
  return `Write ${n} change${n === 1 ? "" : "s"}`;
}

function row(r: ReviewRow, on: boolean, locked: boolean): string {
  return `<li class="rv-row ${on ? "" : "off"}"><label><input type="checkbox" data-role="rv-check" data-id="${r.id}" ${on ? "checked" : ""} ${locked ? "disabled" : ""} aria-label="Include ${esc(r.name)}" />
    <span class="rv-name"><span class="rv-title">${esc(r.name)} <small class="pid mono">#${r.id}</small></span><span class="muted rv-group">${esc(r.group || "Other")}</span></span>
    <span class="rv-diff"><span class="rv-before">${esc(r.before)}</span><span aria-label="changes to" class="rv-arrow">→</span><span class="rv-after">${esc(r.after)}</span></span></label></li>`;
}

export function progressHtml(p: ReviewView["progress"]): string {
  const known = !!p && p.total > 0;
  return `<div class="progress-text">${esc(p?.message ?? "Writing")}${known ? ` (${p!.done}/${p!.total})` : "…"}</div><div class="bar ${known ? "" : "indeterminate"}"><div style="width:${known ? Math.round((p!.done / p!.total) * 100) : 40}%"></div></div>`;
}

export function reviewHtml(v: ReviewView): string {
  const warn = v.warnings
    .map((w, i) => `<div class="rv-warn ${w.level}" role="${w.level === "block" ? "alert" : "status"}">${esc(w.text)}${w.level === "confirm" ? `<label class="check"><input type="checkbox" data-role="rv-confirm" ${v.confirmed ? "checked" : ""} ${v.phase !== "review" ? "disabled" : ""} /> I understand, write anyway</label>` : ""}</div>`.replace("%i", String(i)))
    .join("");
  if (v.phase === "done") {
    return `<h3>Written to the device</h3><div class="rv-success" role="status"><span class="rv-tick" aria-hidden="true">✓</span>${(v.sent ?? 0).toLocaleString()} parameter${v.sent === 1 ? "" : "s"} saved to the device.</div>
      <div class="dialog-actions"><button class="primary" data-action="rv-close">Done</button></div>`;
  }
  if (v.phase === "aborted") {
    return `<h3>Write aborted, nothing was saved</h3><div class="rv-warn block" role="alert">${esc(v.error ?? "")} The device may still hold some of your changes in its unsaved working configuration; reboot it to discard them. Your edits here are kept.</div>
      <div class="dialog-actions"><button data-action="rv-close">Close</button><button class="primary" data-action="rv-back">Back to review</button></div>`;
  }
  const writing = v.phase === "writing";
  const list = v.wholeFile
    ? `<p>This file wasn't read from the connected device, so all ${v.wholeFile.toLocaleString()} parameters are sent, not just your edits. Check that it matches the device's model and firmware.</p>`
    : `<ul class="rv-list" data-role="rv-list">${v.rows.map((r) => row(r, v.selected.has(r.id), writing)).join("")}</ul>`;
  const footLeft = v.wholeFile ? "" : `<span class="muted" data-role="rv-count">${v.selected.size} of ${v.rows.length} selected</span><button class="link" data-action="rv-discard" ${v.selected.size && !writing ? "" : "disabled"}>Discard selected</button>`;
  return `<h3>Review changes</h3><p class="muted">Nothing is sent until you press Write. Untick a change to leave it out.</p>${warn}${list}
    <div data-role="rv-progress" class="rv-progress" ${writing ? "" : "hidden"}>${writing ? progressHtml(v.progress) : ""}</div>
    <div class="dialog-actions rv-foot"><div class="rv-footl">${footLeft}</div><button data-action="rv-close" ${writing ? "disabled" : ""}>Cancel</button><button class="primary" data-action="rv-write" ${canWrite(v) ? "" : "disabled"}>${writing ? "Writing…" : writeLabel(v)}</button></div>`;
}
