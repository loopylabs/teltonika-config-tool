// App shell markup (ui-brief §1): sidebar with Device / Settings / Tools, and the top bar.
import { esc, menu } from "./devices";
import { type IconName, icon } from "./icons";

export type NavItem = { section: string; label: string; icon: IconName; count?: number; disabled?: string };
export type NavGroup = { id: string; title: string; items: NavItem[]; callout?: { text: string; action: string; label: string } };

export function sidebarHtml(groups: NavGroup[], active: string, collapsed: Set<string>): string {
  const item = (i: NavItem) =>
    i.disabled
      ? `<button class="nav-item" disabled title="${esc(i.disabled)}"><span class="nav-ico">${icon(i.icon)}</span><span class="nav-text">${esc(i.label)}</span><span class="soon">Soon</span></button>`
      : `<button class="nav-item ${active === i.section ? "active" : ""}" data-action="section" data-section="${esc(i.section)}" ${active === i.section ? 'aria-current="page"' : ""}><span class="nav-ico">${icon(i.icon)}</span><span class="nav-text">${esc(i.label)}</span><span class="count" data-nav-count="${esc(i.section)}" ${i.count ? "" : "hidden"}>${i.count ?? ""}</span></button>`;
  return `
    <div class="brand"><span class="brand-mark">${icon("settings", 16)}</span>FM Config</div>
    <nav aria-label="Sections">${groups
      .map((g) => {
        const open = !collapsed.has(g.id);
        const callout = g.callout ? `<div class="callout"><p>${esc(g.callout.text)}</p><button data-action="${g.callout.action}">${esc(g.callout.label)}</button></div>` : "";
        return `<div class="nav-group"><button class="nav-label" data-action="nav-group" data-group="${g.id}" aria-expanded="${open}">${esc(g.title)}</button>${open ? g.items.map(item).join("") + callout : ""}</div>`;
      })
      .join("")}</nav>`;
}

export type ChipView = { state: "off" | "pending" | "on"; text: string; title: string; picture?: string };

export function chipHtml(c: ChipView): string {
  return `<button class="chip" data-action="section" data-section="devices" title="${esc(c.title)}"><span class="sdot ${c.state}"></span>${c.picture ?? ""}<span class="chip-text">${esc(c.text)}</span></button>`;
}

export type PrimaryView = { label: string; action: string; disabled: boolean; title: string };

export function actionsHtml(p: PrimaryView, canSave: boolean, hasDoc: boolean, connected: boolean, platformKey: string): string {
  const items = [
    { label: "Save as…", action: "save-as", disabled: !hasDoc },
    { label: "Close file", action: "close", disabled: !hasDoc },
    ...(connected ? [{ label: "Disconnect", action: "disconnect", disabled: false }] : []),
  ];
  return `<button data-action="open-file" title="Open a .cfg file (${platformKey}O)">${icon("folder")}<span>Open</span></button>
    <button data-action="save-file" ${canSave ? "" : "disabled"} title="Save to a .cfg file (${platformKey}S)">Save</button>
    ${menu(items, "File menu")}
    <button class="primary" data-action="${p.action}" ${p.disabled ? "disabled" : ""} title="${esc(p.title)}">${esc(p.label)}</button>`;
}

export function searchHtml(value: string, enabled: boolean, platformKey: string): string {
  return `<label class="gsearch"><span class="gs-ico">${icon("search")}</span><span class="sr-only">Search settings</span><input data-role="filter" type="search" placeholder="Search settings, groups, values, IDs" value="${esc(value)}" ${enabled ? "" : "disabled"} autocomplete="off" spellcheck="false" /><kbd>${platformKey}K</kbd></label>`;
}
