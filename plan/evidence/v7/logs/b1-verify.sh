#!/bin/bash
# Independent cross-check, one line per case dir.
# pid/device SDK/target SDK come from the case's own log when case.txt lacks them
# (the build-side harness recorded less than the review-A1 harness).
E=/tmp/craftbridge-v7/evidence
printf "%-40s|%-9s|%-8s|%-5s|%-5s|%-6s|%-22s|%s\n" CASE APK_MD5 APK DEV TGT PID RESULT OWN_LINES
for d in $(ls "$E" | sort); do
  [ -f "$E/$d/case.txt" ] || continue
  c="$E/$d/case.txt"; L="$E/$d/logcat_full.txt"
  v() { grep -m1 "^$1=" "$c" 2>/dev/null | sed -E "s/^$1=//" | awk '{print $1}'; }
  apk=$(grep -m1 '^apk=' "$c" | sed -E 's/^apk=//')
  md5=$(v apk_md5); [ -z "$md5" ] && [ -f "$apk" ] && md5="$(md5 -q "$apk")(now)"
  dev=$(v device_sdk)
  scr=$(grep -m1 'SERVICE_CREATED' "$L" 2>/dev/null)
  [ -z "$dev" ] && dev=$(echo "$scr" | sed -nE 's/.*sdk=([0-9]+).*/\1/p')
  tgt=$(echo "$scr" | sed -nE 's/.*targetSdk=([0-9]+).*/\1/p')
  pid=$(v pid); [ -z "$pid" ] && pid=$(grep -m1 'ACTIVITY_STARTED_FGS_FROM_FOREGROUND' "$L" 2>/dev/null | sed -E 's/.*CraftFGS\( *([0-9]+)\).*/\1/')
  res=$(grep -m1 '^result=' "$c" | sed -E 's/^result=//' | awk '{print $1}')
  own=$(grep "CraftFGS(" "$L" 2>/dev/null | grep -E "CraftFGS\( *${pid:-NONE}\)" | grep -E 'STARTFOREGROUND|EXPORT_DONE|EXPORT_ABORTED|EXPORT_FAILED|FGS_ON_TIMEOUT|SERVICE_CREATED|START_REQUESTED' | sed -E 's/^.*CraftFGS\([0-9 ]+\): //' | tr '\n' '|')
  printf "%-40s|%-9s|%-8s|%-5s|%-5s|%-6s|%-22s|%s\n" "$d" "${md5:0:8}" "$(basename $(dirname "$apk"))" "${dev:-?}" "${tgt:-?}" "${pid:-?}" "${res:-?}" "$own"
done
