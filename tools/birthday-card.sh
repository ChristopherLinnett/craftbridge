#!/usr/bin/env bash
# Makes the running example's template, `Birthday Card.ectemplate` (plan §5, §25), by replaying the recipe
# conformance/effects/birthday-card.jsonl with the official EffectCraft command line at the pin
# (tools/oracle.sh), and puts it in a folder. Every sample bundles the template this makes and none keeps a
# copy of one, so the template is the recipe's and nothing else's.
#
#   tools/birthday-card.sh <folder>      writes <folder>/Birthday Card.ectemplate (<folder> is made if missing)
#
# The recipe names its picture by a relative path, as the command line reads it from its working folder, so
# the replay runs in a scratch folder holding the corpus's own placeholder photo. The command line runs with
# an environment that holds only its configuration folder, as the conformance runner starts it, so nothing
# of the machine's changes the template.
set -euo pipefail
# shellcheck disable=SC2046 # one variable name per word, split on purpose
unset $(git rev-parse --local-env-vars 2>/dev/null)
# shellcheck source=tools/env.sh
source "$(dirname "$0")/env.sh"

if [ "$#" -ne 1 ]; then
  echo "usage: tools/birthday-card.sh <folder>" >&2
  exit 64
fi
mkdir -p "$1"
destination="$(cd "$1" && pwd)"
cd "$CRAFTBRIDGE_ROOT"

oracle="$(tools/oracle.sh)"
work="$(mktemp -d "${TMPDIR:-/tmp}/craftbridge-birthday-card.XXXXXX")"
craftbridge_remove_on_exit "$work"
cp conformance/effects/fixtures/photo.png "$work/photo.png"
(cd "$work" && env -i EFFECTCRAFT_CONFIG_DIR="$work/config" "$oracle" replay "$CRAFTBRIDGE_ROOT/conformance/effects/birthday-card.jsonl") >&2
cp "$work/Birthday Card.ectemplate" "$destination/Birthday Card.ectemplate"
craftbridge_passed=1
