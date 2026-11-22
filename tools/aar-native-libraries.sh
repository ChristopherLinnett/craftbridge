#!/usr/bin/env bash
# Checks one AAR before it reaches a repository: the module that packages the SDK's native library holds
# that library under every Android ABI the toolchain names, or the publish is refused.
#
#   tools/aar-native-libraries.sh <module> <aar>      the AAR of <module> holds what it must
#
# The thin shell over `cargo xtask aar-native-libraries`, for the build-logic publish task: a Gradle build
# builds no Rust, so this builds xtask into a folder of this run's own, its intermediates in the tree's
# build folder where the gate leaves them warm (tools/env.sh), and asks it. The binary is this run's alone
# because every cargo build replaces the binary in its target folder: the publish tasks of two modules may
# run at once, as may any `cargo xtask` of the tree, and a build into a shared folder would replace the
# binary another is starting. It hands the check the `jar` of the JDK the catalog pins: the JDK Gradle
# runs on, whose own tool lists the AAR. Which module packages the library, what its file is called and
# which ABIs it is built for are the check's alone, read from the layer table and the toolchain file;
# nothing of that is said here, so the release and the publish refuse for one reason, in one wording.
set -euo pipefail
# shellcheck disable=SC2046 # one variable name per word, split on purpose
unset $(git rev-parse --local-env-vars 2>/dev/null)
# shellcheck source=tools/env.sh
source "$(dirname "$0")/env.sh"

if [ "$#" -ne 2 ]; then
  echo "usage: tools/aar-native-libraries.sh <module> <aar>" >&2
  exit 64
fi
module="$1" aar="$2"
cd "$CRAFTBRIDGE_ROOT"

xtask_run="$(mktemp -d "${TMPDIR:-/tmp}/craftbridge-aar-native-libraries.XXXXXX")"
craftbridge_remove_on_exit "$xtask_run"
craftbridge_xtask_build "$xtask_run"
xtask="$xtask_run/debug/xtask"
# The pins name the JDK Gradle runs on, and reading them is what points JAVA_HOME at it.
craftbridge_load_pins < <("$xtask" pins)
"$xtask" aar-native-libraries "$JAVA_HOME/bin/jar" "$module" "$aar"
craftbridge_passed=1
