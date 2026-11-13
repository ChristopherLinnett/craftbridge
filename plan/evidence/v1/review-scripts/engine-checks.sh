#!/bin/zsh
# EffectCraft checks outside the app, with the effectcraft-text revision ecprobe links. Needs the
# app's report from run-sim.sh (CoreText's folders are read from its section 8), in the same
# $CAPTURE_DIR (default capture/; a check run passes a scratch folder, RESULT §8). Outputs there:
#   review-fontsroot-dir.txt, review-coretext-dirs.txt   folder lists (inputs below)
#   review-families-fontsroot.txt, review-families-coretext-dirs.txt,
#   review-families-diff.txt       DirectorySource's families on the Mac, and how they differ
#   review-arabic-face-names.txt   EffectCraft's face names, Mac copy vs runtime copy
#   review-coretext-file-families.txt  the families EffectCraft reads from the files CoreText lists
#   review-face-matching.txt          CoreText's faces vs EffectCraft's, matched face by face
#   review-han-fallback.txt        EffectCraft's fallback, run inside the simulator
#   review-outline-formats.txt     outline formats of every runtime face
set -eu -o pipefail
cd /tmp/craftbridge-v1
mkdir -p "${CAPTURE_DIR:-capture}"
# Absolute, because simctl spawn starts the fallback example in another working directory.
C=${${CAPTURE_DIR:-capture}:A}
# One target folder: simctl spawn below runs the simulator binary from this path.
export CARGO_TARGET_DIR=$PWD/ecprobe/target
(cd ecprobe && cargo build -q --release --examples && cargo build -q --release --target aarch64-apple-ios-sim --example fallback)
E=$CARGO_TARGET_DIR/release/examples
RT=$(xcrun simctl getenv booted IPHONE_SIMULATOR_ROOT)
F="$RT/System/Library/Fonts"

print -r -- "$F" > $C/review-fontsroot-dir.txt
sed -nE "s|^    [0-9]+ in <RuntimeRoot>([^:]*): .*|$RT\1|p" $C/review-documents-report.txt > $C/review-coretext-dirs.txt
[[ -s $C/review-coretext-dirs.txt ]] || { echo "no CoreText folders in the report" >&2; exit 1; }

$E/dirfamilies @$C/review-fontsroot-dir.txt > $C/review-families-fontsroot.txt
$E/dirfamilies @$C/review-coretext-dirs.txt > $C/review-families-coretext-dirs.txt
{
  echo "## families only with [<RuntimeRoot>/System/Library/Fonts]"
  comm -13 $C/review-families-coretext-dirs.txt $C/review-families-fontsroot.txt
  echo "## families only with CoreText's folders"
  comm -23 $C/review-families-coretext-dirs.txt $C/review-families-fontsroot.txt
  echo "## the files behind the first list: EffectCraft's faces in <RuntimeRoot>/System/Library/Fonts/Watch"
  $E/faces "$F"/Watch/*
} > $C/review-families-diff.txt

# The UIFont families DirectorySource lists under their Arabic names: the Mac copy against the runtime copy.
for pair in "Supplemental/Al Nile.ttc:AppFonts/AlNile.ttc" "Supplemental/Damascus.ttc:AppFonts/Damascus.ttc" \
            "Supplemental/Farah.ttc:AppFonts/Farah.ttc" "GeezaPro.ttc:Core/GeezaPro.ttc" "Supplemental/Mishafi.ttf:AppFonts/Mishafi.ttf"; do
  mac=$($E/faces "/System/Library/Fonts/${pair%%:*}" | tail -n +2)
  ios=$($E/faces "$F/${pair#*:}" | tail -n +2)
  echo "## Mac /System/Library/Fonts/${pair%%:*} | runtime <RuntimeRoot>/System/Library/Fonts/${pair#*:}: $([[ "$mac" == "$ios" ]] && echo 'same face names' || echo DIFFERENT)"
  print -r -- "$mac"
done > $C/review-arabic-face-names.txt

# The families EffectCraft reads from the files CoreText lists (section 8 names them per folder): what
# a list_fonts limited to CoreText's files would offer.
files=("${(@f)$(sed -nE "s|^    [0-9]+ in <RuntimeRoot>([^:]*): (.*)|\1	\2|p" $C/review-documents-report.txt |
  while IFS=$'\t' read -r dir names; do for n in ${(s:, :)names}; do print -r -- "$RT$dir/$n"; done; done)}")
for f in "${files[@]}"; do [[ -r $f ]] || { echo "not readable: $f" >&2; exit 1; }; done
fams=("${(@f)$($E/faces "${files[@]}" | sed -nE 's|^  (.*)/[^/]* \(index [0-9]+, full .*|\1|p' | LC_ALL=C sort -u)}")
dots=(${(M)fams:#.*})
{
  echo "## families EffectCraft reads from the ${#files} files CoreText lists: ${#fams} (${#dots} dot-named)"
  print -rl -- "${fams[@]}"
} > $C/review-coretext-file-families.txt

for list in review-fontsroot-dir review-coretext-dirs; do
  xcrun simctl spawn booted $CARGO_TARGET_DIR/aarch64-apple-ios-sim/release/examples/fallback @$C/$list.txt
done > $C/review-han-fallback.txt

# Face by face: the app's own CoreText face identities (section 8 of its report) against the faces
# EffectCraft registers from CoreText's folders; the (file, PostScript name) pair is the identity
# the match uses. facematch's own header lines say what the answer is.
T=$PWD/build/face-match-work; rm -rf $T; mkdir -p $T
sed -n "s|^    face\t||p" $C/review-documents-report.txt | sed "s|<RuntimeRoot>|$RT|" > $T/ct-faces.tsv
sed -n 's/^    uifamily\t//p' $C/review-documents-report.txt > $T/ui-families.txt
[[ -s $T/ct-faces.tsv && -s $T/ui-families.txt ]] || { echo "no face or UIFont lines in the report" >&2; exit 1; }
{
  print -r -- "## Face-by-face identity of CoreText's faces and the faces EffectCraft registers from CoreText's folders,"
  print -r -- "## the (file, PostScript name) pair: the rule FX-3's list_fonts follows (RESULT §4). Made by engine-checks.sh:"
  print -r -- "## sections 1-2 are the face and uifamily lines of the app's report (section 8), sections 3-5 ecprobe's facematch"
  print -r -- "## example over review-coretext-dirs.txt. A \"-\" in a first column means no PostScript name."
  print -r -- "## 1. CoreText's faces: PostScript name, file, family, one line per descriptor, sorted:"
  cat $T/ct-faces.tsv
  print -r -- "## 2. UIFont.familyNames, sorted:"
  cat $T/ui-families.txt
  $E/facematch @$C/review-coretext-dirs.txt $T/ct-faces.tsv $T/ui-families.txt
} > $C/review-face-matching.txt
rm -rf $T

python3 -I review-scripts/outline-formats.py "$RT" > $C/review-outline-formats.txt
echo done
