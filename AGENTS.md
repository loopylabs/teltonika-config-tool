# Teltonika Config Tool: agent instructions

These instructions apply to every coding agent and harness: Claude Code, Codex, Grok, Gemini CLI, Cursor and others. `CLAUDE.md` imports this file, so keep the rules here only.

A cross-platform (macOS, Windows, Linux) configurator for Teltonika FM trackers, built with Tauri 2 (Rust and TypeScript). See README.md for building and layout.

## Clean-room rules (mandatory)

This project reimplements the Windows Teltonika Configurator for interoperability. It uses a **clean-room process** with three roles that never mix:

| Role | May see | Produces |
| --- | --- | --- |
| **Analysis** | Original Configurator, decompiled code, `../teltonika-re/` | Notes (stay in `../teltonika-re/notes/`) and spec drafts (`../teltonika-re/spec-draft/`) |
| **Review** | Spec drafts only | Approve or reject, then copy approved specs to `docs/spec/` and log them in `docs/spec/PROVENANCE.md` |
| **Implementation** | `docs/spec/`, this repo, public Teltonika docs, real-device observations | Code in this repo |

An agent, session or person who has seen decompiled code must never take on the review or implementation role.

**Implementation work (the default role in this repo):**
- Never read, search, open or decompile:
  - `../teltonika-re/` (the analysis workspace, including its notes and spec drafts)
  - the Teltonika Configurator executable or any DLLs extracted from it
  - any decompiled or reconstructed Teltonika code
- Don't run any decompiler, disassembler or reverse-engineering tool (e.g. REA, ILSpy, dnSpy, Ghidra) against Teltonika software.
- If a spec is missing, ambiguous or wrong, don't work around it by looking at the original. Add the question to `docs/spec/open-questions.md` and stop that line of work.
- Never commit vendor binaries, extracted resources, or real device `.cfg` files (they contain credentials).

**Spec drafts** contain behaviour and interface facts only: transport settings, byte layouts, field meanings, command IDs, checksum parameters, flows, error cases and test vectors. They contain no code or pseudocode derived from the original, no vendor-internal class, method or variable names, and no copied comments or text beyond protocol constants and user-visible labels.

## Choosing a model and effort per job

When the harness supports subagents or tasks, delegate each job and choose the model tier and reasoning effort to match. Do trivial one-line checks inline.

| Job | Model tier | Effort |
| --- | --- | --- |
| Mechanical: renames, git and repo setup, running builds and tests, moving files | Small / fast | low |
| Routine coding: UI screens, wiring backend commands, tests, implementing a clear spec section | Standard | medium |
| Hard implementation: device protocol layer, firmware update flow, serial-port and concurrency edge cases | Frontier | high |
| Reverse engineering (analysis role only): decompiling, protocol and parameter-map analysis, spec drafts | Frontier | high or max |
| Clean-room spec review | Standard | high |
| Architecture and design decisions | Frontier | high |

Use the equivalent tier in whichever harness you're running. In Claude Code, for example, the tiers are Haiku, Sonnet and Opus, set through the Agent tool's `model` and `effort`. Other harnesses map the same tiers onto their own models and reasoning-effort settings.

Rules:
- Pick the cheapest tier that can do the job reliably. Move up only when the job needs it or a cheaper attempt failed.
- Run independent jobs in parallel, each with a narrow, self-contained brief.
- Never hand an analysis agent's raw output to an implementation agent. Only reviewed specs cross over.

## Build hygiene

- Rust build artifacts are large. Check `du -sh src-tauri/target` before release or installer builds, and run `cargo clean` afterwards.
- `cd src-tauri && cargo test` runs the Rust tests. `npm run build` type-checks and builds the frontend.
