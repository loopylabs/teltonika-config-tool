//! Tauri commands for device pictures and model names (docs/spec/10-device-presentation.md).

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use super::images::{self, Images, Source};
use super::naming::{ModelHint, ModelName};
use crate::DefsState;

/// Pictures of the loaded Configurator builds, kept in memory as cache file locations.
#[derive(Default)]
pub struct ImagesState(std::sync::Mutex<Images>);

/// Extracts (or finds in the cache) the pictures of a just-loaded Configurator. Best effort:
/// without pictures the UI keeps its own icons.
pub fn add_build(app: &AppHandle, state: &ImagesState, path: &str) {
    let Ok(cache) = app.path().app_data_dir() else { return };
    if let Ok(set) = images::load(std::path::Path::new(path), &cache) {
        state.0.lock().unwrap_or_else(|p| p.into_inner()).add(set);
    }
}

pub fn remove_build(state: &ImagesState, product_version: &str) {
    state.0.lock().unwrap_or_else(|p| p.into_inner()).remove(product_version);
}

#[derive(Serialize)]
pub struct DevicePicture {
    source: Source,
    /// `data:image/png;base64,...`, scaled by the UI in CSS (some pictures are over 4000 px wide).
    data_url: Option<String>,
}

/// The picture for the hardware code as the device reported it (not alias-resolved).
#[tauri::command]
pub async fn device_image(images: State<'_, ImagesState>, hw_code: Option<String>) -> Result<DevicePicture, String> {
    let (source, path) = images.0.lock().unwrap_or_else(|p| p.into_inner()).find(hw_code.as_deref());
    let data_url = match path {
        Some(p) => tauri::async_runtime::spawn_blocking(move || std::fs::read(p).ok())
            .await
            .map_err(|e| e.to_string())?
            .map(|b| format!("data:image/png;base64,{}", images::base64(&b))),
        None => None,
    };
    Ok(match data_url {
        Some(_) => DevicePicture { source, data_url },
        None => DevicePicture { source: Source::None, data_url: None },
    })
}

#[tauri::command]
pub fn device_model(defs: State<'_, DefsState>, hw_code: String) -> ModelName {
    defs.0.lock().unwrap_or_else(|p| p.into_inner()).model_name(&hw_code)
}

/// Model hint for a model name read from a Bluetooth name, e.g. `FMB920`.
#[tauri::command]
pub fn model_hint(defs: State<'_, DefsState>, fm_type: String) -> Option<ModelHint> {
    defs.0.lock().unwrap_or_else(|p| p.into_inner()).hint_for_model(&fm_type)
}
