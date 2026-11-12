# Can a universal Craft core be shared by per-app plugins?

> Evidence file. Facts only, each with an anchor a reader can re-check. Business choices and unknowns go to `plan.mjs q add`, never into assumptions.

## Question this file answers

Which parts of a mobile plugin are the same for every Craft app, and could live in one universal plugin that each app's plugin imports, given how the apps are built today?

## Findings

Survey of the local checkouts at their `origin/main` (full report: `analysis/craft-apps-survey.md`; claims below re-checked by hand on 2026-11-12). LightCraft, DesignCraft and PdfCraft have no local checkout and were not surveyed; PhotoCraft and VectorCraft were out of scope.

### The same architecture, implemented once per app

1. **Every engine app has a command registry of the same shape.** EffectCraft: `CommandSpec { id, label, menu, shortcut, alt_shortcuts, params, enabled, run, journal }` (effectcraft `crates/engine/src/cmd.rs:106-121`), 763 commands at run time (`evidence/03-device-feasibility.md` Finding 3). FilmCraft: `CommandSpec { id, label, menu, shortcut, params, enabled, run, journal, named }` (filmcraft `crates/engine/src/commands.rs:23-36` at `d5ead4ba`), about 230 declared. FigCraft: `CommandSpec { id, label, params, history, run }` (figcraft `crates/engine/src/cmd.rs:36-44` at `a4251ca8`), about 644 declared. In all three, `params` is a documentation string for agents.
2. **The types are separate per app.** Each registry, `Session` and `Services` trait is declared in that app's own engine crate; none is shared. EffectCraft's `Session` takes seven `Arc<dyn …>` services (effectcraft `crates/engine/src/lib.rs:104-144`); FilmCraft's takes `Arc<dyn Services>` (filmcraft `crates/engine/src/lib.rs:405-413`, `Session::new(services)` at `:522`, at `d5ead4ba`); FigCraft's `Session` holds no trait objects (figcraft `crates/engine/src/lib.rs:118`; `grep -c 'Arc<dyn'` = 0).
3. **Every engine app speaks the same control protocol and has an MCP server.** Requests `{method, params}` answered with `{ok, result}` or `{ok: false, error}`, "the same protocol as the desktop control server" (craft-libs `crates/web-api/src/lib.rs:1-8` at `e7c3dda`). The MCP servers are implemented three times: hand-written JSON-RPC (EffectCraft `crates/automation`, FigCraft `crates/mcp`) and `rmcp` 3.5 with tokio (filmcraft `crates/automation/Cargo.toml:18-19` at `d5ead4ba`).
4. **The CLIs differ.** EffectCraft: 12 subcommands plus `bench`, hand-written parser; FilmCraft: 13, hand-written `key=value`; FigCraft: 9, clap (`analysis/craft-apps-survey.md` §1–3).

### What is already shared

5. **`storytold/craft-libs` is the suite's shared library repository**: 51 crates at `e7c3dda` (list in `analysis/craft-apps-survey.md`), consumed by every surveyed app at different revisions (EffectCraft `e7c3dda`, FilmCraft `1b34eba`, FigCraft `e11f9d9`).
6. **`craft-web-api` is the existing universal-API precedent.** "One page command API for every Craft web build": each app hands the crate its request function and a `Methods` table, and the crate installs the same `window.<app>` shape (`request`, `execute`, `commands`, `docInspect`, `uiInspect`, `screenshot`) for every app; "the crate never knows the app's internals" (craft-libs `crates/web-api/src/lib.rs:1-20`). It replaced per-app copies of that code (`:5-7`).
7. **FilmCraft's codecs are already shared with EffectCraft**: EffectCraft decodes footage through `filmcraft-codecs` and `filmcraft-media` at `d5ead4ba` (effectcraft `Cargo.toml:89-90`; `crates/media/src/stream.rs:87`, `crates/media/src/pool.rs:742`).
8. **Native hardware decoding plugs into that shared decoder.** `filmcraft_codecs::register_video_decoder` puts a factory in front of the built-in software decoders, and `software_video_decoder` is the fallback when a hardware decoder fails mid-stream (filmcraft `crates/codecs/src/lib.rs:99-130` at `d5ead4ba`). `filmcraft-platform`, FilmCraft's one crate allowed `unsafe`, registers VideoToolbox (macOS), Media Foundation, NVDEC and VA-API factories and a hardware H.264 encoder factory for `filmcraft-export` (filmcraft `crates/platform/src/lib.rs:1-12`, `:70-73`; `crates/platform/Cargo.toml:9-13`, `:35-39` at `d5ead4ba`). Its VideoToolbox code is compiled for macOS only (`#[cfg(target_os = "macos")]`, `crates/platform/src/lib.rs:38-54`); it has no iOS or Android code.
9. **Shared platform crates stop short of mobile.** `craft-font-catalog` ("discovery on every platform") classifies Android and iOS as having no font folders to scan (craft-libs `crates/font-catalog/src/env.rs:19-27`); `craft-settings` provides platform folders, config files and atomic saves for the suite (craft-libs `crates/settings/Cargo.toml:8`). craft-libs forbids unsafe code (craft-libs `Cargo.toml:86`) but has OS integration crates through `objc2` (`crates/quit/Cargo.toml:20-23`, `crates/dialogs/Cargo.toml:19-23`).

## Calls the design makes into this system

| Call / trigger | Inputs the plan sends | Defaults when an input is absent | Side effects (writes, emails, downstream syncs) | Constraints it enforces (unique keys, one row per …, allowed values, roles) | What happens on failure (rollback? retry? partial state?) | Anchor |
|---|---|---|---|---|---|---|
| `filmcraft_codecs::register_video_decoder(factory)` | a VideoToolbox (iOS) or MediaCodec (Android) decoder factory | built-in software decoders only | process-wide: every later decoder lookup tries the factory first | a factory registered twice is kept once | a factory returning `None` passes to the next; a failing hardware decoder can fall back to `software_video_decoder` | filmcraft `crates/codecs/src/lib.rs:99-130` @ `d5ead4ba` |
| `craft-web-api` pattern (reused, not called) | — | — | — | the API never knows the app's internals; the app supplies its request function and method table | — | craft-libs `crates/web-api/src/lib.rs:1-20` |

## Consequences for the design

- **A universal core is feasible and matches how the suite already shares code.** What every Craft app's plugin needs is the same: a JSON request channel (`request`, `execute`, `commands`, inspect), progress and cancellation, error mapping, GPU device creation for Metal and Vulkan, app-container folders, fonts, native video decoding through FilmCraft's decoder registry, and the Swift and Kotlin packaging. None of it depends on what a command does.
- **What stays per app:** the engine wiring (EffectCraft's `effectcraft_host::session()`, FilmCraft's `Session::new(services)`, FigCraft's bare `Session`), the generated typed facade (each app's registry), and app-specific adapters (EffectCraft's `Gpu::new`, boa's Android feature).
- **The core follows the `craft-web-api` shape**: the app hands the core its request function and a method table, so the core never depends on an app crate. A later FilmCraft or FigCraft plugin adds an app crate and imports the same core.
- **Shared gaps are fixed once:** iOS and Android support for `craft-font-catalog` and for `filmcraft-platform`'s VideoToolbox code, and a MediaCodec decoder, serve every app's plugin.
- **Packaging constraint:** several app plugins in one host app each carry a Rust runtime; how they coexist (dynamic frameworks, or one combined native library) is a platform question (`evidence/05-mobile-platforms.md`).

## Raised as questions

Q9 (build the universal core now or after EffectCraft), Q10 (where the universal core lives).

## Sources

- `analysis/craft-apps-survey.md` (read-only survey of effectcraft `10376058`, filmcraft `f4090d7b`, figcraft `a4251ca8`, craft-launcher `7aafad6`, craft-libs `e7c3dda`), 2026-11-12, re-checked with `git show` at filmcraft `f7db6cdb` and `d5ead4ba`, figcraft `a4251ca8`, and the craft-libs Cargo checkout at `e7c3dda`.
