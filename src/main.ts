import {
  type Entry,
  type PortInfo,
  listPorts,
  openCfg,
  pickCfgToOpen,
  pickCfgToSave,
  saveCfg,
} from "./api";

type Section = "overview" | "parameters";

type Doc = {
  path: string;
  entries: Entry[];
  /** Values as loaded, by entry index, to track unsaved edits. */
  original: string[];
};

const HEADER_KEYS = ["FmType", "HwVersion", "Title", "ConfigurationVersion", "SpecId"];

/** Settings groups from the Windows Configurator, unlocked once each is mapped to parameter IDs. */
const PENDING_SECTIONS = [
  "System",
  "GPRS",
  "Data Acquisition",
  "SMS / Call",
  "Features",
  "Bluetooth",
  "I/O",
];

const DEVICE_ONLY_HINT = "Needs the device protocol, which hasn't been reverse engineered yet";
const TABLE_LIMIT = 500;

const state = {
  section: "overview" as Section,
  ports: [] as PortInfo[],
  portsError: "",
  doc: null as Doc | null,
  filter: "",
  message: "",
};

const app = document.querySelector<HTMLDivElement>("#app")!;

function esc(text: string): string {
  return text.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
}

function changedCount(doc: Doc): number {
  return doc.entries.filter((entry, i) => entry.value !== doc.original[i]).length;
}

function fileName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

function hex(n: number | null): string {
  return n === null ? "" : n.toString(16).padStart(4, "0");
}

// ---------------------------------------------------------------- views

function toolbar(): string {
  const doc = state.doc;
  const deviceButton = (label: string) =>
    `<button disabled title="${DEVICE_ONLY_HINT}">${label}</button>`;
  return `
    <header class="toolbar">
      <span class="brand">FM Config</span>
      <button data-action="open-file">Open file…</button>
      <button data-action="save-file" ${doc && changedCount(doc) ? "" : "disabled"}>Save</button>
      <button data-action="save-as" ${doc ? "" : "disabled"}>Save as…</button>
      <span class="sep"></span>
      ${deviceButton("Load from device")}
      ${deviceButton("Save to device")}
      ${deviceButton("Update firmware")}
      ${deviceButton("Reboot")}
      <span class="grow"></span>
      ${doc ? `<button data-action="close">Close</button>` : ""}
    </header>`;
}

function portRow(port: PortInfo): string {
  const label = port.product ?? port.manufacturer ?? "Serial device";
  const ids = port.vid !== null ? `USB ${hex(port.vid)}:${hex(port.pid)}` : port.kind;
  return `
    <li class="port">
      <div>
        <div class="port-name">${esc(label)}</div>
        <div class="muted mono">${esc(port.name)} · ${esc(ids)}</div>
      </div>
      <button disabled title="${DEVICE_ONLY_HINT}">Connect</button>
    </li>`;
}

function startView(): string {
  const ports = state.portsError
    ? `<p class="error">${esc(state.portsError)}</p>`
    : state.ports.length
      ? `<ul class="ports">${state.ports.map(portRow).join("")}</ul>`
      : `<p class="muted">No serial devices found. Plug in a tracker over USB, then refresh.</p>`;
  return `
    <main class="start">
      <section class="card">
        <div class="card-head">
          <h2>Connected devices</h2>
          <button data-action="refresh-ports">Refresh</button>
        </div>
        ${ports}
        <p class="note">Talking to a device isn't available yet. That comes after we've mapped the protocol.</p>
      </section>
      <section class="card">
        <h2>Configuration file</h2>
        <p class="muted">Open a <span class="mono">.cfg</span> file saved by Teltonika Configurator to view and edit it.</p>
        <button class="primary" data-action="open-file">Open .cfg file…</button>
      </section>
    </main>`;
}

function nav(): string {
  const item = (section: Section, label: string) =>
    `<button class="nav-item ${state.section === section ? "active" : ""}" data-action="section" data-section="${section}">${label}</button>`;
  const pending = PENDING_SECTIONS.map(
    (label) => `<div class="nav-item pending" title="Not mapped to parameter IDs yet">${label}</div>`,
  ).join("");
  return `
    <nav class="nav">
      ${item("overview", "Overview")}
      ${item("parameters", "All parameters")}
      <div class="nav-label">Settings groups</div>
      ${pending}
    </nav>`;
}

function overview(doc: Doc): string {
  const header = new Map(doc.entries.map((e) => [e.key, e.value]));
  const rows = HEADER_KEYS.filter((key) => header.has(key))
    .map((key) => `<dt>${key}</dt><dd class="mono">${esc(header.get(key)!)}</dd>`)
    .join("");
  const parameterCount = doc.entries.filter((e) => /^\d+$/.test(e.key)).length;
  return `
    <section class="pane">
      <h2>${esc(header.get("FmType") ?? fileName(doc.path))}</h2>
      <dl class="facts">
        ${rows}
        <dt>Parameters</dt><dd>${parameterCount.toLocaleString()}</dd>
        <dt>File</dt><dd class="mono">${esc(doc.path)}</dd>
      </dl>
    </section>`;
}

function parameterRows(doc: Doc): string {
  const needle = state.filter.trim().toLowerCase();
  const matches: number[] = [];
  doc.entries.forEach((entry, i) => {
    if (!needle || entry.key.toLowerCase().includes(needle) || entry.value.toLowerCase().includes(needle)) {
      matches.push(i);
    }
  });
  const rows = matches
    .slice(0, TABLE_LIMIT)
    .map((i) => {
      const entry = doc.entries[i];
      const changed = entry.value !== doc.original[i];
      return `
        <tr class="${changed ? "changed" : ""}" data-row="${i}">
          <td class="mono key">${esc(entry.key)}</td>
          <td><input class="mono" data-index="${i}" value="${esc(entry.value)}" spellcheck="false" /></td>
          <td><button class="link" data-action="revert" data-index="${i}" ${changed ? "" : "hidden"}>Revert</button></td>
        </tr>`;
    })
    .join("");
  const more =
    matches.length > TABLE_LIMIT
      ? `<tr><td colspan="3" class="muted">Showing ${TABLE_LIMIT} of ${matches.length.toLocaleString()}. Narrow the search to see more.</td></tr>`
      : "";
  return rows + more || `<tr><td colspan="3" class="muted">No parameters match.</td></tr>`;
}

function parameters(doc: Doc): string {
  return `
    <section class="pane">
      <div class="pane-head">
        <h2>All parameters</h2>
        <input class="search" data-role="filter" placeholder="Search ID or value" value="${esc(state.filter)}" />
      </div>
      <table class="params">
        <thead><tr><th>ID</th><th>Value</th><th></th></tr></thead>
        <tbody>${parameterRows(doc)}</tbody>
      </table>
    </section>`;
}

function workspace(doc: Doc): string {
  return `
    <div class="workspace">
      ${nav()}
      ${state.section === "overview" ? overview(doc) : parameters(doc)}
    </div>`;
}

function statusBar(): string {
  const doc = state.doc;
  const left = doc ? esc(fileName(doc.path)) : "No configuration loaded";
  const right = doc
    ? `${changedCount(doc)} changed · ${doc.entries.length.toLocaleString()} entries`
    : "";
  return `
    <footer class="status">
      <span>${left}</span>
      <span class="message">${esc(state.message)}</span>
      <span data-role="counts">${right}</span>
    </footer>`;
}

function render() {
  app.innerHTML = toolbar() + (state.doc ? workspace(state.doc) : startView()) + statusBar();
}

/** Update counts and Save button without re-rendering the table, so inputs keep focus. */
function refreshChrome() {
  const doc = state.doc!;
  const counts = app.querySelector('[data-role="counts"]');
  if (counts) counts.textContent = `${changedCount(doc)} changed · ${doc.entries.length.toLocaleString()} entries`;
  const save = app.querySelector<HTMLButtonElement>('[data-action="save-file"]');
  if (save) save.disabled = changedCount(doc) === 0;
}

// ---------------------------------------------------------------- actions

async function refreshPorts() {
  try {
    state.ports = await listPorts();
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
    state.doc = { path, entries: file.entries, original: file.entries.map((e) => e.value) };
    state.section = "overview";
    state.filter = "";
    state.message = `Opened ${fileName(path)}`;
  } catch (error) {
    state.message = `Could not open file: ${error}`;
  }
  render();
}

async function saveTo(path: string) {
  const doc = state.doc!;
  try {
    await saveCfg(path, { entries: doc.entries });
    doc.path = path;
    doc.original = doc.entries.map((e) => e.value);
    state.message = `Saved ${fileName(path)}`;
  } catch (error) {
    state.message = `Could not save: ${error}`;
  }
  render();
}

async function saveAs() {
  const path = await pickCfgToSave(state.doc!.path);
  if (path) await saveTo(path);
}

function close() {
  const doc = state.doc;
  if (doc && changedCount(doc) && !confirm("Discard unsaved changes?")) return;
  state.doc = null;
  state.message = "";
  render();
  void refreshPorts();
}

function revert(index: number) {
  const doc = state.doc!;
  doc.entries[index].value = doc.original[index];
  const row = app.querySelector<HTMLTableRowElement>(`tr[data-row="${index}"]`);
  if (row) {
    row.classList.remove("changed");
    row.querySelector<HTMLInputElement>("input")!.value = doc.original[index];
    row.querySelector<HTMLButtonElement>('[data-action="revert"]')!.hidden = true;
  }
  refreshChrome();
}

app.addEventListener("click", (event) => {
  const target = (event.target as HTMLElement).closest<HTMLElement>("[data-action]");
  if (!target || (target as HTMLButtonElement).disabled) return;
  switch (target.dataset.action) {
    case "open-file":
      return void openFile();
    case "save-file":
      return void saveTo(state.doc!.path);
    case "save-as":
      return void saveAs();
    case "close":
      return close();
    case "refresh-ports":
      return void refreshPorts();
    case "section":
      state.section = target.dataset.section as Section;
      return render();
    case "revert":
      return revert(Number(target.dataset.index));
  }
});

app.addEventListener("input", (event) => {
  const input = event.target as HTMLInputElement;
  const doc = state.doc;
  if (!doc) return;
  if (input.dataset.role === "filter") {
    state.filter = input.value;
    app.querySelector(".params tbody")!.innerHTML = parameterRows(doc);
    return;
  }
  if (input.dataset.index === undefined) return;
  const index = Number(input.dataset.index);
  doc.entries[index].value = input.value;
  const changed = input.value !== doc.original[index];
  const row = input.closest("tr")!;
  row.classList.toggle("changed", changed);
  row.querySelector<HTMLButtonElement>('[data-action="revert"]')!.hidden = !changed;
  refreshChrome();
});

window.addEventListener("keydown", (event) => {
  if (!(event.metaKey || event.ctrlKey)) return;
  if (event.key === "o") {
    event.preventDefault();
    void openFile();
  } else if (event.key === "s" && state.doc) {
    event.preventDefault();
    void (event.shiftKey ? saveAs() : saveTo(state.doc.path));
  }
});

render();
void refreshPorts();
