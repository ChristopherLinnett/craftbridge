#!/usr/bin/env bash
# The gate: every check a change must pass before it is pushed or merged (AGENTS.md).
#
#   tools/ci.sh              every stage, the doctor first
#   tools/ci.sh rules swift  only the named stages or groups (names below), and the doctor only if named
#
# Every stage runs even when an earlier one fails, so one run reports everything that is broken. Every
# name is checked before the first stage runs, so a misspelt one costs nothing.
set -euo pipefail
# Git exports its repository variables (GIT_DIR and the rest) to a hook and to whatever `git rebase --exec`
# or `git bisect run` starts. Every git command the gate runs, the rules' and their tests', would then act
# on the repository they name instead of the folder it is given. The gate runs the same whoever starts it.
# shellcheck disable=SC2046 # one variable name per word, split on purpose
unset $(git rev-parse --local-env-vars 2>/dev/null)
# shellcheck source=tools/env.sh
source "$(dirname "$0")/env.sh"
cd "$CRAFTBRIDGE_ROOT"

# rustup selects the pinned toolchain from rust/rust-toolchain.toml only inside rust/. Cargo merges a
# .cargo/config.toml from every folder above the one it runs in, so a configuration at the repository root
# reaches every cargo stage: its `runner` would stand in for the test and rule binaries, which would then
# pass having run nothing, before the rules that refuse the folder could run. So no cargo stage runs
# while the root holds one; rust/.cargo is the one configuration the rules read.
cargo_in_rust() {
  if [ -e .cargo ]; then
    echo "ci: .cargo at the repository root: Cargo would merge it into every cargo stage; the one Cargo configuration is rust/.cargo/config.toml" >&2
    return 1
  fi
  (cd rust && cargo "$@")
}

# The files a stage hands its tool, one per line into the caller's `files`: the rules' own walk chooses
# them (`cargo xtask files`, rust/xtask/src/files.rs), so every stage reads exactly what the rules read and
# build output is told from source in one place. A list that comes back empty fails, as a check that read
# nothing must.
listed() {
  local kind="$1" list file
  files=()
  list="$(mktemp)"
  if ! xtask files "$kind" >"$list"; then
    rm -f "$list"
    return 1
  fi
  while IFS= read -r file; do files+=("$file"); done <"$list"
  rm -f "$list"
  if [ ${#files[@]} -eq 0 ]; then
    echo "ci: no $kind files to check" >&2
    return 1
  fi
}

# The doctor reads the pins from this run's xtask, which the guard below has built before any stage, so it
# rebuilds no binary another run may be starting; handed to the doctor alone, the path reaches no tree's
# doctor but this one's.
stage_doctor() { CRAFTBRIDGE_XTASK="$xtask_run/debug/xtask" tools/doctor.sh --gate; }
stage_rust_fmt() { cargo_in_rust fmt --check; }
# The rule checks are built with `cargo build` and their binary run here, not through the `cargo xtask`
# alias or `cargo run`: a Cargo configuration can redefine an alias, and a `[target.<triple>].runner`
# replaces whatever binary `cargo run` (and `cargo test`) would start, so either would pass these stages
# having checked nothing. `cargo build` runs no runner, and env.sh fixes where it builds; the rules then
# report the alias or the runner. Building xtask compiles none of the workspace's own crates, so no build
# script of theirs runs before `rules` has judged whether it may exist.
# Every cargo build replaces the binary in its target folder, even one with nothing to rebuild, so a second
# gate running in this worktree would replace the binary this one is starting, which is then killed or not
# found. So each run builds xtask once into a target folder of its own: `build.build-dir` keeps the
# intermediate artifacts in env.sh's target folder, where every stage shares them, and only the binary
# lands in the run's folder, which nothing else writes. Each later stage starts that same file.
xtask_run="$(mktemp -d "${TMPDIR:-/tmp}/craftbridge-xtask.XXXXXX")"
trap 'rm -rf "$xtask_run"' EXIT
build_xtask() {
  [ -x "$xtask_run/debug/xtask" ] ||
    CARGO_BUILD_BUILD_DIR="$CARGO_TARGET_DIR" cargo_in_rust build --package xtask --locked --quiet --target-dir "$xtask_run"
}
xtask() {
  build_xtask || return 1
  "$xtask_run/debug/xtask" "$@"
}
# A cargo stage that compiles the workspace prints its diagnostics through `cargo xtask compiled`, which
# then reads the dep-info of every workspace unit the stage built and fails on any file the compiler read
# that is not one of that crate's own .rs files the rules read (AGENTS.md §3). xtask is built first: inside
# the pipe, its build would wait on the build lock while cargo, blocked on the full pipe, held it.
compiling() {
  build_xtask || return 1
  cargo_in_rust "$@" | "$xtask_run/debug/xtask" compiled
}
# --all-features: code behind a feature is code that ships to whoever turns the feature on. What ships is
# linted with every never-crash lint; the tests' build takes back the two that name the panicking calls
# by path, as rust/clippy.toml's `allow-*-in-tests` take back the lints they shadow.
stage_rust_clippy() {
  local status=0
  compiling clippy --workspace --lib --bins --examples --all-features --locked --message-format=json -- -D warnings || status=1
  compiling clippy --workspace --tests --benches --all-features --locked --message-format=json -- -D warnings \
    -A clippy::disallowed_methods -A clippy::disallowed_macros || status=1
  return "$status"
}
stage_rust_test() { compiling test --workspace --locked --message-format=json; }
stage_rust_mobile() {
  local target profile status=0
  # The default members are the libraries an app links; the host tools never run on a phone. Clippy, not
  # a bare check: the platform modules compile only for the phones, so the never-crash lints and the
  # SAFETY comments are only checked on them here, in the release profile apps link as well as the dev
  # one, since code under `cfg(not(debug_assertions))` exists only in the first. Every target is checked
  # even after one fails, so one run names each target that is broken.
  for target in $CRAFTBRIDGE_RUST_TARGETS; do
    for profile in dev release; do
      compiling clippy --locked --all-features --target "$target" --profile "$profile" --message-format=json -- -D warnings || status=1
    done
  done
  return "$status"
}
stage_rust_deny() { cargo_in_rust deny --locked check licenses bans sources; }
stage_rules() { xtask rules; }
stage_generated() { xtask generate --check; }
# The plan's views, weighed by their bytes rather than their times: the kit's own builders rebuild them
# from their sources into a scratch copy of plan/ and every difference fails, named (rust/xtask/src/plan_views.rs).
# It needs Node and `npm ci` in `plan/` and nothing else: `--no-shots` is the kit's own switch that keeps a
# browser down, and the diagram build runs with the absolute path of `true` in draw.io's place (for a
# `DRAWIO_BIN` it cannot find, the kit starts draw.io Desktop), so no app starts: each `.drawio`
# is rewritten from its source and each committed SVG export is trimmed as the kit always trims it, and both
# are weighed, as is an export no source builds any more; only the drawing inside an SVG, draw.io's own, is
# beyond what the stage can rewrite.
# The dependencies are asked first, as the doctor asks them (tools/env.sh): rebuilt from others, the views
# would fail as if edited by hand.
stage_plan_views() {
  if ! craftbridge_plan_dependencies; then
    echo "ci: the plan kit's dependencies are not installed as plan/package-lock.json pins them: run (cd plan && npm ci)" >&2
    return 1
  fi
  xtask plan-views --check
}
stage_swift_format() {
  # Generated Swift is UniFFI's and the generator's style, not ours; xtask checks it is untouched.
  listed swift || return 1
  xcrun swift-format lint --strict --configuration ios/sdk/.swift-format "${files[@]}"
}
stage_swift_lint() {
  # SwiftLint's missing_docs over ios/sdk/.swiftlint.yml, and the Swift never-crash bans, the pointer
  # confinement and the tool directives (AGENTS.md §3, §4, §10) read through the compiler's language
  # server with SwiftPM's settings for every build the package ships — Debug and Release for each Apple
  # slice rust/rust-toolchain.toml lists (rust/xtask/src/swift_lint.rs) — so each `#if` branch a shipping
  # build compiles is read resolved. After the build: a file that does not compile is cheaper to name
  # through the compiler than through the analyser.
  xtask swift-lint
}
stage_swift_build() {
  # Every build the Swift package ships, in the list `cargo xtask swift-builds` prints from the same table
  # the Swift checks read (rust/xtask/src/pins.rs): each configuration of each slice the toolchain file
  # names, once for slices that share a destination, so a slice added there is built with no other edit,
  # and there is no second list of slices to keep in step. A branch that compiles only in one
  # configuration or only off the simulator — `#if !DEBUG`, `#if !targetEnvironment(simulator)` —
  # compiles in one of these builds.
  # DIAGNOSE_MISSING_TARGET_DEPENDENCIES=YES_ERROR makes an import the package does not declare an
  # error. Without it a module an earlier build left in the derived data satisfies the import with only
  # a warning, so `CraftBridge` could import a plugin against AGENTS.md §2 and still build.
  # build-for-testing builds the test targets too, which the package scheme builds only for testing.
  local builds configuration destination status=0
  builds="$(mktemp)"
  if ! xtask swift-builds >"$builds"; then
    rm -f "$builds"
    return 1
  fi
  if [ ! -s "$builds" ]; then
    echo "ci: no Swift builds to run" >&2
    rm -f "$builds"
    return 1
  fi
  while IFS=$'\t' read -r configuration destination; do
    (cd ios/sdk && xcodebuild -scheme CraftBridge-Package -destination "$destination" -configuration "$configuration" -derivedDataPath .build/xcode -quiet build build-for-testing \
      DIAGNOSE_MISSING_TARGET_DEPENDENCIES=YES_ERROR) || status=1
  done <"$builds"
  rm -f "$builds"
  return "$status"
}
stage_android() {
  # A fresh JVM every run: ktlint keeps the .editorconfig files it has parsed for the life of the
  # Gradle daemon, so a warm daemon would go on checking against rules that have since changed.
  # --continue runs every task whose inputs are still good after one fails, and the second build below
  # runs whatever the first found, so a formatting error does not hide a Lint or compile error until the
  # next run.
  # The tests' Kotlin compiles too, under the same warnings-as-errors rules as the code it tests.
  # A task a module's build file disables, or guards with `onlyIf`, checks nothing and still lets the
  # build succeed; `cargo xtask skipped-tasks` reads Gradle's log and fails on any such task of the SDK's
  # projects (rust/xtask/src/gradle_log.rs). --console=plain makes Gradle print every task's outcome even
  # in a terminal.
  # Spotless runs in a build, and so a JVM, of its own: ktlint sets IntelliJ's JVM-wide `idea.home.path`
  # and `idea.config.path` while it starts and then puts back what it found, and Lint, which runs inside
  # Gradle's JVM, loses the value its own start-up has just set and crashes ("Could not find the
  # installation home path") when the two start together.
  local log status=0
  log="$(mktemp)"
  gradle_build() {
    (cd android/sdk && ./gradlew --no-daemon --continue --warning-mode=fail --console=plain "$@") | tee -a "$log"
  }
  gradle_build spotlessCheck || status=1
  # detekt runs in a build of its own too, and its command line in a JVM of its own (plan D-28): its
  # analysis starts the Kotlin compiler's IntelliJ core, which sets IntelliJ's JVM-wide settings as
  # ktlint does, so it never shares a JVM with Lint. detektMain and detektTest are what
  # craftbridge.android.library registers over every variant and every test component of a module, and
  # the build's rule jar carries its own tests, run here beside the rules they check.
  gradle_build detektMain detektTest :build-logic:detekt-rules:test || status=1
  gradle_build lint lintRelease assembleRelease compileDebugUnitTestSources compileDebugAndroidTestSources || status=1
  xtask skipped-tasks "$log" || status=1
  rm -f "$log"
  return "$status"
}
stage_shell() {
  # Every shell script wherever it lives, by its `.sh` ending or its #! line, bar the Gradle wrapper, which
  # Gradle writes. --norc: an rc file beside the scripts would be read as directives for all of them; the
  # rules refuse one, and shellcheck reads none.
  listed shell || return 1
  shellcheck --norc -x "${files[@]}"
}
stage_duplicates() {
  # The owner's first rule: reuse by extraction, never a second copy. Ten repeated lines, or seventy
  # repeated tokens, fail: one pass for each, because jscpd keeps a clone only when it meets both of its
  # minimums. jscpd 5.4.3 keeps a clone whose measured span is longer than --min-lines, and the span of a
  # block that code follows takes in the next line, so 9 makes every ten-line repeat fail (and a nine-line
  # one with code after it); the line pass asks for ten tokens, one per line, so ten blank lines are not
  # ten repeated lines. --min-tokens counts a clone of exactly that many, so 70 is seventy, on any number
  # of lines. jscpd reads exactly the rules' list (generated code, which repeats by design, and build
  # output are not on it): each script the shell stage checks is named to jscpd as bash, so the two stages
  # read the same scripts, and --no-gitignore, with the rules refusing the .ignore files and markers jscpd
  # would honour, keeps that list the only one it follows. --absolute names each copy by its full path;
  # --max-size lifts the 1 MB default under which a large file was skipped unseen; --fail-on-empty fails a
  # run that read nothing.
  local file status=0 pass names="" shell_names=()
  listed duplicates || return 1
  for file in "${files[@]}"; do
    case "$file" in
      *.rs | *.swift | *.kt | *.kts | *.sh) ;;
      *) names="${names:+$names,}${file##*/}" ;;
    esac
  done
  if [ -n "$names" ]; then shell_names=(--formats-names "bash:$names"); fi
  for pass in "ten lines|--min-lines 9 --min-tokens 10" "seventy tokens|--min-lines 0 --min-tokens 70"; do
    echo "ci: duplicates, the ${pass%%|*} pass"
    # shellcheck disable=SC2086 # each pass is two flag pairs, split on purpose
    # ${a[@]+…}: bash 3.2 treats an empty array as unset under `set -u`.
    npx --yes "$CRAFTBRIDGE_JSCPD" ${pass#*|} --format rust,swift,kotlin,bash ${shell_names[@]+"${shell_names[@]}"} --no-gitignore --absolute --max-size 100mb \
      --fail-on-empty --reporters console --exit-code 1 "${files[@]}" || status=1
  done
  return "$status"
}

# `rules` runs before every cargo stage that compiles the workspace: what it reads needs no build, and a
# build script runs as soon as cargo compiles its crate.
stages=(doctor rules rust_fmt rust_clippy rust_test rust_mobile rust_deny generated plan_views swift_format swift_build swift_lint android shell duplicates)
# The stage groups the decision and plan §15 name: Rust and Swift are each several stages.
group_rust=(rust_fmt rust_clippy rust_test rust_mobile rust_deny)
group_swift=(swift_format swift_build swift_lint)
selected=()
for name in "$@"; do
  case "$name" in
    rust) selected+=("${group_rust[@]}") ;;
    swift) selected+=("${group_swift[@]}") ;;
    *)
      if ! declare -F "stage_$name" >/dev/null; then
        echo "ci: no stage '$name' (stages: ${stages[*]}; groups: rust, swift)" >&2
        exit 2
      fi
      selected+=("$name")
      ;;
  esac
done
if [ ${#selected[@]} -eq 0 ]; then selected=("${stages[@]}"); fi

# The gate judges the change by one compiler set: the toolchain rust/rust-toolchain.toml pins, on stable
# rules, with nothing in the environment switching what its tools check. rustup resolves rust/ to that pin
# unless another toolchain stands over it — an environment variable, a rustup override, or a `+toolchain`
# argument to a cargo or rustup the gate runs under, which reaches it as the variable — and each of those
# reaches every cargo call the gate makes. The one variable naming the pin itself, which rustup hands to
# every command it starts, is that toolchain and no override of it; tools/env.sh tells the two apart. The
# pins themselves come from `cargo xtask pins`, the one reader of both pin files, run as every other xtask
# command here is: built, then its binary run.
rust_status="$(craftbridge_toolchain_status)"
case "$rust_status" in
  ok\ *) ;;
  no-rustup) echo "ci: no rustup on PATH: install it from https://rustup.rs" >&2; exit 1 ;;
  missing) echo "ci: the toolchain pinned in rust/rust-toolchain.toml is not installed: cd rust && rustup toolchain install" >&2; exit 1 ;;
  override\ *) echo "ci: rust/ resolves an overridden toolchain, not the pin in rust/rust-toolchain.toml: ${rust_status#override }" >&2; exit 1 ;;
esac
refused="$(craftbridge_refused_environment)"
if [ -n "$refused" ]; then
  printf '%s\n' "$refused" | sed 's/^/ci: /' >&2
  echo "ci: the gate runs the pinned toolchain as rust/Cargo.toml and rust/clippy.toml configure it: unset it and run again" >&2
  exit 1
fi
if pins="$(xtask pins 2>&1)"; then
  craftbridge_load_pins <<<"$pins"
else
  echo "ci: cargo xtask pins could not read rust/rust-toolchain.toml and android/sdk/gradle/libs.versions.toml: $pins" >&2
  exit 1
fi

failed=()
for name in "${selected[@]}"; do
  printf '\n== %s\n' "$name"
  if "stage_$name"; then echo "ci: $name ok"; else failed+=("$name"); echo "ci: $name FAILED"; fi
done

echo
if [ ${#failed[@]} -eq 0 ]; then
  echo "ci: ok (${selected[*]})"
else
  echo "ci: failed: ${failed[*]}"
  exit 1
fi
