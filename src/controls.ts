// Control registry keyed by Control type (values.ts controlFor). Each control renders HTML for one
// parameter and reads the raw 04/05 value back from the element. Add a type = add one entry.
import type { ParamDef } from "./api";
import { type Control, controlFor, hhmmToMinutes, maskEncode, maskKind, maskSelected, minutesToHhmm, parseInput, trimFloat } from "./values";

export const esc = (s: string) =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

export type RenderCtx = { def: ParamDef; value: string; index: number; disabled: boolean };
export type ControlImpl = {
  render(c: RenderCtx): string;
  /** Raw value to store, or null when the current input does not parse (nothing is written). */
  read(el: HTMLElement, def: ParamDef): string | null;
};

const attrs = (c: RenderCtx, ctl: Control) => `data-index="${c.index}" data-ctl="${ctl}" ${c.disabled ? "disabled" : ""}`;
const units = (d: ParamDef) => (d.units ? `<span class="unit">${esc(d.units)}</span>` : "");
const input = (el: HTMLElement) => el as HTMLInputElement;

const plainText = (c: RenderCtx, ctl: Control, extra = "") =>
  `<input class="mono" ${attrs(c, ctl)} value="${esc(c.value)}" spellcheck="false" ${c.def.text_max ? `maxlength="${c.def.text_max + 8}"` : ""} ${extra} />`;

export const CONTROLS: Record<Control, ControlImpl> = {
  toggle: {
    render: (c) => {
      const on = c.def.options.find((o) => o.value === "1")?.label ?? "On";
      const off = c.def.options.find((o) => o.value === "0")?.label ?? "Off";
      const named = on !== "1" && off !== "0";
      return `<label class="switch"><input type="checkbox" ${attrs(c, "toggle")} ${c.value === "1" ? "checked" : ""} /><i></i>${named ? `<span class="muted">${esc(c.value === "1" ? on : off)}</span>` : ""}</label>`;
    },
    read: (el) => (input(el).checked ? "1" : "0"),
  },
  select: {
    render: (c) => {
      const known = c.def.options.some((o) => o.value === c.value);
      const extra = known ? "" : `<option value="${esc(c.value)}" selected>${esc(c.value)} (not in list)</option>`;
      return `<select ${attrs(c, "select")}>${extra}${c.def.options.map((o) => `<option value="${esc(o.value)}" ${o.value === c.value ? "selected" : ""}>${esc(o.label)}</option>`).join("")}</select>`;
    },
    read: (el) => (el as HTMLSelectElement).value,
  },
  radio: {
    render: (c) =>
      `<div class="radios" ${attrs(c, "radio")}>${c.def.options.map((o) => `<label><input type="radio" name="r${c.index}" value="${esc(o.value)}" ${o.value === c.value ? "checked" : ""} ${c.disabled ? "disabled" : ""} /> ${esc(o.label)}</label>`).join("")}</div>`,
    read: (el) => el.querySelector<HTMLInputElement>("input:checked")?.value ?? null,
  },
  mask: {
    render: (c) => {
      const sel = maskSelected(c.def, c.value);
      return `<div class="checks" ${attrs(c, "mask")}>${c.def.options.map((o) => `<label><input type="checkbox" value="${esc(o.value)}" ${sel.includes(o.value) ? "checked" : ""} ${c.disabled ? "disabled" : ""} /> ${esc(o.label)}</label>`).join("")}</div>`;
    },
    read: (el) => maskEncode([...el.querySelectorAll<HTMLInputElement>("input:checked")].map((i) => i.value)),
  },
  // TODO(spec): open question 56 — the multi-field mask encoding is unknown; ticks are display-only and the raw value is edited directly.
  "mask-readonly": {
    render: (c) => {
      const sel = maskKind(c.def) === "multifield" ? [] : maskSelected(c.def, c.value);
      const boxes = c.def.options.map((o) => `<label class="muted"><input type="checkbox" disabled ${sel.includes(o.value) ? "checked" : ""} /> ${esc(o.label)}</label>`).join("");
      return `<div class="checks">${boxes}</div><input class="mono" ${attrs(c, "mask-readonly")} value="${esc(c.value)}" spellcheck="false" title="Raw value; this option encoding is not confirmed, so it is edited as raw text" />`;
    },
    read: (el) => input(el).value,
  },
  number: {
    render: (c) => {
      const float = c.def.value_type === "float" || c.def.value_type === "double";
      const step = float ? "any" : "1";
      return `<input class="mono num" type="text" inputmode="${float ? "decimal" : "numeric"}" ${attrs(c, "number")} value="${esc(float ? trimFloat(c.value) : c.value)}" data-step="${step}" ${c.def.min !== null ? `data-min="${esc(c.def.min)}"` : ""} ${c.def.max !== null ? `data-max="${esc(c.def.max)}"` : ""} spellcheck="false" />${units(c.def)}`;
    },
    read: (el, def) => parseInput(def, input(el).value),
  },
  time: {
    render: (c) => `<input class="mono num" ${attrs(c, "time")} value="${esc(minutesToHhmm(c.value) ?? c.value)}" placeholder="hh:mm" spellcheck="false" /><span class="unit">hh:mm</span>`,
    read: (el) => hhmmToMinutes(input(el).value),
  },
  secret: {
    render: (c) => `<span class="secret"><input class="mono" type="password" ${attrs(c, "secret")} value="${esc(c.value)}" autocomplete="off" spellcheck="false" /><button type="button" class="link" data-action="reveal">Show</button></span>`,
    read: (el) => input(el).value,
  },
  mac: {
    render: (c) => plainText(c, "mac", 'placeholder="12 hex characters" maxlength="12"'),
    read: (el) => input(el).value.trim(),
  },
  text: { render: (c) => plainText(c, "text"), read: (el) => input(el).value },
  raw: { render: (c) => `${plainText(c, "raw")}`, read: (el) => input(el).value },
};

export function renderControl(def: ParamDef, value: string, index: number, disabled = false): string {
  return CONTROLS[controlFor(def)].render({ def, value, index, disabled });
}

export function readControl(el: HTMLElement, def: ParamDef): string | null {
  const impl = CONTROLS[(el.dataset.ctl as Control) in CONTROLS ? (el.dataset.ctl as Control) : "raw"];
  return impl.read(el, def);
}
