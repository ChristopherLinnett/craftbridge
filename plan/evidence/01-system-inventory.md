# System inventory

> Evidence file. Facts only, each with an anchor a reader can re-check. Business choices and unknowns go to `plan.mjs q add`, never into assumptions.

## Question this file answers

Which code, toolchains and platform facilities does a native iOS and Android SDK for EffectCraft build on, and at which revisions?

## Findings

### The upstream codebase

1. **EffectCraft** is the Rust workspace `storytold/effectcraft`, pinned at `origin/main` `10376058` (full `103760584a60074c9aaeef686d2d05d89f08155e`, "Merge pull request #3533", fetched 2026-11-12). Version 0.24.1, edition 2024, MSRV 1.95, licence `MIT OR Apache-2.0` (`Cargo.toml:6-11`). The local checkout `effectcraft/repository` is read through `git show 10376058:<path>`; its working branch is left as it was.
2. **Unsafe code is forbidden workspace-wide**: `unsafe_code = "forbid"` (`Cargo.toml:137-138`). Every production crate denies `unwrap`, `expect`, `panic!` and friends (`AGENTS.md`, "Never crash").
3. **Layering** is enforced by `cargo xtask layers`: each crate has a layer L0–L6, UI and OS crates (`egui`, `eframe`, `winit`, `rfd`, `cpal`, `muda`, `craft-ui`) may not appear below L5, and every L0–L4 crate must build for `wasm32-unknown-unknown` (`xtask/src/main.rs:35-71`, `:96`, `:242-262`). The engine, host and scripting crates are L4, the automation crate L5, the CLI L6 (`xtask/src/main.rs:63-70`).
4. **Pure-Rust media.** Decoding comes from FilmCraft's crates pinned at `d5ead4ba` (`Cargo.toml:89-90`); containers, audio and video encoders (H.264, HEVC, VP9, AV1, ProRes) from `storytold/craft-libs` at `e7c3dda7` (`Cargo.toml:83-85`). No operating-system codec is linked.
5. **GPU** is wgpu 30.0.1 (`Cargo.toml:107`), the same wgpu the desktop window draws with. The compositor asks wgpu for the best adapter on any native backend (`crates/gpu/src/context.rs:1403-1442`) and can also run on a device the host already owns: `Gpu::new(adapter, device, queue)` (`crates/gpu/src/lib.rs:163-166`).
6. **Interpreters, no JIT.** Expressions and scripts run in `boa_engine` 0.22, WebAssembly effect plug-ins in `wasmi` 2, an interpreter (`Cargo.toml:68-70`).

### The CLI the plugins must match

7. The CLI is `apps/effectcraft-cli`, one 1,671-line `main.rs` plus `template.rs` and `project_lock.rs`. Its subcommands are `info`, `commands`, `exec`, `run`, `props`, `get`, `set`, `render-frame`, `render`, `bench`, `script`, `replay` and `mcp` (`apps/effectcraft-cli/src/main.rs:1-43`, dispatch `:622-849`).
8. **The CLI is a thin shell over libraries.** `info`, `props`, `get`, `set` and `commands` call automation tools (`get_project`, `get_comp`, `get_layer`, `get_property`, `set_property`, `list_commands`) through `tool()` (`apps/effectcraft-cli/src/main.rs:509-515`, `:630-672`, `:746-821`); `exec` and `run` call `Backend::exec(id, params)` per step (`:676-745`); `render-frame` calls `Backend::render_to_file` (`:789-821`); `render` drives the engine's Render Queue commands `renderQueue.add` and `renderQueue.render` (`:888-1037`); `mcp` wraps `McpServer` (`:822-845`). What lives only in the binary is argument parsing, output formatting, the project file lock, template preparation and the benchmarks.
9. **The CLI's one UI dependency** is the operations benchmark: `bench --ops` calls `effectcraft_ui_egui::bench::ui_ops` (`apps/effectcraft-cli/src/main.rs:1224`), which pulls the desktop UI toolkit into the CLI (`apps/effectcraft-cli/Cargo.toml:16-30`).

### The engine surface the plugins call

10. **Everything is a command.** A `CommandSpec` holds `id`, `label`, `menu`, `shortcut`, a `params` documentation string, `enabled`, `run` and `journal` (`crates/engine/src/cmd.rs:106-121`); `command_specs()` returns the registry (`crates/engine/src/cmd.rs:175-181`). At run time the registry holds 763 commands (`effectcraft-cli info`, `evidence/03-device-feasibility.md` Finding 3); a source grep for `cmd!("<id>"` finds only 382 distinct ids, so counting the source undercounts and the generator must read the registry itself (counted with `grep -rho 'cmd!("[a-zA-Z0-9_.]*"' crates/engine/src | sort -u | wc -l` at the pin), the largest groups being `layer.` (76), `view.` (38) and `file.` (34).
11. **Parameter schemas carry names, not types.** `params_schema` derives a JSON Schema from the `params` string: one property per accepted key, required unless marked `?` or given alternatives, each described by its piece of the documentation string, with no value types (`crates/engine/src/cmd/schema.rs:77-101`). `check_params` rejects unknown keys (`crates/engine/src/cmd/schema.rs:109-123`).
12. **A headless `Session` with injectable services.** `Session` holds the project, history and editor state, and its platform-facing parts as trait objects: `Services` (file access), `FootageSource` (decoded frames and audio), `ExprHost`, `Accelerator` (GPU compositor), `Importer`, `Exporter` and `StorageQuota` (`crates/engine/src/lib.rs:104-144`; traits at `crates/engine/src/services.rs:4-17`, `crates/render/src/footage.rs:20-47`, `crates/engine/src/render_queue.rs:94-112`).
13. **`effectcraft_host::session()`** builds the wired session every frontend starts from: FilmCraft decoding (`MediaPool`), the media importer, the file exporter, expressions, WebAssembly plug-ins, external effect helpers and scripting (`crates/host/src/lib.rs:96-115`).
14. **A `FootageSource` can be wrapped.** The automation crate's `ConfinedFootage` already wraps another `FootageSource` and delegates to it (`crates/automation/src/confine.rs:430-480`), the pattern a native-decoder adapter can follow.
15. **The encoders are fixed inside `effectcraft-export`.** H.264 export constructs `craft_video::h264::Encoder` directly (`crates/export/src/encode.rs:119-139`); the crate has no encoder trait (no `pub trait` in `crates/export/src`). A hardware encoder can only replace them through a new seam in that crate, or by re-implementing the export pipeline behind `Exporter`.
16. **The automation layer** offers `Backend::headless(session)`, `exec`, `render`, `render_to_file` (`crates/automation/src/backend.rs:33-130`), a catalogue of 36 tools of which 9 need the desktop app (`crates/automation/src/tools.rs:93-104`), and an MCP server whose `handle_line(&str) -> Option<String>` needs no standard input (`crates/automation/src/server.rs:92-102`).

### Where the code assumes a desktop operating system

17. **Config folder.** `platform_dir` knows macOS, Windows and XDG; iOS and Android fall through to `$HOME/.config/effectcraft`. `EFFECTCRAFT_CONFIG_DIR` overrides it (`crates/host/src/app_dirs.rs:59-61`, `:78-88`).
18. **System fonts.** `DirectorySource::system` lists macOS, Windows and Linux/BSD folders; iOS and Android match neither the macOS branch nor real Linux folders. `EFFECTCRAFT_FONT_DIRS` adds folders (`crates/text/src/fonts.rs:79-101`).
19. **Free memory.** `sysinfo::memory()` has branches for Linux, macOS, FreeBSD and Windows only (`crates/engine/src/system/sysinfo.rs:143-160`), so export sizing gets no memory figure on iOS or Android.
20. **Helper processes.** External effect helpers run as child processes on every native target (`crates/host/src/lib.rs:117-129`); only `wasm32` excludes them (`crates/host/src/lib.rs:130-134`).
21. **Brand.** The ArtCraft name and marks are trademarks, not open source; they may not appear in the name, listing or marketing of anything other than EffectCraft without the ArtCraft Team's written permission (`docs/brand/LICENSE-brand.txt:1-42`).

### Toolchains on the build machine (2026-11-12)

22. rustc 1.99.1, Xcode 26.6, Android NDK 27.0.12077973 and 28.2.13676358, iOS 26.5 simulators, an Android emulator image (`Pixel_10_Pro_XL`). Rust targets `aarch64-apple-ios`, `aarch64-apple-ios-sim` and `aarch64-linux-android` were added with `rustup target add` for this investigation.

## Calls the design makes into this system

| Call / trigger | Inputs the plan sends | Defaults when an input is absent | Side effects (writes, emails, downstream syncs) | Constraints it enforces (unique keys, one row per …, allowed values, roles) | What happens on failure (rollback? retry? partial state?) | Anchor |
|---|---|---|---|---|---|---|
| `effectcraft_host::session()` | none | config folder from `EFFECTCRAFT_CONFIG_DIR` or the platform convention; models and plug-ins folders under it; helpers scanned from default paths | sets the helper blocklist folder; installs scripting | none | infallible; missing folders leave `models_dir` and `plugins_dir` `None` | `crates/host/src/lib.rs:96-129` |
| `Backend::exec(id, params)` | command id, JSON params | `null` params become `{}` | runs the command against the session (undoable, journalled when `journal`) | unknown keys rejected by `check_params`; confinement checks when confined | returns `Err`; the guard keeps the document | `crates/automation/src/backend.rs:69-96`, `crates/engine/src/cmd/schema.rs:109-123` |
| `tools::run(backend, name, args)` | tool name, JSON args | per tool | per tool | bridge-only tools refused headless | `Err` | `crates/automation/src/tools.rs:111` |
| `Backend::render_to_file` | comp, time, max side, transparent, path | active comp, current time, full size | writes a PNG | path checked by `check_output` when confined | `Err` | `crates/automation/src/backend.rs:116-130` |
| `renderQueue.add` + `renderQueue.render` | output path, format and render settings | project's render settings | writes the output; progress in `JobState` | format must be in `Exporter::formats()` | a cancelled export removes its files and returns `cancelled` | `apps/effectcraft-cli/src/main.rs:888-1037`, `crates/engine/src/render_queue.rs:94-112` |
| `Gpu::new` / `Gpu::try_headless` | adapter, device, queue (or none) | best adapter, default limits clamped for texture size and buffer size | owns the device; installs readback handlers | requests `wgpu::Limits::default()` for every other limit | `Err(String)`: the session renders on the CPU | `crates/gpu/src/lib.rs:156-166`, `crates/gpu/src/context.rs:1430-1442` |
| `McpServer::handle_line` | one JSON-RPC line | — | per tool | per tool | JSON-RPC error object | `crates/automation/src/server.rs:92-102` |

## Consequences for the design

- The plugin's native core can depend on the unmodified upstream crates `effectcraft-engine`, `effectcraft-host`, `effectcraft-automation` and `effectcraft-gpu`; the CLI binary itself is not a dependency, because its orchestration is already in those libraries.
- The generated API can take command ids, labels, parameter names and optionality from the registry, but parameter value types are not in the registry; typed values need either a hand-kept type map or an upstream change.
- Native decoders slot in behind `FootageSource` with no upstream change; native encoders need a seam in `effectcraft-export`.
- Config, cache and font folders, free memory and helper processes need platform handling on iOS and Android, either through environment overrides or upstream seams.
- The FFI layer needs `unsafe` (exported symbols, raw window handles), which the upstream workspace forbids, so it lives outside that workspace.
- The product name must not use the ArtCraft name unless the ArtCraft Team permits it.

## Raised as questions

Q2 (naming), Q4 (linkage), Q5 (upstream seams), Q6 (scope), Q7 (API shape), Q8 (frame output).

## Sources

- `storytold/effectcraft` at `10376058`, read with `git show` and an exported tree (`git archive 10376058`), 2026-11-12.
- `storytold/filmcraft` at `d5ead4ba` (dependency pin).
- `rustc --version`, `xcodebuild -version`, `rustup target list --installed`, the Android SDK folder, `xcrun simctl list`, 2026-11-12.
