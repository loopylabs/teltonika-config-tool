mod ble;
mod bt_classic;
mod cfg;
mod commands;
mod connection;
mod defs;
mod device;
mod diag;
mod fmbx;
mod framing;
mod live;
mod ports;
mod transport;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use cfg::CfgFile;
use commands::SecurityStatus;
use connection::{Connection, Status};
use device::{DeviceError, Session, Timeouts};
use ports::{PortInfo, PortKind};
use transport::{OpenPolicy, SerialSettings, SerialTransport};

#[tauri::command]
fn list_ports() -> Result<Vec<PortInfo>, String> {
    ports::list()
}

#[tauri::command]
fn open_cfg(path: String) -> Result<CfgFile, String> {
    cfg::read_file(&path)
}

/// Writes text the user chose to save (the device log).
#[tauri::command]
fn save_text(path: String, text: String) -> Result<(), String> {
    std::fs::write(&path, text).map_err(|e| format!("Could not save {path}: {e}"))
}

#[tauri::command]
fn save_cfg(path: String, file: CfgFile, include_keyword: bool) -> Result<Vec<String>, String> {
    cfg::write_file(&path, &file, include_keyword)
}

/// Checks a file-sourced configuration against the connected device before a write.
#[tauri::command]
fn check_write_target(conn: State<'_, Connection>, file: CfgFile) -> cfg::WriteCheck {
    let info = conn.status().info;
    cfg::check_write_target(
        &file,
        info.as_ref().and_then(|i| i.hardware_code.as_deref()),
        info.as_ref().and_then(|i| i.config_version.as_deref()),
    )
}

// ---------------------------------------------------------------- device

#[derive(Clone, Serialize)]
struct Progress {
    operation: &'static str,
    message: String,
    done: usize,
    total: usize,
}

fn emit_status(app: &AppHandle, status: Status) {
    let _ = app.emit("connection-changed", status);
}

fn emit_progress(app: &AppHandle, operation: &'static str, message: String, done: usize, total: usize) {
    let _ = app.emit("progress", Progress { operation, message, done, total });
}

async fn tokio_sleep(ms: u64) {
    let _ = blocking(move || std::thread::sleep(std::time::Duration::from_millis(ms))).await;
}

async fn blocking<T: Send + 'static>(job: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(job).await.map_err(|e| format!("internal error: {e}"))
}

/// Runs one device operation off the UI thread with the session leased from the
/// connection. `keep == false` ends the connection afterwards.
async fn run_op<T: Send + 'static>(
    app: AppHandle,
    conn: &Connection,
    what: &str,
    keep: bool,
    op: impl FnOnce(&mut Session, &AppHandle) -> Result<T, DeviceError> + Send + 'static,
) -> Result<T, String> {
    // Let the background log/status worker hand the session back first.
    conn.user_wait_begin();
    while conn.live_active() {
        tokio_sleep(20).await;
    }
    let leased = conn.lease(what);
    conn.user_wait_end();
    let (mut lease, status) = leased?;
    emit_status(&app, status);
    let handle = app.clone();
    let (result, lease) = blocking(move || {
        let result = op(&mut lease.session, &handle);
        (result, lease)
    })
    .await
    .map_err(|e| {
        // The worker panicked; the session is gone with it.
        let _ = conn.disconnect();
        e
    })?;
    let outcome = result.as_ref().map(|_| ()).map_err(Clone::clone);
    let (status, stale) = conn.finish(lease, &outcome, keep);
    if let Some(status) = status {
        emit_status(&app, status);
    }
    if let Some(session) = stale {
        let _ = blocking(move || drop(session)).await;
    }
    result.map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct Connected {
    status: Status,
    /// True when the device wants a keyword before it can be read or written.
    locked: bool,
}

#[tauri::command]
async fn connect(
    app: AppHandle,
    conn: State<'_, Connection>,
    port: String,
    kind: PortKind,
    settings: Option<SerialSettings>,
) -> Result<Connected, String> {
    let generation = conn.begin_connect(&port, kind)?;
    let cancel = conn.cancel_flag();
    emit_status(&app, conn.status());
    let settings = settings.unwrap_or_else(|| SerialSettings::for_kind(kind));
    let attempt_started = std::time::Instant::now();
    #[cfg(target_os = "macos")]
    let app_rf = app.clone();
    let opened = blocking(move || -> Result<Session, String> {
        // "bt:<address>" = native RFCOMM to the SPP service (macOS fallback when no
        // /dev/cu.* port exists for the paired tracker), 01-transport §2.1.
        let is_rfcomm = port.starts_with("bt:");
        let link = if is_rfcomm { "Bluetooth RFCOMM link" } else { "" };
        log::info!("connect: opening {} ({kind:?}, {})", if is_rfcomm { "RFCOMM address from the port id" } else { port.as_str() }, if is_rfcomm { "direct RFCOMM" } else { "OS serial port" });
        let opened_at = std::time::Instant::now();
        let transport: Box<dyn transport::Transport> = match port.strip_prefix("bt:") {
            #[cfg(target_os = "macos")]
            Some(addr) => Box::new(bt_classic::rfcomm_macos::RfcommTransport::open(app_rf, addr)?),
            #[cfg(not(target_os = "macos"))]
            Some(_) => return Err("Direct Bluetooth connections aren't available on this system. Use the serial port.".into()),
            _ => Box::new(SerialTransport::open(&port, &settings, OpenPolicy::for_kind(kind))?),
        };
        log::info!("connect: transport open in {} ms", opened_at.elapsed().as_millis());
        let name = if is_rfcomm { link } else { "serial port" };
        let transport = Box::new(diag::LoggedTransport::new(transport, name, cancel));
        let mut session = Session::new(transport, kind, Timeouts::for_kind(kind));
        if is_rfcomm {
            session.link = link.into();
        }
        commands::open_session(&mut session).map_err(|e| match e {
            DeviceError::Transport(ref m) if m == "cancelled" => "Connection cancelled.".to_string(),
            DeviceError::Rejected(m) | DeviceError::Invalid(m) => m,
            other => session.explain(&other),
        })?;
        Ok(session)
    })
    .await
    .and_then(|r| r);
    match &opened {
        Ok(_) => log::info!("connect: finished in {} ms", attempt_started.elapsed().as_millis()),
        Err(e) => log::warn!("connect: failed after {} ms: {e}", attempt_started.elapsed().as_millis()),
    }
    match opened {
        Ok(session) => match conn.connected(generation, session) {
            Ok(status) => {
                emit_status(&app, status.clone());
                let locked = status.security.as_ref().is_some_and(|s| s.needs_keyword());
                Ok(Connected { status, locked })
            }
            Err(stale) => {
                let _ = blocking(move || drop(stale)).await;
                Err("The connection attempt was cancelled.".into())
            }
        },
        Err(e) => {
            if let Some(status) = conn.failed(generation, e.to_string()) {
                emit_status(&app, status);
            }
            Err(e.to_string())
        }
    }
}

#[tauri::command]
async fn disconnect(app: AppHandle, conn: State<'_, Connection>) -> Result<(), String> {
    log::info!("disconnect requested");
    let (status, session) = conn.disconnect();
    emit_status(&app, status);
    if let Some(session) = session {
        blocking(move || drop(session)).await?;
    }
    Ok(())
}

#[tauri::command]
fn connection_status(conn: State<'_, Connection>) -> Status {
    conn.status()
}

/// The keyword is passed straight to the device and never stored or logged.
#[tauri::command]
async fn login(app: AppHandle, conn: State<'_, Connection>, keyword: String) -> Result<SecurityStatus, String> {
    run_op(app, &conn, "Logging in", true, move |session, _| commands::login(session, &keyword)).await
}

#[tauri::command]
async fn read_device_config(app: AppHandle, conn: State<'_, Connection>) -> Result<CfgFile, String> {
    run_op(app, &conn, "Reading configuration", true, |session, app| {
        let entries = commands::read_config(session, |n, total| {
            emit_progress(app, "read", format!("Reading configuration (attempt {n} of {total})"), 0, 0)
        })?;
        Ok(CfgFile { entries })
    })
    .await
}

/// Sends only the given (changed) entries, in chunks, then saves. Any failure
/// aborts without saving.
#[tauri::command]
async fn write_device_config(
    app: AppHandle,
    conn: State<'_, Connection>,
    changes: Vec<cfg::Entry>,
) -> Result<(), String> {
    run_op(app, &conn, "Writing configuration", true, move |session, app| {
        commands::write_config(session, &changes, |done, total| {
            emit_progress(app, "write", "Writing configuration".into(), done, total)
        })
    })
    .await
}

/// The device restarts, so the connection ends.
#[tauri::command]
async fn reboot(app: AppHandle, conn: State<'_, Connection>) -> Result<(), String> {
    run_op(app, &conn, "Rebooting", false, |session, _| commands::reboot(session)).await
}

#[tauri::command]
async fn reset_defaults(app: AppHandle, conn: State<'_, Connection>) -> Result<(), String> {
    run_op(app, &conn, "Resetting to defaults", true, |session, _| commands::reset_to_defaults(session)).await
}

// ---------------------------------------------------------------- live data

#[derive(Default)]
struct Live {
    want: std::sync::Mutex<(bool, Option<u64>)>,
    running: std::sync::atomic::AtomicBool,
    log_active: std::sync::atomic::AtomicBool,
}

impl Live {
    fn want(&self) -> live::Want {
        let w = self.want.lock().unwrap_or_else(|p| p.into_inner());
        live::Want { log: w.0, poll: w.1.map(std::time::Duration::from_millis) }
    }

    fn set(&self, f: impl FnOnce(&mut (bool, Option<u64>))) {
        f(&mut self.want.lock().unwrap_or_else(|p| p.into_inner()));
    }
}

/// Starts the one background worker if none runs. It leases the session only between
/// user operations and exits when nothing is wanted or the link is gone.
fn ensure_worker(app: &AppHandle, live: &Live) {
    use std::sync::atomic::Ordering::SeqCst;
    if live.running.swap(true, SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        use tauri::Manager;
        let conn = app.state::<Connection>();
        let live = app.state::<Live>();
        let mut engine = live::Engine::default();
        loop {
            let want = live.want();
            if !conn.is_connected() {
                live.set(|w| *w = (false, None));
                live.log_active.store(false, SeqCst);
                live.running.store(false, SeqCst);
                break;
            }
            let Some(mut lease) = conn.lease_live() else {
                std::thread::sleep(std::time::Duration::from_millis(40));
                continue;
            };
            let kind = conn.status().kind.unwrap_or(PortKind::Usb);
            let emitter = app.clone();
            let result = engine.step(&mut lease.session, kind, &want, &mut |out| match out {
                live::Out::Log(lines) => drop(emitter.emit("device-log", lines)),
                live::Out::Status(ev) => drop(emitter.emit("device-status", ev)),
            });
            let streaming = lease.session.log_streaming;
            live.log_active.store(streaming, SeqCst);
            let (status, stale) = conn.finish_live(lease, &result.clone().map(|_| ()));
            if let Some(status) = status {
                emit_status(&app, status);
            }
            drop(stale);
            match result {
                Err(_) => {
                    live.set(|w| *w = (false, None));
                    live.log_active.store(false, SeqCst);
                    live.running.store(false, SeqCst);
                    break;
                }
                Ok(false) => {
                    if want.idle() && !streaming {
                        live.running.store(false, SeqCst);
                        // A start may have raced in after the check.
                        if live.want().idle() || live.running.swap(true, SeqCst) {
                            break;
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Ok(true) => {}
            }
        }
    });
}

#[tauri::command]
fn start_log(app: AppHandle, conn: State<'_, Connection>, live: State<'_, Live>) -> Result<(), String> {
    if !conn.is_connected() {
        return Err("Not connected to a device.".into());
    }
    live.set(|w| w.0 = true);
    ensure_worker(&app, &live);
    Ok(())
}

/// Returns once `.log:0` has been sent (or the link is gone).
#[tauri::command]
async fn stop_log(app: AppHandle, live: State<'_, Live>) -> Result<(), String> {
    live.set(|w| w.0 = false);
    ensure_worker(&app, &live);
    for _ in 0..250 {
        if !live.log_active.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        tokio_sleep(20).await;
    }
    Ok(())
}

/// Polls status data every `interval_ms` (never overlapping; 250 ms minimum).
#[tauri::command]
fn start_status_polling(app: AppHandle, conn: State<'_, Connection>, live: State<'_, Live>, interval_ms: u64) -> Result<(), String> {
    if !conn.is_connected() {
        return Err("Not connected to a device.".into());
    }
    live.set(|w| w.1 = Some(interval_ms.max(250)));
    ensure_worker(&app, &live);
    Ok(())
}

#[tauri::command]
fn stop_status_polling(live: State<'_, Live>) {
    live.set(|w| w.1 = None);
}

/// Default poll interval for the connected link (USB 2 s, Bluetooth 5 s).
#[tauri::command]
fn default_poll_interval(conn: State<'_, Connection>) -> u64 {
    live::default_interval_ms(conn.status().kind.unwrap_or(PortKind::Usb))
}

/// Loaded Configurator builds (from the user's own downloads), kept in memory.
#[derive(Default)]
struct DefsState(std::sync::Mutex<defs::Library>);

/// Extracts (or loads from the app-data cache) the definitions of a user-supplied
/// Configurator `.zip`/`.exe` and adds the build to the loaded list.
#[tauri::command]
async fn load_definitions(app: AppHandle, state: State<'_, DefsState>, images: State<'_, defs::present::ImagesState>, path: String) -> Result<Vec<defs::Summary>, String> {
    use tauri::Manager;
    let cache = app.path().app_data_dir().ok();
    let path_for_images = path.clone();
    let started = std::time::Instant::now();
    log::info!("definitions: loading a build ({} chars in the path)", path.len());
    let (d, from_cache) = tauri::async_runtime::spawn_blocking(move || defs::load(std::path::Path::new(&path), cache.as_deref()))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| {
            log::error!("definitions: load failed after {} ms: {e}", started.elapsed().as_millis());
            e.to_string()
        })?;
    log::info!("definitions: extracted in {} ms (from cache: {from_cache}), {} families", started.elapsed().as_millis(), d.families.len());
    defs::present::add_build(&app, &images, &path_for_images);
    let mut lib = state.0.lock().unwrap();
    lib.add(d, from_cache);
    let summaries = lib.summaries();
    log::info!("definitions: {} builds loaded, total {} ms", summaries.len(), started.elapsed().as_millis());
    Ok(summaries)
}

#[tauri::command]
fn remove_definitions(state: State<'_, DefsState>, images: State<'_, defs::present::ImagesState>, product_version: String) -> Vec<defs::Summary> {
    defs::present::remove_build(&images, &product_version);
    let mut lib = state.0.lock().unwrap();
    lib.remove(&product_version);
    lib.summaries()
}

#[tauri::command]
fn definitions_summary(state: State<'_, DefsState>) -> Vec<defs::Summary> {
    state.0.lock().unwrap().summaries()
}

/// Composes the parameter set for a document (04 §3) and returns the editor model.
/// The spec ID is 1 unless the connected device/file reported another one; there is
/// no way to choose one freely. Offline the feature mask is unknown: all bits set.
#[tauri::command]
fn compose_definitions(
    state: State<'_, DefsState>,
    hw_code: String,
    cfg_version: String,
    device_spec_id: Option<u32>,
    feature_mask: Option<String>,
) -> Result<defs::editor::EditorModel, String> {
    let lib = state.0.lock().unwrap();
    if lib.is_empty() {
        return Err("no definitions loaded".into());
    }
    let (d, fam) = lib.find(&hw_code).ok_or_else(|| format!("hardware code {hw_code} is not in any loaded Configurator build"))?;
    let fam = &d.families[&fam];
    let spec = device_spec_id.map_or(defs::compose::SpecSource::Standard, defs::compose::SpecSource::ReportedByDevice);
    let started = std::time::Instant::now();
    let c = defs::compose::try_compose(fam, &hw_code, &cfg_version, spec, feature_mask.and_then(|m| m.trim().parse::<u64>().ok()).unwrap_or(u64::MAX))
        .inspect_err(|e| log::warn!("definitions: compose failed for {hw_code} / {cfg_version}: {e}"))?;
    let model = defs::editor::editor_model(&c, fam, &d.product_version);
    log::info!("definitions: composed {hw_code} / {cfg_version} in {} ms", started.elapsed().as_millis());
    Ok(model)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Connection::default())
        .manage(DefsState::default())
        .manage(defs::present::ImagesState::default())
        .manage(Live::default())
        .manage(ble::BleState::default())
        .manage(bt_classic::BtClassicState::default())
        .plugin(diag::plugin())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![list_ports,
            diag::diag_log_info,
            diag::diag_tail,
            diag::diag_frontend_log,
            diag::diag_open_log_dir,
            diag::diag_export,
            ble::start_ble_scan,
            ble::stop_ble_scan,
            bt_classic::bt_classic_start,
            bt_classic::bt_classic_stop,
            bt_classic::bt_classic_poll,
            bt_classic::bt_classic_open_settings,
            bt_classic::bt_classic_pair,
            bt_classic::bt_classic_pair_status,
            open_cfg,
            save_cfg,
            save_text,
            check_write_target,
            connect,
            disconnect,
            connection_status,
            login,
            read_device_config,
            write_device_config,
            reboot,
            reset_defaults,
            start_log,
            stop_log,
            start_status_polling,
            stop_status_polling,
            default_poll_interval,
            load_definitions,
            definitions_summary,
            remove_definitions,
            compose_definitions,
            defs::present::device_image,
            defs::present::device_model,
            defs::present::model_hint
        ])
        .build(tauri::generate_context!())
        .expect("error while building FM Config")
        .run(|app, event| {
            // App close: dropping the session sends .log:0 (it stops the stream first)
            // and the farewell command.
            if let tauri::RunEvent::Exit = event {
                use tauri::Manager;
                let (_, session) = app.state::<Connection>().disconnect();
                drop(session);
            }
        });
}
