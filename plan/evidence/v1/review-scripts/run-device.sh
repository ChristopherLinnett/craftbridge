#!/bin/bash
# Runs the probe on a physical iPhone (RESULT §7): launches it with --console, waits for the
# READ-BACK line that ends the report, stops devicectl with SIGINT (which it forwards to the
# app), then copies the report out of the app's Documents. Writes into ${CAPTURE_DIR:-capture}, so a
# review round's check run leaves the canonical device captures as they are (RESULT §8).
set -u -o pipefail
DEV="$1"; cd "$(dirname "$0")/.." || exit 1
C="${CAPTURE_DIR:-capture}"; mkdir -p "$C"
OUT="$C/device-stdout.txt"
xcrun devicectl device process launch --console --terminate-existing --device "$DEV" com.craftbridge.FontProbe >"$OUT" 2>"$C/device-stderr.txt" &
pid=$!
end=$(( $(date +%s) + 600 ))
until grep -q 'READ-BACK' "$OUT" 2>/dev/null; do
  if ! kill -0 "$pid" 2>/dev/null; then echo "devicectl exited before READ-BACK"; cat "$C/device-stderr.txt"; exit 1; fi
  [ "$(date +%s)" -ge "$end" ] && { echo "no READ-BACK after 10 min"; kill -INT "$pid"; exit 1; }
  sleep 2
done
sleep 2; kill -INT "$pid" 2>/dev/null; wait "$pid" 2>/dev/null
xcrun devicectl device copy from --device "$DEV" --domain-type appDataContainer \
  --domain-identifier com.craftbridge.FontProbe --source Documents/fontprobe_report.txt \
  --destination "$C/device-documents-report.txt" >"$C/device-copy.txt" 2>&1 || { cat "$C/device-copy.txt"; exit 1; }
wc -c "$OUT" "$C/device-documents-report.txt"
