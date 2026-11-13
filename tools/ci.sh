#!/usr/bin/env bash
# The gate: every check a change must pass before it is pushed or merged (AGENTS.md).
#
#   tools/ci.sh              every stage, the doctor first
#   tools/ci.sh rules swift  only the named stages or groups (names below), and the doctor only if named
#
# Every stage runs even when an earlier one fails, so one run reports everything that is broken. Every
# name is checked before the first stage runs, so a misspelt one costs nothing.
set -euo pipefail
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
  # -L follows symbolic links, as SwiftPM and the compilers do: a linked file is compiled, so it is read.
  find -L "$@" -type d \( "${prune[@]}" \) -prune -o -type f -name "$pattern" -print 2>/dev/null | sort
}

# The shell scripts under the given folders: every `*.sh`, and every other file whose first line runs sh
# or bash, the way jscpd tells a script by its shebang. The Gradle wrapper is out: Gradle's wrapper task
# writes it, and a fix to it would be lost the next time the wrapper is renewed. (plan/plan, the plan
# kit's launcher, is outside the code folders, and `plan/plan check --final` checks the plan.)
shell_scripts() {
  local file
  while IFS= read -r file; do
    case "$file" in
      android/sdk/gradlew) continue ;;
      *.sh) printf '%s\n' "$file" ;;
      *) if head -n 1 "$file" 2>/dev/null | grep -Eq '^#!.*([/ ])(ba)?sh([[:space:]]|$)'; then printf '%s\n' "$file"; fi ;;
    esac
  done < <(written_files '*' "$@")
}

stage_doctor() { tools/doctor.sh --gate; }
stage_rust_fmt() { cargo_in_rust fmt --check; }
# --all-features: code behind a feature is code that ships to whoever turns the feature on.
stage_rust_clippy() { cargo_in_rust clippy --workspace --all-targets --all-features --locked -- -D warnings; }
stage_rust_test() { cargo_in_rust test --workspace --locked; }
stage_rust_mobile() {
  local target status=0
  # The default members are the libraries an app links; the host tools never run on a phone. Clippy, not
  # a bare check: the platform modules compile only for the phones, so the never-crash lints and the
  # SAFETY comments are only checked on them here. Every target is checked even after one fails, so one
  # run names each target that is broken.
  for target in $CRAFTBRIDGE_RUST_TARGETS; do cargo_in_rust clippy --locked --all-features --target "$target" -- -D warnings || status=1; done
  return "$status"
}
stage_rust_deny() { cargo_in_rust deny --locked check licenses bans sources; }
# The rule checks are built with `cargo build` and their binary run here, not through the `cargo xtask`
# alias or `cargo run`: a Cargo configuration can redefine an alias, and a `[target.<triple>].runner`
# replaces whatever binary `cargo run` (and `cargo test`) would start, so either would pass these stages
# having checked nothing. `cargo build` runs no runner, and env.sh fixes where it writes; the rules then
# report the alias or the runner.
xtask() {
  cargo_in_rust build --package xtask --locked --quiet || return 1
  "$CARGO_TARGET_DIR/debug/xtask" "$@"
}
stage_rules() { xtask rules; }
stage_generated() { xtask generate --check; }
stage_swift_format() {
  local files=()
  # Generated Swift is UniFFI's and the generator's style, not ours; xtask checks it is untouched.
  while IFS= read -r file; do files+=("$file"); done < <(written_files '*.swift' ios)
  # bash 3.2 treats an empty array as unset under `set -u`, which would end the whole run here.
  if [ ${#files[@]} -eq 0 ]; then
    echo "ci: no Swift under ios/ to lint" >&2
    return 1
  fi
  xcrun swift-format lint --strict --follow-symlinks --configuration ios/sdk/.swift-format "${files[@]}"
}
stage_swift_build() {
  # DIAGNOSE_MISSING_TARGET_DEPENDENCIES=YES_ERROR makes an import the package does not declare an
  # error. Without it a module an earlier build left in the derived data satisfies the import with only
  # a warning, so `CraftBridge` could import a plugin against AGENTS.md §2 and still build.
  # build-for-testing builds the test targets too, which the package scheme builds only for testing.
  (cd ios/sdk && xcodebuild -scheme CraftBridge-Package -destination 'generic/platform=iOS Simulator' -derivedDataPath .build/xcode -quiet build build-for-testing \
    DIAGNOSE_MISSING_TARGET_DEPENDENCIES=YES_ERROR)
}
stage_android() {
  # A fresh JVM every run: ktlint keeps the .editorconfig files it has parsed for the life of the
  # Gradle daemon, so a warm daemon would go on checking against rules that have since changed.
  # --continue runs every task whose inputs are still good after one fails, so a formatting error does
  # not hide a Lint or compile error until the next run.
  # The tests' Kotlin compiles too, under the same warnings-as-errors rules as the code it tests.
  # A task a module's build file disables, or guards with `onlyIf`, checks nothing and still lets the
  # build succeed; `cargo xtask skipped-tasks` reads Gradle's log and fails on any such task of the SDK's
  # projects (rust/xtask/src/gradle_log.rs). --console=plain makes Gradle print every task's outcome even
  # in a terminal.
  local log status=0
  log="$(mktemp)"
  (cd android/sdk && ./gradlew --no-daemon --continue --warning-mode=fail --console=plain \
    spotlessCheck lint assembleRelease compileDebugUnitTestSources compileDebugAndroidTestSources) | tee "$log" || status=1
  xtask skipped-tasks "$log" || status=1
  rm -f "$log"
  return "$status"
}
stage_shell() {
  local scripts=()
  # Every shell script wherever it lives, by extension or by its #! line. --norc: an rc file beside the
  # scripts would be read as directives for all of them; the rules refuse one, and shellcheck reads none.
  while IFS= read -r file; do scripts+=("$file"); done < <(shell_scripts "${code_dirs[@]}")
  shellcheck --norc -x "${scripts[@]}"
}
stage_duplicates() {
  # The owner's first rule: reuse by extraction, never a second copy. Ten repeated lines, or seventy
  # repeated tokens, fail: one pass for each, because jscpd keeps a clone only when it meets both of its
  # minimums. jscpd 5.4.3 keeps a clone whose measured span is longer than --min-lines, and the span of a
  # block that code follows takes in the next line, so 9 makes every ten-line repeat fail (and a nine-line
  # one with code after it); the line pass asks for ten tokens, one per line, so ten blank lines are not
  # ten repeated lines. --min-tokens counts a clone of exactly that many, so 70 is seventy, on any number
  # of lines. Generated code repeats by design, so it is ignored with the rest of env.sh's build output,
  # each path as a glob of everything below it, and that list is the only one jscpd follows:
  # --no-gitignore, and the rules refuse the .ignore files and markers jscpd would honour too. --absolute
  # names each copy by its full path, since each code folder is a root of its own; --max-size lifts the
  # 1 MB default under which a large file was skipped unseen; --fail-on-empty fails a run that read
  # nothing; --follow-symlinks reads a linked file, as the compilers do.
  local output outputs ignore="" status=0 pass
  build_outputs
  for output in "${outputs[@]}"; do ignore="${ignore:+$ignore,}$output/**"; done
  for pass in "--min-lines 9 --min-tokens 10" "--min-lines 0 --min-tokens 70"; do
    # shellcheck disable=SC2086 # each pass is two flag pairs, split on purpose
    npx --yes "$CRAFTBRIDGE_JSCPD" $pass --format rust,swift,kotlin,bash --no-gitignore --absolute --max-size 100mb \
      --fail-on-empty --follow-symlinks --ignore "$ignore" --reporters console --exit-code 1 "${code_dirs[@]}" || status=1
  done
  return "$status"
}

stages=(doctor rust_fmt rust_clippy rust_test rust_mobile rust_deny rules generated swift_format swift_build android shell duplicates)
# The stage groups the decision and plan §15 name: Rust and Swift are each several stages.
group_rust=(rust_fmt rust_clippy rust_test rust_mobile rust_deny)
group_swift=(swift_format swift_build)
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
