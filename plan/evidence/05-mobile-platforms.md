# Mobile platform facts: GPU, bindings, packaging, policy, codecs

> Evidence file. Facts only, each with an anchor a reader can re-check. Business choices and unknowns go to `plan.mjs q add`, never into assumptions.

## Question this file answers

What do iOS, Android and the Rust mobile tooling allow and require for shipping EffectCraft's engine as Swift and Kotlin plugins: GPU backends and surfaces, binding generators, packaging, store and OS policy, native codecs and long-running work?

## Findings

All read 2026-11-12 from the URLs given. "Inference" marks a conclusion not stated by the source.

### GPU (wgpu 30.0.1)

1. **Backends.** wgpu lists Metal as first-class on iOS (Vulkan there only through MoltenVK), and Vulkan as first-class on Android with GLES 3.0+ "best effort" (https://github.com/gfx-rs/wgpu/blob/trunk/README.md). On GLES, compute shaders exist only on ES 3.1+ (wgpu-hal 30.0.1 `src/gles/adapter.rs`, https://docs.rs/crate/wgpu-hal/latest/source/src/gles/adapter.rs).
2. **Vulkan minimum.** wgpu accepts a Vulkan 1.0 device with `VK_KHR_storage_buffer_storage_class` and `VK_KHR_maintenance1` (wgpu v30.0.1 `wgpu-hal/src/vulkan/adapter.rs`). Android requires Vulkan 1.1 on devices launching with Android 10, 1.3 from Android 13 and 1.4 from Android 16 (https://source.android.com/docs/core/graphics/implement-vulkan).
3. **Known mobile driver issues** open against wgpu: Mali workgroup atomics (gfx-rs/wgpu#8643), Mali binding arrays (#8642), a Mali segfault with textures as function parameters (#10186), private-array indexing on Android Vulkan (#2252), an Adreno X1-85 struct-by-value bug (#10621), `draw_indexed_indirect` on Adreno 740/650 (#8801).
4. **Surfaces need `unsafe`.** On iOS a surface from a `CAMetalLayer` goes through `Instance::create_surface_unsafe` with `SurfaceTargetUnsafe::CoreAnimationLayer` (https://raw.githubusercontent.com/gfx-rs/wgpu/v30.0.1/wgpu/src/api/surface.rs). On Android, `ndk::native_window::NativeWindow::from_surface(env, surface)` is `unsafe`; the `NativeWindow` it returns then makes a surface safely (https://github.com/rust-mobile/ndk, `ndk/src/native_window.rs`).
5. **Importing native textures needs `unsafe` and is incomplete on Android.** `Device::create_texture_from_hal` and the Metal and Vulkan `texture_from_raw` are `unsafe` (https://docs.rs/wgpu/latest/wgpu/struct.Device.html). There is no AHardwareBuffer importer; gfx-rs/wgpu#2320 "Texture memory import API" is open (https://github.com/gfx-rs/wgpu/issues/2320). Zero-copy hardware-decoded video into the compositor is therefore not available on Android today.

### Bindings

6. **UniFFI** 0.32.2 (2026-09-22), MPL-2.0, Swift and Kotlin bindings first-party, proc-macro mode without UDL (https://github.com/mozilla/uniffi-rs/blob/main/CHANGELOG.md). Rust `async fn` maps to Swift `async` and Kotlin `suspend` with no cancellation support (https://mozilla.github.io/uniffi-rs/latest/futures.html). Foreign traits let Swift or Kotlin implement a Rust trait (`#[uniffi::export(foreign)]`). Byte arguments can be zero-copy since 0.32; returned bytes are owned `Vec<u8>` (https://github.com/mozilla/uniffi-rs/blob/main/docs/manual/src/types/bytes.md). Error enums become Swift `throws` and Kotlin exceptions. Kotlin bindings use JNA (Apache-2.0 OR LGPL-2.1+).
7. **UniFFI cannot be used in a crate that forbids unsafe code.** Its scaffolding emits `#[unsafe(no_mangle)] pub extern "C" fn` items and `#[unsafe(no_mangle)] pub static UNIFFI_META_*` items (uniffi-rs `uniffi_macros/src/setup_scaffolding.rs`, `uniffi_macros/src/util.rs`); rustc's `unsafe_code` lint covers `no_mangle`, and `forbid` cannot be overridden (`rustc_lint/src/builtin.rs`, `report_unsafe`). Bindings therefore live in separate crates with their own lint table (read from source; not compiled).
8. **One native library per app.** Two Rust static libraries in one iOS app each carry the Rust standard library and collide on symbols such as `rust_eh_personality` (rust-lang/rust#100774). Mozilla's application-services compiles every component into one "megazord" library: one XCFramework on iOS, one AAR that per-component AARs depend on for Android (https://mozilla.github.io/application-services/book/design/megazords.html). UniFFI's external types work across crates in proc-macro mode, but all generated Swift for them must be compiled into one Swift module (https://mozilla.github.io/uniffi-rs/latest/types/remote_ext_types.html).
9. **Alternatives** (one line each): swift-bridge 0.1.59 (Swift only, Apache-2.0/MIT); flapigen 0.12.3 (Java/C++, no Swift, BSD-3-Clause); Diplomat 0.17.0 (no Swift listed, MIT OR Apache-2.0); cxx 1.0.205 (C++ only); Gobley 0.3.7 (UniFFI for Kotlin Multiplatform, MPL-2.0); cargo-swift 0.11.1 (packages a UniFFI crate as a Swift package). Only UniFFI covers Swift and Kotlin from one definition.

### Packaging

10. **iOS:** `xcodebuild -create-xcframework` from one static library per platform (device; simulator slices merged with `lipo`), distributed through Swift Package Manager as `binaryTarget(url:checksum:)` or `path:` (https://developer.apple.com/documentation/xcode/creating-a-multi-platform-binary-framework-bundle, https://developer.apple.com/documentation/xcode/distributing-binary-frameworks-as-swift-packages).
11. **Android:** cargo-ndk produces the `jniLibs` layout per ABI; the NDK's example filter is arm64-v8a plus x86_64, and armeabi-v7a is needed only for 32-bit devices (https://developer.android.com/ndk/guides/abis). Apps targeting Android 15+ must support 16 KB pages; NDK r28+ aligns to 16 KB by default (https://developer.android.com/guide/practices/page-sizes). This machine has NDK 28.2.
12. **Rust targets** `aarch64-apple-ios`, `aarch64-apple-ios-sim`, `x86_64-apple-ios`, `aarch64-linux-android`, `x86_64-linux-android` and `armv7-linux-androideabi` are Tier 2 with std (https://doc.rust-lang.org/nightly/rustc/platform-support.html).

### Store and OS policy

13. **Apple, interpreted code.** App Review Guideline 2.5.2 forbids downloading or executing code that introduces or changes features; 4.7 allows software not embedded in the binary, including plug-ins, under conditions 4.7.1–4.7.5 (https://developer.apple.com/app-store/review/guidelines/, updated 2026-10-16). The Developer Program License Agreement §3.3.1(B) allows downloaded interpreted code that does not change the app's primary purpose or bypass security (https://developer.apple.com/support/terms/apple-developer-program-license-agreement/). No clause bars a bundled, non-JIT interpreter running scripts the user writes. Whether WebAssembly run by an interpreter counts as interpreted code is not addressed.
14. **iOS forbids child processes.** Apple Developer Technical Support: iOS apps may not spawn child processes (https://developer.apple.com/forums/thread/747499).
15. **Android forbids executing app-written binaries.** From API 29, apps cannot `execve()` files in their writable home directory (https://developer.android.com/about/versions/10/behavior-changes-10). EffectCraft's external effect helpers are separate processes (`evidence/01-system-inventory.md` Finding 20), so they cannot run on either platform as designed.

### Native media

16. **Apple bindings:** objc2 0.6.5 (MIT) and objc2-video-toolbox, -av-foundation, -core-video, -metal, -io-surface 0.3.2 (Zlib OR Apache-2.0 OR MIT), the same crates `filmcraft-platform` uses on macOS. Most generated APIs are `unsafe` (https://github.com/madsmtm/objc2/blob/main/crates/header-translator/README.md).
17. **Android bindings:** `ndk` 0.9.0 (MIT OR Apache-2.0) wraps `media_codec`, `media_format` and `image_reader` (API 24); it has no MediaMuxer or MediaExtractor wrappers (https://github.com/rust-mobile/ndk).
18. **Hardware codecs.** iOS: HEVC decode from A9, an AV1 decoder from A17 Pro (Apple Newsroom, 2023-09). Android: H.264 decoder required; HEVC decoder since 5.0; VP9 decoder since 4.4; AV1 encoder and decoder mandatory from Android 14; whether a codec is hardware is a runtime query of `MediaCodecInfo` (https://developer.android.com/media/platform/supported-formats).

### Long-running work

19. **iOS:** `BGContinuedProcessingTask` (iOS 26) continues user-started work in the background with a system progress UI; GPU use in the background needs the `com.apple.developer.background-tasks.continued-processing.gpu` entitlement and device support, and without it Metal rejects background command buffers (https://developer.apple.com/documentation/backgroundtasks/performing-long-running-tasks-on-ios-and-ipados, https://developer.apple.com/documentation/metal/preparing-your-metal-app-to-run-in-the-background). `com.apple.developer.kernel.increased-memory-limit` exists for some models; Apple publishes no memory limits.
20. **Android:** a `mediaProcessing` foreground service (API 35, usable earlier) with the `FOREGROUND_SERVICE_MEDIA_PROCESSING` permission gets 6 hours per 24 hours (https://developer.android.com/develop/background-work/services/fgs/service-types).

### Precedent

21. wgpu-in-app (MIT) is the reference for embedding wgpu in an iOS or Android app without a windowing library; Bevy ships an official mobile example; Gyroflow ships on the App Store and Google Play with GPU processing (its iOS use of wgpu is an inference). Mozilla ships Rust on mobile through UniFFI megazords.

## Calls the design makes into this system

| Call / trigger | Inputs the plan sends | Defaults when an input is absent | Side effects (writes, emails, downstream syncs) | Constraints it enforces (unique keys, one row per …, allowed values, roles) | What happens on failure (rollback? retry? partial state?) | Anchor |
|---|---|---|---|---|---|---|
| `Instance::create_surface_unsafe(CoreAnimationLayer)` (iOS) | the developer's `CAMetalLayer` pointer | — | the surface presents into that layer | the layer must outlive the surface | `Err(CreateSurfaceError)` | wgpu v30.0.1 `wgpu/src/api/surface.rs` |
| `NativeWindow::from_surface` + `create_surface` (Android) | JNI env and the developer's `Surface` | — | holds a reference to the window | the window must stay valid while used | `None` / `Err` | ndk `ndk/src/native_window.rs` |
| UniFFI scaffolding | the FFI crate's exported items | — | exports C symbols and metadata statics | cannot compile under `forbid(unsafe_code)` | compile error | uniffi-rs `uniffi_macros/src/setup_scaffolding.rs` |

## Consequences for the design

- **Bindings:** UniFFI in proc-macro mode, in FFI crates outside the upstream workspace, with their own lint table that allows `unsafe` only in named modules (the same rule `filmcraft-platform` follows: `unsafe_code = "deny"`, `#[allow(unsafe_code)]` per module, a `// SAFETY:` comment per block).
- **One native library per distribution:** the universal core and the app crates compile into one static library (iOS) and one shared library (Android); per-app Swift and Kotlin packages sit on top of it.
- **Cancellation and progress** go through explicit handles and callbacks, because UniFFI futures cannot be cancelled. The engine's export progress callback already returns `false` to cancel (`crates/engine/src/render_queue.rs:103-106`).
- **No helper processes** on either platform: helper effects report themselves unavailable.
- **Android footage** is decoded into CPU images and uploaded, as on desktop, until wgpu can import AHardwareBuffers.
- **Background work** is the host app's to schedule; the SDK must expose progress and cancellation that `BGContinuedProcessingTask` and a foreground service can drive.

## Raised as questions

Q11 (minimum OS versions and architectures), Q12 (native codecs), Q13 (scripts and plug-ins on mobile), Q14 (background exports), Q15 (size).

## Sources

The URLs above, read 2026-11-12 by a research subagent and summarised here; repository source files named with their paths at the versions given.
