// Device pictures, model names and our own fallback icons (docs/spec/10-device-presentation.md).
// Pictures come from the user's own Configurator download via the backend cache; without one we
// draw our own category icons. Markup is built as strings, so `<span class="dpic">` placeholders are
// filled in later by a MutationObserver (no hooks needed in the render code).
import { invoke } from "@tauri-apps/api/core";

export type Category = "obd" | "compact" | "wired" | "heavy" | "unknown";
export type PictureSize = "sm" | "md" | "lg";

export type ModelName = { name: string | null; known: boolean };
type ModelHint = { model: string; hw_code: string | null };
type Picture = { source: "exact" | "unknown" | "none"; data_url: string | null };

const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

// ---------------------------------------------------------------- Bluetooth name hint (our own design)

const BT_NAME = /^([A-Z]{3}[A-Z0-9]{3,4})_(\d{6,8})$/;

/** `FMB920_1234567` -> `FMB920`. Only the whole name may match. */
export function modelFromBluetoothName(name: string | null | undefined): string | null {
  return name ? (BT_NAME.exec(name.trim())?.[1] ?? null) : null;
}

/** The OS name of a paired Bluetooth port: the product, or the port path without `/dev/cu.`-style prefixes. */
export function bluetoothHint(port: { name: string; product: string | null }): string | null {
  const base = port.name.split(/[\\/]/).pop()!.replace(/^(cu|tty)\./, "");
  return modelFromBluetoothName(port.product) ?? modelFromBluetoothName(base);
}

// ---------------------------------------------------------------- categories and our own icons

/** Rough product class from a model name, only to choose one of our own icons. */
export function categoryOf(model: string | null): Category {
  const n = /^[A-Z]{3}[A-Z]?\s*(\d{3,4}|\dXX)/.exec((model ?? "").trim().toUpperCase())?.[1];
  if (!n) return "unknown";
  if (n === "001" || n === "003" || n === "005") return "obd";
  if (n[0] === "9" || n === "010" || n === "020") return "compact";
  if (n[0] === "6" || n[0] === "2") return "heavy";
  if (n[0] === "1") return "wired";
  return "unknown";
}

const CATEGORY_LABEL: Record<Category, string> = { obd: "OBD plug-in tracker", compact: "Compact tracker", wired: "Professional wired tracker", heavy: "Heavy-duty tracker", unknown: "Tracker" };

const ICON_BODY: Record<Category, string> = {
  // A dongle with a plug end.
  obd: '<path d="M14 18h20a4 4 0 0 1 4 4v12H10V22a4 4 0 0 1 4-4z"/><path d="M16 34v8h16v-8M20 42v4M28 42v4"/><circle cx="24" cy="26" r="2.5"/>',
  // A small box with a status LED.
  compact: '<rect x="8" y="14" width="32" height="20" rx="5"/><circle cx="16" cy="24" r="2"/><path d="M24 20h10M24 28h6"/>',
  // A box with a wiring harness.
  wired: '<rect x="12" y="10" width="24" height="20" rx="3"/><circle cx="18" cy="16" r="1.5"/><path d="M18 30v6c0 3 -4 3 -4 6M24 30v12M30 30v6c0 3 4 3 4 6"/>',
  // A rugged housing with mounting ears and a wiring harness.
  heavy: '<rect x="12" y="10" width="24" height="22" rx="2"/><path d="M6 14h6v6H6zM36 14h6v6h-6zM17 32v10M24 32v10M31 32v10"/><path d="M17 17h14M17 23h14"/>',
  unknown: '<rect x="9" y="12" width="30" height="24" rx="4"/><path d="M20 21a4 4 0 1 1 5.5 3.7c-1 .5-1.5 1.1-1.5 2.3M24 31h.01"/>',
};

export const categoryIcon = (c: Category): string =>
  `<svg class="dpic-svg" viewBox="0 0 48 48" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" role="img" aria-label="${CATEGORY_LABEL[c]}"><title>${CATEGORY_LABEL[c]}</title>${ICON_BODY[c]}</svg>`;

// ---------------------------------------------------------------- backend caches

let onChange: (() => void) | null = null;
const pictures = new Map<string, Promise<Picture>>();
const hints = new Map<string, Promise<ModelHint | null>>();
const models = new Map<string, ModelName | "pending">();

/** Called when asynchronous model names arrive, so the screen can repaint. */
export function onPresentationChange(cb: () => void) {
  onChange = cb;
}

/** Definitions were loaded, changed or removed: forget everything the backend told us. */
export function resetPresentation() {
  pictures.clear();
  hints.clear();
  models.clear();
  for (const el of document.querySelectorAll<HTMLElement>(".dpic[data-state]")) delete el.dataset.state;
  schedule();
}

const fetchPicture = (hw: string) => {
  let p = pictures.get(hw);
  if (!p) pictures.set(hw, (p = invoke<Picture>("device_image", { hwCode: hw }).catch(() => ({ source: "none", data_url: null }) as Picture)));
  return p;
};

const fetchHint = (model: string) => {
  const k = model.toUpperCase();
  let p = hints.get(k);
  if (!p) hints.set(k, (p = invoke<ModelHint | null>("model_hint", { fmType: model }).catch(() => null)));
  return p;
};

/** The model name for a hardware code (spec 10 §3), or undefined while it is being looked up. */
export function modelFor(hw: string | null | undefined): ModelName | undefined {
  if (!hw) return undefined;
  const k = hw.toUpperCase();
  const have = models.get(k);
  if (have === "pending") return undefined;
  if (have) return have;
  models.set(k, "pending");
  invoke<ModelName>("device_model", { hwCode: hw })
    .catch(() => ({ name: null, known: false }))
    .then((m) => {
      models.set(k, m);
      onChange?.();
    });
  return undefined;
}

/** What to call a device: the model name, or the raw hardware code with a hint when definitions don't know it. */
export function modelLabel(hw: string | null | undefined): { text: string; hint: string | null; model: string | null } | null {
  if (!hw) return null;
  const m = modelFor(hw);
  if (m?.name) return { text: m.name, hint: hw, model: m.name };
  if (m && !m.known) return { text: hw, hint: "Not in definitions", model: null };
  return { text: hw, hint: null, model: null };
}

// ---------------------------------------------------------------- markup

/**
 * A picture slot. `hw` is the hardware code as reported; `hintModel` is a provisional model name
 * (from a Bluetooth name) whose code is looked up through the definitions. Starts as our own icon.
 */
export function pictureHtml(o: { hw?: string | null; hintModel?: string | null; model?: string | null; size?: PictureSize; label?: string }): string {
  const cat = categoryOf(o.model ?? o.hintModel ?? null);
  const attrs = `${o.hw ? ` data-hw="${esc(o.hw)}"` : ""}${o.hintModel && !o.hw ? ` data-hint-model="${esc(o.hintModel)}"` : ""}`;
  return `<span class="dpic ${o.size ?? "md"}" data-cat="${cat}"${attrs}>${categoryIcon(cat)}</span>`;
}

// ---------------------------------------------------------------- hydration (replace the icon with the picture)

async function hydrate(el: HTMLElement) {
  el.dataset.state = "icon";
  let hw = el.dataset.hw ?? null;
  const hintModel = el.dataset.hintModel;
  if (!hw && hintModel) hw = (await fetchHint(hintModel))?.hw_code ?? null;
  if (!hw) return;
  const pic = await fetchPicture(hw);
  if (!pic.data_url || pic.source === "none" || !el.isConnected) return;
  el.innerHTML = `<img alt="" src="${pic.data_url}" decoding="async" draggable="false" />`;
  el.dataset.state = pic.source === "exact" ? "picture" : "generic";
}

let queued = false;
function schedule() {
  if (queued) return;
  queued = true;
  requestAnimationFrame(() => {
    queued = false;
    for (const el of document.querySelectorAll<HTMLElement>(".dpic:not([data-state])")) void hydrate(el);
  });
}

/** Watches the page for new `.dpic` slots. Call once at startup. */
export function installPictureObserver() {
  new MutationObserver(schedule).observe(document.body, { childList: true, subtree: true });
  schedule();
}

// ---------------------------------------------------------------- last known model per port / serial / IMEI

type Known = { model: string; hw: string | null };
const KEY = "fmconfig.lastModels";

function readKnown(): Record<string, Known> {
  try {
    return JSON.parse(localStorage.getItem(KEY) ?? "{}") as Record<string, Known>;
  } catch {
    return {};
  }
}

/** Remembers the probed model under the port, USB serial number and IMEI. Never stores anything else. */
export function rememberModel(ids: { port?: string | null; serial?: string | null; imei?: string | null }, model: string, hw: string | null) {
  try {
    const all = readKnown();
    for (const [p, v] of [["port", ids.port], ["sn", ids.serial], ["imei", ids.imei]] as const) if (v) all[`${p}:${v}`] = { model, hw };
    localStorage.setItem(KEY, JSON.stringify(all));
  } catch {
    /* private window or blocked storage: the hint just isn't remembered */
  }
}

export function lastKnownModel(ids: { port?: string | null; serial?: string | null; imei?: string | null }): Known | null {
  const all = readKnown();
  for (const [p, v] of [["sn", ids.serial], ["imei", ids.imei], ["port", ids.port]] as const) {
    const hit = v ? all[`${p}:${v}`] : undefined;
    if (hit) return hit;
  }
  return null;
}
