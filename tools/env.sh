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

# A script's own folder for the run, `craftbridge_remove_on_exit <folder> [<cleanup>]`, removed on every way
# out of it, and the script's verdict kept. Bash 3.2, the bash macOS ships, ends a script that meets an unset
# variable under `set -u` with the status of its EXIT trap, which a plain removal leaves 0, so the script
# would pass having stopped half-way, the gate's whole run among them. So an end with 0 is a pass only once
# the script has said so, `craftbridge_passed=1` before it ends; every other end keeps its own status: an
# `exit` with one, a failure `set -e` stops on, a signal. A script that leaves more behind than its folder
# names its own cleanup, a function of the script, which runs first, while the folder is still there. It runs
# in a subshell under `set -e`, as a command of its own: inside the test of an `if`, a `!` or a `&&`, `set -e`
# stops nothing, and a failure would count only if it were the cleanup's last command; and inside the trap,
# bash 3.2 ends a subshell that meets an unset variable with 0 unless `set -e` is on in it. So an unset
# variable or any failing command ends only the cleanup, and the folder still goes. A cleanup that fails, or a
# folder that will not go, fails the run, since it left something behind. The trap is the script's one EXIT
# trap, for one folder, so a second call, which would take the first folder's place and leave it behind, is
# refused, as is a cleanup that names no function, which would never run; either refusal still removes the
# folder it was handed.
craftbridge_remove_on_exit() {
  if [ -n "${craftbridge_run_folder:-}" ]; then
    rm -rf "$1"
    echo "$(basename "$0" .sh): craftbridge_remove_on_exit is called once a run, and $craftbridge_run_folder is this run's folder: make $1 inside it" >&2
    exit 1
  fi
  craftbridge_run_folder="$1"
  craftbridge_run_cleanup=""
  craftbridge_passed=""
  craftbridge_status=1
  craftbridge_left=0
  trap 'craftbridge_status=$?
    set +e
    if [ -n "$craftbridge_run_cleanup" ]; then
      (set -e; "$craftbridge_run_cleanup")
      craftbridge_left=$?
    fi
    rm -rf "$craftbridge_run_folder" || craftbridge_left=1
    if [ "$craftbridge_status" = 0 ] && { [ "$craftbridge_left" != 0 ] || [ "${craftbridge_passed:-}" != 1 ]; }; then craftbridge_status=1; fi
    exit "$craftbridge_status"' EXIT
  if [ -n "${2:-}" ]; then
    if ! declare -F "$2" >/dev/null; then
      echo "$(basename "$0" .sh): craftbridge_remove_on_exit names the cleanup $2, which is no function" >&2
      exit 1
    fi
    craftbridge_run_cleanup="$2"
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
      android-minSdk=*) CRAFTBRIDGE_ANDROID_MIN_SDK="${line#android-minSdk=}" ;;
      android-buildTools=*) CRAFTBRIDGE_ANDROID_BUILD_TOOLS="${line#android-buildTools=}" ;;
      jdk=*) CRAFTBRIDGE_JDK_MAJOR="${line#jdk=}" ;;
    esac
  done
  local toolchain="$CRAFTBRIDGE_ROOT/rust/rust-toolchain.toml" catalog="$CRAFTBRIDGE_ROOT/android/sdk/gradle/libs.versions.toml"
  craftbridge_require "Rust targets" "$CRAFTBRIDGE_RUST_TARGETS" "$toolchain"
  craftbridge_require "android-compileSdk" "$CRAFTBRIDGE_ANDROID_API" "$catalog"
  craftbridge_require "android-minSdk" "$CRAFTBRIDGE_ANDROID_MIN_SDK" "$catalog"
  craftbridge_require "android-buildTools" "$CRAFTBRIDGE_ANDROID_BUILD_TOOLS" "$catalog"
  craftbridge_require "jdk" "$CRAFTBRIDGE_JDK_MAJOR" "$catalog"
  export CRAFTBRIDGE_RUST_TARGETS CRAFTBRIDGE_ANDROID_API CRAFTBRIDGE_ANDROID_MIN_SDK CRAFTBRIDGE_ANDROID_BUILD_TOOLS CRAFTBRIDGE_JDK_MAJOR
  craftbridge_setup_java
}

# rustup installs whatever rust/rust-toolchain.toml names and the machine lacks — the toolchain, a
# component, a target — the moment it resolves that file, for any cargo call or rustup query inside rust/,
# before the doctor has named the gap or the gate has judged anything. The doctor and the gate report on
# the machine as they find it and leave it so: a missing target is a gap with its fix, never an install.
# rustup before 1.28.1 does not know the variable and installs as it always did.
export RUSTUP_AUTO_INSTALL=0

# A command run in a folder, under the toolchain rustup resolves there: `craftbridge_in <dir> <command…>`.
# rustup reads the pin of the folder a command runs in, unless RUSTUP_TOOLCHAIN stands over it, and it
# hands the toolchain it resolved to every command it starts as that variable. The one variable the gate
# and the doctor let through names rust/'s pin (craftbridge_toolchain_status_in says why), which is no pin
# in any other folder: there it would stand over that folder's own, the gate's lint library's nightly, so a
# command run outside rust/ runs without it.
craftbridge_in() {
  local dir="$1"
  shift
  if [ "$dir" -ef "$CRAFTBRIDGE_ROOT/rust" ]; then
    (cd "$dir" && "$@")
  else
    (cd "$dir" && env -u RUSTUP_TOOLCHAIN "$@")
  fi
}

# What toolchain rustup resolves in a folder with a pin, `craftbridge_toolchain_status_in <dir>`, which is
# where it reads `<dir>/rust-toolchain.toml`: rustup itself says the one line `cargo xtask pins` cannot,
# because a pin's file is the toolchain only when no other toolchain stands over it. `ok <toolchain>` is
# that case; the rest name what does. A `+toolchain` argument is one of those: given to cargo or rustup, it
# reaches every command run under it, the gate's included, as RUSTUP_TOOLCHAIN, which is how rustup then
# names it. One spelling of that variable is no other toolchain though: rustup hands the toolchain it
# resolved to every command it starts, so each test `cargo test` runs, and each gate such a test runs,
# inherits a RUSTUP_TOOLCHAIN naming that toolchain — the pin itself whenever rust/ chose it. A variable
# naming the very toolchain the file alone resolves to therefore passes for the pin; one naming any other
# toolchain, a `+toolchain` naming any other, and a rustup directory override all stand over the pin and are
# named back. The answer is rustup's last line, so the `info:` lines of an install by a rustup too old for
# RUSTUP_AUTO_INSTALL cannot pass for it.
craftbridge_toolchain_status_in() {
  local dir="$1" line pin file_line
  if ! command -v rustup >/dev/null 2>&1; then
    echo no-rustup
    return
  fi
  if ! line="$(craftbridge_in "$dir" rustup show active-toolchain 2>&1)"; then
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
  pin="$(cd "$dir" && pwd -P)/rust-toolchain.toml"
  case "$line" in
    *"(overridden by '$pin')") echo "ok ${line%% *}"; return ;;
    *"(overridden by environment variable RUSTUP_TOOLCHAIN)")
      # Take the variable off and ask rustup again: when the file then chooses exactly the toolchain the
      # variable named, the variable names the pin, and the build is the pin's build.
      if file_line="$(craftbridge_in "$dir" env -u RUSTUP_TOOLCHAIN rustup show active-toolchain 2>&1)"; then
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
# The refusal itself, for a script that builds what it checks: each refused variable on a line of its own
# under the script's name, and status 1, so `craftbridge_refuse_switches || exit 1` stops it before any
# build. The gate (tools/ci.sh), the Android build (tools/build-android.sh), a test run on an Android
# device (craftbridge_android_test, below) and the iPhone's device runner (tools/ios-device-test.sh) all
# build this way; the doctor reports the same lines as gaps.
craftbridge_refuse_switches() {
  local refused name
  name="$(basename "$0" .sh)"
  refused="$(craftbridge_refused_environment)"
  [ -n "$refused" ] || return 0
  printf '%s\n' "$refused" | sed "s/^/$name: /" >&2
  echo "$name: the gate runs the pinned toolchain as rust/Cargo.toml and rust/clippy.toml configure it: unset it and run again" >&2
  return 1
}

# Set this shell's test environment to exactly the given `TEST_RUNNER_<name>=<value>` assignments:
# every variable the calling shell exported that would reach the tests is dropped first, and only
# what the caller names is exported after. Two prefixes reach them. `xcodebuild test` hands every
# `TEST_RUNNER_<name>` of its own environment to the process it starts the tests in, under `<name>`
# with the prefix dropped; and `simctl` hands every `SIMCTL_CHILD_<name>` of its own to what it
# starts in the simulator, a boot's whole session among them, so a simulator `cargo xtask simulator`
# boots for the run gives its tests each such variable too. A variable one of the suite's own gates
# reads, left exported in the shell the gate was called from — `TEST_RUNNER_CRAFTBRIDGE_BASELINE=1`
# or `SIMCTL_CHILD_CRAFTBRIDGE_BASELINE=1`, still standing after a baseline measurement asked for by
# hand — would reach that test in every later run, and the gate's result would depend on the
# developer's shell rather than on the tree. swift_test therefore starts its suite after this,
# naming only its sandbox's home, temporary folder and Foundation's home (tools/ci.sh). What it
# drops goes from the caller's own environment too, so a stage runs it in a subshell of its own,
# and every variable without either prefix passes through untouched. A word that is no
# `TEST_RUNNER_` assignment is refused, rather than exported or dropped for a misspelling, and so is
# an inherited variable of either prefix that the shell cannot drop, which would reach the tests. The
# stage calls this on the left of an `&&`, where bash holds its `set -e` off within the function, so
# each drop and each export is checked where it runs: one that failed would otherwise pass, and the
# run would go ahead with a variable left standing or an assignment missing.
craftbridge_set_test_runner_env() {
  local assignment entry name
  # The names are read from the environment the shell hands what it starts, not from its own list of
  # variables: one whose name no shell variable may take, with a dash or a dot in it, is handed on all
  # the same, yet bash 5 leaves it out of `compgen -e` and no bash will unset it.
  while IFS= read -r -d '' entry; do
    name="${entry%%=*}"
    case "$name" in
      TEST_RUNNER_* | SIMCTL_CHILD_*) unset -v "$name" && continue ;;
      *) continue ;;
    esac
    echo "$(basename "$0" .sh): craftbridge_set_test_runner_env cannot drop the inherited $name, a name the shell will not unset, so it would reach the tests: run again without it" >&2
    return 2
  done < <(env -0)
  for assignment in "$@"; do
    case "$assignment" in
      # The case pattern has just told the value to be a `NAME=value` assignment itself, so exporting
      # that value is the export; the `${assignment?}` form is what says so to shellcheck, whose plain
      # `export "$assignment"` reading expects a bare name. A word whose own name the shell will not
      # export — one with a space or a dash where a name may hold only letters, numbers and underscores
      # — fails here and falls to the refusal beside a word without the prefix, rather than leave the
      # run one sandbox assignment short.
      TEST_RUNNER_*=*) export "${assignment?}" && continue ;;
    esac
    echo "$(basename "$0" .sh): craftbridge_set_test_runner_env takes the run's TEST_RUNNER_ assignments, and $assignment is none" >&2
    return 2
  done
}

# Refuse a gate run that would build or test under an Xcode or SwiftPM setting left in place of the tree's
# own: a scheme or a test plan a developer's Xcode left on disk, a workspace or project left beside the
# package's manifest, every file left in the package's own configuration folder, a mirror in the machine's
# own SwiftPM configuration folder, and the mirror file `SWIFTPM_MIRROR_CONFIG` names.
# Given `-scheme <name>`, xcodebuild runs the file `<name>.xcscheme` wherever Xcode keeps a project's or a
# package's schemes in preference to the scheme it generates itself: it builds and tests exactly what that
# file says, its test environment and any test plan it names among them. So a scheme left by opening the
# package in Xcode — `ios/sdk/.swiftpm`, which git ignores — or edited into a sample's project bundle, would
# make the gate answer for what the machine's Xcode left rather than for the tree: one holding
# `CRAFTBRIDGE_BASELINE=1`, with nothing exported in the shell, was shown to make swift_test run the baseline
# measurement beside every other suite. A workspace or project left beside the package's manifest does the
# same with no file in `.swiftpm`: run in the package's folder with no `-project` or `-workspace`, xcodebuild
# opens that bundle in place of the package, and runs the schemes it holds or the schemes of the packages and
# projects it names, one of another checkout among them. So does the configuration left in the package's own
# ignored `configuration` folder: `swift build` and `xcodebuild`, exactly as the stages and the runner call
# them, read a mirror or a registry written there — as the official `swift package config` writes it — when
# they resolve dependencies, and fetch the sources they build from the source a mirror there points at in
# place of the one a dependency's URL names; every file left under `ios/sdk/.swiftpm/configuration` is refused
# from here too, since nothing of the tree sits there. Both tools read mirrors from two more places: the file
# `SWIFTPM_MIRROR_CONFIG` names, and the machine's own SwiftPM configuration folder, which every package's
# resolve reads — under the home `CFFIXED_USER_HOME` names when it is set, as the tools take it, and the
# user's home otherwise. A mirror there is refused too, by the variable's name or the file's, since it fetches
# as one in the package's folder does and neither is the tree's; the rest of the machine's folder, its package
# collections among them, decides nothing the gate builds. Every caller that builds the package through a
# scheme — the Swift stages and the device runner — calls this before it builds, takes a simulator or takes
# the device's lock, naming the sample project bundles it builds through a scheme too, where both the user's
# and the shared schemes sit; the package's own places are read here, from this one spelling of its folder, so
# no caller names them again. Of the tree's places, only a file git does not track is refused, ignored or not:
# a scheme the tree commits, as a project may to fix its build and test settings for everyone, is the tree's
# own, and the gate answers for it as for any other tracked file. Nothing here deletes or edits anything: a
# developer's file stays exactly as it was found, because removing it is the developer's choice, and the
# refusal repeats until it has been made. Each refusal is one line naming the file — one of the tree's from
# the repository's root, the machine's by its own path — or the variable, and what it is, and one with the
# command that removes it, moves it aside or unsets it, quoted so that it pastes, from the root, whatever the
# name holds; the refusal is status 2.
craftbridge_refuse_leftover_xcode_and_swiftpm_settings() {
  local package=ios/sdk listing found kind reason removal quoted tail machine mirrors
  local -a leftovers=() machine_mirrors=()
  # The machine's folder as each tool keeps it: the command line's `.swiftpm/configuration`, on macOS a link
  # to the `Library` folder Xcode reads, which a machine set up before that move may still hold apart.
  machine="${CFFIXED_USER_HOME:-$HOME}"
  for mirrors in "$machine/.swiftpm/configuration/mirrors.json" "$machine/Library/org.swift.swiftpm/configuration/mirrors.json"; do
    [ -e "$mirrors" ] || continue
    [ ${#machine_mirrors[@]} -eq 0 ] || [ ! "$mirrors" -ef "${machine_mirrors[0]}" ] || continue
    machine_mirrors+=("$mirrors")
  done
  # `git ls-files --others` names every file git does not track, the ignored ones too, since it applies
  # no ignore rule unless asked to; the second listing stops at a folder git tracks nothing of
  # (`--directory`), so a workspace or project left whole beside the manifest is one entry, and the build
  # output beside it is read no further. Their answer goes through a file, where its status can be read: a
  # git that cannot answer would otherwise read as a tree with nothing left in it.
  listing="$(mktemp)" || return 2
  if ! {
    git -C "$CRAFTBRIDGE_ROOT" --literal-pathspecs ls-files -z --others -- "$package/.swiftpm" "$@" &&
      git -C "$CRAFTBRIDGE_ROOT" --literal-pathspecs ls-files -z --others --directory -- "$package"
  } >"$listing"; then
    rm -f "$listing"
    echo "$(basename "$0" .sh): craftbridge_refuse_leftover_xcode_and_swiftpm_settings cannot ask git which files the tree tracks, so it cannot tell a scheme Xcode left from one the tree commits" >&2
    return 2
  fi
  while IFS= read -r -d '' found; do
    case "$found" in
      *.xcscheme | *.xctestplan) leftovers+=("$found") ;;
      "$package"/.swiftpm/configuration/*) leftovers+=("$found") ;;
      "$package"/*.xcworkspace/ | "$package"/*.xcodeproj/)
        # Only a bundle beside the manifest is one xcodebuild opens for the package.
        case "${found#"$package"/}" in
          */*/) ;;
          *) leftovers+=("${found%/}") ;;
        esac
        ;;
    esac
  done <"$listing"
  rm -f "$listing"
  if [ ${#leftovers[@]} -eq 0 ] && [ ${#machine_mirrors[@]} -eq 0 ] && [ -z "${SWIFTPM_MIRROR_CONFIG:-}" ]; then
    return 0
  fi
  for found in ${leftovers[@]+"${leftovers[@]}"}; do
    reason="xcodebuild reads the schemes kept there in place of the ones it generates from the tree, and runs the test plans they name"
    tail="so a run would answer for what this machine's Xcode left there, not for the tree"
    removal="rm -f --"
    case "$found" in
      *.xcscheme) kind=scheme ;;
      *.xctestplan) kind="test plan" ;;
      "$package"/.swiftpm/configuration/*)
        kind="package configuration"
        reason="swift build and xcodebuild read the configuration written here when they resolve the package's dependencies, fetching what they build from the source a mirror here points at in place of the one a dependency's URL names"
        tail="so a run would answer for what this machine's swift package configuration left there, not for the tree"
        ;;
      *)
        case "$found" in *.xcworkspace) kind=workspace ;; *) kind=project ;; esac
        reason="xcodebuild opens a workspace or project beside the package's manifest in place of the package, and runs the schemes it holds or names"
        removal="rm -rf --"
        ;;
    esac
    printf -v quoted '%q' "$found"
    echo "$(basename "$0" .sh): craftbridge_refuse_leftover_xcode_and_swiftpm_settings refuses the $kind $found, which git does not track: $reason, $tail" >&2
    echo "$(basename "$0" .sh):   remove it and run again: $removal $quoted" >&2
  done
  for found in ${machine_mirrors[@]+"${machine_mirrors[@]}"}; do
    printf -v quoted '%q' "$found"
    echo "$(basename "$0" .sh): craftbridge_refuse_leftover_xcode_and_swiftpm_settings refuses the machine's mirrors $found: swift build and xcodebuild read the mirrors SwiftPM keeps for the whole machine when they resolve any package's dependencies, fetching what they build from the source a mirror there points at in place of the one a dependency's URL names, so a run would answer for this machine's configuration, not for the tree" >&2
    echo "$(basename "$0" .sh):   move it aside and run again: mv -- $quoted $quoted.aside" >&2
  done
  if [ -n "${SWIFTPM_MIRROR_CONFIG:-}" ]; then
    printf -v quoted '%q' "$SWIFTPM_MIRROR_CONFIG"
    echo "$(basename "$0" .sh): craftbridge_refuse_leftover_xcode_and_swiftpm_settings refuses SWIFTPM_MIRROR_CONFIG=$quoted: swift build and xcodebuild read the mirrors the file it names holds when they resolve the package's dependencies, fetching what they build from the source a mirror there points at in place of the one a dependency's URL names, so a run would answer for what this shell exports, not for the tree" >&2
    echo "$(basename "$0" .sh):   unset it and run again: unset SWIFTPM_MIRROR_CONFIG" >&2
  fi
  return 2
}

# The gate's lint library (AGENTS.md §3), which `cargo dylint` builds with the nightly its own
# rust-toolchain.toml pins and runs over the workspace, and the cargo-dylint release that runs it. The gate
# pins the release: one can change how a library is built and driven, so an upgrade is an edit here and a
# fresh gate run, never a silent surprise.
export CRAFTBRIDGE_LINT_LIBRARY="$CRAFTBRIDGE_ROOT/rust/never-crash-lints"
export CRAFTBRIDGE_DYLINT="6.4.0"
# Each worktree builds into its own rust/target: a machine-wide CARGO_TARGET_DIR would share one target
# folder between worktrees, whose gates would then reuse each other's artifacts. A machine-wide
# CARGO_BUILD_TARGET would send the host build of xtask to a <triple>/ folder, where the gate, which
# starts the binary from debug/, would not find it.
export CARGO_TARGET_DIR="$CRAFTBRIDGE_ROOT/rust/target"
unset CARGO_BUILD_TARGET
# The greps every script here runs. `cargo xtask early-pipes`, the shell stage's own rule, reads a grep's
# options from the grep's own words, which holds only while nothing else hands the grep words or decides
# where its options end: macOS's grep takes option words out of GREP_OPTIONS, so a grep quits early at a
# word no script wrote, and POSIXLY_CORRECT makes either grep read a word after the pattern as a file rather
# than as an option, so a file no script named is read instead of the pipe and its writer dies of SIGPIPE.
# Both out of the environment, from this one definition; a script that sets either is refused by that rule,
# because under it the rule cannot tell what the script's greps read.
unset GREP_OPTIONS POSIXLY_CORRECT

# Every cargo command the scripts run, and never while the repository root holds a Cargo configuration.
# Cargo merges a .cargo/config.toml from every folder above the one it runs in, so one at the root would
# reach every cargo command: its `runner` would stand in for the test and rule binaries, its `[patch]` or
# `[source]` would build another copy around the pin, and the gate would pass having checked nothing,
# before the rules that refuse the folder could run. The one Cargo configuration is rust/.cargo/config.toml,
# which the rules read. rustup selects a toolchain pin from the folder a command runs in, so
# `craftbridge_cargo_in` names that folder and runs there through `craftbridge_in`: `craftbridge_cargo` runs
# in rust/, where the workspace's pin is rust/rust-toolchain.toml, while the gate's lint library runs in its
# own folder, where its own pin is the nightly it names. The guard is the one definition, whichever folder
# is chosen.
craftbridge_cargo_in() {
  local dir="$1"
  shift
  if [ -e "$CRAFTBRIDGE_ROOT/.cargo" ]; then
    echo "$(basename "$0" .sh): .cargo at the repository root: Cargo would merge it into every cargo build; the one Cargo configuration is rust/.cargo/config.toml" >&2
    return 1
  fi
  craftbridge_in "$dir" cargo "$@"
}
craftbridge_cargo() {
  craftbridge_cargo_in "$CRAFTBRIDGE_ROOT/rust" "$@"
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

# Where the runs that share this machine take turns: the iPhones — simulators (`cargo xtask simulator`,
# rust/xtask/src/simulator.rs) and the physical acceptance device (`cargo xtask device-lock`, held by
# tools/ios-device-test.sh, rust/xtask/src/device_lock.rs) alike, one lock file per device in the one
# folder, so the runs of one device wait behind each other and never on another's — and, since `cargo
# xtask release-lock` (rust/xtask/src/release_lock.rs, held by tools/release.sh), a repository being
# staged: one lock file per repository, which a second staging run is refused rather than waits on.
# Outside every checkout, so a run in one worktree sees the runs in another. The name says simulators
# because it is what every checkout on the machine reads, those not yet taking the other locks too, and
# a run's own folder set under it (a review's private one for its plants) would be dropped without a
# word by a new name.
export CRAFTBRIDGE_SIMULATOR_LOCKS="${CRAFTBRIDGE_SIMULATOR_LOCKS:-$HOME/.cache/craftbridge/simulators}"

# The Xcode release the iOS build is made and tested with.
export CRAFTBRIDGE_XCODE="26.6"

# What building for the iOS device takes, written once for every run that compiles or runs one: the gate's
# rust_mobile stage, tools/build-xcframework.sh and tools/ios-device-test.sh. gemm-f16 (through
# candle-core, from EffectCraft) assembles fp16 instructions, which rustc's default CPU for
# `aarch64-apple-ios`, the A7, lacks (`instruction requires: fullfp16`). The device code is built for the
# A12, the oldest chip iOS 17 runs on (the package's minimum, `ios/sdk/Package.swift`), which has them: a
# newer CPU lets LLVM use instructions the A12 lacks (the A13's dot products, for one), which stop an app
# with an illegal instruction on a phone the package supports, so this moves only with that minimum. The
# Simulator target defaults to a newer CPU, so the other triples need nothing. A target-specific flag, never
# `target-cpu=native`, which is the host's. The gate refuses the variable in the environment it starts in,
# so each run sets it itself, for the triple it names, in the shell that starts cargo:
# `craftbridge_ios_target_env <triple>`.
craftbridge_ios_target_env() {
  case "$1" in
    aarch64-apple-ios) export CARGO_TARGET_AARCH64_APPLE_IOS_RUSTFLAGS="-C target-cpu=apple-a12" ;;
  esac
}

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

# The installed NDK of the major above, the newest of them: its folder, or nothing and status 1. `-H` follows
# an ndk/ that is itself a symbolic link; the `|| true` keeps a missing ndk folder from ending a caller that
# runs under pipefail before it has printed the fix.
craftbridge_ndk_home() {
  local ndk
  ndk="$(find -H "$ANDROID_HOME/ndk" -mindepth 1 -maxdepth 1 -name "$CRAFTBRIDGE_NDK_MAJOR.*" 2>/dev/null | sort | tail -n 1 || true)"
  [ -n "$ndk" ] && printf '%s\n' "$ndk"
}

# Whether a Rust target runs on Android, and the toolchain file's targets that do, one per line.
craftbridge_is_android_target() {
  case "$1" in *-linux-android) return 0 ;; *) return 1 ;; esac
}
craftbridge_android_targets() {
  local target
  for target in $CRAFTBRIDGE_RUST_TARGETS; do
    if craftbridge_is_android_target "$target"; then printf '%s\n' "$target"; fi
  done
}

# What building for an Android target takes, written once for every run that compiles or runs one: the
# gate's rust_mobile stage, tools/build-android.sh and a test run on a device or an emulator. The NDK is the
# installed one of the pinned major; the platform level is the module's minSdk, below which the NDK's own
# libraries (the media and image reader calls) lack symbols the library links. The aarch64 library carries
# the `fp16` target feature, Rust's name for LLVM's `fullfp16`, which gemm-f16's half-precision assembly
# assembles only with; rustc passes LLVM's own spelling through only as an unknown feature it warns may be
# unsound. A target-specific flag, never `target-cpu=native`, which is the host's, and none for the other
# target, whose code takes no such feature. Must run in the shell that starts cargo.
craftbridge_android_env() {
  local ndk
  if ! ndk="$(craftbridge_ndk_home)"; then
    echo "$(basename "$0" .sh): no Android NDK r$CRAFTBRIDGE_NDK_MAJOR under $ANDROID_HOME/ndk: run tools/doctor.sh for the fix" >&2
    return 1
  fi
  if ! command -v cargo-ndk >/dev/null 2>&1; then
    echo "$(basename "$0" .sh): no cargo-ndk: run tools/doctor.sh for the fix" >&2
    return 1
  fi
  export ANDROID_NDK_HOME="$ndk" CARGO_NDK_PLATFORM="$CRAFTBRIDGE_ANDROID_MIN_SDK"
  export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="-C target-feature=+fp16"
}
# cargo-ndk, the official tool, names the NDK's compilers and archiver for a target, and only it links: the
# linker it names is cargo-ndk itself, which links with the NDK when `cargo ndk` started the build and not
# otherwise. So a run that links goes through `cargo ndk` (tools/build-android.sh, and a test run below),
# and a check that reads cargo's own messages, which `cargo ndk` would read in its place, takes what
# `cargo ndk-env` prints for each target it checks, which it uses for the compilers alone:
# `craftbridge_android_target_env <triple>`, after `craftbridge_android_env`, gives the build scripts that
# compile C for the phone the phone's compiler.
# cargo-ndk runs as a cargo command through the guarded cargo, as every other does.
craftbridge_android_target_env() {
  local exports
  exports="$(craftbridge_cargo ndk-env -t "$1")" || return 1
  eval "$exports"
}
# A test run on a device or an emulator, `craftbridge_android_test <triple> <cargo test arguments…>`:
# cargo-ndk links each test binary with the NDK, and its runner pushes it to the device adb names
# (ANDROID_SERIAL picks one of several), runs it there and removes it. The run's status is cargo's. It
# tests what ships, so it is refused the switches the gate refuses, and it runs in a subshell: the
# Android environment it sets includes a variable the gate refuses in the shell it starts in, which must
# not stay behind in the caller's.
craftbridge_android_test() (
  target="$1"
  shift
  craftbridge_refuse_switches || exit 1
  craftbridge_android_env || exit 1
  craftbridge_cargo ndk -t "$target" test --locked "$@"
)

# Gradle runs on the catalog's JDK, the release AGP is tested with here — the `jdk` pin craftbridge_load_pins
# read. JAVA_HOME wins when it is that major; else macOS's registry, else Homebrew's keg (keg-only, so the
# registry never lists it), else the JDK Android Studio bundles.
craftbridge_java_major() {
  local majors
  # The whole output is read before its first line is taken: `head -n 1` ends the pipe at that line, sed
  # dies of SIGPIPE with the rest still to write, and under `pipefail` a candidate's major version would
  # depend on timing instead of on what `java -version` printed.
  majors="$( "$1/bin/java" -version 2>&1 | sed -n 's/.*version "\([0-9]*\).*/\1/p' )"
  printf '%s\n' "${majors%%$'\n'*}"
}
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
