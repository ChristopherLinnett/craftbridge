# Verification of anchors, pins and claims

> Evidence file. Facts only, each with an anchor a reader can re-check. Business choices and unknowns go to `plan.mjs q add`, never into assumptions.

## Question this file answers

Does every code anchor in the master still say what its sentence claims at the pins, has anything landed upstream that moves a pin, and is every engineering decision and design-critical behaviour traced to a check?

## Findings

### Anchors (`node build/review.mjs anchors`, 2026-11-12)

1. 62 anchors; 0 errors (none missing, none past the end of a file). Every excerpt in `review/anchors/REPORT.md` was read against its sentence.
2. Corrected while reading: `crates/automation/src/backend.rs:69-96` → `:69-92` and `apps/effectcraft-cli/src/main.rs:1038-1057` → `:1038-1056` (both ran into the next item); `crates/automation/src/server.rs:92-107` → `:92-102` (the end of `handle_line`); the `render::run` sentence in §19 claimed the CLI's loop calls `stop_render`, which it does not (now: new in the library, anchored to `crates/engine/src/render_queue.rs:691-692`); ambiguous paths that exist in several pinned repositories now name the repository (`effectcraft:`, `filmcraft:`).
3. Remaining warnings, each read and accepted:
   - EffectCraft's root `Cargo.toml` (`:89-90`, `:219-221`, `:223-239`): the resolver matches every file ending in `Cargo.toml`; the excerpts are the root file's FilmCraft pins, `[profile.release]` and `[patch.crates-io]`, as the sentences say.
   - `crates/gpu/src/lib.rs:162-166`, `crates/plugin/src/lib.rs:61`, `crates/automation/src/server.rs:58-60`, `crates/automation/src/backend.rs:61-66`, `crates/engine/src/replay.rs:504`: the sentence names the item with its crate prefix (`effectcraft_gpu::Gpu::new`, `effectcraft_plugin::loader`, `McpServer::backend`, `Backend::session`, `effectcraft_engine::replay::run`) and the lines hold that item (`pub fn new`, `pub fn loader`, `pub fn backend`, `pub fn session`, `pub fn run`).

### Work in flight, re-swept (`node build/inflight.mjs --gh`, 2026-11-12 08:31)

4. **effectcraft**: `origin/main` is `24df676a`, 7 commits past the pin; `git diff --stat 10376058 origin/main` over every file the plan anchors or watches is empty. The pin stays `10376058`, the revision the device runs used (`evidence/03-device-feasibility.md`).
5. Two active branches touch seam areas: `origin/codecs-from-craft-libs` (head `3b8d2313`) moves EffectCraft's encoders to craft-libs and edits `crates/export/src/encode.rs` and `lib.rs` (30 lines changed), the files S7 changes; `origin/fix/1615-auto-cpu-frames` edits `crates/host/src/lib.rs`, the file S1 changes. Whichever lands first, the seam rebases onto it (upstream wins, §15). *(Superseded by PD5: EffectCraft is never modified, so there are no seams to rebase; the adaptations AD1–AD7 sit in CraftBridge's own crates, and an update names the ones whose EffectCraft files changed, §15, §24.)*
6. **filmcraft**: pinned at `d5ead4ba` on purpose, as EffectCraft's own dependency revision (`Cargo.toml:89-90` upstream); FilmCraft's `main` (`f7db6cdb`) is not what CraftBridge builds against.
7. **figcraft**: `origin/main` is `0d6ce112`, 472 commits past `a4251ca8`; `crates/engine/src/lib.rs` changed, and its `Session` still holds no `Arc<dyn …>` field at `0d6ce112` (`grep -c 'Arc<dyn'` = 0), so `evidence/04-craft-suite-shared-core.md` Finding 2 holds. FigCraft is read for the assessment only.
8. No branch, pull request or issue for a mobile SDK, FFI layer or UniFFI appeared since `evidence/02-in-flight-work.md`.

### V1: the iOS system fonts (#28, 2026-11-13)

Measured in an app in the iOS 26.5 simulator and on the acceptance iPhone (iPhone 12 Pro, iOS 26.6.1), against EffectCraft's text crate at `10376058` linked into the app; the full record is `evidence/v1/RESULT.md` with its captures, probe sources and scripts (the built binaries it cites are listed in `evidence/v1/MANIFEST.md`).

9. In the simulator the app's `/System/Library/Fonts` is the Mac's folder (372 files, the Mac volume's `st_dev`), so it says nothing about an iPhone's sandbox. CoreText in the same app reports the iOS fonts: `CTFontCollectionCreateFromAvailableFonts` gives 303 faces in 103 files in 8 folders, one of them outside `Fonts/` (`PrivateFrameworks/FontServices.framework/CorePrivate`, which holds PingFang), and the app reads all 103 files, on the simulator and on the iPhone (`OK 103, FAIL 0`).
10. `effectcraft_text::fonts::add_source(&DirectorySource { dirs })` on CoreText's folders registers 633 faces from 263 files in 311 families, the same on both; every one of the 86 `UIFont.familyNames` families is among them, 5 (Al Nile, Damascus, Farah, Geeza Pro, Mishafi) under their Arabic names. On the iPhone a hard-coded `/System/Library/Fonts` gives 613 faces and misses `CorePrivate`.
11. Matched face by face, by (file, PostScript name), 266 of the 633 faces are the ones CoreText offers, and their families are exactly the 86, on the simulator and on the iPhone.
12. iOS 26's PingFang has only `hvgl` outlines, which skrifa 0.44.0 cannot draw; of EffectCraft's 20 Chinese fallback families iOS has only PingFang SC and TC, so 汉 (zh-Hans) gets no face and 漢 (zh-Hant) Hiragino Sans. Chinese needs a bundled CJK font.
13. Registering only the 266 matched faces would leave 134 script families with no covering face (for example 𞤀 → `Noto Sans Adlam / Regular` with all 633, none with 266).
14. **Decided on FX-3 (#113):** `dirs` comes from CoreText's folders, never a hard-coded `/System/Library/Fonts`; every face stays registered; `craftbridge-effects` filters the list to CoreText's faces plus the bundled ones on every response that comes back through it carrying `text.fonts`' output: `exec` of `text.fonts` and the `run` method, the MCP tools `list_fonts`, `execute_command`, `command_run`, `command_batch` and `batch` with `text.fonts`, and every `text.fonts` entry of the `results` `engine.batch` and `file.runScript` (inline `steps`, or a command list it reads from a file) return, each step's raw output (`effectcraft: crates/engine/src/cmd/batch.rs:66-119`, `crates/engine/src/cmd/file_more.rs:516-556`); what runs inside EffectCraft's own dispatch sees the full registry, which the owner accepted: scripts (the `script` method, `script.run`, the MCP `run_script` tool, and `file.runScript` of a `.js`/`.jsx` file or a named script), `engine.batch`'s `$N` references, substituted inside EffectCraft, and `replay`'s `expect` steps, which compare inside it (`crates/engine/src/replay.rs:477-483`); Noto Sans SC and TC Regular (noto-cjk's region-subset OTFs, 14.0 MB, SIL OFL 1.1) are bundled. R3 is settled.

### V7: the foreground-service type for a long Android export (#30, 2026-11-13)

Measured with a test app's 45-second and 660-second exports on emulator images of API 29 (`QSR1.211112.011`), 31 (`SE1A.220630.001.A1`), 34 (`UE1A.230829.050`, the first Android 14 release) and 37 (`CP41.260828.004.A7`); the full record is `evidence/v7/RESULT.md` with its evidence folders, the test app's sources and scripts (each case's full logcat and the saved documentation pages are listed in `evidence/v7/MANIFEST.md`).

15. **The declaration:** `android:foregroundServiceType="mediaProcessing|dataSync"`, with `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_DATA_SYNC` and `FOREGROUND_SERVICE_MEDIA_PROCESSING`; it installs on every level measured.
16. **The runtime type, always passed explicitly:** `mediaProcessing` when `SDK_INT >= 35`, `dataSync` below. On API 34 a type-less start inherits the unknown `mediaProcessing` bit from that manifest and throws `InvalidForegroundServiceTypeException` (C3: `review-a1-a34-combo-none`, `review-v7-a34-combo-none`).
17. **Per level:** 29 and 31 enforce no type policy, only that the runtime type is a subset of the manifest's (C4: `review-b1-a29-combo-short-notdeclared`, `review-b1-a31-combo-short-notdeclared`); 34 rejects an explicit `mediaProcessing` (C2: `a34-full-mediaproc`, `a34-full-ts34-mediaproc`, `review-v7-a34-combo-mediaproc`); `dataSync` is accepted on 29, 31 and 34 and a 660-second export started in the foreground keeps running after HOME, with no time limit below 35 (C5: `review-v7-a29-combo-auto-long`, `review-v7-a34-combo-auto-long`, the 15 earlier 660-second runs); 37 accepts `mediaProcessing` (`a37-full-mediaproc`).
18. **API 30, 32 and 33 rest on documentation, not measurement:** the owner chose not to download those images. Android's "Changes to foreground services" (saved as `docs/fgs-changes.html`, listed in `evidence/v7/MANIFEST.md` with its SHA-256) changes nothing for these types before Android 14 (Android 11 adds only the camera and microphone types; 12L and 13 have no entry), so they are expected to behave like 29 and 31.
19. **Amendment to §25 PD14:** Android uses `mediaProcessing` only from Android 15 on, and `dataSync` on Android 10–14. Of Android 14's two compat switches for `dataSync`, only `FGS_TYPE_DATA_SYNC_DISABLED_CHANGE_ID` (255659651) refuses it, with an `InvalidForegroundServiceTypeException` the host app catches; the deprecation one (255039210) only logs a warning (`final-v7-a34-combo-datasync-compat-disabled`, `-deprecated`). `specialUse` works on 29, 31, 34 and 37 as a Play-reviewed fallback.

### V2: VideoToolbox decodes the running example's footage in hardware (#93, 2026-11-18)

Measured on the acceptance iPhone (iPhone 12 Pro, A14, iOS 26.6.1) with `tools/ios-device-test.sh craftbridge-media -- t23 --nocapture`: the media crate's test binary for `aarch64-apple-ios`, signed, installed, run to its end and uninstalled by the runner, on the tree of `main` at `78ef8c8`.

20. T23's footage is the movie a phone's hardware export writes in the running example's size, rate and length (1080 × 1080, 30 frames a second, 6 seconds, H.264 with a key frame every 2 seconds), written by the test through the phone's own hardware encoder; the test decodes it with the device's VideoToolbox session and with FilmCraft's software decoder, and compares every picture.
21. The branch it took, as the test says it: `T23, the running example's footage: hardware, VideoToolbox H.264: 180 pictures, each the software decoder's`. `platform::videotoolbox::tests::t23_the_running_examples_footage_gives_the_pictures_the_software_decoder_gives` passed (`1 passed`, 9.9 s).
22. The test fails on the phone when a picture is wrong: with each hardware picture replaced by the one VideoToolbox gave before it, the same run fails with `T23, the running example's footage: picture 1, at 1, is not the software decoder's`.

### V3: MediaCodec decodes the running example's footage in hardware (#94, 2026-11-18)

Measured with the media crate's own tests, built for `aarch64-linux-android` and run from a shell on three devices, on the tree of `main` at `cd381b9` with the T23 test added: the acceptance phone (Xiaomi 17 Pro Max, Android 17, API 37) and the shared arm64 emulators (Pixel 6 images of Android 14, API 34, and Android 10, API 29). Each run read the codec list from a file of what the Kotlin SDK's own `deviceCodecs()` answered on that device, so the list the decoders chose from was the SDK's, not a second reading of the platform. On each device the SDK's device checks also passed (`stream=OK (9 checks)`), among them that an engine started through `Engine.start` reports `info.hardwareDecoding` as the device's own list has it.

23. T23's footage on Android is the running example's movie as the device's encoder writes it (finding 20's size, rate and length): by the hardware encoder `c2.qti.avc.encoder` on the phone, and by the device's own software encoder on both emulators, whose lists name no hardware one. It is decoded through FilmCraft's registry after `register(true)`, which is what an engine's start leaves, and every picture is held to the software decoder's as it arrives.
24. The branch each device took, as the test says it:
    - Xiaomi 17 Pro Max: `T23, the running example's footage: hardware, c2.qti.avc.decoder (MediaCodec H.264): 180 pictures, each the software decoder's`. The set's streams took `c2.qti.avc.decoder` (H.264, 4 pictures; H.264 with B pictures, 6; the same from before time zero, 6), `c2.qti.hevc.decoder` (HEVC, 12; 10-bit HEVC, 24) and `c2.qti.av1.decoder` (AV1, 6), each picture the software decoder's.
    - Android 14 emulator: T23 took `c2.goldfish.h264.decoder`, 180 of 180; the H.264 streams of the set took it too, and HEVC, 10-bit HEVC and AV1 declined (`this device has no hardware decoder for video/hevc`, `…for video/av01`), the software decoder giving the same pictures.
    - Android 10 emulator: no hardware decoder is listed, so every stream, T23 included, declined (`this device has no hardware decoder for video/avc`, and the same for `video/hevc` and `video/av01`), and the registry's software decoder gave the same pictures, T23's 180 among them.
25. The test fails when a picture is wrong, on each device: with the reference picture one late (the comparison's own plant) it fails on the phone and on both emulators; with the decoder showing the picture before each one, it fails T23's footage on the phone and the API 34 emulator at picture 1, and does not touch the API 29 one, whose branch is the software decoder. With one bit of the decoder's last picture flipped (its last Cr sample), it fails T23's footage at picture 179 on the API 34 emulator; with the device's last picture of the footage lost, it fails on both emulators (`T23, the running example's footage: the device's pictures, and the software decoder's`, 179 against 180).
26. The Android 14 emulator's H.264 hardware decoder, `c2.goldfish.h264.decoder`, shows the wrong window of a stream whose crop starts off the coded picture's corner (the set's offset-crop H.264 stream: picture 0 is not the software decoder's), where the phone's `c2.qti.*` decoders give the right one. T23's footage has no such crop. The Android T23 test leaves both offset-crop streams of the set out, and MED-2h (#322) puts them in the Android device tests.
27. Not tried: a phone with a Mali GPU, the known gap of this check. The Android 14 emulator's HEVC, at 8 and 10 bits, and AV1 declined for lack of a hardware decoder, so only the phone shows those through hardware.

### Claims

| Claim | How checked | Date | Result |
|---|---|---|---|
| D-1 · core and plugin as sibling folders build in one workspace | design choice; PD9, PD10, PD16 recorded with the owner | 2026-11-12 | decided |
| D-2 · rustc 1.99.1 builds the engine for iOS and Android | `cargo check`/`cargo build` runs with rustc 1.99.1 (`analysis/device-probe/*.log`) | 2026-11-12 | true |
| D-3 · UniFFI is the only generator covering Swift and Kotlin from one definition | release notes and READMEs of UniFFI, swift-bridge, flapigen, Diplomat, cxx, Gobley (`evidence/05-mobile-platforms.md` Finding 9) | 2026-11-12 | true |
| D-4 · two Rust static libraries in one iOS app collide | rust-lang/rust#100774; Mozilla megazord design note (`evidence/05-mobile-platforms.md` Finding 8) | 2026-11-12 | true (documented; not reproduced here) |
| D-5 · a `Backend` owns one `Session` | `crates/automation/src/backend.rs:10-21` @ `10376058` | 2026-11-12 | true |
| D-6 · the JSON request shape matches `craft-web-api` | craft-libs `crates/web-api/src/lib.rs:1-20` @ `e7c3dda` | 2026-11-12 | true |
| D-7 · default limits fail on the simulator; clamped limits run Metal; Android's default pick is GL without compute; Vulkan runs | iOS Simulator and Android emulator runs (`evidence/03-device-feasibility.md` Findings 6–7) | 2026-11-12 | true |
| D-7 · `Gpu::new` accepts a device the caller made | `crates/gpu/src/lib.rs:162-166` @ `10376058` | 2026-11-12 | true |
| D-8 · `jsvalue-enum` makes boa run on Android | emulator run before and after the feature (`evidence/03-device-feasibility.md` Finding 9) | 2026-11-12 | true |
| D-9 · `render_display` leaves the frame on the GPU | `crates/gpu/src/lib.rs:224-236` @ `10376058` | 2026-11-12 | true |
| D-10 · `render_rgba8_alpha` gives straight alpha when transparent | `crates/engine/src/session/render.rs:132-143` @ `10376058` | 2026-11-12 | true |
| D-11 · parameter types exist only as prose | `crates/engine/src/cmd/schema.rs:77-101` @ `10376058` | 2026-11-12 | true |
| D-12 · encoded bytes follow the core count | `crates/video/src/h264/encode/mod.rs:524-536` @ craft-libs `e7c3dda`; macOS vs emulator exports with identical rendered frames | 2026-11-12 | true |
| D-13 · EffectCraft decodes through FilmCraft's registry at `d5ead4ba` | `Cargo.toml:89-90`, `crates/media/src/stream.rs:87`, `crates/media/src/pool.rs:742` @ `10376058`; filmcraft `crates/codecs/src/lib.rs:99-120` @ `d5ead4ba` | 2026-11-12 | true |
| D-13 · a factory registered there runs inside EffectCraft | scratch CLI with a counting VideoToolbox factory in the iOS Simulator: queried, declined, software fallback byte-identical (`evidence/03-device-feasibility.md` Finding 14) | 2026-11-12 | true |
| D-14 · VideoToolbox reports hardware support per codec | filmcraft `crates/platform/src/videotoolbox.rs:416-424` @ `d5ead4ba`; simulator decline | 2026-11-12 | true |
| D-15 · `ndk` wraps `AImageReader` | `ndk` 0.9 `media` feature (`evidence/05-mobile-platforms.md` Finding 17) | 2026-11-12 | true |
| D-16 · `Confinement` confines footage and file reads | `crates/automation/src/backend.rs:42-51`, `crates/automation/src/confine.rs:430-480` @ `10376058` | 2026-11-12 | true |
| D-17 · the exporter removes files only on cancel | `crates/engine/src/render_queue.rs:103-106` @ `10376058` | 2026-11-12 | true |
| D-18 · `McpServer` exposes its `Backend` | `crates/automation/src/server.rs:58-60` @ `10376058` | 2026-11-12 | true |
| D-19 · JNA cannot pass a JNI environment; `NativeWindow::from_surface` needs one | UniFFI Kotlin uses JNA (`evidence/05-mobile-platforms.md` Finding 6); `ndk` `native_window.rs` (Finding 4) | 2026-11-12 | true |
| D-20 · UniFFI scaffolding needs `unsafe` allowed | uniffi-rs `setup_scaffolding.rs`, `util.rs`; rustc `report_unsafe` (`evidence/05-mobile-platforms.md` Finding 7) | 2026-11-12 | true (read from source; not compiled) |
| D-21 · `[patch]` applies only at the root workspace; EffectCraft patches hayro and the egui family | Cargo's documented behaviour; `Cargo.toml:223-239` @ `10376058` | 2026-11-12 | true |
| D-22 · the active `gh` login is another account; ChristopherLinnett's token has `repo` and `workflow`, not `write:packages` | `gh auth status` | 2026-11-12 | true |
| D-23 · additive changes are minor, breaking major | design choice (B4, B5) | 2026-11-12 | decided |
| The CLI's document capabilities run on iOS and Android | `analysis/device-probe/conformance-*.log` | 2026-11-12 | true |
| CPU frames are byte-identical across macOS, iOS and Android | `cmp` of `out-*/cpu.png` and a 60-frame PNG sequence | 2026-11-12 | true |
| The registry holds 763 commands; MCP lists 27 tools headless | `effectcraft-cli info`, `mcp tools/list` on both devices | 2026-11-12 | true |
| The CLI's export loop polls every 100 ms through `poll_render` and `render_progress` | `apps/effectcraft-cli/src/main.rs:983-1013` @ `10376058` | 2026-11-12 | true |
| `Session::stop_render` stops a render | `crates/engine/src/render_queue.rs:691-692` @ `10376058` | 2026-11-12 | true |
| `PluginLoader` is a plain function pointer taking the source | `crates/engine/src/lib.rs:294` @ `10376058` | 2026-11-12 | true |
| Template rendering and render timings exist only in the CLI binary | `apps/effectcraft-cli/src/template.rs`, `apps/effectcraft-cli/src/main.rs:1265-1350`, `:1507-1616` @ `10376058` | 2026-11-12 | true |
| Release library size about 62 MB stripped per architecture | Android release build of the CLI (`analysis/device-probe/build-release.log`) | 2026-11-12 | true (the CLI; the plugin library is measured in V5) |
| An iOS app can list and read its system fonts | an app in the iOS 26.5 simulator and on the acceptance iPhone (findings 9–14) | 2026-11-13 | true, through CoreText's folders; the simulator's `/System/Library/Fonts` is the Mac's (V1) |
| Android 10–14 accept `dataSync` for a long export, `mediaProcessing` only from 15 | emulator runs on API 29, 31, 34 and 37; 30, 32 and 33 from Android's documentation (findings 15–19) | 2026-11-13 | true where measured; expected from the documentation on 30, 32 and 33 (V7) |
| VideoToolbox decodes the running example's footage in hardware on the acceptance iPhone, every frame equal to the software decoder's | T23 through `tools/ios-device-test.sh` on an iPhone 12 Pro, iOS 26.6.1 (findings 20–22) | 2026-11-18 | true: the hardware branch, VideoToolbox H.264, 180 of 180 pictures (V2) |
| MediaCodec decodes the running example's footage in hardware on the acceptance Android phone, every frame equal to the software decoder's | T23 through the media crate's tests on a Xiaomi 17 Pro Max, Android 17, and the API 34 and API 29 emulators, with the Kotlin SDK's own codec list (findings 23–27) | 2026-11-18 | true: hardware on the phone (`c2.qti.avc.decoder`) and the API 34 emulator (`c2.goldfish.h264.decoder`), declined to software on the API 29 one, 180 of 180 pictures on each (V3); a Mali-GPU phone untried |
| Physical-device GPU limits, performance | not checkable without devices | — | open: V4–V5 |

## Calls the design makes into this system

None beyond those traced in `evidence/01`–`05`; this file records their verification.

## Consequences for the design

- The pin stays `10376058`; nothing anchored has changed upstream.
- Seams S1 and S7 rebase onto `fix/1615-auto-cpu-frames` and `codecs-from-craft-libs` if those land first. *(Superseded by PD5: there are no seams; the adaptations that rely on those files are rechecked when an update moves the pin.)*
- AD2 builds its iOS folders from CoreText and `craftbridge-effects` filters the font list; FX-3 grows to 1.5 engineer-days (V1).
- The Android background recipe (PD14) uses `mediaProcessing` from Android 15 and `dataSync` on 10–14 (V7).
- P2 gains the `write:packages` scope for publishing.

## Raised as questions

None.

## Sources

`review/anchors/REPORT.md`, `review/inflight/REPORT.md`, `git` at effectcraft `10376058` and `24df676a`, filmcraft `d5ead4ba` and `f7db6cdb`, figcraft `a4251ca8` and `0d6ce112`, craft-libs `e7c3dda`, `gh auth status`, `analysis/device-probe/`, all 2026-11-12.
