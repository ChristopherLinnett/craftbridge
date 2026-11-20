#!/usr/bin/env bash
# The gate: every check of the repository (AGENTS.md). Hosted CI runs every stage of it, each in at least one
# of its jobs, on every pull request to main and every push to it (.github/workflows/ci.yml), and is the full
# gate; locally a change runs only the stages for what it touches (AGENTS.md §12).
#
#   tools/ci.sh                     every stage, the doctor first
#   tools/ci.sh rules swift         only the named stages or groups (names below), and the doctor only if named
#   tools/ci.sh --except rust swift every stage but the named ones, in the order below
#   tools/ci.sh --hosted            the stages of the hosted job of the runner's system, which RUNNER_OS names
#                                   as GitHub sets it, Linux or macOS (the split below)
#   tools/ci.sh --list <selection>  prints the stages a selection above runs, one line, and runs none
#
# Every stage runs even when an earlier one fails, so one run reports everything that is broken. Every
# name is checked before the first stage runs, so a misspelt one costs nothing. `--except` lets a job run
# what another job leaves, so a stage added below runs there with no second list to extend.
set -euo pipefail
# Git exports its repository variables (GIT_DIR and the rest) to a hook and to whatever `git rebase --exec`
# or `git bisect run` starts. Every git command the gate runs, the rules' and their tests', would then act
# on the repository they name instead of the folder it is given. The gate runs the same whoever starts it.
# shellcheck disable=SC2046 # one variable name per word, split on purpose
unset $(git rev-parse --local-env-vars 2>/dev/null)
# shellcheck source=tools/env.sh
source "$(dirname "$0")/env.sh"
cd "$CRAFTBRIDGE_ROOT"

# Every cargo command the stages run goes through env.sh's one guarded cargo, `craftbridge_cargo_in <dir>`,
# which `craftbridge_cargo` calls for rust/: rustup selects a pinned toolchain from the folder a command runs
# in — rust/rust-toolchain.toml inside rust/, the lint library's own pin inside its folder — and no cargo
# runs while the repository root holds a Cargo configuration that every stage would merge (tools/env.sh).
# The one exception is the lint library's run, which `cargo xtask dylint` starts itself so the run holds
# that check's shared lock (rust/xtask/src/lints.rs), in the same folder the guard chooses.

# Cargo inside the lint library's folder, its messages through `cargo xtask compiled` like every other
# compilation's: `in_library <cargo arguments>`. rustup reads a toolchain pin from the folder a command runs
# in, and the library's pin is its nightly, so this cargo runs there, through the same guarded
# `craftbridge_cargo_in`, and clippy finds rust/clippy.toml by its own search upward.
in_library() {
  build_xtask || return 1
  craftbridge_cargo_in "$CRAFTBRIDGE_LINT_LIBRARY" "$@" | "$xtask_run/debug/xtask" compiled
}

# What an xtask command lists, one item per line, into the caller's `items`: `items_of <what> <command>…`.
# A list that comes back empty fails, naming what it lists, as a check that read nothing must.
items_of() {
  local what="$1" list item
  shift
  items=()
  list="$(mktemp)"
  if ! xtask "$@" >"$list"; then
    rm -f "$list"
    return 1
  fi
  while IFS= read -r item; do items+=("$item"); done <"$list"
  rm -f "$list"
  if [ ${#items[@]} -eq 0 ]; then
    echo "ci: no $what" >&2
    return 1
  fi
}
# The files a stage hands its tool, into the caller's `files`: the rules' own walk chooses them (`cargo
# xtask files`, rust/xtask/src/files.rs), so every stage reads exactly what the rules read and build output
# is told from source in one place.
listed() {
  items_of "$1 files to check" files "$1" || return 1
  files=("${items[@]}")
}

# The doctor reads the pins from this run's xtask, which the guard below has built before any stage, so it
# rebuilds no binary another run may be starting; handed to the doctor alone, the path reaches no tree's
# doctor but this one's.
stage_doctor() { CRAFTBRIDGE_XTASK="$xtask_run/debug/xtask" tools/doctor.sh --gate; }
# The workspace's own crates, and the gate's lint library, which is no workspace member and so is outside
# the workspace's `cargo fmt`: the same rustfmt and the same configuration file, through its own manifest.
stage_rust_fmt() {
  local status=0
  craftbridge_cargo fmt --check || status=1
  craftbridge_cargo fmt --check --manifest-path "$CRAFTBRIDGE_LINT_LIBRARY/Cargo.toml" || status=1
  return "$status"
}
# Every cargo build replaces the binary in its target folder, even one with nothing to rebuild, so a second
# gate running in this worktree would replace the binary this one is starting, which is then killed or not
# found. So this run builds xtask once into a folder of its own (tools/env.sh), where nothing else writes;
# each later stage starts that same file.
xtask_run="$(mktemp -d "${TMPDIR:-/tmp}/craftbridge-xtask.XXXXXX")"
craftbridge_remove_on_exit "$xtask_run"
build_xtask() {
  [ -x "$xtask_run/debug/xtask" ] || craftbridge_xtask_build "$xtask_run"
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
  craftbridge_cargo "$@" | "$xtask_run/debug/xtask" compiled
}
# The lint library's run over the workspace, `lint_library <cargo check arguments>`. `cargo dylint` builds
# the library with a plain `cargo build`, which would rewrite its Cargo.lock when the manifest has moved,
# so the lock is read `--locked` first, as every other build here reads its own. The run goes through
# `cargo xtask dylint` rather than `compiling`: that command holds the one file lock every run that builds
# the lint library takes turns on (rust/xtask/src/lints.rs), so a workspace test running the library's
# probe tests into this same build folder beside a gate never replaces what the gate is reading, nor the
# other way about. The command starts its own cargo in rust/ and passes cargo's messages through whole,
# so `xtask compiled` sees exactly what `compiling` handed it. Of the two things the guarded cargo adds, the
# command runs in rust/ itself, and a Cargo configuration at the repository root is refused by the guarded
# `cargo metadata` this function runs first, whichever stages the gate was given, so none reaches the run.
lint_library() {
  craftbridge_cargo metadata --manifest-path "$CRAFTBRIDGE_LINT_LIBRARY/Cargo.toml" --locked --format-version 1 >/dev/null || return 1
  build_xtask || return 1
  "$xtask_run/debug/xtask" dylint --path "$CRAFTBRIDGE_LINT_LIBRARY" -- "$@" | "$xtask_run/debug/xtask" compiled
}
# --all-features: code behind a feature is code that ships to whoever turns the feature on. What ships is
# linted with every never-crash lint; the tests' build takes back the two that name the panicking calls
# by path, as rust/clippy.toml's `allow-*-in-tests` take back the lints they shadow.
# The lint library's run refuses the `[]` only the type checker can name — on a map, a `VecDeque`, a type
# with an `Index` of its own — which no clippy lint reads and `disallowed-methods` cannot name as an
# operator: the gate's lint library (rust/never-crash-lints) adds it as a forbid lint. `cargo dylint` builds
# the library with the nightly the library's own rust-toolchain.toml pins and checks the same shipping
# targets clippy's first run lints with that compiler; the workspace's own toolchain and flags are
# untouched. Tests are outside, as clippy's `allow-*-in-tests` keep their indexing: this run compiles no
# test target. The library is gate machinery written in Rust, so it is held as the workspace is: the same
# two clippy runs over its targets, under the workspace's table its manifest carries (AGENTS.md §3), and
# its own lint over what it ships.
# The shipping targets of a selection of packages, which clippy's first run and the lint library's run read:
# `"${shipping[@]}"`.
shipping=(--lib --bins --examples --all-features --locked --message-format=json)
# The levels the tests are linted at, on the host and on every phone target: `"${tests_lints[@]}"`.
tests_lints=(-D warnings -A clippy::disallowed_methods -A clippy::disallowed_macros)
# Clippy's two runs over a selection of packages through one cargo runner: `clippy_runs compiling
# --workspace` for the workspace, `clippy_runs in_library --workspace` for the lint library, and
# `clippy_runs compiling "${host_tools[@]}"` for the host tools.
clippy_runs() {
  local run="$1" status=0
  shift
  "$run" clippy "$@" "${shipping[@]}" -- -D warnings || status=1
  "$run" clippy "$@" --tests --benches --all-features --locked --message-format=json -- "${tests_lints[@]}" || status=1
  return "$status"
}
stage_rust_clippy() {
  local status=0
  clippy_runs compiling --workspace || status=1
  lint_library --workspace "${shipping[@]}" || status=1
  clippy_runs in_library --workspace || status=1
  lint_library --manifest-path "$CRAFTBRIDGE_LINT_LIBRARY/Cargo.toml" --workspace "${shipping[@]}" || status=1
  return "$status"
}
stage_rust_test() { compiling test --workspace --locked --message-format=json; }
# The host tools whose code reads the host they run on: craftbridge-process (the process group's liveness,
# read from `/proc` on Linux and from `pgrep` elsewhere), the conformance runner and xtask. Their clippy and
# lint library runs and their tests are the part of `rust_clippy` and `rust_test` that holds on Linux too,
# where nothing the workspace ships is built, so the hosted Linux job lints and tests the code only Linux
# compiles (`cfg(target_os = "linux")`); the macOS job takes them through the two whole-workspace stages.
host_tools=(--package craftbridge-process --package craftbridge-effects-conformance --package xtask)
stage_rust_host_clippy() {
  local status=0
  clippy_runs compiling "${host_tools[@]}" || status=1
  lint_library "${host_tools[@]}" "${shipping[@]}" || status=1
  return "$status"
}
stage_rust_host_test() { compiling test --locked --message-format=json "${host_tools[@]}"; }
# The desktop conformance corpus against the EffectCraft command line (plan §28). Its own stage, after the
# Rust tests: it needs the runner built and the tests of the code that compares green first, and the oracle
# is an EffectCraft build of its own (tools/oracle.sh: minutes cold, a second warm), which the Rust stages
# have no use for, so `tools/ci.sh conformance` runs it alone. The runner starts from the run's own folder,
# as xtask does: another gate's workspace-wide test build rebuilds its binary in the shared folder while
# this one could be starting it.
stage_conformance() {
  local oracle
  oracle="$(tools/oracle.sh)" || return 1
  CARGO_BUILD_BUILD_DIR="$CARGO_TARGET_DIR" compiling build --package craftbridge-effects-conformance \
    --locked --message-format=json --target-dir "$xtask_run" || return 1
  "$xtask_run/debug/craftbridge-effects-conformance" --oracle "$oracle"
}
stage_rust_mobile() {
  local target status=0
  # Every target is checked even after one fails, so one run names each target that is broken. Each runs in
  # a subshell of its own, so what tools/env.sh sets for a phone target reaches none of the stages after
  # this one, the doctor among them: an Android target's NDK environment would reach their host builds,
  # and the flags it sets for a device's code (Android's aarch64 target feature, the iOS device's CPU
  # target) are variables the gate refuses in the environment it starts in.
  for target in $CRAFTBRIDGE_RUST_TARGETS; do
    (rust_mobile_target "$target") || status=1
  done
  return "$status"
}
# One phone target's checks, built as the phone's own build is, from tools/env.sh, the one place every
# build and device run for a phone takes its settings from. For an Android target that is the NDK, its
# compilers and the aarch64 target feature, so a dependency's build script that compiles C for the phone
# finds the phone's compiler, and the lint reads the code the phone is built with; for the iOS device it
# is the CPU target.
rust_mobile_target() {
  local target="$1" profile status=0
  if craftbridge_is_android_target "$target"; then
    craftbridge_android_env && craftbridge_android_target_env "$target" || return 1
  fi
  craftbridge_ios_target_env "$target"
  # The default members are the libraries an app links; the host tools never run on a phone. Clippy, not
  # a bare check: the platform modules compile only for the phones, so the never-crash lints and the
  # SAFETY comments are only checked on them here, in the release profile apps link as well as the dev
  # one, since code under `cfg(not(debug_assertions))` exists only in the first; the lint library's run
  # follows each clippy run for the same reason. Their tests are linted on each target too, at the levels
  # the host's are, in the dev profile they build in: a test that compiles only for a phone (the font tests
  # the simulator and the emulator run) is read by no other run.
  for profile in dev release; do
    compiling clippy --locked --all-features --target "$target" --profile "$profile" --message-format=json -- -D warnings || status=1
    lint_library --locked --all-features --target "$target" --profile "$profile" --message-format=json || status=1
  done
  compiling clippy --locked --all-features --tests --target "$target" --message-format=json -- "${tests_lints[@]}" || status=1
  return "$status"
}
# The lint library's dependencies are built and run wherever the gate runs, outside the workspace's graph,
# so they meet the same allowlist, bans and sources through its own manifest.
stage_rust_deny() {
  local status=0
  craftbridge_cargo deny --locked check licenses bans sources || status=1
  craftbridge_cargo deny --manifest-path "$CRAFTBRIDGE_LINT_LIBRARY/Cargo.toml" --locked check licenses bans sources || status=1
  return "$status"
}
# The Swift package's binary target is the XCFramework tools/build-xcframework.sh builds and nobody
# commits. SwiftPM neither builds nor describes a package whose binary is missing, and xcodebuild resolves
# a project's local packages before it answers anything about the project, so the four stages that need it
# — the guide check, which asks xcodebuild what each sample's build compiles its Swift under, the builds,
# the lint, which builds the package for the language server, and the suite's run — build it first, once
# per run; the script joins it again only when what it holds changed. The rules read the package through
# its manifest alone, which needs no binary. --keep-bindings: the `generated` stage must see a hand edit
# to the committed Swift bindings, which a regeneration here would write over before that stage looked.
xcframework_built=no
xcframework() {
  if [ "$xcframework_built" = no ]; then
    tools/build-xcframework.sh --keep-bindings >&2 || return 1
    xcframework_built=yes
  fi
}
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
# Every Swift and Kotlin block the guides show must be a run of lines of a file the samples' builds compile,
# as the samples' one definition lists them (rust/xtask/src/guide_samples.rs, rust/xtask/src/samples.rs),
# and never a run lying wholly on lines that compile to nothing: which lines those are is each language's
# own parser's answer — detekt's `CompilesToNothing`, run by the sample build itself over the folders its
# variants compile, for Kotlin, and tools/guide-swift-lines.swift, reading the Swift with the pinned Xcode's
# swift-syntax on every build swift_build makes, under the conditions Xcode resolves for each, for Swift — so
# a block quoted from inside a comment, a KDoc, a string or a branch no shipping build compiles is refused. The compiling of those
# files is the swift_build and android_samples stages' own job.
stage_guide_samples() {
  # The Swift side asks xcodebuild what each sample's build compiles its Swift under, and xcodebuild
  # resolves the sample's local package first, whose binary target is the XCFramework. Like the Swift
  # stages this check reports on, the guide check therefore takes that framework from the shared step
  # before it asks, so it answers on any tree whichever stages ran on it before.
  xcframework && xtask guide-samples
}
# swift-format over every Swift file the rules' walk lists, generated Swift included: it ships as written
# Swift does, so it meets the same rules, bar those UniFFI's fixed templates cannot meet, which the folder
# its bindings are generated into alone is exempt from (rust/xtask/src/swift_format.rs).
stage_swift_format() { xtask swift-format; }
stage_swift_lint() {
  # SwiftLint's missing_docs over ios/sdk/.swiftlint.yml, and the Swift never-crash bans, the pointer
  # confinement and the tool directives (AGENTS.md §3, §4, §10) read through the compiler's language
  # server with SwiftPM's settings for every build the package ships — Debug and Release for each Apple
  # slice rust/rust-toolchain.toml lists (rust/xtask/src/swift_lint.rs) — so each `#if` branch a shipping
  # build compiles is read resolved; UniFFI's generated bindings alone are exempt from the rules their fixed
  # templates cannot meet, named there. The same resolved package answers what the rules cannot ask without
  # the binary built: where each target sits and what it compiles. After the build: a file that does not
  # compile is cheaper to name through the compiler than through the analyser.
  xcframework && xtask swift-lint
}
stage_swift_build() {
  # Every build the Swift package ships, in the list `cargo xtask swift-builds` prints from the same table
  # the Swift checks read (rust/xtask/src/pins.rs): each configuration of each slice the toolchain file
  # names, once for slices that share a destination, so a slice added there is built with no other edit,
  # and there is no second list of slices to keep in step. A branch that compiles only in one
  # configuration or only off the simulator — `#if !DEBUG`, `#if !targetEnvironment(simulator)` —
  # compiles in one of these builds. Each is built for the architectures of the slices the toolchain
  # names for its destination (ARCHS), not every architecture a generic destination offers: the native
  # library holds the slices it is built for, and a test target links it.
  # DIAGNOSE_MISSING_TARGET_DEPENDENCIES=YES_ERROR makes an import the package does not declare an
  # error. Without it a module an earlier build left in the derived data satisfies the import with only
  # a warning, so `CraftBridge` could import a plugin against AGENTS.md §2 and still build.
  # build-for-testing builds the test targets too, which the package scheme builds only for testing.
  # Besides this build of the whole package, every build line below also builds each library product
  # again the way an app extension links it, the products listed from the manifest by
  # `cargo xtask swift-products`; why, in the comment on that build.
  # Each build also writes the compiler's symbol graphs of every module into a folder of its own, which
  # `cargo xtask swift-api` reads once all have built: no public member written by hand may take the name
  # of one generated Swift gives the same type (rust/xtask/src/swift_api.rs). The folders sit inside the
  # derived data and keep their place from run to run: a module that is up to date compiles nothing and
  # writes no graph, so its last graph, written from the sources it still has, is the one read, and a
  # clean build folder loses both together.
  local builds configuration destination architectures status=0 build=0 folder graphs=() library libraries=() project projects=()
  # The sample apps, each an Xcode project of its own that takes the package by local path, as an app
  # does, listed by `cargo xtask swift-samples` (rust/xtask/src/samples.rs), the one list of them, whose
  # pins the licence check reads.
  items_of "sample projects to build" swift-samples || return 1
  projects=("${items[@]}")
  # First, before anything is built: every xcodebuild below asks through a `-scheme <name>`, so a scheme or
  # test plan git does not track — one Xcode left in the package's ignored state folder, a workspace left
  # beside its manifest, or a user's or shared scheme edited into a sample's project bundle — or a mirror or
  # registry left in the state folder's own configuration, or a mirror the machine's own configuration or
  # the shell names, which the resolve of every build reads and fetches a dependency's sources through, would
  # be what they build, rather than what the tree gives (tools/env.sh, which reads the package's places and
  # the mirrors' itself and is given the samples').
  craftbridge_refuse_leftover_xcode_and_swiftpm_settings "${projects[@]}" || return 1
  xcframework || return 1
  # Each sample bundles the running example's template from the `Template` folder beside its project,
  # which git ignores and the sample's build refuses to start without: it is made here from its recipe by
  # the EffectCraft command line at the pin (tools/birthday-card.sh), so no sample holds a template made
  # another way.
  for project in "${projects[@]}"; do
    tools/birthday-card.sh "$(dirname "$project")/Template" >&2 || return 1
  done
  # The library products, listed from the manifest through `cargo xtask swift-products` (rust/xtask/src/
  # pins.rs), so a product added to the package joins the extension builds below with no second list here
  # to keep in step.
  items_of "Swift library products to build" swift-products || return 1
  libraries=("${items[@]}")
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
  while IFS=$'\t' read -r configuration destination architectures; do
    build=$((build + 1))
    folder="$PWD/ios/sdk/.build/xcode/symbol-graphs/$build"
    graphs+=("$folder")
    mkdir -p "$folder" || return 1
    (cd ios/sdk && xcodebuild -scheme CraftBridge-Package -destination "$destination" -configuration "$configuration" -derivedDataPath .build/xcode -quiet build build-for-testing \
      DIAGNOSE_MISSING_TARGET_DEPENDENCIES=YES_ERROR ARCHS="$architectures" \
      OTHER_SWIFT_FLAGS="\$(inherited) -emit-symbol-graph -emit-symbol-graph-dir \"$folder\"") || status=1
    # And each library product again, the way an app's own extension links it: with the extension-only
    # API restriction, so an API the system takes from extensions — `UIApplication.shared` among them —
    # named anywhere the extension would link fails here, in the file and line that name it, rather than
    # at the app's first extension build (plan §28 runs the engine in one). Warnings are errors in these
    # builds: a key path to such an API, `#keyPath(UIApplication.shared)`, is graded only a warning, and a
    # warning no stage fails on is a check that never ran. A product's build compiles the targets it
    # depends on with the same settings, so the generated bindings are read as an extension links them too.
    # Only library products are built this way: an extension links the libraries, never a package's test
    # targets, and a test may call what an extension may not. These builds take derived data of their own:
    # their settings differ from a shipping build's, so in its folder they would keep rebuilding the very
    # products the later stages reuse. It is a folder per build line, as the symbol graphs are, because in
    # one folder all the lines share, every warm run does work again, several times what it takes in
    # folders of their own. The folders are not locked, here or for `.build/xcode`, which `swift_test` builds
    # in too, so a lock this stage took alone would still let it meet that stage there: two runs in one
    # worktree meet on xcodebuild's build database and fail ("database is locked"), never passing falsely.
    # Runs in different worktrees never meet, each holding its own folders.
    for library in "${libraries[@]}"; do
      (cd ios/sdk && xcodebuild -scheme "$library" -destination "$destination" -configuration "$configuration" -derivedDataPath ".build/xcode-extension/$build" -quiet build \
        DIAGNOSE_MISSING_TARGET_DEPENDENCIES=YES_ERROR ARCHS="$architectures" \
        APPLICATION_EXTENSION_API_ONLY=YES SWIFT_TREAT_WARNINGS_AS_ERRORS=YES) || status=1
    done
    # And every sample app, in the same configuration for the same destination, built as an app is: signing
    # is off, since the gate has no profile and a build that needs none still compiles every file, links
    # the package and bundles the template. Its packages resolve only from the `Package.resolved` the project
    # commits (-disableAutomaticPackageResolution): resolved here, a published package the sample takes would
    # be written into a new one after the licence check had read the tree, and ship with its licence, and
    # those of the packages it takes, unread. Their derived data sits under the package's `.build`, which
    # every rule and walk skips as build output, so no sample's intermediate Swift is read as source.
    for project in "${projects[@]}"; do
      xcodebuild -project "$project" -scheme "$(basename "$project" .xcodeproj)" -destination "$destination" -configuration "$configuration" \
        -derivedDataPath "ios/sdk/.build/xcode-samples/$build" -disableAutomaticPackageResolution -quiet build \
        DIAGNOSE_MISSING_TARGET_DEPENDENCIES=YES_ERROR ARCHS="$architectures" CODE_SIGNING_ALLOWED=NO || status=1
    done
  done <"$builds"
  rm -f "$builds"
  # A build that failed wrote the graphs of only what it compiled, so they are read only once every build
  # has built.
  if [ "$status" -eq 0 ]; then
    xtask swift-api "${graphs[@]}" || status=1
  fi
  return "$status"
}
# The Swift Testing suite, run on the iOS Simulator. `swift_build` compiles the suite's targets and runs
# none of them, so until here no gate ever ran a Swift test: a test no gate runs is a test that drifts. plan
# §28 puts the suite — the Swift unit tests and the corpus read through the Swift API — in the gate, on the
# iPhone Simulator. The destination is the table's own (`cargo xtask swift-suite`, rust/xtask/src/pins.rs),
# the one list of slices, with one of this machine's own simulators named in it, which `cargo xtask
# simulator` chooses and shares one at a time with every other run on the machine
# (rust/xtask/src/simulator.rs): the stage creates no device, boots an iPhone simulator only when none it
# may take is running, waits its turn when another run is testing on the simulator it chose, and takes
# none that a run started by hand is seen using, since that run shuts down when it ends a simulator it
# booted — another available iPhone of its runtime is booted in its place, or the stage waits for that run
# to end — and says which it did, and why; it shuts down only one whose boot began under a run of it, never
# one somebody else had booted, once nothing uses it; and its
# test runner takes a sandbox of its own, so a run of the suite started by hand on the same simulator
# never meets this one's files. The run starts with no `TEST_RUNNER_` or `SIMCTL_CHILD_` variable of the
# environment the stage was called in and only those the stage names for it, so what a developer's shell
# happens to export cannot decide which tests the suite runs; the stage refuses to start at all while the
# package's ignored state folder holds a scheme or test plan an Xcode session left there, a workspace
# stands beside its manifest, its configuration folder holds a mirror or registry, or the machine's SwiftPM
# configuration or the shell names a mirror (tools/env.sh).
# The suite runs in the one configuration that command names, the one a developer and `swift test` run it
# in. The Simulator is macOS's own, so the hosted macOS job runs this stage on the runner's simulator.
stage_swift_test() {
  local suites configuration destination architectures status=0
  # First, before anything is built and before any simulator is taken: the runs below are asked through
  # `-scheme CraftBridge-Package`, so a scheme or test plan Xcode left in the package's ignored state
  # folder, a workspace left beside its manifest, or a mirror or registry left in the state folder's own
  # configuration, or a mirror the machine's own configuration or the shell names, which the resolve of every
  # build reads and fetches a dependency's sources through, would be what they run, rather than what the
  # manifest gives (tools/env.sh, which reads the package's places and the mirrors' itself).
  craftbridge_refuse_leftover_xcode_and_swiftpm_settings || return 1
  xcframework || return 1
  suites="$(mktemp)"
  if ! xtask swift-suite >"$suites"; then
    rm -f "$suites"
    return 1
  fi
  if [ ! -s "$suites" ]; then
    echo "ci: no Swift suite to run" >&2
    rm -f "$suites"
    return 1
  fi
  while IFS=$'\t' read -r configuration destination architectures; do
    swift_suite_on "$configuration" "$destination" "$architectures" || status=1
  done <"$suites"
  rm -f "$suites"
  return "$status"
}
# One run of the suite: its build, its run on a simulator, and what its own report says ran.
swift_suite_on() {
  local configuration="$1" destination="$2" architectures="$3"
  local package run_folder result summary sandbox tests skipped status=0
  # The architectures stay the toolchain's, so nothing is built for a slice the native library does not
  # hold.
  package=(-scheme CraftBridge-Package -configuration "$configuration" -derivedDataPath .build/xcode ARCHS="$architectures")
  # Built first, for the table's destination as it stands, which names no device, so that a simulator is
  # taken only once the tests are ready to run in it: a simulator held while it ran nothing would look
  # unused to another run deciding whether to shut it down. The run is of the very products this build made.
  (cd ios/sdk && xcodebuild "${package[@]}" -destination "$destination" build-for-testing) || return 1
  # The run's own folder for its report: a result bundle path that already exists is refused by xcodebuild,
  # and one that a previous run left would be read as this run's.
  run_folder="$(mktemp -d)"
  result="$run_folder/suite.xcresult"
  summary="$run_folder/summary.json"
  # The run's own sandbox. The suite's tests start engines in the app's sandbox (`Folders.appDefault`),
  # whose export journal one process holds at a time, and a Swift package's tests on the simulator have no
  # app container: their sandbox is the simulator's own data folder, which every run on that simulator
  # shares, so a run started by hand beside this one would fail on this one's journal, or this one on its.
  # The test runner takes its home and its temporary folder in this run's folder instead, which goes with
  # the run and leaves nothing on the simulator: Foundation's home on the simulator is `CFFIXED_USER_HOME`,
  # EffectCraft resolves an unsaved project's relative output against `HOME`, and the temporary folder sits
  # inside the home, as an app's does, since the engine reads only inside its container. xcodebuild hands
  # each `TEST_RUNNER_` variable to the test runner without the prefix.
  sandbox="$run_folder/sandbox"
  # `generic/platform=iOS Simulator` names a whole platform, which a build builds for and no run runs on: a
  # test run names one device, so `generic/` — any device of this platform — is replaced by the simulator
  # `cargo xtask simulator` puts where the argument says, and the destination stays the table's.
  (mkdir -p "$sandbox/tmp" && cd ios/sdk &&
    # The run takes the environment its tests see from the call below alone: what the calling shell
    # exports and leaves standing — `TEST_RUNNER_CRAFTBRIDGE_BASELINE=1`, or the same with
    # `SIMCTL_CHILD_`, which the boot of the run's simulator hands on, after a baseline measurement
    # asked for by hand — would reach the test that reads it, running the measurement beside every
    # other suite, and the gate's result would depend on the developer's shell rather than on the
    # tree. The sandbox assignments are the run's own, named in that call, and stay.
    craftbridge_set_test_runner_env "TEST_RUNNER_HOME=$sandbox" "TEST_RUNNER_CFFIXED_USER_HOME=$sandbox" "TEST_RUNNER_TMPDIR=$sandbox/tmp/" &&
    xtask simulator -- xcodebuild "${package[@]}" -destination "${destination#generic/},id={simulator}" \
      -resultBundlePath "$result" test-without-building) || status=1
  # How many tests ran is read from the report the tool that ran them wrote, through that tool: a run that
  # collected no test, or skipped every one, exits 0 and checked nothing, so the suite's own count of the
  # tests it ran, which are those it did not skip, is what says it ran at all.
  tests=''
  skipped=''
  if xcrun xcresulttool get test-results summary --path "$result" >"$summary"; then
    tests="$(plutil -extract totalTestCount raw -o - "$summary")" || tests=''
    skipped="$(plutil -extract skippedTests raw -o - "$summary")" || skipped=''
  fi
  if [ -z "$tests" ] || [ -z "$skipped" ]; then
    echo "ci: the Swift suite ran no test: its own report names no count of tests" >&2
    status=1
  elif [ "$((tests - skipped))" -le 0 ]; then
    echo "ci: the Swift suite ran no test: its own report counts $tests, and skipped $skipped" >&2
    status=1
  else
    echo "ci: $((tests - skipped)) tests ran on the simulator, $skipped skipped"
  fi
  rm -rf "$run_folder"
  return "$status"
}
# One run of Gradle, through android/sdk's wrapper, on one build of the repository, its log kept for
# `cargo xtask skipped-tasks`: `android_gradle <log> <build folder> <task>…`, the folder from android/sdk. One
# wrapper (one Gradle version, §5) runs every build, the SDK's and each sample's.
android_gradle() {
  local log="$1" project="$2"
  shift 2
  (cd android/sdk && ./gradlew -p "$project" --no-daemon --continue --warning-mode=fail --console=plain "$@") | tee -a "$log"
}
# What every Android build's modules are checked with; the SDK's also compiles its instrumented checks, and a
# sample builds no instrumented-test APK.
android_checks=(lint lintRelease assembleRelease compileDebugUnitTestSources testDebugUnitTest)
stage_android() {
  # A fresh JVM every run: ktlint keeps the .editorconfig files it has parsed for the life of the
  # Gradle daemon, so a warm daemon would go on checking against rules that have since changed.
  # --continue runs every task whose inputs are still good after one fails, and the second build below
  # runs whatever the first found, so a formatting error does not hide a Lint or compile error until the
  # next run.
  # The tests' Kotlin compiles too, under the same warnings-as-errors rules as the code it tests, and the
  # unit tests run, on the TestNG craftbridge.android.library gives every module; a library variant of
  # AGP 9 builds unit tests for the debug build type alone. Every test task carries its own verifyTestCounts
  # finalizer, which names any test the build compiled that TestNG did not run and prints the counts of
  # written and run tests per module (android/sdk/build-logic/shared).
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
  android_gradle "$log" . spotlessCheck || status=1
  # detekt runs in a build of its own too, and its command line in a JVM of its own (plan D-28): its
  # analysis starts the Kotlin compiler's IntelliJ core, which sets IntelliJ's JVM-wide settings as
  # ktlint does, so it never shares a JVM with Lint. detektMain and detektTest are what
  # craftbridge.android.library registers over every variant and every test component of a module, and
  # the build's rule jar carries its own tests, run here beside the rules they check.
  android_gradle "$log" . detektMain detektTest :build-logic:detekt-rules:test || status=1
  android_gradle "$log" . "${android_checks[@]}" compileDebugAndroidTestSources || status=1
  xtask skipped-tasks "$log" || status=1
  rm -f "$log"
  return "$status"
}
# The Android samples' own builds, each a Gradle build that takes the SDK as an included build and the SDK's
# convention plugins and detekt rules from build-logic, so a sample's sources are held to the rules of the SDK's
# modules by the same code, in the same three runs as the SDK's and for the same reasons (stage_android). They are
# listed by `cargo xtask android-samples` (rust/xtask/src/samples.rs), the one list the licence check and the
# build-file rules read too. Each bundles the running example's template, which its build makes with
# tools/birthday-card.sh, and so with the EffectCraft command line at the pin (tools/oracle.sh): a stage of its
# own, so the hosted job that builds that command line for the conformance stage builds the samples with it too.
stage_android_samples() {
  local log status=0 build builds
  items_of "Android sample builds" android-samples || return 1
  builds=("${items[@]}")
  log="$(mktemp)"
  for build in "${builds[@]}"; do
    android_gradle "$log" "$CRAFTBRIDGE_ROOT/$build" spotlessCheck || status=1
    android_gradle "$log" "$CRAFTBRIDGE_ROOT/$build" detektMain detektTest || status=1
    android_gradle "$log" "$CRAFTBRIDGE_ROOT/$build" "${android_checks[@]}" || status=1
  done
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
stages=(doctor rules rust_fmt rust_clippy rust_host_clippy rust_test rust_host_test conformance rust_mobile rust_deny generated plan_views guide_samples swift_format swift_build swift_test swift_lint android android_samples shell duplicates)
# The stage groups the decision and plan §15 name: Rust and Swift are each several stages.
group_rust=(rust_fmt rust_clippy rust_test rust_mobile rust_deny)
group_swift=(swift_format swift_build swift_test swift_lint)
# The hosted jobs' split of the stages (.github/workflows/ci.yml), made here and nowhere else: each job runs
# `tools/ci.sh --hosted`, which takes its stages from its runner's system. The Linux job runs `linux_only`,
# the stages that read the same on Linux and need nothing of macOS; the macOS job runs every other stage, so
# every stage runs in at least one job: a stage added above runs there, and says below why it is left to
# macOS, or joins the Linux job.
linux_only=(rust_fmt rust_host_clippy rust_host_test rust_deny plan_views android duplicates)
# Why each stage the Linux job does not run is left to macOS: `<stage>…: <reason>`.
left_to_macos=(
  "doctor: it checks Xcode, the NDK and the phone targets"
  "rules: they ask SwiftPM through xcrun"
  "generated: it formats the Swift bindings with the Xcode toolchain"
  "rust_clippy: the whole workspace's lints, whose shipping crates are built for iOS and Android and tested on macOS alone: built for Linux, they hold code only those systems call (craftbridge-media's encoder tests, craftbridge-core-ffi's surface), which -D warnings refuses as dead; its host tools' part runs on Linux as rust_host_clippy"
  "rust_test: the whole workspace's tests, among them the core's that draw on the host's GPU (rust/core/craftbridge-core/src/testing.rs), which a hosted Linux runner lacks; its host tools' part runs on Linux as rust_host_test"
  "conformance: its steps that ask for the GPU fail where the machine gives none (rust/effects/craftbridge-effects-conformance/src/host.rs), and the oracle they are compared with is an EffectCraft build of its own (tools/oracle.sh)"
  "rust_mobile: the iOS targets need Xcode's SDKs, and the Android targets the NDK r28 tools/env.sh pins, which the macOS image carries and Ubuntu's does not"
  "swift_format swift_build swift_lint: Xcode and SwiftPM"
  "guide_samples: it reads which files each iOS sample's project compiles, and under what conditions, through plutil, xcodebuild and the Xcode toolchain's own swift-syntax, macOS's own readers of the project, its settings and the language"
  "swift_test: the suite runs on the iOS Simulator, which is macOS's own, so the hosted macOS job runs it on the runner's simulator"
  "android_samples: each sample bundles the running example's template, which the EffectCraft command line at the pin makes (tools/birthday-card.sh): the macOS job builds that command line once, for conformance, and the Linux job would build it a second time, cold"
  "shell: Homebrew's shellcheck is the release the doctor's fix installs, while Ubuntu's is an older one whose checks differ"
)
# Is `name` one of the items after it: `among <name> <item>…`.
among() {
  local name="$1" item
  shift
  for item in "$@"; do
    if [ "$item" = "$name" ]; then return 0; fi
  done
  return 1
}
# The split is held before any name is read, whatever the selection, so no change loses a stage from both jobs
# or leaves one unexplained: every stage function is a stage `stages` lists, which the macOS job reads; and
# every stage the Linux job does not run has one reason in `left_to_macos`, which gives none for a stage the
# Linux job runs or for no stage at all.
split=()
for name in $(compgen -A function stage_); do
  among "${name#stage_}" "${stages[@]}" || split+=("$name is a stage \`stages\` does not list, so no hosted job runs it")
done
reasoned=()
for entry in "${left_to_macos[@]}"; do
  read -r -a names <<<"${entry%%: *}"
  if [ "${entry#*: }" = "$entry" ] || [ -z "${entry#*: }" ] || [ ${#names[@]} -eq 0 ]; then
    split+=("left_to_macos holds '$entry', which is no '<stage>…: <reason>'")
    continue
  fi
  for name in "${names[@]}"; do
    if ! among "$name" "${stages[@]}"; then
      split+=("left_to_macos gives a reason for $name, which is no stage")
    elif among "$name" "${linux_only[@]}"; then
      split+=("left_to_macos gives a reason for $name, which the Linux job runs")
    elif among "$name" ${reasoned[@]+"${reasoned[@]}"}; then
      split+=("left_to_macos gives $name a second reason")
    fi
    reasoned+=("$name")
  done
done
for name in "${stages[@]}"; do
  among "$name" "${linux_only[@]}" ${reasoned[@]+"${reasoned[@]}"} || split+=("$name runs in no Linux job, and left_to_macos gives no reason it is left to macOS")
done
if [ ${#split[@]} -gt 0 ]; then
  printf 'ci: %s\n' "${split[@]}" >&2
  exit 2
fi
list=0
if [ "${1:-}" = --list ]; then
  list=1
  shift
fi
# A hosted job names no stage, so the workflow holds no list or group to get wrong: its runner's system,
# which GitHub sets in RUNNER_OS, chooses them.
if [ "${1:-}" = --hosted ]; then
  if [ "$#" -gt 1 ]; then
    echo "ci: --hosted names no stage: RUNNER_OS, the runner's system, chooses the hosted job's stages" >&2
    exit 2
  fi
  case "${RUNNER_OS:-}" in
    Linux) set -- "${linux_only[@]}" ;;
    macOS) set -- --except "${linux_only[@]}" ;;
    *)
      echo "ci: --hosted runs the hosted job of the runner's system, which RUNNER_OS names Linux or macOS; it names '${RUNNER_OS:-}'" >&2
      exit 2
      ;;
  esac
fi
except=0
if [ "${1:-}" = --except ]; then
  except=1
  shift
  if [ "$#" -eq 0 ]; then
    echo "ci: --except names no stage to leave out: name them, or run tools/ci.sh alone for every stage" >&2
    exit 2
  fi
fi
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
if [ "$except" = 1 ]; then
  kept=()
  for name in "${stages[@]}"; do
    among "$name" "${selected[@]}" || kept+=("$name")
  done
  if [ ${#kept[@]} -eq 0 ]; then
    echo "ci: --except leaves no stage to run" >&2
    exit 2
  fi
  selected=("${kept[@]}")
elif [ ${#selected[@]} -eq 0 ]; then
  selected=("${stages[@]}")
fi
# What a selection runs, asked without running it: a test reads each hosted job's stages here, where they are
# chosen.
if [ "$list" = 1 ]; then
  echo "${selected[*]}"
  craftbridge_passed=1
  exit 0
fi

# The gate judges the change by one compiler set: the toolchain rust/rust-toolchain.toml pins, on stable
# rules, with nothing in the environment switching what its tools check. rustup resolves rust/ to that pin
# unless another toolchain stands over it — an environment variable, a rustup override, or a `+toolchain`
# argument to a cargo or rustup the gate runs under, which reaches it as the variable — and each of those
# reaches every cargo call the gate makes. The one variable naming the pin itself, which rustup hands to
# every command it starts, is that toolchain and no override of it; tools/env.sh tells the two apart. The
# pins themselves come from `cargo xtask pins`, the one reader of both pin files, run as every other xtask
# command here is: built, then its binary run. The gate's lint library builds and is linted with the nightly
# its own folder pins, which rustup reads there, so that folder is held to its pin the same way.
pinned_toolchain() {
  local dir="${1#"$CRAFTBRIDGE_ROOT"/}" status
  status="$(craftbridge_toolchain_status_in "$1")"
  case "$status" in
    ok\ *) ;;
    no-rustup) echo "ci: no rustup on PATH: install it from https://rustup.rs" >&2; exit 1 ;;
    missing) echo "ci: the toolchain pinned in $dir/rust-toolchain.toml is not installed: cd $dir && rustup toolchain install" >&2; exit 1 ;;
    override\ *) echo "ci: $dir/ resolves an overridden toolchain, not the pin in $dir/rust-toolchain.toml: ${status#override }" >&2; exit 1 ;;
  esac
}
pinned_toolchain "$CRAFTBRIDGE_ROOT/rust"
pinned_toolchain "$CRAFTBRIDGE_LINT_LIBRARY"
craftbridge_refuse_switches || exit 1
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
  craftbridge_passed=1
else
  echo "ci: failed: ${failed[*]}"
  exit 1
fi
