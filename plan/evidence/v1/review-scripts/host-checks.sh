#!/bin/zsh
# Host-side checks for the review: compares what FontProbe reported with the Mac's and the
# simulator runtime's font folders. Usage: host-checks.sh <probe report>
set -u
REPORT=$1
RT=$(xcrun simctl getenv booted IPHONE_SIMULATOR_ROOT)
F="$RT/System/Library/Fonts"
echo "## runtime root: $RT"
echo "## st_dev (Mac folder, runtime copy)"
stat -f '%d %N' /System/Library/Fonts "$F"
echo "## app's sections 2+3 file list vs the Mac's /System/Library/Fonts (2 levels)"
diff <(grep -E '^    file +/' "$REPORT" | sed -E 's/^    file +//' | sort) <(find /System/Library/Fonts -maxdepth 2 -type f | sort) && echo "identical: $(grep -cE '^    file +/' "$REPORT") files"
echo "## app's sections 2+3 file list vs the runtime copy"
diff <(grep -E '^    file +/' "$REPORT" | sed -E 's/^    file +//; s|^/System/Library/Fonts/||' | sort) <(find "$F" -type f | sed "s|$F/||" | sort) >/dev/null && echo identical || echo "different: the app did not list the runtime copy"
echo "## runtime copy: files per subfolder, by extension, depth"
for d in "$F"/*/; do echo "  $(find "$d" -type f | wc -l | tr -d ' ') $(basename "$d")"; done
find "$F" -type f | sed -E 's/.*\///; s/^[^.]*$/(none)/; s/.*\.//' | tr 'A-Z' 'a-z' | sort | uniq -c | sed 's/^/  /'
echo "  files deeper than one subfolder: $(find "$F" -mindepth 3 -type f | wc -l | tr -d ' ')"
echo "  files at its top level: $(find "$F" -maxdepth 1 -type f | wc -l | tr -d ' '); symlinks anywhere in it: $(find "$F" -type l | wc -l | tr -d ' ')"
echo "## fonts outside /System/Library/Fonts in the runtime (FontServices)"
find "$RT/System/Library/PrivateFrameworks/FontServices.framework" -type f \( -iname '*.ttf' -o -iname '*.ttc' -o -iname '*.otf' -o -iname '*.otc' \) | sed "s|$RT||"
echo "## same-named fonts differ between the Mac and the runtime (SHA-256)"
for pair in "Helvetica.ttc:Core/Helvetica.ttc" "Apple Color Emoji.ttc:CoreAddition/AppleColorEmoji-160px.ttc" "GeezaPro.ttc:Core/GeezaPro.ttc"; do
  m=${pair%%:*}; r=${pair#*:}
  echo "  mac $(shasum -a 256 "/System/Library/Fonts/$m" | cut -c1-16) $(stat -f %z "/System/Library/Fonts/$m") B  $m"
  echo "  ios $(shasum -a 256 "$F/$r" | cut -c1-16) $(stat -f %z "$F/$r") B  $r"
done
echo "## CoreText's files (section 8) exist at the same path on the Mac?"
grep -E '^    .* -> .* at <RuntimeRoot>' "$REPORT" | sed -E 's/.* at <RuntimeRoot>//' | while read -r p; do
  [[ -e "$p" ]] && echo "  on Mac too: $p" || echo "  not on the Mac: $p"
done
