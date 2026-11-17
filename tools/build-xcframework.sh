#!/usr/bin/env bash
# Builds the iOS distribution library and joins it into the XCFramework the Swift package's binary target
# links (plan §22), which is never committed.
#
#   tools/build-xcframework.sh                  regenerate the Swift bindings, build, join
#   tools/build-xcframework.sh --keep-bindings  build and join, leaving the committed bindings as they are
#
# `cargo xtask xcframework` does the building and the joining (rust/xtask/src/xcframework.rs): it builds
# the distribution library the SDK ships, the one the layer table gives (rust/xtask/src/layers.rs), with
# --release for every Apple target rust/rust-toolchain.toml lists, for the iOS the package's manifest
# names, and joins each static library with the headers and module map of the crates'
# low-level Swift modules, which links the system frameworks and libraries rustc says that library needs,
# into the folder the manifest gives its binary target, only when what the framework would hold changed.
# The Swift itself is written by `cargo xtask generate`, whose registered generator keeps
# ios/sdk's committed bindings in step with the library. The gate passes --keep-bindings: its `generated`
# stage must find a hand edit to the committed bindings, which a regeneration here would write over before
# the stage looked.
set -euo pipefail
# shellcheck disable=SC2046 # one variable name per word, split on purpose
unset $(git rev-parse --local-env-vars 2>/dev/null)
# shellcheck source=tools/env.sh
source "$(dirname "$0")/env.sh"
cd "$CRAFTBRIDGE_ROOT"
# What ships is built as the gate builds: a `RUSTFLAGS` or a profile in the shell's environment, which cargo
# reads over the CPU target tools/env.sh sets, would change the library without a word.
craftbridge_refuse_switches || exit 1

regenerate=yes
case "$*" in
  "") ;;
  --keep-bindings) regenerate=no ;;
  *)
    echo "usage: tools/build-xcframework.sh [--keep-bindings]" >&2
    exit 64
    ;;
esac

# xtask is built into a folder of this run only, as the gate does: a concurrent build would replace the
# shared binary this script is starting (tools/env.sh).
work="$(mktemp -d "${TMPDIR:-/tmp}/craftbridge-xcframework.XXXXXX")"
trap 'rm -rf "$work"' EXIT
craftbridge_xtask_build "$work"
xtask="$work/debug/xtask"

if [ "$regenerate" = yes ]; then "$xtask" generate; fi
# The device slice is built for the CPU target tools/env.sh sets once for every build for the iOS device,
# and says why, so the library that ships is the code the gate's iOS clippy and the device runner compile;
# xtask hands the variable on to the cargo it starts.
craftbridge_ios_target_env aarch64-apple-ios
"$xtask" xcframework
