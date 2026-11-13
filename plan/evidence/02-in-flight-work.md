# Work in flight on the surfaces

> Evidence file. What is moving on the plan's surfaces right now: other people's attempts at the same work, recorded as evidence. The plan stays the source of truth and says what the build keeps, replaces or removes. Read-only: fetch, list, read — never check out.

## Question this file answers

Which branches, pull requests, issues and deployments are changing the surfaces this plan touches, and what does each mean for the design?

## Sweep

- **effectcraft**: fetched 2026-11-12 07:05; pin `10376058` is `origin/main` (0 commits ahead). `node build/inflight.mjs --gh` lists 21 recently active branches touching watched files (`review/inflight/REPORT.md`). Searches: `gh pr list` / `gh issue list -R storytold/effectcraft --state all --search '<word> in:title'` for mobile, iOS, android, uniffi, ffi, app store, staticlib; `git branch -r | grep -iE 'mobile|ios|android|ffi|uniffi|sdk|swift|kotlin'` (no branch names match).
- **filmcraft**: fetched 2026-11-12; EffectCraft pins it at `d5ead4ba`, and `origin/main` is now `f7db6cdb`. The plan uses FilmCraft only through EffectCraft's pin, so FilmCraft's newer commits enter when EffectCraft moves its pin.
- No deployments: EffectCraft ships desktop and web builds; nothing mobile is deployed.

## Findings

1. **No mobile SDK, FFI layer or UniFFI work exists upstream.** The searches above return no pull request, issue or branch for an SDK, a C ABI, UniFFI, Swift or Kotlin bindings.
2. **storytold/effectcraft#1401 — "iOS: add initial native UIKit and Metal port"** (open, head `4a6d4d13`, conflicting with `main`, last updated 2026-10-29). It adds `apps/effectcraft-ios`, which runs the desktop editor (egui) through UIKit and wgpu/Metal, minimum iOS 18. It widens the config folder's macOS branch to iOS (`crates/host/src/app_dirs.rs`, `cfg!(any(target_os = "macos", target_os = "ios"))`) and touches `crates/render/src/disk_cache.rs`. A maintainer asked for a rebase on 2026-10-29. It ports the desktop UI, not an SDK, but it changes two seams this plan needs (Findings 17–18 of `evidence/01-system-inventory.md`).
3. **Android desktop-UI port, split for review**: #2361 "the desktop app becomes a library" (open, head `c58c935c`, conflicting), #2359 winit Android input (open), #2365 touch pass (open); #2290, #2364, #2366 and #1508 closed. A maintainer noted on #2361 (2026-10-29) that the Android entry crate "needs a maintainer decision on `unsafe` first".
4. **storytold/effectcraft#2846 — "Android: the one unsafe the NativeActivity entry point needs — how should the ban carry that exception?"** (open issue, no maintainer decision yet). It records that an exported C entry symbol cannot be written under `unsafe_code = "forbid"`. The same holds for every symbol an SDK exports to Swift or Kotlin.
5. **storytold/effectcraft#819 — hardware video decoding through `filmcraft-platform`** (closed 2026-11-05). The maintainer's decision, 2026-10-30: "We're keeping the shipped binary free of OS and driver FFI for now, though, even inside a FilmCraft crate … So we won't take `filmcraft-platform` here." The PR measured hardware H.264/HEVC decoding at 5–31× less CPU per frame, bit-exact with the software decoder.
6. **storytold/effectcraft#1840 — "Add cross-platform hardware video encoding"** (open, head `3c3f6032`, conflicting, last updated 2026-11-08). It adds hardware H.264/HEVC export (VideoToolbox on macOS, VA-API, NVENC) with a `hardwareEncoding auto/on/off` render setting, touching `crates/export/src/encode.rs`, `lib.rs`, `pipeline.rs` and new `hardware_*.rs` modules. It reports a 4K HEVC export at 23.5 s with VideoToolbox against 1,600.8 s in software on a Mac. Given the decision in Finding 5, its OS FFI is unlikely to merge as written; its export-pipeline plumbing is the seam a plugin-side hardware encoder needs.
7. **storytold/effectcraft#1518 — "Web build fails to start on Android: requests max_texture_dimension_2d 8192, adapter only provides 4096"** (open). The same class of failure as the iOS Simulator's inter-stage limit (`evidence/03-device-feasibility.md`): the compositor requests limits a mobile adapter may not offer.
8. **storytold/effectcraft#1410 — "Add Android and iOS support, at least for tablets"** (open feature request): demand for mobile, aimed at the editor.
9. **Branches on watched files are hardening work, not mobile work.** The 21 branches in the report harden decode budgets, import limits, confinement, Skip Existing and the export memory heuristic (`fix/1615-auto-cpu-frames` touches `crates/host/src/lib.rs` and `crates/export/src/pipeline.rs`). None changes the session constructor's signature, the command registry or the GPU device API the plan calls.

## Consequences for the design

- **Leaves alone:** the desktop-UI ports (#1401, #2361 series). The SDK does not ship the egui editor; developers bring their own interface (RQ7).
- **Builds on, when merged:** #1401's iOS config folder. Until it merges, the SDK supplies the folder through `EFFECTCRAFT_CONFIG_DIR` or the upstream seam the plan proposes (Q5). Whichever lands first, the other adapts. *(Superseded by PD5, 2026-11-13: EffectCraft is never modified, so there are no seams; what a phone needs is an adaptation in CraftBridge's own crates on EffectCraft's public interfaces, plan §15.)*
- **Keeps platform FFI out of upstream:** the maintainers' decision on #819 means hardware codecs, exported symbols and raw window handles live in the plugin repository, never in the EffectCraft workspace. This rules out putting the mobile crates inside storytold/effectcraft without a policy change (Q4, option C).
- **Uses FilmCraft's FFI crate, outside EffectCraft:** `filmcraft-platform` is FilmCraft's one crate allowed `unsafe`; the plugin may link it because the plugin is not the shipped EffectCraft binary.
- **Sequences after #1840 for hardware encoding:** the encoder seam it needs in `effectcraft-export` is the same seam the plugin needs. The plan proposes an FFI-free version of that seam upstream (Q5) and keeps software encoding until it lands. *(Superseded by PD5, 2026-11-13: EffectCraft is never modified, so there are no seams; what a phone needs is an adaptation in CraftBridge's own crates on EffectCraft's public interfaces, plan §15.)*
- **Learns from #1518:** the plugin creates the GPU device itself with limits clamped to the adapter and passes it to `Gpu::new`, rather than relying on the compositor's default request.

## Raised as questions

Q4 and Q5 carry the consequences (linkage, upstream seams). No new question.

## Sources

- `node build/inflight.mjs --gh` → `review/inflight/REPORT.md`, 2026-11-12 07:17.
- `gh pr view 1401 | 2361 | 1840 | 819 -R storytold/effectcraft` (body, files, comments, head, mergeable), `gh pr diff 1401`, `gh issue view 2846`, 2026-11-12.
- `git -C filmcraft/repository rev-parse --short origin/main` → `f7db6cdb`, 2026-11-12.
