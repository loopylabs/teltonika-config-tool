# UI/UX brief

Goal: make FM Config feel like a modern settings tool. Someone should be able to connect a tracker, find a setting by name, understand each value, and safely write changes back, without needing to know parameter IDs.

The references are Mobbin screens. Open the links for the visuals; the notes say which pattern to take from each.

## 1. App shell and navigation
- **Left sidebar with labelled sections:**
  - **Device:** Overview, Status.
  - **Settings:** the definition groups, such as System, GPRS, Data Acquisition, Bluetooth, I/O.
  - **Tools:** All parameters, Records, Firmware.
  - The active item is highlighted, and groups can be collapsed. See [Attio settings](https://mobbin.com/screens/0a791f8c-b118-47fd-86c7-b448949c8951) and [Deputy settings](https://mobbin.com/screens/1fcd6e6f-d3bd-4687-b6aa-f036bf06a2ad).
- **Top bar:**
  - On the left, the connected device chip: model, short IMEI and a status dot.
  - In the middle, global search (⌘K / Ctrl+K) across names, groups, value labels and IDs.
  - On the right, primary actions.
  - See [Kraken](https://mobbin.com/screens/fbbfc3b8-8ae7-46cc-946b-fd6ce2dca710) and [Shopify](https://mobbin.com/screens/258a6718-2a0e-40b2-8d3e-d00215d9632d) for search placement.
- **Primary action button:** the main button reflects state:
  - not connected → "Connect device"
  - connected with no changes → "Read from device"
  - with changes → "Review & write (N)"

## 2. Settings rows
- **Row layout:**
  - On the left: the label in medium weight, with a one-line description or range hint below in muted text, and the ID as a small `#802` badge.
  - On the right: the control, right-aligned at a fixed column width.
  - See [Exa settings form](https://mobbin.com/screens/fd8b084b-9914-4966-b35c-9d21c1e90345), which has a label with helper text and a toggle or select on the right, and [Attio](https://mobbin.com/screens/0a791f8c-b118-47fd-86c7-b448949c8951), which has a dropdown per row.
- **Controls by type:**
  - toggle for boolean
  - dropdown, or a segmented control for 2–4 options, for single choice
  - checkbox group or chips for multiple choice
  - number field with a unit suffix and a friendly hint, e.g. "1440 min · 24 h"
  - text field with a counter
  - masked field with reveal for PINs and keywords
  - list editor for lists
  - hh:mm field for times
- **States:**
  - **Changed rows:** a left accent bar plus a "Modified" chip, and a per-row revert (↺).
  - **Invalid rows:** a red helper text under the control.
  - **Default values:** a "Reset to default" item in the row's ⋯ menu.
- **Group page header:** title, short description, and a count like "12 settings · 2 modified".
- **Grouped cards:** group related rows into cards with sub-headings. See [GitBook](https://mobbin.com/screens/5e25f82f-e668-4a52-8a81-eab81441a987) and [Squarespace](https://mobbin.com/screens/3c1a8c75-175f-49ec-96df-24d3074af92f).

## 3. Connect / devices screen
- **Device table:**
  - Columns: device name or type with a USB/Bluetooth icon, port, status dot with text, model and IMEI once known, and last seen.
  - A "⋯" menu on each row.
  - Sections: USB, Bluetooth (paired), and Nearby BLE.
  - See [Tailscale Machines](https://mobbin.com/screens/7060540b-2ee8-4d9d-b37d-80ce117a9188), which has a search and filter bar, version column and status dots, and [Vanta](https://mobbin.com/screens/3a576fae-5de1-4241-97bd-420225181c90), which has status icons per column.
- **Empty state:**
  - An illustration or icon, a one-line explanation, OS-specific pairing help, and a "Refresh" button.
  - Below it, "Open a .cfg file" as a secondary path.
  - See [Shopify Devices empty state](https://mobbin.com/screens/258a6718-2a0e-40b2-8d3e-d00215d9632d).
- **After connecting:** show a device details card with model, IMEI, firmware, hardware code, SIM status and connection type. See [Surfshark device details](https://mobbin.com/screens/cdd9bbfb-7e95-4357-8d36-03ac1d32b2cf).

## 4. Review before writing to the device (most important safety UX)
- **Review dialog:** "Write to device" always opens a review dialog first.
  - Each row: a checkbox, the setting name and group, then Before → After with value labels and units, not raw values.
  - Footer: a "N of M selected" counter, "Discard selected", and a primary "Write N changes".
  - See [Arcade Review changes](https://mobbin.com/screens/b850abdb-2418-47b3-adaf-eac362756c6d) for Before→After with checkboxes, [Jira Unsaved changes](https://mobbin.com/screens/29e2f1b6-8fb3-434a-9714-1a21e2440393), and [Vercel staged changes](https://mobbin.com/screens/d9755cad-4651-4459-b76c-3933115e38fc) for "Discard all / Publish".
- **Warnings:** shown at the top of the dialog, not in a separate popup. They include hardware mismatch (blocking), newer config version (confirm), and keyword inclusion.
- **Progress:** after confirming, show inline progress in the dialog (chunk x/y). End with a success state, or the honest abort message.

## 5. Dangerous actions
- **Danger zone:** "Reboot", "Reset to defaults" and (later) "Firmware update" go in a Danger zone section on the Device → Overview page. Use red text buttons with one-line consequences, and a typed or explicit confirm for reset. See [Whop danger zone](https://mobbin.com/screens/6edb82e3-1556-483f-82df-7b74e08b30d2).

## 6. General
- **Theme:** light and dark via CSS tokens, the system font, and an 8-px spacing scale. Monospace is used only for IDs and raw values.
- **Keyboard:**
  - ⌘K search
  - ⌘S save file
  - ⌘⇧W review & write
  - Esc closes dialogs
  - arrow keys in lists
- **Feedback:**
  - Toasts for success, bottom-right and auto-dismissing.
  - Inline errors for failures, never console-only.
  - Skeleton loaders while reading from the device.
- **Accessibility:** every control has a label, focus rings are visible, and contrast is AA.
- **Layout:** stays usable at the minimum window size of 900×600.
- **Clean-room:** don't copy Teltonika Configurator's look. We follow general settings-app conventions. Teltonika's public UI screenshots, if the owner supplies them, may be used only to confirm which settings belong together.
