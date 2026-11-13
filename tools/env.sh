# shellcheck shell=bash
# Sourced by every script in tools/: the environment the build runs in, resolved in one place so
# the doctor checks exactly what the gate then uses.

CRAFTBRIDGE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export CRAFTBRIDGE_ROOT

# The Rust toolchain pinned by rust/rust-toolchain.toml; rustup picks it up inside rust/.
CRAFTBRIDGE_RUST_CHANNEL="$(sed -n 's/^channel = "\(.*\)"$/\1/p' "$CRAFTBRIDGE_ROOT/rust/rust-toolchain.toml")"
CRAFTBRIDGE_RUST_TARGETS="$(sed -n 's/^targets = \[\(.*\)\]$/\1/p' "$CRAFTBRIDGE_ROOT/rust/rust-toolchain.toml" | tr -d '",')"
export CRAFTBRIDGE_RUST_CHANNEL CRAFTBRIDGE_RUST_TARGETS

# The Xcode release the iOS build is made and tested with.
export CRAFTBRIDGE_XCODE="26.6"

# Android: the SDK from the usual variables, else Android Studio's default location on macOS.
if [ -z "${ANDROID_HOME:-}" ]; then
  if [ -n "${ANDROID_SDK_ROOT:-}" ]; then ANDROID_HOME="$ANDROID_SDK_ROOT"; else ANDROID_HOME="$HOME/Library/Android/sdk"; fi
fi
export ANDROID_HOME
export CRAFTBRIDGE_ANDROID_API="37"
# The NDK the doctor asks a machine to install: the default NDK of the Android Gradle plugin the version
# catalog pins (plan/evidence/07-repository-conventions.md, Finding 14). NDK r28 and later align native
# libraries to 16 KB pages, so the doctor accepts any r28 already installed.
export CRAFTBRIDGE_NDK_VERSION="28.2.13676358"
export CRAFTBRIDGE_NDK_MAJOR="${CRAFTBRIDGE_NDK_VERSION%%.*}"

# Gradle runs on JDK 21, the release AGP is tested with here. JAVA_HOME wins when it is 21; else
# macOS's registry, else the JDK Android Studio bundles.
craftbridge_java_major() { "$1/bin/java" -version 2>&1 | sed -n 's/.*version "\([0-9]*\).*/\1/p' | head -n 1; }
craftbridge_find_jdk() {
  local candidate
  for candidate in "${JAVA_HOME:-}" "$(/usr/libexec/java_home -v 21 2>/dev/null || true)" "/Applications/Android Studio.app/Contents/jbr/Contents/Home"; do
    if [ -n "$candidate" ] && [ -x "$candidate/bin/java" ] && [ "$(craftbridge_java_major "$candidate")" = "21" ]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  return 1
}
if CRAFTBRIDGE_JDK="$(craftbridge_find_jdk)"; then export JAVA_HOME="$CRAFTBRIDGE_JDK"; fi

# The copy-paste detector, pinned so every machine finds the same clones.
export CRAFTBRIDGE_JSCPD="jscpd@5.4.3"

# Where tools write output instead of hand-written code, as paths from the repository root: the build
# folders .gitignore names, and generated code, which is committed but written by a generator. The shell
# stages prune these paths and jscpd ignores them, so a source folder that only shares a name with build
# output (a Rust module `src/target/`) is still read. rust/xtask/src/scan.rs makes the same cut by its
# own rule, a build folder only beside the file of the tool that writes it: a folder added to one list
# belongs in the other.
export CRAFTBRIDGE_BUILD_OUTPUTS="rust/target ios/**/.build ios/**/.swiftpm android/**/build android/**/.gradle android/**/.kotlin **/generated **/Generated **/node_modules"
