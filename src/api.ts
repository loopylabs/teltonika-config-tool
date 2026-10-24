import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";

export type Entry = { key: string; value: string };
export type CfgFile = { entries: Entry[] };

export type PortInfo = {
  name: string;
  kind: "usb" | "bluetooth" | "pci" | "unknown";
  vid: number | null;
  pid: number | null;
  manufacturer: string | null;
  product: string | null;
  serialNumber: string | null;
};

const CFG_FILTERS = [{ name: "Teltonika configuration", extensions: ["cfg"] }];

export const listPorts = () => invoke<PortInfo[]>("list_ports");
export const openCfg = (path: string) => invoke<CfgFile>("open_cfg", { path });
export const saveCfg = (path: string, file: CfgFile) => invoke<void>("save_cfg", { path, file });

export async function pickCfgToOpen(): Promise<string | null> {
  const picked = await open({ multiple: false, directory: false, filters: CFG_FILTERS });
  return typeof picked === "string" ? picked : null;
}

export function pickCfgToSave(defaultPath?: string): Promise<string | null> {
  return save({ defaultPath, filters: CFG_FILTERS });
}
