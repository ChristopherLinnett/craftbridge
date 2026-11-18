#!/usr/bin/env bash
# Builds what the Android library ships: the one native library for each Android target of
# rust/rust-toolchain.toml, into the `jniLibs` folder of the module that packages it, and the Kotlin
# bindings it is called through.
#
#   tools/build-android.sh      build, then check the alignment of what was built
#
# The NDK, cargo-ndk and the target feature are set once in tools/env.sh, as they are for the gate's
# rust_mobile stage and for a test run on a device. The Kotlin bindings are written by `cargo xtask
# generate`, whose registered generators run the workspace's uniffi-bindgen over a host build of the same
# library, so they always describe the library built here.
set -euo pipefail
# shellcheck disable=SC2046 # one variable name per word, split on purpose
unset $(git rev-parse --local-env-vars 2>/dev/null)
# shellcheck source=tools/env.sh
source "$(dirname "$0")/env.sh"
cd "$CRAFTBRIDGE_ROOT"
# What ships is built as the gate builds: a `RUSTFLAGS` or a profile in the shell's environment, which cargo
# reads over the target feature tools/env.sh sets, would change the library without a word.
craftbridge_refuse_switches || exit 1

xtask_run="$(mktemp -d "${TMPDIR:-/tmp}/craftbridge-xtask.XXXXXX")"
craftbridge_remove_on_exit "$xtask_run"
craftbridge_xtask_build "$xtask_run"
xtask="$xtask_run/debug/xtask"
craftbridge_load_pins < <("$xtask" pins)
craftbridge_android_env

# The module that packages the native library, and where its libraries go.
jni_libs="android/sdk/effects/src/main/jniLibs"
# The native library the SDK ships, as the layer table gives it (rust/xtask/src/layers.rs): the package
# cargo builds, and its library's name, which the file of each Android build carries as lib<name>.so. The
# table is the one place that says which crate is the distribution library, so the script names neither.
sdk_library="$("$xtask" sdk-library)"
package="" name=""
while IFS= read -r line; do
  case "$line" in
    package=*) package="${line#package=}" ;;
    library=*) name="${line#library=}" ;;
  esac
done <<<"$sdk_library"
if [ -z "$package" ] || [ -z "$name" ]; then
  echo "build-android: cargo xtask sdk-library named no package or no library: $sdk_library" >&2
  exit 1
fi
library="lib$name.so"
targets=()
while IFS= read -r target; do targets+=(-t "$target"); done < <(craftbridge_android_targets)
if [ ${#targets[@]} -eq 0 ]; then
  echo "build-android: rust/rust-toolchain.toml names no Android target" >&2
  exit 1
fi

# The folder is the library's alone: a library of a target no longer built would be packaged anyway.
rm -rf "$jni_libs"
craftbridge_cargo ndk "${targets[@]}" -o "../$jni_libs" build --release --locked -p "$package"

# cargo-ndk copies every dynamic library the build reports, and a dependency (boa_engine) is built as one
# too, which no app loads: the distribution library is the one native library of the AAR (AGENTS.md §2).
find "$jni_libs" -name '*.so' ! -name "$library" -delete

# V6: every segment of every library is aligned to the 16 KB pages Android 15 maps with. The NDK's own
# llvm-readelf reads the program headers, and xtask judges what it printed.
readelf="$(find "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt" -name llvm-readelf | head -n 1)"
libraries=()
while IFS= read -r file; do libraries+=("$file"); done < <(find "$jni_libs" -name "$library" | sort)
"$xtask" elf-alignment "$readelf" "${libraries[@]}"

"$xtask" generate
craftbridge_passed=1
