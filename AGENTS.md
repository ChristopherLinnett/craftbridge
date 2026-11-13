# CraftBridge: rules for agents and contributors

These rules apply to every human and AI contributor and are never broken. `CLAUDE.md` points here. A rule marked **(gate)** is checked by `tools/ci.sh`, which fails when it is broken; the others are checked in review. Each rule exists because of a decision recorded as a GitHub issue labelled `decision`; a rule changes only through such an issue and a pull request that changes this file and its check together. The layout, the layers and the gate are designed in the plan of record, [`plan/EFFECTCRAFT-PLUGIN-PORT-SCOPING.md`](plan/EFFECTCRAFT-PLUGIN-PORT-SCOPING.md) §15; the conventions behind these rules, with their sources, are in [`plan/evidence/07-repository-conventions.md`](plan/evidence/07-repository-conventions.md).

## 1. Layout

- The repository root holds only `AGENTS.md`, `CLAUDE.md`, `README.md`, `NOTICE`, `ATTRIBUTION.md`, the two licences, `CHANGELOG.md`, the git and editor files, and the folders `rust/`, `ios/`, `android/`, `conformance/`, `docs/`, `tools/`, `plan/` and `.github/`. Nothing that belongs to a platform sits at the root. **(gate)**
- Each platform folder is a complete build of its own: `rust/` (the Cargo workspace), `ios/sdk/` (the Swift package), `android/sdk/` (the Gradle build). `ios/` and `android/` hold only `sdk/` and `samples/`. **(gate)**
- A library folder holds the library and nothing else. Samples live in `samples/` and consume the library the way an app does: the Swift package by local path, the Gradle build as an included build.
- A platform folder never refers to another platform's code. Every platform reads the shared data in `conformance/`.

## 2. Dependency direction

- The **universal core** knows nothing about any Craft app. A core crate never depends on an `effectcraft-*` crate or on an app crate; `CraftBridge` and `:core` never import a plugin. **(gate)**
- An **app plugin** depends on the core and on its own Craft app, never on another app's plugin or on a sibling Craft app's code. Code two plugins need moves into the core. **(gate)**
- Within the core and within a plugin, dependencies point down the layers listed in `rust/xtask/src/layers.rs`. A new crate is added to that table in the same change. **(gate)**
- `uniffi` appears only in `-ffi` crates. Each app has **one native library**, its `craftbridge-<app>-ffi` crate, the only crate built as a `staticlib` or `cdylib`: two Rust libraries in one app collide. **(gate)**
- Host tools (`xtask`, `uniffi-bindgen`, the codegen and conformance crates) may depend on anything; nothing depends on a tool. **(gate)**

## 3. Never crash

People trust the engine with hours of work. Bad input, a bad call or a full disk produces an error the app can act on, never a crash and never lost work. This outranks feature work.

- **Rust.** No `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!` or `unimplemented!` outside tests. These lints are denied once, in `rust/Cargo.toml`; every crate inherits them with `[lints] workspace = true`, and neither the table nor a crate root may weaken them. **(gate)** A build script may not weaken them either: a crate has a `build.rs` or a `package.build` key only when its row in `rust/xtask/src/layers.rs` marks it. **(gate)** The only exception is a provably infallible literal, written as an item-level `#[allow(clippy::expect_used)]` and `.expect("why this cannot fail")`.
- Fail with `Result` and `?`, adding context. Treat every number derived from input as hostile: `get()` instead of indexing, checked or saturating arithmetic, capped allocations, bounded recursion. Handle lock poisoning; treat thread joins as results.
- `panic = "unwind"` stays: the engine thread catches a panic at every call, keeps the document and returns `Internal`. No panic crosses into Swift or Kotlin. **(gate)**
- **Swift.** No force unwrap, `as!`, `try!` or implicitly unwrapped optional (swift-format), and no `.unsafelyUnwrapped`, `fatalError`, `precondition`, `preconditionFailure`, `assert`, `assertionFailure`, `exit` or `abort()` outside tests. **(gate)** Failures are thrown errors.
- **Kotlin.** No `!!`, `TODO()`, `exitProcess` or `System.exit` in library or sample code. **(gate)** Failures are documented exceptions.
- Every crash fix comes with a test that crashed before the fix.

## 4. Unsafe code

- Hand-written `unsafe` lives only in `src/platform/` of the crates the layer table marks as platform crates, and every block carries a `// SAFETY:` comment. **(gate)**
- `allow(unsafe_code)` appears only crate-wide in an `-ffi` crate (UniFFI's generated scaffolding needs it) and on a `mod platform` declaration. **(gate)**
- Swift pointer APIs (`Unmanaged`, `Unsafe*Pointer`, `withUnsafe…`, `OpaquePointer`, `unsafeBitCast`) appear only under a `Platform/` folder. Kotlin `external` (JNI) declarations appear only under a `platform/` folder. **(gate)**

## 5. No copy-paste

Never duplicate code by copying it into a second place: not functions, types, widgets, scripts or configuration blocks, and never a "near-identical" variant. Reuse the existing definition; if it almost fits, extend that one definition and repoint every caller; if it lives in a private scope, promote it to one shared definition and consume it everywhere, including the original site. If two pieces only look alike and genuinely cannot share code, say why in the pull request before writing the second one.

- A block of ten lines repeated anywhere in Rust, Swift, Kotlin or shell fails the gate. **(gate)**
- Configuration follows the same rule: every version once (`[workspace.dependencies]`, `gradle/libs.versions.toml`); shared Gradle setup once (`android/sdk/build-logic`); the licence allowlist once (`rust/deny.toml`); the build environment once (`tools/env.sh`).
- Code two Craft app plugins need lives in the universal core, never borrowed from a sibling Craft app.

## 6. Generated code

- Code and data that a generator in this repository writes live only in directories named `generated` or `Generated`, carry an `@generated` header where the format allows comments, and are written only by `cargo xtask generate`. **(gate)** Two kinds of written-for-you file are outside this rule: the files a build tool writes for itself (`Cargo.lock`, `Package.resolved`, the Gradle wrapper), which change only through that tool; and the plan's views in `plan/`, which the plan kit builds from `plan/EFFECTCRAFT-PLUGIN-PORT-SCOPING.md` and `plan/diagrams/src/` and `plan/plan check --final` checks.
- Generated files are committed and never edited by hand. To change one, change its generator, or for a plan view its source. The gate regenerates everything `cargo xtask generate` writes and fails on any difference. **(gate)**

## 7. Upstream: EffectCraft

- **EffectCraft is never modified.** CraftBridge builds official `storytold/effectcraft` revisions as they are, never a fork or a patched branch, and opens nothing upstream. The fork `ChristopherLinnett/effectcraft` is only kept in step with upstream.
- What a phone needs EffectCraft to do differently is an **adaptation** in CraftBridge's own crates, written against EffectCraft's public interfaces: it calls EffectCraft, wraps it or follows its behaviour, and never carries a copy of EffectCraft code. Behaviour an adaptation follows from EffectCraft's command line is checked against that command line by the conformance corpus.
- Every adaptation is listed in `tools/adaptations.toml` with the EffectCraft files and items it relies on, in the same change that adds or alters it, so an update names the adaptations to recheck.
- **Upstream wins.** When EffectCraft gains a public interface that does an adaptation's job, the adaptation moves onto it and leaves the list.
- The pin moves only through `tools/update-upstream.sh`. The `[patch]` tables, the release profile and the starting `Cargo.lock` come from the pinned EffectCraft and are never written by hand.
- One copy of each EffectCraft, FilmCraft and Craft library crate: CraftBridge's decoders register into the same registry EffectCraft decodes through. **(gate)**
- GitHub commands name their account (`GH_TOKEN=$(gh auth token --user ChristopherLinnett) gh …`), because a machine's active login may be another account.

## 8. Licences, assets and brand

- Dependencies use only the licences allowed in `rust/deny.toml`: checked for Cargo by cargo-deny, and for Gradle libraries and SwiftPM packages through `tools/licences.toml`. **(gate)** An identifier is added to the allowlist only through a decision issue. MPL-2.0 is allowed only for dependencies used unmodified; JNA is used under Apache-2.0.
- Clean room: no GPL, LGPL or AGPL code; no code, assets or strings copied from another product. Every asset is original or openly licensed and listed in `ATTRIBUTION.md` in the same change.
- CraftBridge is an independent SDK: no ArtCraft name, wordmark or logo in its names, icons, listings or marketing. It says, in plain text, that it is based on EffectCraft by the ArtCraft team.

## 9. Naming

- Rust crates are `craftbridge-<area>`; an app plugin's crates carry the app's noun (`craftbridge-effects`, later `craftbridge-film`); a crate's folder has the crate's name. **(gate)**
- Swift: package `CraftBridge`, modules `CraftBridge`, `CraftBridge<App>` and the generated `CraftBridgeFFI`. Kotlin: packages `io.github.christopherlinnett.craftbridge.<module>`, generated bindings under `io.github.christopherlinnett.craftbridge.ffi.<module>`. Maven: `io.github.christopherlinnett.craftbridge:<module>`.
- Public APIs follow the Rust API Guidelines, the Swift API Design Guidelines, the Kotlin coding conventions and the AndroidX API guidelines ([`plan/evidence/07-repository-conventions.md`](plan/evidence/07-repository-conventions.md)).

## 10. Formatting and lints

- rustfmt (`rust/rustfmt.toml`) and clippy with warnings as errors; `swift format lint --strict` (`ios/sdk/.swift-format`); Spotless with ktlint, Android Lint and Kotlin warnings as errors, Kotlin explicit API mode; shellcheck for every script. **(gate)**
- The gate's own machinery — `tools/ci.sh`, `tools/env.sh`, `tools/hooks/`, `rust/xtask/`, `rust/.cargo/config.toml`, `android/sdk/settings.gradle.kts`, `android/sdk/gradle.properties`, `android/sdk/build-logic/`, and the single copy of each of `rust/clippy.toml`, `rust/rustfmt.toml`, `rust/deny.toml`, `ios/sdk/.swift-format` and `tools/licences.toml` — decides what the gate checks, so it cannot guard itself: a change to any of those files is a gate change, which its pull request names and its review checks for weakened rules. The gate fails on a second copy of any listed configuration file, or of a tool configuration that would act like one, anywhere in the tree. Gradle reads a `gradle.properties` in a project's own folder as that project's properties, so no other `gradle.properties` sits under `android/sdk` — the build's own is `android/sdk/gradle.properties` alone, and `android/sdk/build-logic`, an included build with properties of its own, is where its own may sit. **(gate)**
- An Android module's `android/sdk/<module>/build.gradle.kts` — one of which every module the settings include has — holds only comments, `plugins { id("craftbridge.android.library") }`, `android { namespace = "io.github.christopherlinnett.craftbridge.<module>" }`, and a `dependencies` block whose entries are `api`, `implementation`, `testImplementation` or `androidTestImplementation` of a `project(":…")` or a `libs.<alias>`. The root `android/sdk/build.gradle.kts` holds only comments and a `plugins` block of `alias(libs.plugins.<alias>) apply false` lines and `id("craftbridge.quality")`. The build is written in Kotlin DSL only: Gradle would read a Groovy `build.gradle` or `settings.gradle` in place of the Kotlin file beside it, so those are refused unread. Everything else a build needs is configured once in `android/sdk/build-logic`, which is why a module's own build file never needs to rewire anything. **(gate)**
- A lint is never switched off to make a change pass. Fix the code; for a genuine false positive, allow it on the one item, with a comment saying why.
- Comments explain why the code is the way it is: an invariant, a constraint, a trade-off. Never issue numbers or history; those belong in the pull request. A guard that looks removable says what it prevents.

## 11. Versions and releases

- One version for every artifact: the Rust crates, the Swift package and both Maven artifacts. It is written only in `rust/Cargo.toml` (`[workspace.package] version`); the release script passes it to Gradle and tags the release with it.
- Semantic versioning: a breaking change is a major version from 0.1.0 on, an addition a minor version, a fix a patch. Tags are `X.Y.Z`. Every user-visible change adds a line to `CHANGELOG.md` under Unreleased in the same pull request.
- Nothing is published without the owner's go-ahead.

## 12. Changes, commits and pull requests

- Every piece of work is a GitHub issue in the CraftBridge project (`https://github.com/users/ChristopherLinnett/projects/9`), with its acceptance and the issues that block it. Work starts only on an issue whose blockers are all done. An issue with sub-issues is never worked directly: it is finished through its sub-issues and closes with the last of them.
- Each issue is built on its own branch, `<task-id>-<slug>` in lower case (`ops-3-gate`), in its own git worktree outside the checkout (`../craftbridge-worktrees/<branch>`), so every ready issue can proceed at once; only two changes to the same files wait for each other. The worktree and its branch are removed once the pull request has merged. The branch merges by pull request into the iteration branch of its wave (`iteration/w0`, `iteration/w1`, …). An iteration branch reaches `main` through one pull request the owner approves. `main` and the iteration branches change only through pull requests, squash-merged once the gate is green. The pull request title and the squash commit are `<TASK-ID>: <summary>` (`OPS-3: Workspace skeleton and gate`).
- Closing an issue is what unblocks the issues that depend on it, so an issue is closed when, and only when, its pull request has merged into the iteration branch. GitHub closes issues automatically only on the default branch, so the issue is closed explicitly at that merge; the project's Done status follows the closing, never the other way round. Nothing is marked done that has not merged.
- **The review loop.** A change is built by one side and reviewed by the other: the reviewer reviews the whole change, fixes everything it finds and reports; the builder then reviews the whole change again, the reviewer's fixes included, fixes everything it finds and reports; the same two keep alternating until neither finds anything in the other's work. Only then does the change go up. Every round is a full review with fixes, never a single remark handed back. The pull request records the rounds.
- A question about whether a syntax is legal is settled by the compiler or a syntax-aware analyser, not by argument; a reviewer unsure of a language or tool reads its documentation before claiming a finding.
- Run the gate before pushing (`tools/doctor.sh --install-hooks` makes git do it). Report only what ran: say what was tested and what could not be run.
- A decision becomes the issue that carries out its action (labelled `decision`, stating what to do and why), closed by the pull request that does it. Decisions are not written into the repository.
- No agent attribution in commits, pull requests or comments.
- Check, then claim: read the issue and the open pull requests on the same surface before starting, and verify a definition before using it.
