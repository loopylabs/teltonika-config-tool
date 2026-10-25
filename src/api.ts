import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";

export type Entry = { key: string; value: string };
export type CfgFile = { entries: Entry[] };

export type PortKind = "usb" | "bluetooth" | "other";

export type PortInfo = {
  name: string;
  kind: PortKind;
  /** USB VID:PID matches a known tracker (01-transport §1.1). */
  likelyTracker: boolean;
  vid: number | null;
  pid: number | null;
  manufacturer: string | null;
  product: string | null;
  serialNumber: string | null;
};

const CFG_FILTERS = [{ name: "Teltonika configuration", extensions: ["cfg"] }];

export const listPorts = () => invoke<PortInfo[]>("list_ports");
export const openCfg = (path: string) => invoke<CfgFile>("open_cfg", { path });
/** Resolves to non-blocking warnings; rejects (nothing written) when a value would corrupt the file. */
export const saveCfg = (path: string, file: CfgFile, includeKeyword: boolean) =>
  invoke<string[]>("save_cfg", { path, file, includeKeyword });

export type WriteCheck = { verdict: "ok" } | { verdict: "block" | "confirm"; message: string };
export const checkWriteTarget = (file: CfgFile) => invoke<WriteCheck>("check_write_target", { file });

export async function pickCfgToOpen(): Promise<string | null> {
  const picked = await open({ multiple: false, directory: false, filters: CFG_FILTERS });
  return typeof picked === "string" ? picked : null;
}

export function pickCfgToSave(defaultPath?: string): Promise<string | null> {
  return save({ defaultPath, filters: CFG_FILTERS });
}

// ---------------------------------------------------------------- device

export type DeviceInfo = {
  firmware: string | null;
  configVersion: string | null;
  hardwareCode: string | null;
  imei: string | null;
  keywordSupported: boolean;
  specId: number;
  /** Feature mask as decimal text (64 bits). */
  features?: string;
};

export type SecurityStatus = {
  lastOk: boolean;
  secured: boolean;
  authorized: boolean;
  locked: boolean;
  retriesLeft: number;
};

export type ConnectionState = "disconnected" | "connecting" | "connected" | "busy" | "error";

export type ConnectionStatus = {
  state: ConnectionState;
  port: string | null;
  kind: PortKind | null;
  info: DeviceInfo | null;
  security: SecurityStatus | null;
  /** The error text, or what a busy device is doing. */
  message: string | null;
};

export type Progress = { operation: "read" | "write"; message: string; done: number; total: number };

export const connectDevice = (port: string, kind: PortKind) =>
  invoke<{ status: ConnectionStatus; locked: boolean }>("connect", { port, kind });
export const disconnectDevice = () => invoke<void>("disconnect");
export const connectionStatus = () => invoke<ConnectionStatus>("connection_status");
/** The keyword goes straight to the device; callers must not keep it. */
export const loginDevice = (keyword: string) => invoke<SecurityStatus>("login", { keyword });
export const readDeviceConfig = () => invoke<CfgFile>("read_device_config");
export const writeDeviceConfig = (changes: Entry[]) => invoke<void>("write_device_config", { changes });
export const rebootDevice = () => invoke<void>("reboot");
export const resetDefaults = () => invoke<void>("reset_defaults");

export const onConnectionChanged = (handler: (status: ConnectionStatus) => void) =>
  listen<ConnectionStatus>("connection-changed", (e) => handler(e.payload));
export const onProgress = (handler: (progress: Progress) => void) =>
  listen<Progress>("progress", (e) => handler(e.payload));

/** One loaded Configurator build. The newest is primary; older ones only add what it lacks. */
export interface DefinitionsSummary {
  product_version: string;
  families: string[];
  document_count: number;
  warnings: string[];
  from_cache: boolean;
  primary: boolean;
  extra_hardware: string[];
}

export const loadDefinitions = (path: string) => invoke<DefinitionsSummary[]>("load_definitions", { path });
export const removeDefinitions = (productVersion: string) =>
  invoke<DefinitionsSummary[]>("remove_definitions", { productVersion });
export const definitionsSummary = () => invoke<DefinitionsSummary[]>("definitions_summary");

export async function pickConfigurator(): Promise<string | null> {
  const picked = await open({
    multiple: false,
    directory: false,
    filters: [{ name: "Teltonika Configurator", extensions: ["zip", "exe"] }],
  });
  return typeof picked === "string" ? picked : null;
}
// ---------------------------------------------------------------- Bluetooth LE discovery

export type BleDevice = {
  id: string;
  /** Absent where the OS hides the address (macOS). */
  address: string | null;
  name: string | null;
  rssi: number | null;
  /** Signal bars 0-4, derived on the Rust side. */
  bars: number;
  /** `companyId:hex` per manufacturer record. */
  manufacturerData: string[];
  services: string[];
  /** Milliseconds since the Unix epoch. */
  lastSeen: number;
};

export type BleScanState =
  | "idle"
  | "scanning"
  | "bluetooth-off"
  | "permission-denied"
  | "no-adapter"
  | "unsupported"
  | "error";
export type BleScanStatus = { state: BleScanState; message: string | null };

export const startBleScan = (timeoutSecs = 15) => invoke<void>("start_ble_scan", { timeoutSecs });
export const stopBleScan = () => invoke<void>("stop_ble_scan");
export const onBleDevice = (handler: (device: BleDevice) => void) =>
  listen<BleDevice>("ble-device", (e) => handler(e.payload));
export const onBleScanState = (handler: (status: BleScanStatus) => void) =>
  listen<BleScanStatus>("ble-scan-state", (e) => handler(e.payload));

// ---------------------------------------------------------------- editor model (04 §3)

export type EnumItem = { value: string; label: string };
export type ParamDef = {
  id: number;
  label: string;
  value_type: string;
  control: "number" | "text" | "enum" | "other";
  default: string | null;
  min: string | null;
  max: string | null;
  min_id: number | null;
  max_id: number | null;
  units: string | null;
  text_min: number | null;
  text_max: number | null;
  validate: string[];
  options: EnumItem[];
  bitmask: boolean;
  mask: string | null;
  converter: string | null;
  list: string | null;
  slot: number | null;
};
export type EditorNode = { kind: "page" | "group" | "tab"; title: string; ids: number[]; children: EditorNode[] };
export type IoElement = { id: number; name: string; unit: string | null; group: string | null };
export type EditorModel = {
  model_name: string;
  hw_code: string;
  build: string;
  chain: string[];
  pages: EditorNode[];
  params: Record<string, ParamDef>;
  /** I/O elements of this document's definitions (name, unit, group). */
  io_items: IoElement[];
};

export const composeDefinitions = (args: {
  hwCode: string;
  cfgVersion: string;
  deviceSpecId?: number;
  /** Decimal text: a 64-bit mask does not fit a JS number. */
  featureMask?: string;
}) => invoke<EditorModel>("compose_definitions", args);

// ---------------------------------------------------------------- live data (log + status)

export type LogLine = { atMs: number; text: string };
export type StatusField = {
  id: number;
  label: string;
  group: string;
  display: string;
  rawHex: string;
  typeCode: number;
  /** False: the spec doesn't define it, show raw (TODO(spec)). */
  defined: boolean;
  unit: string | null;
  /** Numeric value; none for text or an out-of-range coordinate. */
  num: number | null;
  /** An I/O element record (ID outside the device-status range 256-826). */
  io: boolean;
};
export type StatusEvent = {
  kind: PortKind;
  atMs: number;
  rttMs: number | null;
  /** Failed polls in a row. */
  failures: number;
  ok: boolean;
  /** Polling uses 0x0102 (all groups). */
  extended: boolean;
  fields: StatusField[];
};

export const startLog = () => invoke<void>("start_log");
/** Resolves once `.log:0` has been sent. */
export const stopLog = () => invoke<void>("stop_log");
export const startStatusPolling = (intervalMs: number) => invoke<void>("start_status_polling", { intervalMs });
export const stopStatusPolling = () => invoke<void>("stop_status_polling");
export const defaultPollInterval = () => invoke<number>("default_poll_interval");
export const saveText = (path: string, text: string) => invoke<void>("save_text", { path, text });
export const pickCsvToSave = (defaultPath: string) => save({ defaultPath, filters: [{ name: "CSV", extensions: ["csv"] }] });
export const pickTextToSave = (defaultPath: string) => save({ defaultPath, filters: [{ name: "Text", extensions: ["txt"] }] });

export const onDeviceLog = (handler: (lines: LogLine[]) => void) => listen<LogLine[]>("device-log", (e) => handler(e.payload));
export const onDeviceStatus = (handler: (event: StatusEvent) => void) => listen<StatusEvent>("device-status", (e) => handler(e.payload));
