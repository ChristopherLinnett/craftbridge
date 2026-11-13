#!/bin/zsh
# The drawing cost of the filtering-FontSource list_fonts rule (RESULT §4): what EffectCraft can
# still draw when a phone registers only the faces CoreText offers (the 266 + bundled) instead of
# every face in CoreText's folders (the 633 + bundled). Needs the app's report and
# review-coretext-dirs.txt in $CAPTURE_DIR (default capture/), as engine-checks.sh leaves them.
#
# Runs ecprobe's fallbackcost example twice in the booted simulator, as two clean processes:
# EffectCraft's font registry is process state, and add_source does not clear the fallback search
# cache (fonts.rs:396-413 clears only the usual-family cache at the pinned rev), so the two
# registrations must not share a process. Each process derives the same sample characters (each
# unregistered family's first cmap character that no registered/bundled face maps), registers its
# set through fonts::add_source, and answers every sample with the public fonts::fallback_for_locale
# (preferred face Inter Regular, locale en-US). Output: review-fallback-cost.txt.
set -eu -o pipefail
cd /tmp/craftbridge-v1
C=${${CAPTURE_DIR:-capture}:A}
[[ -s $C/review-coretext-dirs.txt && -s $C/review-documents-report.txt ]] || { echo "run run-sim.sh and engine-checks.sh into $C first" >&2; exit 1; }
export CARGO_TARGET_DIR=$PWD/ecprobe/target
(cd ecprobe && cargo build -q --offline --release --target aarch64-apple-ios-sim --example fallbackcost)
E=$CARGO_TARGET_DIR/aarch64-apple-ios-sim/release/examples/fallbackcost
T=$PWD/build/fallback-cost-work; rm -rf $T; mkdir -p $T
sed -n "s|^    face\t||p" $C/review-documents-report.txt | sed "s|<RuntimeRoot>|$(xcrun simctl getenv booted IPHONE_SIMULATOR_ROOT)|" > $T/ct-faces.tsv
[[ -s $T/ct-faces.tsv ]] || { echo "no face lines in the report" >&2; exit 1; }
xcrun simctl spawn booted $E @$C/review-coretext-dirs.txt $T/ct-faces.tsv filtered > $T/filtered.tsv
xcrun simctl spawn booted $E @$C/review-coretext-dirs.txt $T/ct-faces.tsv full > $T/full.tsv
{
  print -r -- "## What the filtering-FontSource list_fonts rule costs in drawing (RESULT §4)."
  print -r -- "## Made by review-scripts/fallback-cost.sh: fallbackcost runs twice in the simulator as clean processes (the font"
  print -r -- "## registry is process state and add_source does not clear the fallback search cache), once registering only"
  print -r -- "## the faces CoreText offers the app (the 266 matched by (file, PostScript name), facematch's rule), once all 633;"
  print -r -- "## both start with EffectCraft's 7 bundled faces, and on this target scan_system adds nothing (§3)."
  print -r -- "## Sample per family of the 367 unregistered faces: its first character that the face's cmap maps to a glyph"
  print -r -- "## (skrifa mappings, the first condition of EffectCraft's own covers) and no face kept registered maps at all."
  print -r -- "## Answers are fonts::fallback_for_locale (Inter Regular preferred, en-US): the picked family / style, or none"
  print -r -- "## when only a bundled face answers (nothing covers the sample)."
  print -r -- "## family = the unregistered face's family (its script); under633 = face under the full 633; under266 = under 266."
  print -r -- "## sample lines:"
  grep '^sample' $T/filtered.tsv | awk -F'\t' 'NR==FNR { if ($1=="sample") a[$2]=$5; next } $1=="sample" { print "## " $2 "\t" $3 " (" $4 ")\t" a[$2] "\t" $5 }' $T/full.tsv -
  print -r -- "## summary:"
  print -r -- "## $(grep '^meta' $T/filtered.tsv | sed 's/^meta\t//; s/\t/ /g') (filtered run); $(grep '^meta' $T/full.tsv | sed 's/^meta\t//; s/\t/ /g') (full run)"
  awk -F'\t' 'NR==FNR { if ($1=="sample") a[$2]=$5; next } $1=="sample" { s++; if ($5=="none" && a[$2]!="none") lost++ } END { print "## sample families: " s "; losing every covering face under 266: " lost }' $T/full.tsv $T/filtered.tsv
  print -r -- "## families of the 367 whose every cmap character the kept faces already map (lose nothing): $(grep -c '^nosample' $T/filtered.tsv)"
  grep '^nosample' $T/filtered.tsv | sed 's/^nosample\t/## none-of-its-own: /'
} > $C/review-fallback-cost.txt
rm -rf $T
echo done
