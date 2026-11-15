# shellcheck shell=bash
# Sourced by every script in tools/: the environment the build runs in, resolved in one place so
# the doctor checks exactly what the gate then uses.

CRAFTBRIDGE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export CRAFTBRIDGE_ROOT

# A value the build cannot do without stops every script here, naming the file to fix, so no check can
# pass having checked nothing.
craftbridge_require() {
  if [ -z "$(printf '%s' "$2" | tr -d '[:space:]')" ]; then
    echo "tools/env.sh: no $1 in ${3#"$CRAFTBRIDGE_ROOT"/}" >&2
    exit 1
  fi
}

# The pins come from `cargo xtask pins` (rust/xtask/src/pins.rs), which parses rust/rust-toolchain.toml
# and the version catalog with the `toml` crate, the parser rustup itself reads the toolchain file with.
# No script picks a pin out of either file by text: a text reader knows one spelling of each value, while
# rustup and Gradle accept literal strings, inline tables and multi-line arrays alike. The caller runs the
# command — the doctor when its Rust checks pass, the gate before its stages — and pipes its output in;
# every pin the command must name is required here, so no check can pass having read nothing.
craftbridge_load_pins() {
  local line
  CRAFTBRIDGE_RUST_TARGETS=""
  while IFS= read -r line; do
    case "$line" in
      rust-target=*) CRAFTBRIDGE_RUST_TARGETS="$CRAFTBRIDGE_RUST_TARGETS ${line#rust-target=}" ;;
      android-compileSdk=*) CRAFTBRIDGE_ANDROID_API="${line#android-compileSdk=}" ;;
      android-buildTools=*) CRAFTBRIDGE_ANDROID_BUILD_TOOLS="${line#android-buildTools=}" ;;
      jdk=*) CRAFTBRIDGE_JDK_MAJOR="${line#jdk=}" ;;
    esac
  done
  local toolchain="$CRAFTBRIDGE_ROOT/rust/rust-toolchain.toml" catalog="$CRAFTBRIDGE_ROOT/android/sdk/gradle/libs.versions.toml"
  craftbridge_require "Rust targets" "$CRAFTBRIDGE_RUST_TARGETS" "$toolchain"
  craftbridge_require "android-compileSdk" "$CRAFTBRIDGE_ANDROID_API" "$catalog"
  craftbridge_require "android-buildTools" "$CRAFTBRIDGE_ANDROID_BUILD_TOOLS" "$catalog"
  craftbridge_require "jdk" "$CRAFTBRIDGE_JDK_MAJOR" "$catalog"
  export CRAFTBRIDGE_RUST_TARGETS CRAFTBRIDGE_ANDROID_API CRAFTBRIDGE_ANDROID_BUILD_TOOLS CRAFTBRIDGE_JDK_MAJOR
  craftbridge_setup_java
}

# rustup installs whatever rust/rust-toolchain.toml names and the machine lacks — the toolchain, a
# component, a target — the moment it resolves that file, for any cargo call or rustup query inside rust/,
# before the doctor has named the gap or the gate has judged anything. The doctor and the gate report on
# the machine as they find it and leave it so: a missing target is a gap with its fix, never an install.
# rustup before 1.28.1 does not know the variable and installs as it always did.
export RUSTUP_AUTO_INSTALL=0

# What toolchain rustup resolves inside rust/, which is where it reads rust/rust-toolchain.toml: rustup
# itself says the one line `cargo xtask pins` cannot, because a pin's file is the toolchain only when no
# other toolchain stands over it. `ok <toolchain>` is that case; the rest name what does. A `+toolchain`
# argument is one of those: given to cargo or rustup, it reaches every command run under it, the gate's
# included, as RUSTUP_TOOLCHAIN, which is how rustup then names it. One spelling of that variable is no
# other toolchain though: rustup hands the toolchain it resolved to every command it starts, so each test
# `cargo test` runs, and each gate such a test runs, inherits a RUSTUP_TOOLCHAIN naming that toolchain —
# the pin itself whenever rust/ chose it. A variable naming the very toolchain the file alone resolves to
# therefore passes for the pin; one naming any other toolchain, a `+toolchain` naming any other, and a
# rustup directory override all stand over the pin and are named back. The answer is rustup's last line, so
# the `info:` lines of an install by a rustup too old for RUSTUP_AUTO_INSTALL cannot pass for it.
craftbridge_toolchain_status() {
  local line pin file_line
  if ! command -v rustup >/dev/null 2>&1; then
    echo no-rustup
    return
  fi
  if ! line="$(cd "$CRAFTBRIDGE_ROOT/rust" && rustup show active-toolchain 2>&1)"; then
    # rustup names what chose the toolchain it lacks. Only the toolchain file is the pin missing: a
    # variable or a directory override naming a toolchain this machine lacks still stands over the pin,
    # and installing what rustup then resolves would install that other toolchain, not the pin.
    line="${line##*$'\n'}"
    case "$line" in
      *"RUSTUP_TOOLCHAIN environment variable"* | *"directory override"*) echo "override $line" ;;
      *) echo missing ;;
    esac
    return
  fi
  line="${line##*$'\n'}"
  pin="$(cd "$CRAFTBRIDGE_ROOT/rust" && pwd -P)/rust-toolchain.toml"
  case "$line" in
    *"(overridden by '$pin')") echo "ok ${line%% *}"; return ;;
    *"(overridden by environment variable RUSTUP_TOOLCHAIN)")
      # Take the variable off and ask rustup again: when the file then chooses exactly the toolchain the
      # variable named, the variable names the pin, and the build is the pin's build.
      if file_line="$(cd "$CRAFTBRIDGE_ROOT/rust" && env -u RUSTUP_TOOLCHAIN rustup show active-toolchain 2>&1)"; then
        file_line="${file_line##*$'\n'}"
        if [ "$file_line" = "${line%% *} (overridden by '$pin')" ]; then
          echo "ok ${line%% *}"
          return
        fi
      fi
      ;;
  esac
  echo "override $line"
}

# The environment the gate refuses: every variable that switches what its Rust tools check. The list is
# one table, tools/refused-switches.tsv: it names each switch by the Cargo configuration key that sets it
# and by every variable Cargo reads it from, with the reason beside it, and the configuration check of the
# `upstream` rule reads that same table (rust/xtask/src/upstream.rs), so no second list of these switches
# stands anywhere. One line per variable set, naming it; nothing printed when nothing is refused. A broken
# table is itself refused: silence here would read as a clean environment.
craftbridge_refused_environment() {
  local table name key patterns _tree_ok why extra pattern words index
  local -a refused_patterns=() refused_whys=()
  table="$(dirname "${BASH_SOURCE[0]}")/refused-switches.tsv"
  if [ ! -r "$table" ]; then
    echo "tools/env.sh: cannot read the switch table at tools/refused-switches.tsv, so the refused environment cannot be told"
    return 0
  fi
  # The table is read once, into one pattern and its reason per entry. The tree-ok column belongs to the
  # Rust side's check; a shell read collapses empty tab-separated fields, so the table spells every field
  # and this one is read to be dropped.
  while IFS=$'\t' read -r key patterns _tree_ok why extra; do
    case "$key" in
      '' | '#'*) continue ;;
    esac
    if [ -n "$extra" ] || [ -z "$why" ]; then
      echo "tools/env.sh: a line of tools/refused-switches.tsv holds more or fewer than its four tab-separated fields, so the refused environment cannot be told"
      return 0
    fi
    # The patterns split into words with no pathname expansion: unquoted, `CARGO_PROFILE_*` would turn
    # into the name of a file in the folder the doctor runs from, and the family would pass unrefused.
    # `-` is a configuration-only line, which no variable matches.
    read -r -a words <<<"$patterns"
    for pattern in ${words[@]+"${words[@]}"}; do
      if [ "$pattern" != - ]; then
        refused_patterns+=("$pattern")
        refused_whys+=("$why")
      fi
    done
  done <"$table"
  while IFS= read -r name; do
    index=0
    while [ "$index" -lt "${#refused_patterns[@]}" ]; do
      # The glob is the point here: a pattern is how one line names a whole derived family
      # (CARGO_TARGET_<triple>_RUNNER, CARGO_PROFILE_…), so the right side must stay unquoted.
      # shellcheck disable=SC2053
      if [[ "$name" == ${refused_patterns[$index]} ]]; then
        if [ -n "${!name}" ]; then echo "$name=${!name} ${refused_whys[$index]}"; fi
        break
      fi
      index=$((index + 1))
    done
  done < <(compgen -e)
  return 0
}

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
# The NDK the doctor asks a machine to install for the library builds: the default NDK of the Android
# Gradle plugin the version catalog pins (plan/evidence/07-repository-conventions.md, Finding 14). NDK r28
# and later align native libraries to 16 KB pages, so the doctor accepts any r28 already installed.
export CRAFTBRIDGE_NDK_VERSION="28.2.13676358"
export CRAFTBRIDGE_NDK_MAJOR="${CRAFTBRIDGE_NDK_VERSION%%.*}"

# Gradle runs on the catalog's JDK, the release AGP is tested with here — the `jdk` pin craftbridge_load_pins
# read. JAVA_HOME wins when it is that major; else macOS's registry, else Homebrew's keg (keg-only, so the
# registry never lists it), else the JDK Android Studio bundles.
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
craftbridge_setup_java() {
  if CRAFTBRIDGE_JDK="$(craftbridge_find_jdk)"; then export JAVA_HOME="$CRAFTBRIDGE_JDK"; fi
}

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
