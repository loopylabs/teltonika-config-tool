mod cfg;
mod ports;

use cfg::CfgFile;
use ports::PortInfo;

#[tauri::command]
fn list_ports() -> Result<Vec<PortInfo>, String> {
    ports::list()
}

#[tauri::command]
fn open_cfg(path: String) -> Result<CfgFile, String> {
    cfg::read_file(&path)
}

#[tauri::command]
fn save_cfg(path: String, file: CfgFile) -> Result<(), String> {
    cfg::write_file(&path, &file)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![list_ports, open_cfg, save_cfg])
        .run(tauri::generate_context!())
        .expect("error while running FM Config");
}
