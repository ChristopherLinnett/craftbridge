# Changelog

Every artifact (the Rust crates, the Swift package and both Maven artifacts) shares one version. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html), with breaking changes in a major version from 0.1.0 on.

## [Unreleased]

### Added

- The repository: its rules in `AGENTS.md`, the licences, and the plan of record in `plan/`.
- `tools/doctor.sh`: checks every prerequisite of the build and prints the fix for each gap; `--gate` checks only what the gate needs.
- The workspace skeleton (the Cargo workspace in `rust/`, the Swift package in `ios/sdk/`, the Gradle build in `android/sdk/`) and the gate, `tools/ci.sh`: formatters, linters, tests, `cargo xtask rules` and `cargo xtask generate --check`; `tools/doctor.sh --install-hooks` runs it before every push.
