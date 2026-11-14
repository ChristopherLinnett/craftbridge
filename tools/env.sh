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
# CARGO_BUILD_TARGET would send the host build of xtask to rust/target/<triple>/, and the gate would run
# whatever older binary sits at rust/target/debug/xtask.
export CARGO_TARGET_DIR="$CRAFTBRIDGE_ROOT/rust/target"
unset CARGO_BUILD_TARGET

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
