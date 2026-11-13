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
  gap "Rust $CRAFTBRIDGE_RUST_CHANNEL" "rustup toolchain install $CRAFTBRIDGE_RUST_CHANNEL --component rustfmt,clippy"
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
if [ "$xcode" = "$CRAFTBRIDGE_XCODE" ]; then
  ok "Xcode $xcode"
else
  gap "Xcode $CRAFTBRIDGE_XCODE (found: ${xcode:-none})" "install Xcode $CRAFTBRIDGE_XCODE from https://developer.apple.com/download/applications/ and select it: sudo xcode-select -s /Applications/Xcode.app"
fi
if xcrun --sdk iphonesimulator --show-sdk-path >/dev/null 2>&1; then ok "iOS Simulator SDK"; else gap "iOS Simulator SDK" "Xcode > Settings > Components: install the iOS platform"; fi
if xcrun --find swift-format >/dev/null 2>&1; then ok "swift-format $(xcrun swift-format --version)"; else gap "swift-format" "it ships with Xcode $CRAFTBRIDGE_XCODE; select that Xcode"; fi

# Android: the SDK platform and build tools Gradle compiles against, the NDK, and JDK 21.
if [ -d "$ANDROID_HOME/platforms" ]; then
  ok "Android SDK at $ANDROID_HOME"
  if compgen -G "$ANDROID_HOME/platforms/android-$CRAFTBRIDGE_ANDROID_API*" >/dev/null; then ok "Android platform $CRAFTBRIDGE_ANDROID_API"; else gap "Android platform $CRAFTBRIDGE_ANDROID_API" "sdkmanager \"platforms;android-$CRAFTBRIDGE_ANDROID_API.0\""; fi
  if compgen -G "$ANDROID_HOME/build-tools/$CRAFTBRIDGE_ANDROID_API.*" >/dev/null; then ok "Android build tools $CRAFTBRIDGE_ANDROID_API"; else gap "Android build tools $CRAFTBRIDGE_ANDROID_API" "sdkmanager \"build-tools;$CRAFTBRIDGE_ANDROID_API.0.0\""; fi
  # `|| true`: with no ndk folder find fails, and under pipefail that would end the doctor before it
  # printed the fix.
  ndk="$(find "$ANDROID_HOME/ndk" -mindepth 1 -maxdepth 1 -name "$CRAFTBRIDGE_NDK_MAJOR.*" 2>/dev/null | sort | tail -n 1 || true)"
  if [ -n "$ndk" ]; then ok "Android NDK $(basename "$ndk")"; else gap "Android NDK r$CRAFTBRIDGE_NDK_MAJOR" "sdkmanager \"ndk;$CRAFTBRIDGE_NDK_VERSION\" (NDK r$CRAFTBRIDGE_NDK_MAJOR aligns native libraries to 16 KB pages)"; fi
else
  gap "Android SDK (looked in $ANDROID_HOME)" "install Android Studio, or set ANDROID_HOME to an Android SDK"
fi
if [ -n "${CRAFTBRIDGE_JDK:-}" ]; then ok "JDK 21 at $CRAFTBRIDGE_JDK"; else gap "JDK 21 for Gradle" "brew install openjdk@21, or install Android Studio (it bundles JDK 21), or point JAVA_HOME at a JDK 21"; fi

# The gate's other tools.
if has shellcheck; then ok "shellcheck $(shellcheck --version | sed -n 's/^version: //p')"; else gap "shellcheck" "brew install shellcheck"; fi
node_major="$(node --version 2>/dev/null | sed -n 's/^v\([0-9]*\).*/\1/p' || true)"
if [ -n "$node_major" ] && [ "$node_major" -ge 18 ] && has npx; then ok "Node.js $(node --version) (runs $CRAFTBRIDGE_JSCPD)"; else gap "Node.js 18 or later" "brew install node"; fi

if [ "$mode" = full ]; then
  # The mobile library builds.
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
    for scope in repo workflow; do
      if grep -qx "$scope" <<<"$scopes"; then ok "GitHub token for $owner: $scope"; else gap "GitHub token for $owner: $scope scope" "gh auth refresh -s $scope --user $owner"; fi
    done
    if grep -qx "write:packages" <<<"$scopes"; then ok "GitHub token for $owner: write:packages"; else note "GitHub token for $owner lacks write:packages, which only publishing the Android libraries needs: gh auth refresh -s write:packages --user $owner"; fi
  fi
fi

if [ "$install_hooks" = 1 ]; then
  git -C "$CRAFTBRIDGE_ROOT" config core.hooksPath tools/hooks
  ok "git hooks: tools/hooks (the gate runs before every push)"
elif [ "$(git -C "$CRAFTBRIDGE_ROOT" config --get core.hooksPath 2>/dev/null || true)" = "tools/hooks" ]; then
  ok "git hooks: tools/hooks"
else
  note "the gate does not run before pushes here: tools/doctor.sh --install-hooks"
fi

exit "$missing"
