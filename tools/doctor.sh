#!/usr/bin/env bash
# Checks every prerequisite of the CraftBridge build and prints the fix for each gap.
#
#   tools/doctor.sh                  everything: the gate's needs, the mobile builds, GitHub access
#   tools/doctor.sh --gate           only what tools/ci.sh needs (it runs this first)
#   tools/doctor.sh --install-hooks  also run the gate before every push
#
# Exit status 1 when anything required is missing.
set -euo pipefail
# shellcheck source=tools/env.sh
source "$(dirname "$0")/env.sh"

mode=full
install_hooks=0
for arg in "$@"; do
  case "$arg" in
    --gate) mode=gate ;;
    --install-hooks) install_hooks=1 ;;
    *) echo "usage: tools/doctor.sh [--gate] [--install-hooks]" >&2; exit 2 ;;
  esac
done

missing=0
ok() { printf 'ok       %s\n' "$1"; }
gap() { printf 'MISSING  %s\n         fix: %s\n' "$1" "$2"; missing=1; }
note() { printf 'note     %s\n' "$1"; }
has() { command -v "$1" >/dev/null 2>&1; }

# The environment first: a variable that switches what the gate's Rust tools check reaches the pins read
# below as it reaches every cargo call of the gate, which refuses to run on any of them (tools/env.sh).
refused="$(craftbridge_refused_environment)"
if [ -n "$refused" ]; then
  while IFS= read -r line; do gap "$line" "unset it: the gate runs the pinned toolchain as rust/Cargo.toml and rust/clippy.toml configure it"; done <<<"$refused"
fi
# Rust: the toolchain rustup itself resolves in rust/ — rustup's own reader, rust/rust-toolchain.toml
# included — and no other toolchain may stand over the pin: an override by environment variable or
# directory reaches every cargo call of the gate, and a nightly or other toolchain changes which clippy
# and rustfmt judge the code. The pins, then, are read by `cargo xtask pins` as soon as the toolchain
# stands: one command parses both pin files (rust/xtask/src/pins.rs), and nothing reads the version
# catalog before the Rust gaps above are closed.
rust_toolchain=""
pins_ok=0
rust_status="$(craftbridge_toolchain_status_in "$CRAFTBRIDGE_ROOT/rust")"
case "$rust_status" in
  ok\ *) rust_toolchain="${rust_status#ok }" ;;
  no-rustup) gap "rustup" "install it from https://rustup.rs" ;;
  missing) gap "the Rust toolchain pinned in rust/rust-toolchain.toml" "cd rust && rustup toolchain install: rustup reads that file itself — its channel, its components, its targets" ;;
  override\ *)
    line="${rust_status#override }"
    case "$line" in
      *RUSTUP_TOOLCHAIN*) fix="unset RUSTUP_TOOLCHAIN, and run outside any cargo +toolchain or rustup run, which set it: the build is made by the toolchain rust/rust-toolchain.toml pins" ;;
      *directory\ override*) fix="cd rust && rustup override unset: the build is made by the toolchain rust/rust-toolchain.toml pins" ;;
      *) fix="make rust/ resolve rust/rust-toolchain.toml again: no RUSTUP_TOOLCHAIN, no rustup override" ;;
    esac
    gap "Rust toolchain overridden ($line)" "$fix"
    ;;
esac
if [ -n "$rust_toolchain" ]; then
  ok "Rust $(rustup run "$rust_toolchain" rustc --version | cut -d' ' -f2) (rust/rust-toolchain.toml)"
  # Cargo merges a .cargo at the repository root into this very command, so the pins would come from
  # whatever an alias there names. The gate refuses the folder outright (tools/ci.sh); the doctor names it.
  if [ -e "$CRAFTBRIDGE_ROOT/.cargo" ]; then
    gap "no Cargo configuration at the repository root (found .cargo)" "move it away: Cargo merges it into every cargo stage of the gate and into the pins read here"
  # Inside the gate the pins come from the xtask it built for this run (tools/ci.sh): a `cargo xtask`
  # rebuilds rust/target/debug/xtask, and every cargo build replaces that file under whatever other run
  # in this worktree is starting it. Run by hand, the doctor asks through the alias a person runs.
  elif pins="$(if [ -n "${CRAFTBRIDGE_XTASK:-}" ]; then "$CRAFTBRIDGE_XTASK" pins; else cd "$CRAFTBRIDGE_ROOT/rust" && cargo xtask pins; fi 2>&1)"; then
    craftbridge_load_pins <<<"$pins"
    pins_ok=1
    installed="$(rustup target list --installed --toolchain "$rust_toolchain")"
    for target in $CRAFTBRIDGE_RUST_TARGETS; do
      if grep -qx "$target" <<<"$installed"; then ok "Rust target $target"; else gap "Rust target $target" "rustup target add --toolchain $rust_toolchain $target"; fi
    done
  else
    gap "the build pins in rust/rust-toolchain.toml and android/sdk/gradle/libs.versions.toml" "cargo xtask pins could not read them: $pins"
  fi
fi
if has cargo-deny; then ok "cargo-deny $(cargo-deny --version | cut -d' ' -f2)"; else gap "cargo-deny" "cargo install --locked cargo-deny"; fi
# The mobile targets of one toolchain: `rust_targets <toolchain> <how the toolchain is named here>`.
rust_targets() {
  local installed target
  installed="$(rustup target list --installed --toolchain "$1")"
  for target in $CRAFTBRIDGE_RUST_TARGETS; do
    if grep -qx "$target" <<<"$installed"; then ok "$2 target $target"; else gap "$2 target $target" "rustup target add --toolchain $1 $target"; fi
  done
}
dylint_fix="cargo install --locked cargo-dylint@$CRAFTBRIDGE_DYLINT"
if has cargo-dylint && [ "$(cargo-dylint dylint --version)" = "cargo-dylint $CRAFTBRIDGE_DYLINT" ]; then
  ok "cargo-dylint $CRAFTBRIDGE_DYLINT"
elif has cargo-dylint; then
  gap "cargo-dylint $CRAFTBRIDGE_DYLINT (found: $(cargo-dylint dylint --version))" "$dylint_fix"
else
  gap "cargo-dylint $CRAFTBRIDGE_DYLINT" "$dylint_fix"
fi
# The lint library `cargo dylint` runs over the workspace (AGENTS.md §3) builds with the nightly its own
# rust-toolchain.toml pins, which rustup reads in the library's folder, and the gate runs clippy over the
# library there: rustup's answer in that folder is the library's toolchain only when it comes from that
# pin, as in rust/ (tools/env.sh). `rustup toolchain install` run there installs it with the components the
# file lists; the mobile targets are the workspace's, which the library's phone runs check too.
if has rustup; then
  lint_library="${CRAFTBRIDGE_LINT_LIBRARY#"$CRAFTBRIDGE_ROOT"/}"
  lint_status="$(craftbridge_toolchain_status_in "$CRAFTBRIDGE_LINT_LIBRARY")"
  case "$lint_status" in
    ok\ *)
      lint_toolchain="${lint_status#ok }"
      ok "lint library toolchain $lint_toolchain ($lint_library/rust-toolchain.toml)"
      rust_targets "$lint_toolchain" "lint library toolchain"
      # clippy is one of the components the library's pin lists and the folder's install brings.
      if rustup component list --installed --toolchain "$lint_toolchain" | grep -q '^clippy-'; then
        ok "lint library toolchain clippy"
      else
        gap "clippy on the lint library toolchain" "(cd $lint_library && rustup toolchain install)"
      fi
      ;;
    missing) gap "the lint library's toolchain ($lint_library/rust-toolchain.toml)" "(cd $lint_library && rustup toolchain install)" ;;
    *) gap "lint library toolchain overridden (${lint_status#override })" "(cd $lint_library && rustup override unset): the library builds with the toolchain $lint_library/rust-toolchain.toml pins" ;;
  esac
fi

# Apple: the pinned Xcode, its iOS Simulator SDK and the formatter it ships.
xcode="$(xcodebuild -version 2>/dev/null | sed -n 's/^Xcode //p' || true)"
selected="$(xcode-select -p 2>/dev/null || true)"
if [ "$xcode" = "$CRAFTBRIDGE_XCODE" ]; then
  ok "Xcode $xcode"
elif [ -z "$xcode" ] && [ -d "$selected" ] && [ "${selected%/Contents/Developer}" != "$selected" ]; then
  # An Xcode is selected but will not run: its licence is not accepted or its first launch is pending.
  # -runFirstLaunch installs Xcode's system components and agrees to the licence (`man xcodebuild`); both
  # need an administrator.
  gap "Xcode $CRAFTBRIDGE_XCODE (the selected ${selected%/Contents/Developer} does not run)" "sudo xcodebuild -runFirstLaunch"
else
  gap "Xcode $CRAFTBRIDGE_XCODE (found: ${xcode:-none})" "install Xcode $CRAFTBRIDGE_XCODE from https://developer.apple.com/download/applications/ and select it: sudo xcode-select -s /Applications/Xcode.app"
fi
# Both iOS SDKs: the Swift stage reads the package as the device build compiles it as well as the
# simulator build the swift_build stage makes.
sim_sdk=0
for sdk in "iphonesimulator:iOS Simulator SDK" "iphoneos:iOS device SDK"; do
  if xcrun --sdk "${sdk%%:*}" --show-sdk-path >/dev/null 2>&1; then
    ok "${sdk#*:}"
    if [ "${sdk%%:*}" = iphonesimulator ]; then sim_sdk=1; fi
  else
    gap "${sdk#*:}" "Xcode > Settings > Components: install the iOS platform"
  fi
done
# The iOS Simulator runtime: the SDK inside Xcode compiles for the simulator, but the runtime a simulator
# boots is a separate download under Xcode 26, and xcodebuild builds the simulator slices the toolchain
# names only with the runtime of its simulator SDK's version installed — the one `xcodebuild
# -downloadPlatform iOS` fetches for the selected Xcode. simctl lists the runtimes CoreSimulator holds in
# its JSON form, and plutil, macOS's own reader of property lists and JSON, reads each entry: an iOS
# runtime simctl marks available, of the SDK's major and minor version, matches.
if [ "$sim_sdk" = 1 ]; then
  sdk_version="$(xcrun --sdk iphonesimulator --show-sdk-version 2>/dev/null | cut -d. -f1,2 || true)"
  runtime=""
  runtimes="$(mktemp)"
  runtime_field() { plutil -extract "runtimes.$1.$2" raw -o - "$runtimes" 2>/dev/null || true; }
  if xcrun simctl list --json runtimes >"$runtimes" 2>/dev/null; then
    count="$(plutil -extract runtimes raw -o - "$runtimes" 2>/dev/null || echo 0)"
    index=0
    while [ "$index" -lt "$count" ]; do
      if [ "$(runtime_field "$index" platform)" = iOS ] && [ "$(runtime_field "$index" isAvailable)" = true ] \
        && [ "$(runtime_field "$index" version | cut -d. -f1,2)" = "$sdk_version" ]; then
        runtime="$(runtime_field "$index" version)"
      fi
      index=$((index + 1))
    done
  fi
  rm -f "$runtimes"
  if [ -n "$runtime" ]; then ok "iOS Simulator runtime $runtime"; else gap "iOS Simulator runtime ${sdk_version:-?} for Xcode $CRAFTBRIDGE_XCODE" "xcodebuild -downloadPlatform iOS"; fi
fi
if xcrun --find swift-format >/dev/null 2>&1; then ok "swift-format $(xcrun swift-format --version)"; else gap "swift-format" "it ships with Xcode $CRAFTBRIDGE_XCODE; select that Xcode"; fi
swiftlint_fix="install the pinned copy: curl -fsSL $CRAFTBRIDGE_SWIFTLINT_URL | tar -xz -C /usr/local/bin swiftlint && chmod +x /usr/local/bin/swiftlint"
if has swiftlint && [ "$(swiftlint version)" = "$CRAFTBRIDGE_SWIFTLINT" ]; then
  ok "swiftlint $(swiftlint version)"
elif has swiftlint; then
  gap "swiftlint $CRAFTBRIDGE_SWIFTLINT (found: $(swiftlint version))" "$swiftlint_fix"
else
  gap "swiftlint $CRAFTBRIDGE_SWIFTLINT" "$swiftlint_fix"
fi
if xcrun --find sourcekit-lsp >/dev/null 2>&1; then ok "sourcekit-lsp"; else gap "sourcekit-lsp" "it ships with Xcode $CRAFTBRIDGE_XCODE; select that Xcode"; fi

# Android: the SDK platform and build tools Gradle compiles against (both pinned in the version catalog),
# and the JDK Gradle runs on — levels this doctor only knows once cargo xtask pins has read the catalog.
# The fix for an SDK package: the SDK's own sdkmanager, told which SDK it installs into. Another one on
# PATH (Homebrew's, say) ignores ANDROID_HOME and installs somewhere the build never looks.
sdk_fix() {
  local sdkmanager="$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager"
  local install="\"$sdkmanager\" --sdk_root=\"$ANDROID_HOME\" \"$1\""
  if [ -x "$sdkmanager" ]; then
    printf '%s' "$install"
  else
    printf 'install the Android SDK Command-line Tools (Android Studio > Settings > Android SDK > SDK Tools), then %s' "$install"
  fi
}
if [ "$pins_ok" = 1 ]; then
  if [ -d "$ANDROID_HOME/platforms" ]; then
    ok "Android SDK at $ANDROID_HOME"
    # A platform is the pinned API level exactly, as its own source.properties gives it: the SDK names a
    # minor-versioned platform `android-37.0` and an older one `android-36`, and a prefix of either
    # (`android-370`, `android-37.1`) is another platform. The fix names the package the SDK's way: from API
    # 37 on, with its minor version. With no platform installed the pattern stays unexpanded, and reading that
    # missing path would end the script, so it is skipped.
    platform_found=""
    for properties in "$ANDROID_HOME"/platforms/*/source.properties; do
      [ -f "$properties" ] || continue
      level="$(sed -n 's/^AndroidVersion.ApiLevel=//p' "$properties" | tr -d '[:space:]')"
      if [ "$level" = "$CRAFTBRIDGE_ANDROID_API" ] || [ "$level" = "$CRAFTBRIDGE_ANDROID_API.0" ]; then platform_found=1; fi
    done
    if [ "$CRAFTBRIDGE_ANDROID_API" -ge 37 ] 2>/dev/null; then platform_package="platforms;android-$CRAFTBRIDGE_ANDROID_API.0"; else platform_package="platforms;android-$CRAFTBRIDGE_ANDROID_API"; fi
    if [ -n "$platform_found" ]; then ok "Android platform $CRAFTBRIDGE_ANDROID_API"; else gap "Android platform $CRAFTBRIDGE_ANDROID_API" "$(sdk_fix "$platform_package")"; fi
    if [ -d "$ANDROID_HOME/build-tools/$CRAFTBRIDGE_ANDROID_BUILD_TOOLS" ]; then ok "Android build tools $CRAFTBRIDGE_ANDROID_BUILD_TOOLS"; else gap "Android build tools $CRAFTBRIDGE_ANDROID_BUILD_TOOLS" "$(sdk_fix "build-tools;$CRAFTBRIDGE_ANDROID_BUILD_TOOLS")"; fi
  else
    gap "Android SDK (looked in $ANDROID_HOME)" "install Android Studio, or set ANDROID_HOME to an Android SDK"
  fi
  if [ -n "${CRAFTBRIDGE_JDK:-}" ]; then
    ok "JDK $CRAFTBRIDGE_JDK_MAJOR at $CRAFTBRIDGE_JDK"
  else
    gap "JDK $CRAFTBRIDGE_JDK_MAJOR for Gradle" "brew install openjdk@$CRAFTBRIDGE_JDK_MAJOR (tools/env.sh finds Homebrew's keg), or point JAVA_HOME at a JDK $CRAFTBRIDGE_JDK_MAJOR (tools/env.sh also takes Android Studio's bundled JDK when it is that major)"
  fi
else
  # The catalog's levels are read by `cargo xtask pins`, which needs the pinned toolchain; what it could not
  # read cannot be checked, and the Rust gap above is the one to fix first. Nothing here read the catalog.
  note "the catalog's Android levels were not checked: fix the Rust gaps above, which cargo xtask pins needs to read them"
fi

# Cargo merges a .cargo/config.toml (or .cargo/config) from every folder above the one it runs in, up to
# the file system's root, and then $CARGO_HOME's. One above the repository is this machine's, not the
# tree's, so no rule sees it, yet its `runner` would stand in for every test binary of the gate.
dir="$(dirname "$CRAFTBRIDGE_ROOT")"
cargo_home="$(cd "${CARGO_HOME:-$HOME/.cargo}" 2>/dev/null && pwd -P || true)"
while :; do
  if [ -d "$dir/.cargo" ] && [ "$(cd "$dir/.cargo" && pwd -P)" != "$cargo_home" ]; then
    for config in "$dir/.cargo/config.toml" "$dir/.cargo/config"; do
      if [ -f "$config" ]; then gap "no Cargo configuration above the repository (found $config)" "move it away: Cargo merges it into every cargo stage of the gate"; fi
    done
  fi
  if [ "$dir" = "/" ]; then break; fi
  dir="$(dirname "$dir")"
done

# The gate's other tools.
if has shellcheck; then ok "shellcheck $(shellcheck --version | sed -n 's/^version: //p')"; else gap "shellcheck" "brew install shellcheck"; fi
node_major="$(node --version 2>/dev/null | sed -n 's/^v\([0-9]*\).*/\1/p' || true)"
if [ -n "$node_major" ] && [ "$node_major" -ge 20 ] && has npx; then ok "Node.js $(node --version) (runs $CRAFTBRIDGE_JSCPD)"; else gap "Node.js 20 or later" "brew install node"; fi
# The plan stage rebuilds the plan's views with the kit's own builders, on the same Node the line above
# checks for, from the dependencies as plan/package-lock.json pins them (tools/env.sh says why).
if craftbridge_plan_dependencies; then ok "the plan kit's dependencies, as plan/package-lock.json pins them"; else gap "the plan kit's dependencies, as plan/package-lock.json pins them" "(cd plan && npm ci)"; fi

if [ "$mode" = full ]; then
  # The mobile library builds. No gate stage compiles native code for Android yet (rust_mobile builds no
  # C), so the NDK is a need of the library builds alone. `-H` follows an ndk/ that is itself a symbolic
  # link; `|| true`: with no ndk folder find fails, and under pipefail that would end the doctor before
  # it printed the fix.
  if [ -d "$ANDROID_HOME/platforms" ]; then
    ndk="$(find -H "$ANDROID_HOME/ndk" -mindepth 1 -maxdepth 1 -name "$CRAFTBRIDGE_NDK_MAJOR.*" 2>/dev/null | sort | tail -n 1 || true)"
    if [ -n "$ndk" ]; then ok "Android NDK $(basename "$ndk")"; else gap "Android NDK r$CRAFTBRIDGE_NDK_MAJOR" "$(sdk_fix "ndk;$CRAFTBRIDGE_NDK_VERSION") (NDK r$CRAFTBRIDGE_NDK_MAJOR aligns native libraries to 16 KB pages)"; fi
  fi
  if has cargo-ndk; then ok "cargo-ndk $(cargo ndk --version | cut -d' ' -f2)"; else gap "cargo-ndk" "cargo install --locked cargo-ndk"; fi
  if [ -d "$CRAFTBRIDGE_ROOT/rust/uniffi-bindgen" ]; then
    ok "uniffi-bindgen (rust/uniffi-bindgen, at the library's UniFFI version)"
  else
    note "uniffi-bindgen is built from rust/uniffi-bindgen, so its version always matches the library's UniFFI; that crate does not exist yet"
  fi

  # GitHub: the account that owns the repository and the EffectCraft fork. It is named per command,
  # because the machine's active gh login may be another account.
  owner="${CRAFTBRIDGE_GH_USER:-ChristopherLinnett}"
  if ! has gh; then
    gap "GitHub CLI" "brew install gh"
  elif ! token="$(gh auth token --user "$owner" 2>/dev/null)"; then
    gap "a GitHub token for $owner" "gh auth login (as $owner)"
  else
    # One scope per line, matched whole: `repo:status` is not `repo`. `|| true` keeps a failed request
    # a missing scope rather than the end of the doctor.
    scopes="$(GH_TOKEN="$token" gh api -i user 2>/dev/null | sed -n 's/^[Xx]-[Oo][Aa]uth-[Ss]copes: //p' | tr -d '\r ' | tr ',' '\n' || true)"
    # `gh auth refresh` acts on the active account only (it has no --user), so the fix switches to the
    # owner's account first and back afterwards.
    refresh() { printf 'gh auth switch --user %s && gh auth refresh -s %s, then gh auth switch back to the account you use' "$owner" "$1"; }
    for scope in repo workflow; do
      if grep -qx "$scope" <<<"$scopes"; then ok "GitHub token for $owner: $scope"; else gap "GitHub token for $owner: $scope scope" "$(refresh "$scope")"; fi
    done
    if grep -qx "write:packages" <<<"$scopes"; then ok "GitHub token for $owner: write:packages"; else note "GitHub token for $owner lacks write:packages, which only publishing the Android libraries needs: $(refresh write:packages)"; fi
  fi
fi

if [ "$install_hooks" = 1 ]; then
  # Written to the repository's shared configuration, so every worktree of it gets the setting at once;
  # the path is relative, so each runs its own tools/hooks/pre-push.
  git -C "$CRAFTBRIDGE_ROOT" config core.hooksPath tools/hooks
  ok "git hooks: tools/hooks, for every worktree of this repository (each push is gated on every commit it carries, checked out into a temporary worktree of its own)"
elif [ "$(git -C "$CRAFTBRIDGE_ROOT" config --get core.hooksPath 2>/dev/null || true)" = "tools/hooks" ]; then
  ok "git hooks: tools/hooks"
else
  note "the gate does not run before pushes here: tools/doctor.sh --install-hooks"
fi

exit "$missing"
