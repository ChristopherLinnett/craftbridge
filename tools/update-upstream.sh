#!/usr/bin/env bash
# The one way the EffectCraft pin moves, and the whole upstream update after it (AGENTS.md §7, plan §24).
#
#   tools/update-upstream.sh <rev>
#
# <rev> is the id of a commit on official storytold/effectcraft main, in full or abbreviated. The script
# hands it to `cargo xtask upstream update` (rust/xtask/src/update.rs). Its steps 1 and 2, in
# rust/xtask/src/upstream.rs, fetch official main into the machine's read-only cache of the official
# repository when the cache does not reach <rev> yet, refuse a <rev> that is a branch, a tag or any name
# that moves, or a commit official main does not reach, then write the EffectCraft and FilmCraft pins,
# the [patch] tables and [profile.release] of rust/Cargo.toml from that revision's own manifest — copied,
# never retyped — seed rust/Cargo.lock from that revision's own lock, and let Cargo resolve CraftBridge's
# own crates on top (D-21). FilmCraft is pinned to the revision EffectCraft itself names (plan §24),
# never to one of CraftBridge's choosing.
#
# Steps 3 to 7 follow in the same run (rust/xtask/src/update.rs), each named by what a person does with
# its result: 3, the adaptations of tools/adaptations.toml whose upstream items the revision change
# touched, listed with their diffs to be rechecked, and the listed lines that only moved, with the lines
# they moved to; 4, the API surface and the facades regenerated and diffed against the surface the tree
# held; 5, the gate, tools/ci.sh, whose conformance stage compares the new revision's command line with
# the corpus; 6, the record appended to docs/upstream-updates.md, whose recheck lines the person
# completes; 7, every version of android/sdk/gradle/libs.versions.toml and of the Gradle wrapper that has
# a newer release, for a change of its own. A run on the revision the tree already holds stops after
# step 2: it fetches only what the cache lacks, and looks up no release.
#
# Exit codes: 0 for an update that stands, up to date, clean or with adaptations to recheck; 2 for a
# breaking API change, which makes the next release a major version; 1 for a failed gate, which the
# run's output names, and for a step that could not finish; 64 for a misused script.
#
# Running the script again on the revision the tree already holds writes nothing, so it changes nothing
# byte for byte; on a machine without the cache, that run is how the cache gets the pin. The `upstream`
# rule refuses a hand-written [patch] entry, a pin that is no official revision, and a registry, fork,
# path or other-commit copy of a pinned crate, together with any Cargo configuration that Cargo reads for
# a build in rust/ — the repository's own, one in a folder above the checkout, the cargo home's, a file
# those include, or the environment the gate runs in — carrying a [patch], a [source] replacement or
# paths of its own, so the script's output is the only form the gate accepts.
#
# Order matters when a pin gains or loses crates: run this script first, then let a crate manifest say
# `effectcraft-engine.workspace = true`. The workspace must parse for the script to build its own writer,
# so a manifest may never reference a pin before the script has written it.
#
# The cache is a bare clone outside the repository, a fetch cache and not a checkout: nothing builds from
# it and nothing edits it, and its git reads no machine or user git configuration, so no URL rewrite
# there can fill it from anywhere but the official repository. tools/env.sh gives it a path; a machine
# may aim CRAFTBRIDGE_UPSTREAM_CACHE elsewhere, at a bare clone of the official repository.
set -euo pipefail
# Git exports its repository variables to a hook and to what `git rebase --exec` starts; git commands
# here must act on the cache they are given, wherever the script was started from.
# shellcheck disable=SC2046 # one variable name per word, split on purpose
unset $(git rev-parse --local-env-vars 2>/dev/null)
# shellcheck source=tools/env.sh
source "$(dirname "$0")/env.sh"
cd "$CRAFTBRIDGE_ROOT"

if [ "$#" -ne 1 ]; then
  echo "usage: tools/update-upstream.sh <effectcraft-rev> — the id of the official storytold/effectcraft commit to build against" >&2
  exit 64
fi

# The writer of the tables builds from the workspace's own lock, `--locked`, so the build itself never
# rewrites the lock and a rerun on the same revision changes nothing; the pin itself then runs the resolve.
# It is built into a folder of this run only, as the gate builds it: a concurrent build would replace the
# shared binary this script is starting (tools/env.sh).
xtask_run="$(mktemp -d "${TMPDIR:-/tmp}/craftbridge-upstream.XXXXXX")"
trap 'rm -rf "$xtask_run"' EXIT
# A writer that does not build is a step that could not finish, never Cargo's own exit code.
craftbridge_xtask_build "$xtask_run" || exit 1
"$xtask_run/debug/xtask" upstream update "$1"
