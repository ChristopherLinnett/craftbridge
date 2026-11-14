#!/usr/bin/env bash
# The one way the EffectCraft pin moves (AGENTS.md §7, plan §24 steps 1 and 2).
#
#   tools/update-upstream.sh <rev>
#
# <rev> is the id of a commit on official storytold/effectcraft main, in full or abbreviated. The script
# hands it to `cargo xtask upstream pin` (rust/xtask/src/upstream.rs), which does every step in one
# place: it fetches official main into the machine's read-only cache of the official repository when the
# cache does not reach <rev> yet, refuses a <rev> that is a branch, a tag or any name that moves, or a
# commit official main does not reach, then writes the EffectCraft and FilmCraft pins, the [patch]
# tables and [profile.release] of rust/Cargo.toml from that revision's own manifest — copied, never
# retyped — seeds rust/Cargo.lock from that revision's own lock, and lets Cargo resolve CraftBridge's own
# crates on top (D-21). FilmCraft is pinned to the revision EffectCraft itself names (plan §24), never to
# one of CraftBridge's choosing.
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
  exit 2
fi

# The writer of the tables builds from the workspace's own lock; the pin itself then runs the resolve.
(cd rust && cargo build --quiet --package xtask)
"$CARGO_TARGET_DIR/debug/xtask" upstream pin "$1"
