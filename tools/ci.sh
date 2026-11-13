#!/usr/bin/env bash
# The gate: every check a change must pass before it is pushed or merged (AGENTS.md).
#
#   tools/ci.sh              every stage
#   tools/ci.sh rules swift  only the named stages (names below)
#
# Every stage runs even when an earlier one fails, so one run reports everything that is broken.
set -euo pipefail
# shellcheck source=tools/env.sh
source "$(dirname "$0")/env.sh"
cd "$CRAFTBRIDGE_ROOT"

# rustup selects the pinned toolchain from rust/rust-toolchain.toml only inside rust/.
cargo_in_rust() { (cd rust && cargo "$@"); }

# The folders that hold the repository's own code and scripts, those that exist so far. The shell and
# duplicate stages read the same ones, so no script is shellchecked without being compared, or the
# reverse. plan/ is out: it holds the plan of record and the kit that builds it, which the scoping-plan
# installer writes rather than a person, and `plan/plan check --final` checks the plan instead.
code_dirs=()
for dir in rust ios android conformance tools docs .github; do
  if [ -d "$dir" ]; then code_dirs+=("$dir"); fi
done

# Sets the caller's `outputs` to env.sh's build-output paths, one per element: `read` splits them
# without expanding their globs.
build_outputs() { read -r -a outputs <<<"$CRAFTBRIDGE_BUILD_OUTPUTS"; }

# The hand-written files matching a -name pattern under the given folders, sorted. Build output is pruned
# rather than filtered, so a large target/ is not walked; `find -path` lets `*` cross `/`, so a `**` in
# a build-output path matches any depth.
written_files() {
  local pattern="$1" output outputs prune=()
  shift
  build_outputs
  for output in "${outputs[@]}"; do
    if [ ${#prune[@]} -gt 0 ]; then prune+=(-o); fi
    prune+=(-path "$output")
  done
  find "$@" -type d \( "${prune[@]}" \) -prune -o -type f -name "$pattern" -print | sort
}

stage_doctor() { tools/doctor.sh --gate; }
stage_rust_fmt() { cargo_in_rust fmt --check; }
stage_rust_clippy() { cargo_in_rust clippy --workspace --all-targets --locked -- -D warnings; }
stage_rust_test() { cargo_in_rust test --workspace --locked; }
stage_rust_mobile() {
  local target status=0
  # The default members are the libraries an app links; the host tools never run on a phone. Every
  # target is checked even after one fails, so one run names each target that is broken.
  for target in $CRAFTBRIDGE_RUST_TARGETS; do cargo_in_rust check --locked --target "$target" || status=1; done
  return "$status"
}
stage_rust_deny() { cargo_in_rust deny --locked check licenses bans sources; }
# The rule checks run through `cargo run`, not the `cargo xtask` alias: rust/.cargo/config.toml can
# redefine an alias, and an alias that runs something else would pass these stages having checked
# nothing. Cargo's built-in `run` cannot be redefined, and the rules then report a bad alias.
xtask() { cargo_in_rust run --package xtask --locked --quiet -- "$@"; }
stage_rules() { xtask rules; }
stage_generated() { xtask generate --check; }
stage_swift_format() {
  local files=()
  # Generated Swift is UniFFI's and the generator's style, not ours; xtask checks it is untouched.
  while IFS= read -r file; do files+=("$file"); done < <(written_files '*.swift' ios)
  xcrun swift-format lint --strict --configuration ios/sdk/.swift-format "${files[@]}"
}
stage_swift_build() {
  # DIAGNOSE_MISSING_TARGET_DEPENDENCIES=YES_ERROR makes an import the package does not declare an
  # error. Without it a module an earlier build left in the derived data satisfies the import with only
  # a warning, so `CraftBridge` could import a plugin against AGENTS.md §2 and still build.
  (cd ios/sdk && xcodebuild -scheme CraftBridge-Package -destination 'generic/platform=iOS Simulator' -derivedDataPath .build/xcode -quiet build \
    DIAGNOSE_MISSING_TARGET_DEPENDENCIES=YES_ERROR)
}
stage_android() {
  # A fresh JVM every run: ktlint keeps the .editorconfig files it has parsed for the life of the
  # Gradle daemon, so a warm daemon would go on checking against rules that have since changed.
  # --continue runs every task whose inputs are still good after one fails, so a formatting error does
  # not hide a Lint or compile error until the next run.
  # A task a module's build file disables, or guards with `onlyIf`, checks nothing and still lets the
  # build succeed; Gradle reports it SKIPPED, whatever the reason or however late it was set, so any
  # SKIPPED task of the SDK's projects fails the stage. The included build-logic is the gate's own
  # machinery, where the Kotlin plugin skips one of its checks by design. --console=plain makes Gradle
  # print every task's outcome even in a terminal.
  local log status=0 skipped
  log="$(mktemp)"
  (cd android/sdk && ./gradlew --no-daemon --continue --warning-mode=fail --console=plain spotlessCheck lint assembleRelease) | tee "$log" || status=1
  skipped="$(grep -E '^> Task :[^ ]+ SKIPPED$' "$log" | grep -v '^> Task :build-logic:' || true)"
  rm -f "$log"
  if [ -n "$skipped" ]; then
    printf 'ci: these tasks were skipped, so they checked nothing:\n%s\n' "$skipped" >&2
    status=1
  fi
  return "$status"
}
stage_shell() {
  local scripts=()
  # Every shell script wherever it lives, and the git hooks, which carry no extension. The Gradle
  # wrapper `gradlew` is out: Gradle's wrapper task writes it, and a fix to it would be lost the next
  # time the wrapper is renewed.
  while IFS= read -r file; do scripts+=("$file"); done < <(written_files '*.sh' "${code_dirs[@]}")
  shellcheck -x "${scripts[@]}" tools/hooks/*
}
stage_duplicates() {
  # The owner's first rule: reuse by extraction, never a second copy. A block of ten lines repeated
  # anywhere fails. jscpd also requires a token count; thirty keeps ten near-empty lines from counting
  # while still catching shell, which has few tokens per line. Generated code repeats by design, so it
  # is ignored with the rest of env.sh's build output, each path as a glob of everything below it.
  local output outputs ignore=""
  build_outputs
  for output in "${outputs[@]}"; do ignore="${ignore:+$ignore,}$output/**"; done
  npx --yes "$CRAFTBRIDGE_JSCPD" --min-lines 10 --min-tokens 30 --format rust,swift,kotlin,bash \
    --ignore "$ignore" --reporters console --exit-code 1 "${code_dirs[@]}"
}

stages=(doctor rust_fmt rust_clippy rust_test rust_mobile rust_deny rules generated swift_format swift_build android shell duplicates)
selected=("$@")
if [ ${#selected[@]} -eq 0 ]; then selected=("${stages[@]}"); fi

failed=()
for name in "${selected[@]}"; do
  if ! declare -F "stage_$name" >/dev/null; then
    echo "ci: no stage '$name' (stages: ${stages[*]})" >&2
    exit 2
  fi
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
