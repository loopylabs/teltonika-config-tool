// Tools → Diagnostics: live tail of the app log (written redacted by the backend), with a level
// selector and a text filter, "Open log folder" and "Export diagnostics…". Self-contained: main.ts
// renders `diagnosticsHtml()` and calls `initDiagnostics()` once.
import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";

export const LEVELS = ["TRACE", "DEBUG", "INFO", "WARN", "ERROR"] as const;
export type Level = (typeof LEVELS)[number];

const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

/** The level of one log line (`[date][time][target][LEVEL] text`); lines without one belong to the previous line. */
export function lineLevel(line: string): Level | null {
  const m = /\]\[(TRACE|DEBUG|INFO|WARN|ERROR)\]/.exec(line);
  return m ? (m[1] as Level) : null;
}

/** Keeps lines at `min` severity or above that contain `query` (case-insensitive). Continuation lines follow their parent. */
export function filterLog(text: string, min: Level, query: string): string[] {
  const floor = LEVELS.indexOf(min);
  const q = query.trim().toLowerCase();
  const out: string[] = [];
  let keep = true;
  for (const line of text.split("\n")) {
    if (!line) continue;
    const lv = lineLevel(line);
    if (lv) keep = LEVELS.indexOf(lv) >= floor;
    if (keep && (!q || line.toLowerCase().includes(q))) out.push(line);
  }
  return out;
}

const view = { text: "", level: "DEBUG" as Level, query: "", follow: true, path: "" };

function paint() {
  const pre = document.querySelector<HTMLElement>('[data-role="diag-log"]');
  if (!pre) return;
  const lines = filterLog(view.text, view.level, view.query);
  const shown = lines.slice(-1500);
  const stick = view.follow && pre.scrollHeight - pre.scrollTop - pre.clientHeight < 40;
  pre.innerHTML = shown.length
    ? shown.map((l) => `<div class="dl dl-${(lineLevel(l) ?? "INFO").toLowerCase()}">${esc(l)}</div>`).join("")
    : `<div class="muted">No log lines match.</div>`;
  const count = document.querySelector<HTMLElement>('[data-role="diag-count"]');
  if (count) count.textContent = `${shown.length.toLocaleString()} of ${lines.length.toLocaleString()} lines`;
  if (stick || view.follow) pre.scrollTop = pre.scrollHeight;
}

async function refresh() {
  if (!document.querySelector('[data-role="diag-log"]')) return;
  try {
    view.text = await invoke<string>("diag_tail", { maxBytes: 300_000 });
    if (!view.path) view.path = (await invoke<{ file: string }>("diag_log_info")).file;
    const where = document.querySelector<HTMLElement>('[data-role="diag-path"]');
    if (where) where.textContent = view.path;
  } catch (error) {
    view.text = `Could not read the log: ${error}`;
  }
  paint();
}

export function diagnosticsHtml(): string {
  const opts = LEVELS.map((l) => `<option value="${l}" ${l === view.level ? "selected" : ""}>${l[0]}${l.slice(1).toLowerCase()} and above</option>`).join("");
  setTimeout(() => void refresh(), 0);
  return `
    <main class="pane devices">
      <header class="phead"><div><h1>Diagnostics</h1><p class="muted">The app log, live. Keywords, PINs and configuration values are never written to it. Log file: <span class="mono" data-role="diag-path">${esc(view.path || "…")}</span></p></div></header>
      <section class="card diag">
        <div class="diag-bar">
          <label>Level <select data-diag="level">${opts}</select></label>
          <input type="search" data-diag="query" placeholder="Filter text" value="${esc(view.query)}" aria-label="Filter log text">
          <span class="muted" data-role="diag-count"></span>
          <span class="grow"></span>
          <button data-diag-action="open-dir">Open log folder</button>
          <button class="primary" data-diag-action="export">Export diagnostics…</button>
        </div>
        <div class="diag-log mono" data-role="diag-log" tabindex="0" aria-label="Application log" aria-live="off"></div>
        <p class="muted" data-role="diag-msg" role="status"></p>
      </section>
    </main>`;
}

function say(text: string) {
  const el = document.querySelector<HTMLElement>('[data-role="diag-msg"]');
  if (el) el.textContent = text;
}

let inited = false;
export function initDiagnostics() {
  if (inited || typeof document === "undefined") return;
  inited = true;
  const style = document.createElement("style");
  style.textContent = `
.diag { padding: 0; overflow: hidden; }
.diag-bar { display: flex; gap: 10px; align-items: center; flex-wrap: wrap; padding: 12px 16px; border-bottom: 1px solid var(--border); }
.diag-bar input[type=search] { min-width: 180px; }
.diag-log { height: min(60vh, 560px); overflow: auto; padding: 8px 16px; font-size: 12px; line-height: 1.5; white-space: pre-wrap; word-break: break-all; }
.dl-warn { color: var(--warn, #b7791f); }
.dl-error { color: var(--danger, #c53030); }
.dl-trace, .dl-debug { color: var(--muted); }
.diag p[role=status] { padding: 0 16px 12px; margin: 0; }`;
  document.head.appendChild(style);
  setInterval(() => void refresh(), 1500);
  document.addEventListener("input", (e) => {
    const t = e.target as HTMLElement;
    if (t.dataset?.diag === "query") {
      view.query = (t as HTMLInputElement).value;
      paint();
    }
  });
  document.addEventListener("change", (e) => {
    const t = e.target as HTMLElement;
    if (t.dataset?.diag === "level") {
      view.level = (t as HTMLSelectElement).value as Level;
      paint();
    }
  });
  document.addEventListener("click", async (e) => {
    const btn = (e.target as HTMLElement).closest<HTMLElement>("[data-diag-action]");
    if (!btn) return;
    try {
      if (btn.dataset.diagAction === "open-dir") {
        await invoke("diag_open_log_dir");
      } else {
        const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-");
        const path = await save({ defaultPath: `fm-config-diagnostics-${stamp}.txt`, filters: [{ name: "Text", extensions: ["txt"] }] });
        if (!path) return;
        await invoke("diag_export", { path });
        say(`Saved ${path}`);
      }
    } catch (error) {
      say(`Failed: ${error}`);
    }
  });
}

/** Forwards console.error / console.warn and unhandled rejections to the Rust log. */
export function forwardConsole() {
  if (typeof window === "undefined") return;
  const send = (level: string, args: unknown[]) => {
    const message = args.map((a) => (a instanceof Error ? `${a.name}: ${a.message}` : typeof a === "string" ? a : safeJson(a))).join(" ");
    invoke("diag_frontend_log", { level, message }).catch(() => {});
  };
  for (const level of ["error", "warn"] as const) {
    const original = console[level].bind(console);
    console[level] = (...args: unknown[]) => {
      original(...args);
      send(level, args);
    };
  }
  window.addEventListener("unhandledrejection", (e) => send("error", ["Unhandled promise rejection:", e.reason]));
  window.addEventListener("error", (e) => send("error", [`Uncaught: ${e.message} (${e.filename}:${e.lineno})`]));
}

function safeJson(v: unknown): string {
  try {
    return JSON.stringify(v) ?? String(v);
  } catch {
    return String(v);
  }
}
