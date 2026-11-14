# Changelog

Every artifact (the Rust crates, the Swift package and both Maven artifacts) shares one version. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html), with breaking changes in a major version from 0.1.0 on.

## [Unreleased]

### Added

- The repository: its rules in `AGENTS.md`, the licences, and the plan of record in `plan/`.
- `tools/doctor.sh`: checks every prerequisite of the build and prints the fix for each gap; `--gate` checks only what the gate needs.
- The workspace skeleton (the Cargo workspace in `rust/`, the Swift package in `ios/sdk/`, the Gradle build in `android/sdk/`) and the gate, `tools/ci.sh`: formatters, linters, tests, `cargo xtask rules`, `cargo xtask generate --check` and `cargo xtask plan-views --check`, which rebuilds the plan's views from their sources and fails on any byte that differs bar the drawing in each diagram's SVG export; `tools/doctor.sh --install-hooks` runs it before every push.
- The EffectCraft pin: `tools/update-upstream.sh` is the one way it moves, fetching an official `storytold/effectcraft` revision and writing the `[workspace.dependencies]` pins, the `[patch]` tables, the release profile and the starting `Cargo.lock` from that revision itself; `tools/adaptations.toml` lists every adaptation and the EffectCraft items it relies on; the `upstream` rule refuses a pin that is not a commit of official EffectCraft main, a hand-written patch, and any registry, fork, path or other-commit copy of a pinned crate, and no Cargo configuration Cargo reads for a build in `rust/` — the repository's own, those in every folder above the checkout, the cargo home's, whatever those `include`, and the environment the gate runs in — may hold a `[patch]`, `paths` or a `[source]` replacement.
- `craftbridge-core`: `Folders` and `Confinement` — the container and bundle read roots and the container write root an engine starts with, prepared for it — and `CraftError`, the one error every call over the native boundary fails with, with the plan's whole mapping of failures onto it.
- `craftbridge-core`'s engine actor: the `CraftApp` trait an app plugin implements, one engine thread per engine answering its calls in order over oneshot channels, a panic inside any call answered as `Internal` with the session kept serving, and closing or dropping an engine ending its session and joining its thread.

### Changed

- `tools/hooks/pre-push` gates each commit a push carries, checked out into a temporary worktree of its own, instead of the working tree the push comes from: an edit that is not part of the push neither passes nor fails it. Each gated tree gets the plan kit's dependencies from `npm ci`, as `plan/package-lock.json` pins them. A commit whose `tools/ci.sh` is gone or cannot run is refused. A repository's pushes gate one at a time, in one build folder kept warm from push to push.
