# Feasibility verdict: is the plugin port feasible, on what conditions, at what cost?

> Evidence file. Facts only, each with an anchor a reader can re-check. Business choices and unknowns go to `plan.mjs q add`, never into assumptions.

## Question this file answers

Can everything the EffectCraft CLI does be offered as native iOS (Swift) and Android (Kotlin) plugin APIs, running on each platform's own GPU and media facilities, kept in step with upstream EffectCraft, and is a universal Craft core worth building alongside?

## Verdict

**Feasible.** The upstream engine compiles for iOS and Android without a source change, and the unmodified CLI's document capabilities ran on an iPhone simulator and an Android emulator, with the GPU compositor on Metal and on Vulkan and CPU output byte-identical to the macOS desktop build (`evidence/03-device-feasibility.md` Findings 1–9). What remains is integration work around the engine, not changes to it.

## Findings

### Requirement by requirement

| Requirement | Feasible? | Basis | Condition |
|---|---|---|---|
| RQ1 Swift and Kotlin plugins | Yes | the engine stack type-checks and the CLI runs on both platforms (`evidence/03-device-feasibility.md` Findings 1–3) | an FFI layer outside the upstream workspace, which forbids `unsafe`; UniFFI's scaffolding cannot compile under that rule (`evidence/01-system-inventory.md` Finding 2, `evidence/05-mobile-platforms.md` Findings 6–7) |
| RQ2 everything the CLI can do | Yes, for every document capability | `info`, `commands`, `exec`, `run`, `props`, `get`, `set`, `render-frame`, `render` (H.264, HEVC, GIF), `script`, `replay`, `mcp` ran on device (`evidence/03-device-feasibility.md` Findings 3, 9) | the CLI's `--bridge` mode and its UI and Roto Brush benchmarks drive or measure the desktop app (Q6) |
| RQ3 native GPU | Yes | Metal on iOS, Vulkan on Android, through the same wgpu compositor (`evidence/03-device-feasibility.md` Findings 6–8) | the plugin creates the device (backend, clamped limits) and passes it to `Gpu::new`; GL-only Android devices render on the CPU |
| RQ3 native media | Decode: yes. Encode: needs a seam | hardware decoders plug into FilmCraft's decoder registry, which EffectCraft already uses (`evidence/04-craft-suite-shared-core.md` Finding 8); EffectCraft's encoders are fixed inside `effectcraft-export` (`evidence/01-system-inventory.md` Finding 15) | VideoToolbox code exists for macOS only and MediaCodec code does not exist yet; hardware encoding waits for an encoder seam upstream (#1840's area) *(Superseded by PD5, 2026-11-13: EffectCraft is never modified, so there are no seams; what a phone needs is an adaptation in CraftBridge's own crates on EffectCraft's public interfaces, plan §15.)* |
| RQ4 linked to upstream | Yes, mechanically | the plugin depends on upstream crates at a pinned revision; the registry (763 commands, names and optionality) and the tool catalogue (27 headless tools) are machine-readable at run time (`evidence/01-system-inventory.md` Findings 10–11, 16) | parameter value types are prose hints, so typed wrappers beyond names and optionality need a hand-kept map or an upstream schema change (Q7) |
| RQ8 universal core | Yes | three Craft engines share the registry, session and control-protocol shape; `craft-web-api` already shares the web equivalent (`evidence/04-craft-suite-shared-core.md` Findings 1–6) | per-app wiring stays per app; the core and every app crate in one host app compile into one native library, because two Rust static libraries collide (`evidence/05-mobile-platforms.md` Finding 8) |

### What the build has to add around the unchanged engine

1. **An FFI layer** in the plugin repository (exported symbols, platform handles), generating Swift and Kotlin bindings.
2. **GPU device creation** per platform: Metal on iOS, Vulkan on Android, limits clamped to the adapter, handed to `Gpu::new` (`evidence/03-device-feasibility.md` Findings 6–8).
3. **boa's `jsvalue-enum` feature on Android**, or every script and expression panics (`evidence/03-device-feasibility.md` Finding 9).
4. **App-container folders** for config, cache, models and plug-ins, and **font discovery** for system fonts, which neither EffectCraft nor `craft-font-catalog` provides on mobile (`evidence/03-device-feasibility.md` Findings 10–11; `evidence/04-craft-suite-shared-core.md` Finding 9).
5. **Native video decoders** registered through `filmcraft_codecs::register_video_decoder` (VideoToolbox on iOS, MediaCodec on Android).
6. **No helper processes**: external effect helpers spawn child processes (`evidence/01-system-inventory.md` Finding 20), which iOS forbids and Android allows only for binaries shipped in the APK (`evidence/05-mobile-platforms.md` Findings 14–15).
7. **Packaging**: one native library per distribution holding the core and the app crates, wrapped as an XCFramework for Swift Package Manager and an AAR for Gradle (`evidence/05-mobile-platforms.md` Findings 8, 10–11).
8. **A conformance suite** that replays the CLI's capabilities through the plugin APIs and compares rendered frames byte-for-byte with the desktop build (`evidence/03-device-feasibility.md` Findings 4–5).

### Risks

| Risk | Evidence | Mitigation the plan can take |
|---|---|---|
| Physical-device GPU limits and drivers differ from the simulator and emulator | only the simulator and emulator were tested (`evidence/03-device-feasibility.md` Findings 6–7) | device acceptance on a physical iPhone and on Adreno and Mali phones before release |
| Size: about 62 MB stripped per architecture | `evidence/03-device-feasibility.md` Finding 13 | measure the plugin library itself; feature-gating engine crates is an upstream change (Q5) |
| Upstream renames or removes commands | 763 commands, changing weekly (`evidence/02-in-flight-work.md` Finding 9) | the update script diffs the generated API surface and fails on removals |
| Hardware encoding depends on upstream accepting a seam | #819 decision, #1840 open and conflicting (`evidence/02-in-flight-work.md` Findings 5–6) | software encoders ship first; hardware encoding behind the seam once it exists *(Superseded by PD5, 2026-11-13: EffectCraft is never modified, so there are no seams; what a phone needs is an adaptation in CraftBridge's own crates on EffectCraft's public interfaces, plan §15.)* |
| Mobile GPU driver bugs (Mali, Adreno) in wgpu | open wgpu issues (`evidence/05-mobile-platforms.md` Finding 3) | the compositor already falls back to the CPU per effect; device acceptance covers both GPU families |
| App Review treats downloaded WebAssembly plug-ins as executable code | Apple's rules do not address interpreted WebAssembly (`evidence/05-mobile-platforms.md` Finding 13) | the host app controls which plug-ins load (Q13) |
| No zero-copy footage on Android | no AHardwareBuffer import in wgpu (`evidence/05-mobile-platforms.md` Finding 5) | decoded frames go through CPU images, as on desktop |

## Calls the design makes into this system

None. This file draws conclusions from `evidence/01`–`05`; their Calls tables hold every call.

## Consequences for the design

- Go: proceed to the scoping plan once the business questions are answered.
- The plan builds around the engine, never inside it: upstream stays unmodified except for the seams the user approves (Q5). *(Superseded by PD5, 2026-11-13: EffectCraft is never modified, so there are no seams; what a phone needs is an adaptation in CraftBridge's own crates on EffectCraft's public interfaces, plan §15.)*

## Raised as questions

All in the first round (Q1–Q15).

## Sources

`evidence/01-system-inventory.md` – `evidence/05-mobile-platforms.md`, `analysis/device-probe/`, 2026-11-12.
