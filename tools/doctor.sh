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

# Rust: the pinned toolchain and its mobile targets.
if ! has rustup; then
  gap "rustup" "install it from https://rustup.rs"
elif ! rustup run "$CRAFTBRIDGE_RUST_CHANNEL" rustc --version >/dev/null 2>&1; then
  # One fix for the toolchain and its targets, so a second run finds nothing left to do.
  gap "Rust $CRAFTBRIDGE_RUST_CHANNEL" "rustup toolchain install $CRAFTBRIDGE_RUST_CHANNEL --component rustfmt,clippy --target $(printf '%s' "$CRAFTBRIDGE_RUST_TARGETS" | tr ' ' ',')"
else
  ok "Rust $(rustup run "$CRAFTBRIDGE_RUST_CHANNEL" rustc --version | cut -d' ' -f2)"
  installed="$(rustup target list --installed --toolchain "$CRAFTBRIDGE_RUST_CHANNEL")"
  for target in $CRAFTBRIDGE_RUST_TARGETS; do
    if grep -qx "$target" <<<"$installed"; then ok "Rust target $target"; else gap "Rust target $target" "rustup target add --toolchain $CRAFTBRIDGE_RUST_CHANNEL $target"; fi
  done
fi
if has cargo-deny; then ok "cargo-deny $(cargo deny --version | cut -d' ' -f2)"; else gap "cargo-deny" "cargo install --locked cargo-deny"; fi

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
if xcrun --sdk iphonesimulator --show-sdk-path >/dev/null 2>&1; then ok "iOS Simulator SDK"; else gap "iOS Simulator SDK" "Xcode > Settings > Components: install the iOS platform"; fi
if xcrun --find swift-format >/dev/null 2>&1; then ok "swift-format $(xcrun swift-format --version)"; else gap "swift-format" "it ships with Xcode $CRAFTBRIDGE_XCODE; select that Xcode"; fi

# Android: the SDK platform and build tools Gradle compiles against (both pinned in the version catalog),
# and the JDK Gradle runs on.
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
if [ -n "$node_major" ] && [ "$node_major" -ge 18 ] && has npx; then ok "Node.js $(node --version) (runs $CRAFTBRIDGE_JSCPD)"; else gap "Node.js 18 or later" "brew install node"; fi

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
  ok "git hooks: tools/hooks, for every worktree of this repository (the gate runs on the working tree a push comes from)"
elif [ "$(git -C "$CRAFTBRIDGE_ROOT" config --get core.hooksPath 2>/dev/null || true)" = "tools/hooks" ]; then
  ok "git hooks: tools/hooks"
else
  note "the gate does not run before pushes here: tools/doctor.sh --install-hooks"
fi

exit "$missing"
