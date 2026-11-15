# shellcheck shell=bash
# Sourced by every script in tools/: the environment the build runs in, resolved in one place so
# the doctor checks exactly what the gate then uses.

CRAFTBRIDGE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export CRAFTBRIDGE_ROOT

# Every pin is read from the file that pins it, never written here a second time. This reader knows one
# form, a basic (double-quoted) TOML string, whatever the spacing around `=` and whatever comment follows
# it: `craftbridge_toml_string <file> <key>`. A key written any other way (a 'literal' string, say, which
# rustup and Gradle read too) stops every script, naming the file, rather than being misread.
craftbridge_toml_string() {
  if grep -Eq "^[[:space:]]*$2[[:space:]]*=[[:space:]]*'" "$1"; then
    echo "tools/env.sh: $2 in ${1#"$CRAFTBRIDGE_ROOT"/} is a 'literal' string: write it as a \"basic\" one, the form this reader knows" >&2
    exit 1
  fi
  sed -n "s/^[[:space:]]*$2[[:space:]]*=[[:space:]]*\"\([^\"]*\)\".*$/\1/p" "$1" | head -n 1
}
# A value the build cannot do without stops every script here, naming the file to fix, so no check can
# pass having checked nothing.
craftbridge_require() {
  if [ -z "$(printf '%s' "$2" | tr -d '[:space:]')" ]; then
    echo "tools/env.sh: no $1 in ${3#"$CRAFTBRIDGE_ROOT"/}" >&2
    exit 1
  fi
}

# The Rust toolchain pinned by rust/rust-toolchain.toml; rustup picks it up inside rust/. The target list
# is read from `targets = [` to its `]`, on one line or many, without comments, double quotes or commas; a
# 'literal' string there stops every script, as for any pin.
craftbridge_toolchain="$CRAFTBRIDGE_ROOT/rust/rust-toolchain.toml"
CRAFTBRIDGE_RUST_CHANNEL="$(craftbridge_toml_string "$craftbridge_toolchain" channel)"
CRAFTBRIDGE_RUST_TARGETS="$(awk '
  { sub(/#.*/, "") }
  !inside && /^[[:space:]]*targets[[:space:]]*=/ { inside = 1; sub(/^[^=]*=/, "") }
  inside { done = index($0, "]") > 0; if (index($0, "\047") > 0) { print "LITERAL"; exit } gsub(/[][",]/, " "); printf "%s ", $0; if (done) exit }
' "$craftbridge_toolchain" | tr -s '[:space:]' ' ' | sed 's/^ //; s/ $//')"
if [ "$CRAFTBRIDGE_RUST_TARGETS" != "${CRAFTBRIDGE_RUST_TARGETS%LITERAL}" ]; then
  echo "tools/env.sh: targets in rust/rust-toolchain.toml holds a 'literal' string: write each as a \"basic\" one, the form this reader knows" >&2
  exit 1
fi
craftbridge_require "Rust channel" "$CRAFTBRIDGE_RUST_CHANNEL" "$craftbridge_toolchain"
craftbridge_require "Rust targets" "$CRAFTBRIDGE_RUST_TARGETS" "$craftbridge_toolchain"
export CRAFTBRIDGE_RUST_CHANNEL CRAFTBRIDGE_RUST_TARGETS
# Each worktree builds into its own rust/target: a machine-wide CARGO_TARGET_DIR would share one target
# folder between worktrees, whose gates would then reuse each other's artifacts. A machine-wide
# CARGO_BUILD_TARGET would send the host build of xtask to a <triple>/ folder, where the gate, which
# starts the binary from debug/, would not find it.
# The pre-push hook gates each pushed commit in a temporary worktree of its own, and with
# CRAFTBRIDGE_PUSH_TARGET names the one build folder all its gates share, kept warm from push to push.
# That sharing is safe where a machine-wide folder is not: the hook runs one gate at a time and checks each
# commit out only once the gate before it has finished, so every file a gate builds is newer than every
# artifact in the folder, and Cargo, which judges a workspace crate by file times, rebuilds it rather than
# take another tree's artifacts for it. With no hook naming a folder, each tree keeps its own rust/target.
export CARGO_TARGET_DIR="${CRAFTBRIDGE_PUSH_TARGET:-$CRAFTBRIDGE_ROOT/rust/target}"
unset CARGO_BUILD_TARGET

# Every cargo command the scripts run: in rust/, where rustup selects the pinned toolchain from
# rust/rust-toolchain.toml, and never while the repository root holds a Cargo configuration. Cargo merges a
# .cargo/config.toml from every folder above the one it runs in, so one at the root would reach every cargo
# command: its `runner` would stand in for the test and rule binaries, its `[patch]` or `[source]` would
# build another copy around the pin, and the gate would pass having checked nothing, before the rules that
# refuse the folder could run. The one Cargo configuration is rust/.cargo/config.toml, which the rules read.
craftbridge_cargo() {
  if [ -e "$CRAFTBRIDGE_ROOT/.cargo" ]; then
    echo "$(basename "$0" .sh): .cargo at the repository root: Cargo would merge it into every cargo build; the one Cargo configuration is rust/.cargo/config.toml" >&2
    return 1
  fi
  (cd "$CRAFTBRIDGE_ROOT/rust" && cargo "$@")
}
# The rule checks are built with `cargo build` and their binary run by the caller, not through the
# `cargo xtask` alias or `cargo run`: a Cargo configuration can redefine an alias, and a
# `[target.<triple>].runner` replaces whatever binary `cargo run` (and `cargo test`) would start, so
# either would pass having checked nothing. `cargo build` runs no runner, and CARGO_TARGET_DIR above
# fixes where the intermediates build; the rules then report the alias or the runner. Building xtask
# compiles none of the workspace's own crates, so no build script of theirs runs before `rules` has
# judged whether it may exist. Every cargo build replaces the binary in its target folder, even one with
# nothing to rebuild, so each caller names a folder of its own for the binary, which nothing else writes;
# `build.build-dir` keeps the intermediate artifacts in the shared target folder, where every build
# finds them.
craftbridge_xtask_build() {
  CARGO_BUILD_BUILD_DIR="$CARGO_TARGET_DIR" craftbridge_cargo build --package xtask --locked --quiet --target-dir "$1"
}

# The machine-local git cache of official EffectCraft, a bare clone: tools/update-upstream.sh fetches
# official main into it, and the upstream rule only reads it. Outside the repository on purpose — a
# fetch cache is not a source, and no build reads it.
export CRAFTBRIDGE_UPSTREAM_CACHE="${CRAFTBRIDGE_UPSTREAM_CACHE:-$HOME/.cache/craftbridge/upstream/effectcraft}"

# The Xcode release the iOS build is made and tested with.
export CRAFTBRIDGE_XCODE="26.6"

# SwiftLint runs the Swift stage's built-in rules from `ios/sdk/.swiftlint.yml` (`missing_docs`). The gate
# pins the version: a release can add a rule or change what a rule reads, so an upgrade is an edit here and
# a fresh gate run, never a silent surprise. The portable zip of the release carries the binary; put it
# where PATH finds it.
export CRAFTBRIDGE_SWIFTLINT="0.65.1"
export CRAFTBRIDGE_SWIFTLINT_URL="https://github.com/realm/SwiftLint/releases/download/$CRAFTBRIDGE_SWIFTLINT/portable_swiftlint.zip"

# Android: the SDK from the usual variables, else Android Studio's default location on macOS.
if [ -z "${ANDROID_HOME:-}" ]; then
  if [ -n "${ANDROID_SDK_ROOT:-}" ]; then ANDROID_HOME="$ANDROID_SDK_ROOT"; else ANDROID_HOME="$HOME/Library/Android/sdk"; fi
fi
export ANDROID_HOME
# The Android levels and the JDK, from the version catalog the build itself reads.
craftbridge_catalog="$CRAFTBRIDGE_ROOT/android/sdk/gradle/libs.versions.toml"
CRAFTBRIDGE_ANDROID_API="$(craftbridge_toml_string "$craftbridge_catalog" android-compileSdk)"
CRAFTBRIDGE_ANDROID_BUILD_TOOLS="$(craftbridge_toml_string "$craftbridge_catalog" android-buildTools)"
CRAFTBRIDGE_JDK_MAJOR="$(craftbridge_toml_string "$craftbridge_catalog" jdk)"
craftbridge_require "android-compileSdk" "$CRAFTBRIDGE_ANDROID_API" "$craftbridge_catalog"
craftbridge_require "android-buildTools" "$CRAFTBRIDGE_ANDROID_BUILD_TOOLS" "$craftbridge_catalog"
craftbridge_require "jdk" "$CRAFTBRIDGE_JDK_MAJOR" "$craftbridge_catalog"
export CRAFTBRIDGE_ANDROID_API CRAFTBRIDGE_ANDROID_BUILD_TOOLS CRAFTBRIDGE_JDK_MAJOR
# The NDK the doctor asks a machine to install for the library builds: the default NDK of the Android
# Gradle plugin the version catalog pins (plan/evidence/07-repository-conventions.md, Finding 14). NDK r28
# and later align native libraries to 16 KB pages, so the doctor accepts any r28 already installed.
export CRAFTBRIDGE_NDK_VERSION="28.2.13676358"
export CRAFTBRIDGE_NDK_MAJOR="${CRAFTBRIDGE_NDK_VERSION%%.*}"

# Gradle runs on the catalog's JDK, the release AGP is tested with here. JAVA_HOME wins when it is that
# major; else macOS's registry, else Homebrew's keg (keg-only, so the registry never lists it), else the
# JDK Android Studio bundles.
craftbridge_java_major() { "$1/bin/java" -version 2>&1 | sed -n 's/.*version "\([0-9]*\).*/\1/p' | head -n 1; }
craftbridge_find_jdk() {
  local candidate
  for candidate in "${JAVA_HOME:-}" "$(/usr/libexec/java_home -v "$CRAFTBRIDGE_JDK_MAJOR" 2>/dev/null || true)" \
    "${HOMEBREW_PREFIX:-/opt/homebrew}/opt/openjdk@$CRAFTBRIDGE_JDK_MAJOR/libexec/openjdk.jdk/Contents/Home" \
    "/Applications/Android Studio.app/Contents/jbr/Contents/Home"; do
    if [ -n "$candidate" ] && [ -x "$candidate/bin/java" ] && [ "$(craftbridge_java_major "$candidate")" = "$CRAFTBRIDGE_JDK_MAJOR" ]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  return 1
}
if CRAFTBRIDGE_JDK="$(craftbridge_find_jdk)"; then export JAVA_HOME="$CRAFTBRIDGE_JDK"; fi

# The copy-paste detector, pinned so every machine finds the same clones.
export CRAFTBRIDGE_JSCPD="jscpd@5.4.3"

# The plan kit's dependencies, installed as plan/package-lock.json pins them: every package plan/package.json
# names stands in plan/node_modules at the version the lock records for it. The plan stage rebuilds from a
# scratch inside that folder, whose imports resolve there before anywhere else, so that copy is the one the
# rebuild runs, and a version drifted from the lock can render the views differently: they would read as
# edited by hand on one machine and pass on another. `npm ls` judges by the ranges in plan/package.json,
# which a drifted version still meets, and Node's own import takes any copy it finds further up. The doctor
# reports this prerequisite, and the plan stage asks it before rebuilding.
craftbridge_plan_dependencies() {
  (cd "$CRAFTBRIDGE_ROOT/plan" && node --input-type=module --eval '
    import fs from "node:fs";
    const read = (file) => JSON.parse(fs.readFileSync(file, "utf8"));
    const pinned = read("package-lock.json").packages;
    for (const name of Object.keys(read("package.json").dependencies || {})) {
      const manifest = "node_modules/" + name + "/package.json";
      const version = (pinned["node_modules/" + name] || {}).version;
      if (!version || !fs.existsSync(manifest) || read(manifest).version !== version) process.exit(1);
    }
  ' >/dev/null 2>&1)
}
