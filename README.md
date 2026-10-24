# FM Config

A cross-platform (macOS, Windows, Linux) configurator for Teltonika FM-series GPS trackers. It's meant to replace the Windows-only Teltonika Configurator.

Built with [Tauri 2](https://tauri.app): a Rust backend with a TypeScript and Vite frontend.

## Status

| Feature | State |
| --- | --- |
| Open and save `.cfg` files (gzip `id:value;` format, byte-identical round trip) | ✅ |
| Edit parameters (search, change tracking, revert) | ✅ |
| List serial ports (USB and Bluetooth) | ✅ |
| Connect to a device, read and write its config, reboot | ⏳ waiting on protocol reverse engineering |
| Firmware update | ⏳ |
| Named settings groups (System, GPRS, I/O…) mapped to parameter IDs | ⏳ |

## Development

Prerequisites: Node 22+, Rust (stable), and the [Tauri system dependencies](https://tauri.app/start/prerequisites/).
On Linux that also includes `libudev-dev`, which serial port access needs.

```sh
npm install
npm run tauri dev      # run the app
npm run tauri build    # build an installer for the current OS
```

Rust tests:

```sh
cd src-tauri
cargo test
# Also check that a real Configurator export round-trips byte for byte:
FMCONFIG_SAMPLE_CFG="/path/to/device.cfg" cargo test
```

`.cfg` files are gitignored because real exports contain server addresses and APN credentials.

## Layout

```
src/                 frontend (vanilla TypeScript)
  api.ts             typed wrappers around the Tauri commands
  main.ts            UI: start screen, workspace, parameter editor
src-tauri/src/
  cfg.rs             .cfg file parsing and serialization
  ports.rs           serial port listing
  lib.rs             Tauri commands
```
