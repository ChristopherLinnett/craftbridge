#!/bin/zsh
# Builds ecprobe + FontProbe (Release), runs it in the booted simulator and collects the report.
# Outputs, in $CAPTURE_DIR (default capture/; a check run passes a scratch folder, RESULT §8):
# review-documents-report.txt (Documents copy), review-stdout.txt (stdout copy; must be
# byte-identical), review-stderr.txt, review-oslog.txt (the run's os_log pointer line),
# review-host-checks.txt. The sandbox canary the app reads stays in capture/.
# Any failing step stops the run (pipefail too: the launch below is a pipeline), so a failed build
# never installs the previous build's app.
set -eu -o pipefail
cd /tmp/craftbridge-v1
C=${CAPTURE_DIR:-capture}
mkdir -p "$C"
BID=com.craftbridge.FontProbe
for t in aarch64-apple-ios-sim aarch64-apple-ios; do
  (cd ecprobe && CARGO_TARGET_DIR=$PWD/target cargo build -q --release --target $t)
done
mkdir -p build
# The full log stays in build/; on failure its errors are shown, not just "** BUILD FAILED **".
xcodebuild -project FontProbe.xcodeproj -target FontProbe -configuration Release -sdk iphonesimulator \
  SYMROOT=/tmp/craftbridge-v1/build CODE_SIGNING_ALLOWED=NO build > build/xcodebuild-iphonesimulator.log 2>&1 || {
  grep -E 'error:' build/xcodebuild-iphonesimulator.log >&2
  echo "xcodebuild failed; full log: build/xcodebuild-iphonesimulator.log" >&2
  exit 1
}
grep -E '^\*\* BUILD' build/xcodebuild-iphonesimulator.log
printf 'sandbox canary: a Mac file outside every app container (written by the review capture)\n' > capture/sandbox-canary.txt
xcrun simctl terminate booted $BID 2>/dev/null || true
xcrun simctl uninstall booted $BID
xcrun simctl install booted build/Release-iphonesimulator/FontProbe.app
# simctl resolves --stdout/--stderr paths inside the simulated device (its /tmp is <device>/data/tmp).
DEV=$(xcrun simctl getenv booted SIMULATOR_SHARED_RESOURCES_DIRECTORY 2>/dev/null || true)
DEV=${DEV:-$HOME/Library/Developer/CoreSimulator/Devices/$(xcrun simctl list devices booted | grep -oE '[0-9A-F-]{36}' | head -1)/data}
rm -f "$DEV/tmp/fontprobe-stdout.txt" "$DEV/tmp/fontprobe-stderr.txt"
START=$(date '+%Y-%m-%d %H:%M:%S')
PID=$(xcrun simctl launch --stdout=/tmp/fontprobe-stdout.txt --stderr=/tmp/fontprobe-stderr.txt booted $BID | grep -oE '[0-9]+$')
# The app writes the READ-BACK line to stdout last, after the Documents copy is complete.
for i in {1..120}; do grep -q 'READ-BACK' "$DEV/tmp/fontprobe-stdout.txt" 2>/dev/null && break; sleep 1; done
grep -q 'READ-BACK' "$DEV/tmp/fontprobe-stdout.txt" || { echo "no complete report after 120 s" >&2; exit 1; }
CONT=$(xcrun simctl get_app_container booted $BID data)
cp "$CONT/Documents/fontprobe_report.txt" "$C/review-documents-report.txt"
cp "$DEV/tmp/fontprobe-stdout.txt" "$C/review-stdout.txt"
cp "$DEV/tmp/fontprobe-stderr.txt" "$C/review-stderr.txt"
cmp "$C/review-stdout.txt" "$C/review-documents-report.txt"
echo "stdout == Documents copy"
shasum -a 256 "$C/review-documents-report.txt"
xcrun simctl spawn booted log show --start "$START" --style compact \
  --predicate "processIdentifier == $PID AND eventMessage BEGINSWITH \"FontProbe report\"" 2>/dev/null > "$C/review-oslog.txt"
review-scripts/host-checks.sh "$C/review-documents-report.txt" > "$C/review-host-checks.txt"
echo done
