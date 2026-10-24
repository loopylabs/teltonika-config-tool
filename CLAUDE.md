# Teltonika Config Tool

A cross-platform (macOS, Windows, Linux) configurator for Teltonika FM trackers, built with Tauri 2 (Rust and TypeScript). See README.md for building and layout.

## Clean-room rules (mandatory)

This project reimplements the Windows Teltonika Configurator for interoperability. It uses a **clean-room process**:

- **Implementation work in this repo uses only the released specs in `docs/spec/`**, plus public Teltonika documentation and observations of real devices.
- **Never read, search, open or decompile:**
  - `../teltonika-re/` (the analysis team's workspace, including its notes and spec drafts)
  - the Teltonika Configurator executable or any DLLs extracted from it
  - any decompiled or reconstructed Teltonika code
- Don't use REA or any other decompiler or disassembler against Teltonika software from an implementation session.
- If a spec is missing, ambiguous or wrong, don't work around it by looking at the original. Add the question to `docs/spec/open-questions.md` and stop that line of work.
- Never commit vendor binaries, extracted resources, or real device `.cfg` files (they contain credentials).

## Spec flow

1. The analysis team writes drafts in `../teltonika-re/spec-draft/`.
2. A separate reviewer, who has not seen the decompiled code, checks each draft for code, internal identifiers or copied text.
3. Only approved specs are copied into `docs/spec/`, and each is recorded in `docs/spec/PROVENANCE.md`.

## Build hygiene

- Rust build artifacts are large. Check `du -sh src-tauri/target` before release or installer builds, and run `cargo clean` afterwards.
- `cd src-tauri && cargo test` runs the Rust tests. `npm run build` type-checks and builds the frontend.
