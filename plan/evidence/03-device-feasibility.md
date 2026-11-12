# Does the unmodified engine run on iOS and Android, and what breaks?

> Evidence file. Facts only, each with an anchor a reader can re-check. Business choices and unknowns go to `plan.mjs q add`, never into assumptions.

## Question this file answers

Does EffectCraft's engine, built from the upstream source at the pin, run every CLI capability on iOS and Android, on which GPU path, and what has to change for it to do so?

## Method

- The tree at `10376058` was exported with `git archive` to a scratch folder, so the user's checkout was not touched. Builds used their own target folder.
- **Type-check:** `cargo check --locked --target <T> -p effectcraft-host -p effectcraft-automation -p effectcraft-gpu -p effectcraft-engine` for `aarch64-apple-ios` and `aarch64-linux-android` (`analysis/device-probe/check-mobile.sh`, log `check-mobile.log`).
- **Run:** the EffectCraft CLI itself was built for `aarch64-apple-ios-sim` and `aarch64-linux-android` (`build-cli-mobile.sh`) and run on an iPhone 17 Pro simulator (iOS 26.5, `xcrun simctl spawn`) and an Android emulator (Pixel 10 Pro XL image, API 37, arm64-v8a, `adb shell`). `conformance.sh ios|android` drives the same command list on both; logs `conformance-ios.log`, `conformance-android.log`; outputs in `analysis/device-probe/out-*`.
- **Scratch patches** used to get past the failures below are in `analysis/device-probe/probe-scratch.diff`. They exist only to find what breaks; the design decides where each fix belongs.
- Exported media were checked with `ffprobe` (the upstream rules allow ffmpeg tools as test oracles only).
- All runs on 2026-11-12, Xcode 26.6, NDK 28.2.13676358, rustc 1.99.1.

## Findings

1. **The engine stack compiles unchanged for both platforms.** `cargo check` of engine, host, automation and GPU finished with no errors for `aarch64-apple-ios` (38 s) and `aarch64-linux-android` (16 s) (`analysis/device-probe/check-mobile.log`).
2. **The CLI binary builds unchanged for the iOS Simulator** (`build-cli-mobile.log`, first run). **For Android it fails** in `android-activity`, pulled in through `effectcraft-ui-egui` → winit, the CLI's one desktop-UI dependency, used only by `bench --ops` (`apps/effectcraft-cli/src/main.rs:1224`). Removing that one call and dependency builds it (`probe-scratch.diff`, `build-cli-mobile-2.log`). A plugin does not depend on the CLI or on `effectcraft-ui-egui`, so this does not affect it.
3. **Every document capability runs on both platforms.** On iOS and Android: `info` (version 0.24.1, **763 commands, 308 effects**), `commands` (763 listed), `run` (comp, solid, text and Gaussian Blur created and saved, 4 steps), `props` (29 properties on the text layer), `set` and `get`, `render-frame`, `render` to H.264, HEVC and GIF, `replay` (Android) and `mcp` (`initialize`, `tools/list` with **27 tools**, `tools/call list_fonts`) (`conformance-ios.log`, `conformance-android.log`).
4. **CPU rendering is bit-identical across desktop and mobile.** The demo project's frame at 2 s, 960 × 540, rendered on the CPU, is byte-identical on macOS (desktop build), the iOS Simulator and the Android emulator; a 60-frame PNG sequence rendered at half resolution is byte-identical on macOS and Android (`cmp`, `analysis/device-probe/out-*`).
5. **Encoded files are valid but depend on the core count.** The H.264 export (60 frames, 960 × 540) is byte-identical between macOS and the iOS Simulator (both report 18 cores) and differs on the 4-core Android emulator, although every rendered frame is identical (Finding 4). craft-video's H.264 encoder chooses its slice count from `std::thread::available_parallelism()` (craft-libs `crates/video/src/h264/encode/mod.rs:524-536` at `e7c3dda`), so the bitstream follows the device's core count. This holds on desktops too.
6. **GPU on iOS: Metal works once the requested limits fit the device.** The compositor requests `wgpu::Limits::default()` for every limit except texture size and buffer size (`crates/gpu/src/context.rs:1430-1435`). The iOS Simulator's Metal device offers 15 inter-stage shader variables against the default 16, so the request fails and the CLI reports "no usable GPU adapter" (`conformance-ios.log`, first run). With the limits clamped to the adapter's (`or_worse_values_from`), `render.backend` reports `active: gpu`, adapter "Apple iOS simulator GPU (Metal)", and the frame matches the CPU frame visually (`out-ios/gpu.png`). A physical iPhone has not been tested; whether it offers the default limits is unverified.
7. **GPU on Android: Vulkan works, but the default adapter pick chooses GL.** Asked for the best adapter, wgpu picked the emulator's OpenGL ES translator, which has no compute shaders, and the compositor declined it ("no compute shaders"). With `WGPU_BACKEND=vulkan` the compositor runs on "Goldfish GFXStream (Apple M5 Max) (Vulkan)": a GPU frame (`out-android/gpu.png`) and a GPU-composited H.264 export (`out-android/demo-gpu.mp4`, 60 frames). A physical Android GPU (Adreno, Mali) has not been tested.
8. **The host can bring its own GPU device.** `Gpu::new(adapter, device, queue)` accepts a device the caller created (`crates/gpu/src/lib.rs:162-166`), so a plugin can choose the backend (Metal, Vulkan) and the limits itself without changing upstream code. The scratch patch only showed that the compositor runs within the clamped limits.
9. **JavaScript panics on Android until boa uses its enum values.** `script --eval` failed with "the script engine stopped unexpectedly"; the backtrace shows `boa_engine` 0.22 refusing NaN-boxed values on this platform: "this platform is not compatible with a nan-boxed `JsValueInner` — enable the `jsvalue-enum` feature". Android's heap pointers carry a tag in their top byte. Enabling `boa_engine`'s `jsvalue-enum` feature for Android in the depending crate (Cargo unifies features) fixes it: the script prints `items: 11`, and an expression `time*90` on Rotation evaluates to 90 at 1 s. iOS needs no change. Both `effectcraft-expr` and `effectcraft-script` use boa (`crates/expr/Cargo.toml`, `crates/script/Cargo.toml`).
10. **Only bundled fonts are found on both platforms.** `list_fonts` reports 3 families, all `origin: bundled` (Inter, JetBrains Mono and one more), and `added: 0` system fonts, matching the font-folder gap in `evidence/01-system-inventory.md` Finding 18.
11. **The config folder needs an override on Android.** Runs used `EFFECTCRAFT_CONFIG_DIR` and `HOME` pointing at the app-writable folder (`conformance.sh`); without them the session's config folder falls back to `$HOME/.config/effectcraft` (`crates/host/src/app_dirs.rs:78-88`).
12. **Timings are indicative only (debug build, simulator and emulator on an Apple M5 Max).** CPU frame 960 × 540: 61 ms (iOS Simulator, first frame), 28 ms (Android emulator); 2-second H.264 export at 960 × 540: 0.24 s (iOS Simulator), 0.56 s (Android emulator). They show the work completes; device performance has to be measured on hardware.
13. **Size.** The release CLI for Android arm64 (workspace release profile: thin LTO, one codegen unit) is 79.2 MB, 62.4 MB stripped and 26.7 MB compressed with `gzip -9` (`analysis/device-probe/build-release.log`). It links every engine crate: PSD, AEP, PDF, SVG, Lottie, 3D models, interchange, SAM 2 and all codecs, which `effectcraft-engine` depends on unconditionally (`crates/engine/Cargo.toml:12-48`). A plugin library of the same crates is of the same order per architecture.
14. **FilmCraft's VideoToolbox decoder compiles for iOS and falls back cleanly.** Widening `filmcraft-platform`'s platform gates from macOS to macOS-or-iOS (three gates in `crates/platform/Cargo.toml` and `crates/platform/src/lib.rs` at `d5ead4ba`; diff in `analysis/device-probe/filmcraft-platform-ios.diff`) type-checks for `aarch64-apple-ios`. Registered in front of EffectCraft's decoders through `filmcraft_codecs::register_video_decoder` (scratch CLI, `probe-scratch.diff`), it runs in the iOS Simulator, queries VideoToolbox and declines with "hardware decoding declined for h264 video: no hardware decoder", because the simulator has none (`VTIsHardwareDecodeSupported`, filmcraft `crates/platform/src/videotoolbox.rs:416-424`). EffectCraft's software decoder then renders the footage frame byte-identically to a run without the factory (`out-ios-hwdecode/`). Hardware decoding itself needs a physical iPhone to verify.

## Calls the design makes into this system

| Call / trigger | Inputs the plan sends | Defaults when an input is absent | Side effects (writes, emails, downstream syncs) | Constraints it enforces (unique keys, one row per …, allowed values, roles) | What happens on failure (rollback? retry? partial state?) | Anchor |
|---|---|---|---|---|---|---|
| `Gpu::new(adapter, device, queue)` | a Metal (iOS) or Vulkan (Android) device created by the plugin with limits clamped to the adapter | — | builds compute pipelines lazily; kernels a device can't build fall back to the CPU per effect | none beyond the device's own limits | `Err(String)`; the session keeps rendering on the CPU | `crates/gpu/src/lib.rs:162-166`, `crates/gpu/src/context.rs:831` |
| `boa_engine` feature `jsvalue-enum` (Android) | Cargo feature on the plugin's dependency | NaN-boxed values (panics on Android) | none | — | without it every script and expression panics inside a caught boundary | boa_engine 0.22 `Cargo.toml:106` |
| `EFFECTCRAFT_CONFIG_DIR` / `EFFECTCRAFT_FONT_DIRS` | the app's container folders | `$HOME/.config/effectcraft`; no system fonts | settings, models and plug-ins folders resolve under it | read at session start | wrong folder, not an error | `crates/host/src/app_dirs.rs:59-61`, `crates/text/src/fonts.rs:99` |

## Consequences for the design

- **Feasible on both platforms with the upstream engine unchanged.** Every capability of the CLI's document surface ran on iOS and Android. The plugin needs four things the CLI never needed: its own GPU device (backend and limits), boa's `jsvalue-enum` on Android, app-container folders for config and fonts, and no dependency on the desktop UI crate.
- **Native GPU path:** Metal on iOS and Vulkan on Android through wgpu, the same compositor the desktop uses. GL on Android has no compute shaders here, so it is not a GPU path for this compositor; such devices render on the CPU.
- **Conformance:** rendered frames can be compared byte-for-byte across desktop, iOS and Android; encoded files are compared by decoding and comparing frames within a tolerance, because the encoder's slice count follows the core count.
- **Hardware verification** on a physical iPhone and a physical Android phone (Adreno and Mali) is part of the build's acceptance; the simulator and emulator prove the code paths, not device limits or performance.

## Raised as questions

Q11 (minimum OS versions and architectures), Q12 (native codecs), Q13 (scripts and plug-ins on device) and Q15 (size), raised with the platform research (`evidence/05-mobile-platforms.md`).

## Sources

- `analysis/device-probe/` (scripts, scratch diff, logs, outputs), 2026-11-12.
- `storytold/effectcraft` at `10376058`; `storytold/craft-libs` at `e7c3dda` (Cargo git checkout); `boa_engine` 0.22.0 (crates.io source).
