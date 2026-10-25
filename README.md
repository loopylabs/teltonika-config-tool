# FM Config

A cross-platform desktop app for configuring **Teltonika FM-series GPS trackers** over **USB** or **Bluetooth**, on **macOS, Windows and Linux**. It's an open alternative to the Windows-only Teltonika Configurator.

Built with [Tauri 2](https://tauri.app): a Rust backend and a TypeScript + Vite frontend. Installers are small (roughly 5–15 MB) because the app uses the operating system's own web view.

> **Not affiliated with Teltonika.** "Teltonika", "FMB", "FMC", "FMM" and model names are trademarks of their owners and are used only to say which devices this tool works with. This project ships **no Teltonika software, data, images or firmware**. See [How it works](#how-it-works-and-what-you-need) and [Clean-room approach](#clean-room-approach).

---

## Features

| Area | What you get | Status |
| --- | --- | --- |
| **Connect** | USB serial; Bluetooth Classic (direct RFCOMM, or the OS serial port of a paired tracker); in-app Bluetooth scan with a **Tracker** badge; pairing with a PIN (default `5555`) | ✅ USB and Bluetooth (RFCOMM, tested on a real FMB150) |
| **Device info** | Model name and picture, IMEI, firmware, hardware code, configuration version, spec | ✅ |
| **Read and write configuration** | Read the full configuration from the device, edit it, then **Review & write**: every change shown as before → after, with checkboxes, sent in chunks; on any failure nothing is saved | ✅ (writing not yet tested on hardware) |
| **Readable editor** | Settings grouped as on the device, with names first (ID as small text), units (`1440 min (1 d)`), and the right control per type: toggles, dropdowns, checkboxes, number ranges, hh:mm, masked PINs and list editors. Validation, revert and reset to default. ⌘K / Ctrl+K search by name, group, value or ID | ✅ needs definitions (see below) |
| **`.cfg` files** | Open and save Teltonika `.cfg` files with a byte-identical round trip. Safety rules: refuses values with `;`, leaves the device keyword out unless you tick a box, blocks writing a file made for different hardware | ✅ |
| **Telemetry** | Live tiles (position, fix, external and battery voltage, device time, uptime, GSM counters…), map with breadcrumb trail (OpenStreetMap), live I/O table with names and units, sparklines, record session to CSV | ⚠️ UI done; the live status exchange doesn't return data on the one tracker tested yet ([open question 78](docs/spec/open-questions.md)) |
| **Device log** | Live debug log from the tracker: start and stop, filter, save to `.txt` | ✅ |
| **Device actions** | Reboot, reset to defaults (type RESET to confirm) | ✅ |
| **BLE scanner** | Tools → BLE scanner: nearby Bluetooth LE sensors and beacons (trackers are configured over USB or Bluetooth Classic, not BLE) | ✅ |
| **Diagnostics** | Tools → Diagnostics: live app log, log level, "Export diagnostics…". Keywords, PINs and configuration values are always redacted | ✅ |
| **Firmware update** | — | ⏳ Not available: official firmware files are encrypted, and this project won't bypass that ([spec 06](docs/spec/06-firmware.md)) |
| **Records download** | Read stored AVL records (Codec 8) | ⏳ Spec ready, UI not built |

## Supported devices

The app works with the Teltonika **FMB family** and the **FMB640 family**. It covers every model in the hardware table of the Configurator you load: **76 hardware codes** with Configurator 1.8.21, plus `FMM0:31` if you also load 1.8.10. Examples:

| Category | Models (examples) |
| --- | --- |
| Compact / Easy & Slim | FMB900, FMB910, FMB920, FMC920, FMM920, FMT100, FMP100 |
| OBD plug-in | FMB001, FMB002, FMB003, FMB010, FMB020, FMC001, FMC003, FMC00A, FMM001, FMM003, FMM00A |
| Professional | FMB140, FMB150 and the rest of the FMB1xx line |
| Heavy duty (FMB640 family) | FMB640, FMB641, FMC640, FMM640, FMC650, FMM650 |

**Tested on real hardware so far:**
- **FMB150:** connect over Bluetooth RFCOMM, device info, configuration session, keyword status.
- **FMB140:** a real `.cfg` export opens with 100% of its 8,514 parameters named and validated.

Everything else is tested against the protocol specs and recorded test vectors. Please [open an issue](../../issues) with your model and results.

## How it works (and what you need)

The app talks to the tracker itself. **Parameter names, groups, units and ranges, and device pictures, come from your own copy of the official Teltonika Configurator.** The app reads them locally and caches them on your computer. It never uploads or redistributes them.

1. Download **Teltonika Configurator** from Teltonika's website. Use the `.zip`; you don't need to run it, and you don't need Windows. Version **1.8.21** is tested. You can load several versions at once, e.g. 1.8.21 and 1.8.10; the newest is used first.
2. In FM Config, go to **Devices → Parameter definitions → Load definitions…** and pick the `.zip`.

Without definitions you can still connect, read, write and edit. Settings then show as "Unnamed parameter #id", apart from a few the app knows by itself, such as the Bluetooth settings.

## Install

There are no prebuilt releases yet, so build from source (below). The installer ends up in `src-tauri/target/release/bundle/`:

| OS | File | Notes |
| --- | --- | --- |
| **macOS** | `bundle/dmg/FM Config_<version>_<arch>.dmg` | The app isn't signed yet: right-click → **Open** the first time. Allow **Bluetooth** access when asked. |
| **Windows** | `bundle/msi/*.msi` or `bundle/nsis/*-setup.exe` | Windows may show a SmartScreen warning for unsigned apps: **More info → Run anyway**. Uses the WebView2 runtime, included in Windows 10/11. |
| **Linux** | `bundle/deb/*.deb`, `bundle/appimage/*.AppImage` | Add yourself to the `dialout` group for USB serial (`sudo usermod -aG dialout $USER`, then log in again). |

### Connecting a tracker

- **USB:** plug it in, then go to **Devices → USB → Connect**.
- **Bluetooth:**
  1. Go to **Devices → Nearby Bluetooth → Scan**. Your tracker appears with a **Tracker** badge.
  2. Click **Pair…**. The default PIN is **5555**, unless it has been changed on the tracker (parameter 802).
  3. Click **Connect**.
  
  Trackers already paired in your OS also appear under **Bluetooth (paired)**. Make sure the tracker is powered and awake, with Bluetooth enabled and visible.
- **Password-protected trackers** ask for the configuration keyword. It's never stored or logged.

## Build from source

### Prerequisites

- **Node.js** 22+ and npm
- **Rust** (stable): <https://rustup.rs>
- **Tauri system dependencies** for your OS: <https://tauri.app/start/prerequisites/>
  - **macOS:** Xcode Command Line Tools (`xcode-select --install`)
  - **Windows:** Microsoft C++ Build Tools and WebView2 (preinstalled on Windows 10/11)
  - **Linux (Debian/Ubuntu):**
    ```sh
    sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev \
      libssl-dev libayatana-appindicator3-dev librsvg2-dev libudev-dev libdbus-1-dev
    ```
    `libudev-dev` is for serial ports and `libdbus-1-dev` is for Bluetooth (BlueZ).

### Build and run

```sh
git clone https://github.com/loopylabs/teltonika-config-tool.git
cd teltonika-config-tool
npm install

npm run tauri dev      # run in development mode, with live reload
npm run tauri build    # build an optimized installer for this OS (see "Install")
```

Rust release builds take a few minutes and use a lot of disk. Run `cargo clean` in `src-tauri/` afterwards if you need the space back.

### Tests

```sh
npm run build                              # type-check and build the frontend
npm test                                   # frontend unit tests
cd src-tauri && cargo test                 # Rust unit tests (protocol, framing, definitions, ...)
```

Optional checks against your own files run only when these environment variables are set. They print counts and IDs only, never values:

```sh
FMCONFIG_CONFIGURATOR=~/Downloads/Teltonika.Configurator_1.8.21_B.4.01_R.10.zip \
FMCONFIG_SAMPLE_CFG="/path/to/your-device.cfg" \
  cargo test
```

Never commit `.cfg` files. They contain server addresses and APN credentials, and `.gitignore` excludes them.

### Platform notes

- **macOS:** Bluetooth Classic scanning, pairing and direct RFCOMM work. When running `npm run tauri dev`, the Bluetooth permission belongs to the terminal app that started it.
- **Windows:** USB and paired Bluetooth (the OS serial port) work. In-app Bluetooth scanning and pairing aren't implemented yet; pair in **Settings → Bluetooth** instead.
- **Linux:** USB and paired Bluetooth (`/dev/rfcommN`) work. The BlueZ scan backend is written but not yet tested.

Windows and Linux builds aren't tested on hardware yet. Reports are welcome.

## Clean-room approach

The original Configurator was studied **only to write behaviour specifications** (in [`docs/spec/`](docs/spec/)), never to copy code. Three roles never mix:

1. **Analysis:** studies the original tool and writes spec drafts: wire formats, commands, file formats and test vectors.
2. **Review:** someone who never saw the original checks every draft for copied code, internal names or text before releasing it ([`PROVENANCE.md`](docs/spec/PROVENANCE.md)).
3. **Implementation:** writes this app **only from the released specs**, public documentation and real-device observations.

Also:
- No vendor cryptographic secrets are extracted or used.
- Encrypted firmware isn't decrypted.
- No vendor data is redistributed.
- Questions from implementers go to [`docs/spec/open-questions.md`](docs/spec/open-questions.md).

Rules for contributors, including AI coding agents, are in [`AGENTS.md`](AGENTS.md).

## Project layout

```
src/                       frontend (TypeScript, Vite)
  main.ts                  app controller and views
  ui/                      shell, devices, nearby Bluetooth, telemetry, map, review dialog, diagnostics…
  controls.ts values.ts    per-type editors and value formatting (units, hh:mm, enums, masks)
src-tauri/src/             backend (Rust)
  transport.rs ports.rs    serial transport and port discovery
  framing.rs fmbx.rs       text and binary (FMBX, CRC-16/ARC) framing
  device.rs commands.rs    session and device commands (info, read/write config, keyword, reboot…)
  connection.rs live.rs    connection state, status polling and device log
  bt_classic/              Bluetooth Classic scan, pairing and RFCOMM (macOS; Linux backend)
  ble.rs                   BLE scanner
  defs/                    reads definitions and pictures from your Configurator; per-device composition
  cfg.rs                   .cfg file format
  diag.rs                  logging with redaction
docs/spec/                 clean-room specifications (01 transport … 10 device presentation)
docs/design/ui-brief.md    UI/UX design brief
```

## Status and roadmap

This is early software: use it on a test tracker before touching vehicles in service.

Next up:
- fix live status on real devices (open question 78)
- test writing a configuration on real hardware
- records download
- CI builds for Windows and Linux
- signed releases

## License

No license has been chosen yet. Until one is added, all rights are reserved by the authors.
