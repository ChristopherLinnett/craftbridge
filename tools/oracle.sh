#!/usr/bin/env bash
# Builds the oracle the conformance corpus is compared with: the official EffectCraft command line at the
# pinned revision (AGENTS.md §7), and prints the path of the binary on stdout (everything else goes to
# stderr).
#
#   tools/oracle.sh
#
# The source is what `cargo xtask upstream source` prints: the official repository and the full commit
# rust/Cargo.toml's pin names, held to official main through the upstream cache exactly as the `upstream`
# rule holds the workspace to it, so the oracle is the commit the workspace builds, never a fork, a
# checkout or a binary found on the machine. A full commit Cargo has already fetched for the workspace is
# built without a fetch. `cargo install --locked` builds it with the EffectCraft workspace's own lock and
# tables, the lock the pin seeds rust/Cargo.lock from, which is how that revision is built upstream. The
# binary and its build folder live under rust/target, per worktree like every other build output: Cargo
# keeps the build folder warm, skips the install while the installed source is the pinned commit, and
# reinstalls once the pin names another, since the source it records is the commit's.
set -euo pipefail
# shellcheck disable=SC2046 # one variable name per word, split on purpose
unset $(git rev-parse --local-env-vars 2>/dev/null)
# shellcheck source=tools/env.sh
source "$(dirname "$0")/env.sh"
cd "$CRAFTBRIDGE_ROOT"

# xtask is built into a folder of this run only, as the gate does: a concurrent build would replace the
# shared binary this script is starting (tools/env.sh).
xtask_run="$(mktemp -d "${TMPDIR:-/tmp}/craftbridge-oracle.XXXXXX")"
craftbridge_remove_on_exit "$xtask_run"
craftbridge_xtask_build "$xtask_run"
source_line="$("$xtask_run/debug/xtask" upstream source)"
read -r url commit <<<"$source_line"
oracle="$CARGO_TARGET_DIR/oracle"
craftbridge_cargo install --git "$url" --rev "$commit" --locked --root "$oracle" --target-dir "$oracle-build" effectcraft-cli >&2
echo "$oracle/bin/effectcraft-cli"
craftbridge_passed=1
